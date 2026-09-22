//! Client-side rate limiting: bounded concurrency plus a global minimum
//! interval between request starts, with 429/`Retry-After` awareness.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tokio::sync::Semaphore;

/// Guards every outgoing API request.
///
/// Guarantees:
/// * at most `max_concurrency` in flight,
/// * at least `min_interval` between request starts (process-wide),
/// * callers observe `Retry-After` externally before re-acquiring.
#[derive(Debug)]
pub struct RateLimiter {
    semaphore: Arc<Semaphore>,
    last: Mutex<Instant>,
    min_interval: Duration,
}

impl RateLimiter {
    pub fn new(max_concurrency: usize, min_interval: Duration) -> RateLimiter {
        let max_concurrency = max_concurrency.clamp(1, 32);
        RateLimiter {
            semaphore: Arc::new(Semaphore::new(max_concurrency)),
            last: Mutex::new(Instant::now() - min_interval),
            min_interval,
        }
    }

    /// Acquire a permit and honor the global spacing rule.
    ///
    /// The caller must hold the returned permit for the duration of the
    /// request so concurrency stays bounded end-to-end.
    pub async fn acquire(&self) -> tokio::sync::OwnedSemaphorePermit {
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore is never closed");

        {
            let mut last = self.last.lock().await;
            let now = Instant::now();
            let since = now.duration_since(*last);
            if since < self.min_interval {
                let wait = self.min_interval - since;
                // Release the lock while sleeping: use a guard-scoped sleep
                // on a copy of the timestamp we write after waking.
                drop(last);
                tokio::time::sleep(wait).await;
                let mut last = self.last.lock().await;
                let after = Instant::now();
                if after.duration_since(*last) < self.min_interval {
                    let extra = self.min_interval - after.duration_since(*last);
                    drop(last);
                    tokio::time::sleep(extra).await;
                    let mut last = self.last.lock().await;
                    *last = Instant::now();
                } else {
                    *last = after;
                }
            } else {
                *last = now;
            }
        }

        permit
    }
}

/// Exponential backoff with jitter, capped at 8 seconds.
pub fn backoff(attempt: u32) -> Duration {
    let base = 500u64.saturating_mul(1u64 << attempt.min(6));
    let capped = base.min(8_000);
    let jitter = nanos() % 250;
    Duration::from_millis(capped + jitter)
}

fn nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0)
}

/// Parse an HTTP `Retry-After` header (delta-seconds or HTTP-date).
pub fn parse_retry_after(value: &str) -> Option<Duration> {
    let value = value.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs.min(3600)));
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(value) {
        let delta = dt.with_timezone(&chrono::Utc) - chrono::Utc::now();
        let secs = delta.num_seconds().max(0) as u64;
        return Some(Duration::from_secs(secs.min(3600)));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let b0 = backoff(0);
        let b3 = backoff(3);
        assert!(b0 >= Duration::from_millis(500));
        assert!(b3 > b0);
        assert!(backoff(20) <= Duration::from_millis(8_250));
    }

    #[test]
    fn retry_after_seconds() {
        assert_eq!(parse_retry_after("5"), Some(Duration::from_secs(5)));
        assert_eq!(parse_retry_after(" 12 "), Some(Duration::from_secs(12)));
        assert!(parse_retry_after("0").is_some());
        assert_eq!(parse_retry_after("999999"), Some(Duration::from_secs(3600)));
    }

    #[test]
    fn retry_after_http_date() {
        let now = chrono::Utc::now();
        let future = (now + chrono::Duration::seconds(30)).to_rfc2822();
        let d = parse_retry_after(&future).expect("should parse date");
        assert!(d <= Duration::from_secs(30));
    }

    #[test]
    fn retry_after_garbage_is_none() {
        assert_eq!(parse_retry_after("soon"), None);
        assert_eq!(parse_retry_after(""), None);
    }

    #[tokio::test]
    async fn limiter_enforces_interval() {
        let limiter = RateLimiter::new(4, Duration::from_millis(100));
        let start = Instant::now();
        let _p1 = limiter.acquire().await;
        let _p2 = limiter.acquire().await;
        assert!(start.elapsed() >= Duration::from_millis(90));
    }
}
