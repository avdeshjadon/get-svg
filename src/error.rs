use std::fmt;

/// Top-level error type for GET SVG.
///
/// Display implementations are intentionally human-readable: they are shown
/// directly to users. Technical detail lives in the variant payload and is
/// only shown in debug mode.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Network(#[source] NetworkError),

    #[error("Wikimedia Commons rejected the request (HTTP {status}).")]
    Http { status: u16, detail: String },

    #[error("Rate limited by Wikimedia Commons. Retry in {retry_after_secs}s.")]
    RateLimited { retry_after_secs: u64 },

    #[error("The API returned a malformed response: {0}")]
    Malformed(String),

    #[error("The response was too large ({got} bytes, limit {limit} bytes).")]
    ResponseTooLarge { got: usize, limit: usize },

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    #[error("Unsafe path rejected: {0}")]
    UnsafePath(String),

    #[error("Unsafe file name rejected: {0}")]
    UnsafeFilename(String),

    #[error("Download failed: {0}")]
    Download(String),

    #[error("Not enough disk space for {needed} bytes.")]
    InsufficientSpace { needed: u64 },

    #[error("Operation cancelled.")]
    Cancelled,

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to parse JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Failed to parse configuration: {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}

impl Error {
    /// A short, stable machine code for scripting and support.
    pub fn code(&self) -> &'static str {
        match self {
            Error::Network(_) => "NETWORK",
            Error::Http { .. } => "HTTP_ERROR",
            Error::RateLimited { .. } => "RATE_LIMITED",
            Error::Malformed(_) => "MALFORMED_RESPONSE",
            Error::ResponseTooLarge { .. } => "RESPONSE_TOO_LARGE",
            Error::InvalidUrl(_) => "INVALID_URL",
            Error::UnsafePath(_) => "UNSAFE_PATH",
            Error::UnsafeFilename(_) => "UNSAFE_FILENAME",
            Error::Download(_) => "DOWNLOAD_FAILED",
            Error::InsufficientSpace { .. } => "INSUFFICIENT_DISK_SPACE",
            Error::Cancelled => "CANCELLED",
            Error::Io(_) => "IO_ERROR",
            Error::Json(_) => "JSON_ERROR",
            Error::Config(_) => "CONFIG_ERROR",
            Error::Other(_) => "ERROR",
        }
    }

    /// Network problems get a friendlier, non-technical message plus a hint.
    pub fn friendly(&self) -> String {
        match self {
            Error::Network(e) => {
                let mut out = String::new();
                if e.is_timeout() {
                    out.push_str("Could not reach Wikimedia Commons in time.\n\nCheck your internet connection and try again.");
                } else if e.is_connect() {
                    out.push_str("Could not connect to Wikimedia Commons.\n\nCheck your internet connection and try again.");
                } else if e.is_redirect() {
                    out.push_str(
                        "The server redirected the request unexpectedly.\n\nTry again later.",
                    );
                } else if e.is_request() {
                    out.push_str("A network error occurred while talking to Wikimedia Commons.\n\nCheck your connection and try again.");
                } else {
                    out.push_str(
                        "A network error occurred.\n\nCheck your connection and try again.",
                    );
                }
                out.push_str(&format!("\n\nError: {}", e.code()));
                out
            }
            _ => format!("{}\n\nError: {}", self, self.code()),
        }
    }
}

/// A thin wrapper around `reqwest::Error` so we can classify network problems
/// and keep them out of the rest of the code base.
#[derive(Debug)]
pub struct NetworkError {
    kind: NetworkKind,
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkKind {
    Timeout,
    Connect,
    Redirect,
    Request,
    Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkCode {
    NetworkTimeout,
    NetworkConnect,
    NetworkRedirect,
    Network,
    NetworkBody,
}

impl NetworkError {
    fn classify(err: &reqwest::Error) -> Self {
        let kind = if err.is_timeout() {
            NetworkKind::Timeout
        } else if err.is_connect() {
            NetworkKind::Connect
        } else if err.is_redirect() {
            NetworkKind::Redirect
        } else if err.is_body() || err.is_decode() {
            NetworkKind::Body
        } else {
            NetworkKind::Request
        };
        let message = kind_label(kind);
        Self { kind, message }
    }

    pub fn is_timeout(&self) -> bool {
        self.kind == NetworkKind::Timeout
    }
    pub fn is_connect(&self) -> bool {
        self.kind == NetworkKind::Connect
    }
    pub fn is_redirect(&self) -> bool {
        self.kind == NetworkKind::Redirect
    }
    pub fn is_request(&self) -> bool {
        matches!(self.kind, NetworkKind::Request)
    }
    pub fn code(&self) -> &'static str {
        match self.kind {
            NetworkKind::Timeout => "NETWORK_TIMEOUT",
            NetworkKind::Connect => "NETWORK_CONNECT",
            NetworkKind::Redirect => "NETWORK_REDIRECT",
            NetworkKind::Request => "NETWORK_ERROR",
            NetworkKind::Body => "NETWORK_BODY",
        }
    }
}

fn kind_label(kind: NetworkKind) -> String {
    let label = match kind {
        NetworkKind::Timeout => "request timed out",
        NetworkKind::Connect => "connection failed",
        NetworkKind::Redirect => "redirect failed",
        NetworkKind::Request => "request failed",
        NetworkKind::Body => "response body was invalid",
    };
    label.to_string()
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for NetworkError {}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Error::Network(NetworkError::classify(&err))
    }
}

impl From<url::ParseError> for Error {
    fn from(err: url::ParseError) -> Self {
        Error::InvalidUrl(err.to_string())
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(err: zip::result::ZipError) -> Self {
        Error::Other(format!("zip archive error: {err}"))
    }
}

impl From<toml::ser::Error> for Error {
    fn from(err: toml::ser::Error) -> Self {
        Error::Config(err.to_string())
    }
}

impl From<toml::de::Error> for Error {
    fn from(err: toml::de::Error) -> Self {
        Error::Config(err.to_string())
    }
}

/// Convenience alias used across the crate.
pub type Result<T> = std::result::Result<T, Error>;
