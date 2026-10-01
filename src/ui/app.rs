//! Application state machine and event loop.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::widgets::ListState;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;

use super::keymap::Keymap;
use super::render;
use super::theme::Theme;
use crate::api::WikimediaClient;
use crate::cache::Cache;
use crate::config::Settings;
use crate::download::{BatchContext, BatchEvent, DownloadStats};
use crate::error::{Error, Result};
use crate::metadata;
use crate::models::{Asset, SearchPage};
use crate::search;

/// Minimum usable terminal size.
pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    SearchInput,
    Searching,
    Results,
    Details,
    Downloading,
}

/// Messages posted by background jobs.
pub enum AppEvent {
    SearchDone {
        query: String,
        offset: u64,
        result: Result<(SearchPage, bool)>,
    },
    BatchTick {
        stats: DownloadStats,
    },
    BatchDone {
        stats: DownloadStats,
        dest: PathBuf,
        result: Result<()>,
    },
    ZipTick {
        phase: String,
        done: usize,
        total: usize,
        name: Option<String>,
    },
    ZipDone {
        result: Result<(PathBuf, usize, u64)>,
    },
    PreviewReady {
        url: String,
        fallback: Option<String>,
        lines: Vec<ratatui::text::Line<'static>>,
    },
    PreviewFailed {
        url: String,
    },
}

#[derive(Debug, Clone)]
pub struct BatchUi {
    pub dest: PathBuf,
    pub stats: DownloadStats,
    pub finished: bool,
}

#[derive(Debug)]
pub struct ZipUi {
    pub phase: String,
    pub done: usize,
    pub total: usize,
    pub name: Option<String>,
    pub result: Option<Result<(PathBuf, usize, u64)>>,
    pub running: bool,
}

pub struct ErrorState {
    pub title: String,
    pub message: String,
}

pub struct App {
    pub settings: Settings,
    pub provider: WikimediaClient,
    pub cache: Cache,
    pub theme: Theme,
    pub keymap: Keymap,

    tx: UnboundedSender<AppEvent>,
    rx: UnboundedReceiver<AppEvent>,

    pub screen: Screen,
    pub should_quit: bool,
    pub cancel: CancellationToken,
    pub frame: u64,

    // Search input
    pub input: String,
    pub search_query: String,

    // Results
    pub assets: Vec<Asset>,
    pub total_hits: Option<u64>,
    pub list_state: ListState,
    pub selected: HashSet<usize>,
    pub detail_index: usize,
    pub next_offset: u64,
    pub loading_more: bool,
    pub from_cache: bool,
    pub searching: bool,

    // Background jobs
    pub batch: Option<BatchUi>,
    pub zip: Option<ZipUi>,

    // Overlays / status
    pub zip_confirm: bool,
    pub error: Option<ErrorState>,
    pub status: String,

    pub preview_cache: std::collections::HashMap<String, Vec<ratatui::text::Line<'static>>>,
    pub preview_loading: std::collections::HashSet<String>,

    // pending search bookkeeping
    pending_search_query: String,
    pending_refresh: bool,
}

impl App {
    pub fn new(settings: Settings) -> Result<App> {
        let provider = WikimediaClient::new(&settings)?;
        let cache = Cache::new(&settings);
        let theme = Theme::load(&settings);
        let (tx, rx) = unbounded_channel();

        let mut list_state = ListState::default();
        list_state.select(Some(0));

        Ok(App {
            provider,
            cache,
            theme,
            keymap: Keymap::default(),
            tx,
            rx,
            screen: Screen::SearchInput,
            should_quit: false,
            cancel: CancellationToken::new(),
            frame: 0,
            input: String::new(),
            search_query: String::new(),
            assets: Vec::new(),
            total_hits: None,
            list_state,
            selected: HashSet::new(),
            detail_index: 0,
            next_offset: 0,
            loading_more: false,
            from_cache: false,
            searching: false,
            batch: None,
            zip: None,
            zip_confirm: false,
            error: None,
            status: format!(
                "{} v{} — {}",
                crate::APP_NAME,
                crate::VERSION,
                crate::TAGLINE
            ),
            preview_cache: std::collections::HashMap::new(),
            preview_loading: std::collections::HashSet::new(),
            pending_search_query: String::new(),
            pending_refresh: false,
            settings,
        })
    }

    // ------------------------------------------------------------------
    // Event loop
    // ------------------------------------------------------------------

    pub fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<i32> {
        terminal.clear()?;
        loop {
            while let Ok(event) = self.rx.try_recv() {
                self.on_app_event(event);
            }

            terminal.draw(|f| render::draw(f, self))?;
            self.frame = self.frame.wrapping_add(1);

            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(key)
                        if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                    {
                        self.on_key(key);
                    }
                    Event::Resize(_, _) => { /* next draw picks it up */ }
                    _ => {}
                }
            }

            if self.should_quit {
                self.cancel.cancel();
                return Ok(0);
            }
        }
    }

    // ------------------------------------------------------------------
    // Keyboard
    // ------------------------------------------------------------------

    fn on_key(&mut self, key: KeyEvent) {
        if Keymap::is_force_quit(&key) {
            self.should_quit = true;
            return;
        }
        // Error overlays capture input first.
        if self.error.is_some() {
            self.on_key_error(key);
            return;
        }

        match self.screen {
            Screen::SearchInput => self.on_key_search_input(key),
            Screen::Searching => {
                if key.code == KeyCode::Esc {
                    self.cancel.cancel();
                    self.searching = false;
                    self.screen = if self.assets.is_empty() {
                        Screen::SearchInput
                    } else {
                        Screen::Results
                    };
                }
            }
            Screen::Results => self.on_key_results(key),
            Screen::Details => self.on_key_details(key),
            Screen::Downloading => self.on_key_downloading(key),
        }
    }

    fn on_key_search_input(&mut self, key: KeyEvent) {
        if let KeyCode::Esc = key.code {
            self.input.clear();
            self.status = "Type a keyword (e.g. Amazon) and press Enter.".into();
            return;
        }
        match key.code {
            KeyCode::Enter => {
                let query = self.input.trim().to_string();
                if query.is_empty() {
                    return;
                }
                self.start_search(query, false);
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => {
                self.input.push(c);
            }
            _ => {}
        }
    }

    fn on_key_results(&mut self, key: KeyEvent) {
        if self.keymap.matches(&key, self.keymap.quit) {
            self.should_quit = true;
            return;
        }

        // Handle ZIP download confirmation prompt
        if self.zip_confirm {
            match key.code {
                KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                    self.zip_confirm = false;
                    self.start_zip_all();
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                    self.zip_confirm = false;
                }
                _ => {}
            }
            return;
        }

        let count = self.assets.len();
        if matches!(key.code, KeyCode::Char('z' | 'Z')) {
            if !self.assets.is_empty() {
                self.zip_confirm = true;
            } else {
                self.status = "Nothing to archive.".into();
            }
            return;
        }
        match key.code {
            KeyCode::Esc => {
                self.screen = Screen::SearchInput;
                self.input = self.search_query.clone();
                self.status = "New search — type a keyword and press Enter.".into();
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_results_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_results_selection(1),
            KeyCode::Char(' ' | 'x') => {
                if let Some(i) = self.list_state.selected() {
                    if i < count {
                        toggle(&mut self.selected, i);
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some(i) = self.list_state.selected() {
                    if i < count {
                        if let Some(asset) = self.assets.get(i).cloned() {
                            self.spawn_batch_download(vec![asset], self.settings.download_dir.clone());
                        }
                    }
                }
            }
            KeyCode::Enter => {
                let sel = self.list_state.selected().unwrap_or(0);
                if sel < count {
                    self.detail_index = sel;
                    self.screen = Screen::Details;
                    self.trigger_preview_load();
                } else {
                    let action = sel.saturating_sub(count);
                    if self.selected.is_empty() {
                        if action >= 1 {
                            self.zip_confirm = true;
                        }
                    } else {
                        match action {
                            1 => {
                                let chosen = selected_assets(&self.assets, &self.selected);
                                if !chosen.is_empty() {
                                    self.spawn_batch_download(chosen, self.settings.download_dir.clone());
                                }
                            }
                            2 => self.zip_confirm = true,
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn on_key_details(&mut self, key: KeyEvent) {
        if self.keymap.matches(&key, self.keymap.quit) {
            self.should_quit = true;
            return;
        }
        // 'n', 'N', Esc, 'b' -> decline download / go back to Results
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('b' | 'n' | 'N')) {
            self.screen = Screen::Results;
            self.list_state.select(Some(self.detail_index));
            return;
        }
        // 'y', 'Y', Enter, 'd' -> confirm and download this single file
        if matches!(key.code, KeyCode::Enter | KeyCode::Char('y' | 'Y' | 'd')) {
            if let Some(asset) = self.assets.get(self.detail_index).cloned() {
                self.spawn_batch_download(vec![asset], self.settings.download_dir.clone());
            }
            return;
        }
        if matches!(key.code, KeyCode::Char('z' | 'Z')) {
            if let Some(asset) = self.assets.get(self.detail_index).cloned() {
                let name = asset.original_name.clone();
                self.spawn_zip(vec![asset], name);
            }
            return;
        }
        match key.code {
            KeyCode::Up | KeyCode::Left | KeyCode::Char('k' | 'h') => {
                if self.detail_index > 0 {
                    self.detail_index -= 1;
                    self.trigger_preview_load();
                }
            }
            KeyCode::Down | KeyCode::Right | KeyCode::Char('j' | 'l') => {
                if self.detail_index + 1 < self.assets.len() {
                    self.detail_index += 1;
                    self.trigger_preview_load();
                }
            }
            _ => {}
        }
    }



    fn on_key_downloading(&mut self, key: KeyEvent) {
        if self.keymap.matches(&key, self.keymap.quit) {
            self.should_quit = true;
            return;
        }
        if key.code == KeyCode::Esc || key.code == KeyCode::Char('b') {
            self.screen = if self.assets.is_empty() {
                Screen::SearchInput
            } else {
                Screen::Results
            };
            self.list_state.select(Some(
                self.detail_index
                    .min(self.assets.len().saturating_sub(1)),
            ));
        }
    }

    fn on_key_error(&mut self, key: KeyEvent) {
        if self.keymap.matches(&key, self.keymap.quit)
            || self.keymap.matches(&key, self.keymap.back)
        {
            self.error = None;
            if self.assets.is_empty() {
                self.screen = Screen::SearchInput;
            } else {
                self.screen = Screen::Results;
                self.list_state.select(Some(0));
            }
            return;
        }
        if let KeyCode::Char('r') = key.code {
            let q = self.pending_search_query.clone();
            self.error = None;
            if !q.is_empty() {
                self.start_search(q, self.pending_refresh);
            }
        }
    }

    /// Move the results cursor. Rows below the assets select the
    /// download actions; loading more only happens on asset rows.
    fn move_results_selection(&mut self, delta: i32) {
        let action_rows = if self.selected.is_empty() { 2 } else { 3 };
        let total = self.assets.len() + action_rows;
        if total == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0) as i32;
        let mut next = (current + delta).clamp(0, total as i32 - 1);
        if next as usize == self.assets.len() && delta > 0 && next + 1 < total as i32 {
            next += 1;
        } else if next as usize == self.assets.len() && delta < 0 && next > 0 {
            next -= 1;
        }
        self.list_state.select(Some(next as usize));
        if (next as usize) < self.assets.len() {
            self.maybe_load_more();
            self.prefetch_preview_for(next as usize);
        }
    }

    // ------------------------------------------------------------------
    // Actions
    // ------------------------------------------------------------------

    /// Bundle every result into one ZIP in the configured download folder.
    fn start_zip_all(&mut self) {
        if self.assets.is_empty() {
            self.status = "Nothing to archive.".into();
            return;
        }
        self.spawn_zip(self.assets.clone(), self.search_query.clone());
    }

    // ------------------------------------------------------------------
    // Background jobs
    // ------------------------------------------------------------------

    fn start_search(&mut self, query: String, fresh: bool) {
        self.search_query = query.clone();
        self.pending_search_query = query.clone();
        self.pending_refresh = fresh;
        self.searching = true;
        self.screen = Screen::Searching;
        self.error = None;
        self.selected.clear();

        let provider = self.provider.clone();
        let cache = self.cache.clone();
        let tx = self.tx.clone();
        let cancel = self.cancel.child_token();

        tokio::spawn(async move {
            let result = if fresh {
                search::fetch_page_fresh(&provider, &cache, &query, 0, 50)
                    .await
                    .map(|p| (p.page, p.from_cache))
            } else {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => Err(Error::Cancelled),
                    res = search::fetch_page(&provider, &cache, &query, 0, 50) => res.map(|p| (p.page, p.from_cache)),
                }
            };
            let _ = tx.send(AppEvent::SearchDone {
                query,
                offset: 0,
                result,
            });
        });
    }

    fn maybe_load_more(&mut self) {
        if self.loading_more || self.search_query.is_empty() {
            return;
        }
        let total = match self.total_hits {
            Some(t) => t,
            None => return,
        };
        if (self.assets.len() as u64) >= total {
            return;
        }
        let selected = self.list_state.selected().unwrap_or(0);
        if selected + 5 < self.assets.len() {
            return; // only fetch when the cursor nears the end
        }

        self.loading_more = true;
        let provider = self.provider.clone();
        let cache = self.cache.clone();
        let tx = self.tx.clone();
        let query = self.search_query.clone();
        let offset = self.next_offset;
        let cancel = self.cancel.child_token();

        tokio::spawn(async move {
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(Error::Cancelled),
                res = search::fetch_page(&provider, &cache, &query, offset, 50) => res.map(|p| (p.page, p.from_cache)),
            };
            let _ = tx.send(AppEvent::SearchDone {
                query,
                offset,
                result,
            });
        });
    }

    fn spawn_batch_download(&mut self, assets: Vec<Asset>, dest: PathBuf) {
        let count = assets.len();
        self.batch = Some(BatchUi {
            dest: dest.clone(),
            stats: DownloadStats {
                total: count,
                ..DownloadStats::default()
            },
            finished: false,
        });
        self.screen = Screen::Downloading;
        self.status = format!("Downloading {count} file(s)…");

        let client = self.provider.client();
        let concurrency = self.settings.max_concurrency;
        let query = self.search_query.clone();
        let tx = self.tx.clone();
        let cancel = self.cancel.child_token();

        tokio::spawn(async move {
            let _ = std::fs::create_dir_all(&dest);
            let (etx, mut erx) = tokio::sync::mpsc::unbounded_channel::<BatchEvent>();
            let ctx = std::sync::Arc::new(BatchContext {
                client,
                dest: dest.clone(),
                overwrite: false,
                concurrency,
                cancel: cancel.clone(),
                events: Some(etx),
            });

            let for_meta = assets.clone();
            let download = crate::download::download_many(ctx, assets);
            let forward = async {
                while let Some(ev) = erx.recv().await {
                    if let BatchEvent::Batch { stats } = ev {
                        let _ = tx.send(AppEvent::BatchTick { stats });
                    }
                }
            };

            let (stats, _) = tokio::join!(download, forward);
            let (stats, result) = match stats {
                Ok(stats) => {
                    let result = metadata::write_metadata_dir(&dest, &query, &for_meta).map(|_| ());
                    (stats, result)
                }
                Err(e) => (DownloadStats::default(), Err(e)),
            };
            let _ = tx.send(AppEvent::BatchDone {
                stats,
                dest,
                result,
            });
        });
    }

    fn spawn_zip(&mut self, assets: Vec<Asset>, query: String) {
        let count = assets.len();
        self.zip = Some(ZipUi {
            phase: "Collecting SVGs…".into(),
            done: 0,
            total: count,
            name: None,
            result: None,
            running: true,
        });
        self.screen = Screen::Downloading;
        self.status = format!("Creating ZIP of {count} file(s)…");

        let client = self.provider.client();
        let tx = self.tx.clone();
        let cancel = self.cancel.child_token();
        let dest_dir = self.settings.download_dir.clone();
        let stem = zip_stem(&query);

        tokio::spawn(async move {
            let _ = std::fs::create_dir_all(&dest_dir);
            let zip_path = dest_dir.join(format!("{stem}.zip"));

            let cb = {
                let tx = tx.clone();
                move |ev: crate::archive::ZipEvent| {
                    let phase = match ev.phase {
                        crate::archive::ZipPhase::Downloading => "Downloading SVGs".to_string(),
                        crate::archive::ZipPhase::Packing => "Creating ZIP".to_string(),
                        crate::archive::ZipPhase::Finalizing => "Finalizing".to_string(),
                    };
                    let _ = tx.send(AppEvent::ZipTick {
                        phase,
                        done: ev.done,
                        total: ev.total,
                        name: ev.current_name,
                    });
                }
            };

            let result = crate::archive::create_zip(
                &client,
                &assets,
                &zip_path,
                &stem,
                &query,
                4,
                &cancel,
                Some(Box::new(cb)),
            )
            .await;

            let out = match result {
                Ok(report) => Ok((report.path, report.files, report.bytes)),
                Err(e) => Err(e),
            };
            let _ = tx.send(AppEvent::ZipDone { result: out });
        });
    }

    pub fn prefetch_preview_for(&mut self, index: usize) {
        let Some(asset) = self.assets.get(index) else {
            return;
        };
        let thumb_url = asset.thumb_url.clone();
        let direct_url = asset.url.clone();
        let Some(url) = thumb_url.as_ref().or(direct_url.as_ref()).cloned() else {
            return;
        };
        if self.preview_cache.contains_key(&url) || self.preview_loading.contains(&url) {
            return;
        }

        self.preview_loading.insert(url.clone());
        if let Some(fb) = &direct_url {
            self.preview_loading.insert(fb.clone());
        }

        let tx = self.tx.clone();
        let client = self.provider.client();
        let user_agent = self.settings.user_agent();
        let cancel = self.cancel.child_token();
        let fallback = direct_url.clone();

        tokio::spawn(async move {
            let fetch = |target: String| {
                let client = client.clone();
                let user_agent = user_agent.clone();
                async move {
                    client
                        .get(&target)
                        .header(reqwest::header::USER_AGENT, user_agent)
                        .header(reqwest::header::REFERER, "https://commons.wikimedia.org/")
                        .header(
                            reqwest::header::ACCEPT,
                            "image/svg+xml,image/png,image/*;q=0.8",
                        )
                        .send()
                        .await
                }
            };

            let mut fetched_bytes: Option<Vec<u8>> = None;

            let res = tokio::select! {
                biased;
                _ = cancel.cancelled() => None,
                resp = fetch(url.clone()) => match resp {
                    Ok(r) if r.status().is_success() => r.bytes().await.ok().map(|b| b.to_vec()),
                    _ => None,
                }
            };

            if let Some(b) = res {
                fetched_bytes = Some(b);
            } else if let Some(fb_url) = fallback.as_ref().filter(|u| *u != &url) {
                let res2 = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => None,
                    resp = fetch(fb_url.clone()) => match resp {
                        Ok(r) if r.status().is_success() => r.bytes().await.ok().map(|b| b.to_vec()),
                        _ => None,
                    }
                };
                if let Some(b) = res2 {
                    fetched_bytes = Some(b);
                }
            }

            if let Some(bytes) = fetched_bytes {
                if let Some(lines) = crate::ui::preview::render_halfblocks(&bytes, 36, 13) {
                    let _ = tx.send(AppEvent::PreviewReady {
                        url: url.clone(),
                        fallback,
                        lines,
                    });
                    return;
                }
            }
            let _ = tx.send(AppEvent::PreviewFailed { url });
        });
    }

    pub fn trigger_preview_load(&mut self) {
        self.prefetch_preview_for(self.detail_index);
    }

    // ------------------------------------------------------------------
    // App events
    // ------------------------------------------------------------------

    fn on_app_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::SearchDone {
                query,
                offset,
                result,
            } => {
                self.searching = false;
                match result {
                    Ok((page, from_cache)) => {
                        if offset == 0 {
                            self.assets = page.assets;
                            self.selected.clear();
                            self.list_state.select(Some(0));
                            self.from_cache = from_cache;
                            self.total_hits = page.total_hits;
                            self.next_offset = page.offset + page.per_page;
                            self.screen = Screen::Results;
                            if !self.assets.is_empty() {
                                self.prefetch_preview_for(0);
                            }
                            self.status = if from_cache {
                                format!("Showing cached results for \"{query}\"")
                            } else {
                                match self.total_hits {
                                    Some(t) => format!(
                                        "{} of {t} result(s) for \"{query}\" — use ↓ to reach the download options",
                                        self.assets.len()
                                    ),
                                    None => {
                                        format!("{} result(s) for \"{query}\"", self.assets.len())
                                    }
                                }
                            };
                        } else {
                            self.assets.extend(page.assets);
                            self.next_offset = offset + page.per_page;
                            self.loading_more = false;
                            self.status = format!("Loaded {} results", self.assets.len());
                        }
                        self.loading_more = false;
                    }
                    Err(Error::Cancelled) => {
                        self.loading_more = false;
                        self.status = "Search cancelled".into();
                    }
                    Err(e) => {
                        self.loading_more = false;
                        // Offline fallback: cached results still render.
                        let cached = self.cache.get::<SearchPage>(
                            crate::cache::NS_SEARCH,
                            &format!("{}@0", query.trim().to_lowercase()),
                        );
                        if let Some(page) = cached {
                            self.assets = page.assets;
                            self.total_hits = page.total_hits;
                            self.list_state.select(Some(0));
                            self.screen = Screen::Results;
                            self.from_cache = true;
                            self.status = "Wikimedia unreachable — showing cached results".into();
                        }
                        self.error = Some(ErrorState {
                            title: "Search failed".into(),
                            message: e.friendly(),
                        });
                    }
                }
            }
            AppEvent::BatchTick { stats } => {
                if let Some(batch) = &mut self.batch {
                    batch.stats = stats;
                }
            }
            AppEvent::BatchDone {
                stats,
                dest,
                result,
            } => {
                if let Some(batch) = &mut self.batch {
                    batch.stats = stats.clone();
                    batch.finished = true;
                }
                match result {
                    Ok(()) => {
                        self.status = format!(
                            "Download complete — completed {}, failed {}, skipped {} → {}",
                            stats.completed,
                            stats.failed,
                            stats.skipped,
                            dest.display()
                        );
                    }
                    Err(e) => {
                        self.status = format!("Download failed: {e}");
                        self.error = Some(ErrorState {
                            title: "Download failed".into(),
                            message: e.friendly(),
                        });
                    }
                }
            }
            AppEvent::ZipTick {
                phase,
                done,
                total,
                name,
            } => {
                if let Some(zip) = &mut self.zip {
                    zip.phase = phase;
                    zip.done = done;
                    zip.total = total;
                    zip.name = name;
                }
            }
            AppEvent::ZipDone { result } => {
                match &result {
                    Ok((path, files, bytes)) => {
                        self.status = format!(
                            "ZIP created — {files} SVG(s), {} → {}",
                            crate::models::format_size(*bytes),
                            path.display()
                        );
                    }
                    Err(e) => {
                        self.status = format!("ZIP failed: {e}");
                        if !matches!(e, Error::Cancelled) {
                            self.error = Some(ErrorState {
                                title: "ZIP creation failed".into(),
                                message: e.friendly(),
                            });
                        }
                    }
                }
                if let Some(zip) = &mut self.zip {
                    zip.running = false;
                    zip.result = Some(result);
                }
            }
            AppEvent::PreviewReady { url, fallback, lines } => {
                self.preview_loading.remove(&url);
                if let Some(fb) = &fallback {
                    self.preview_loading.remove(fb);
                    self.preview_cache.insert(fb.clone(), lines.clone());
                }
                self.preview_cache.insert(url, lines);
            }
            AppEvent::PreviewFailed { url } => {
                self.preview_loading.remove(&url);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn toggle(set: &mut HashSet<usize>, index: usize) {
    if !set.remove(&index) {
        set.insert(index);
    }
}

/// Collect the selected assets in display order.
fn selected_assets(assets: &[Asset], selected: &HashSet<usize>) -> Vec<Asset> {
    let mut idx: Vec<usize> = selected.iter().copied().collect();
    idx.sort_unstable();
    idx.into_iter()
        .filter_map(|i| assets.get(i).cloned())
        .collect()
}

fn zip_stem(query: &str) -> String {
    let cleaned: String = query
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");
    if cleaned.is_empty() {
        "get-svg".to_string()
    } else {
        format!("{cleaned}-svg")
    }
}

/// Entry point used by `commands::dispatch`.
pub fn run_interactive() -> Result<i32> {
    let settings = Settings::load()?;
    let mut app = App::new(settings)?;
    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_stem_sanitizes_query() {
        let s = zip_stem("../../etc/passwd");
        assert!(!s.contains('/'));
        assert!(!s.contains(".."));
    }

    #[test]
    fn zip_stem_is_filesystem_safe() {
        let s = zip_stem("amazon logos & icons!");
        assert!(!s.contains('/'));
        assert!(!s.contains('!'));
        assert!(s.ends_with("-svg"));
        assert_eq!(zip_stem(""), "get-svg");
    }

    #[test]
    fn min_terminal_size_is_standard() {
        assert_eq!(MIN_WIDTH, 80);
        assert_eq!(MIN_HEIGHT, 24);
    }

    #[test]
    fn selected_assets_are_ordered() {
        fn asset(name: &str) -> Asset {
            Asset {
                title: format!("File:{name}"),
                page_id: 1,
                original_name: name.to_string(),
                file_name: name.to_string(),
                ..Asset::default()
            }
        }
        let assets = vec![asset("a"), asset("b"), asset("c")];
        let mut selected = HashSet::new();
        selected.insert(2);
        selected.insert(0);
        let chosen = selected_assets(&assets, &selected);
        let names: Vec<_> = chosen.iter().map(|a| a.original_name.as_str()).collect();
        assert_eq!(names, vec!["a", "c"]);
    }

    #[test]
    fn toggle_adds_and_removes() {
        let mut set = HashSet::new();
        toggle(&mut set, 3);
        assert!(set.contains(&3));
        toggle(&mut set, 3);
        assert!(!set.contains(&3));
    }

    #[test]
    fn details_screen_navigation() {
        let settings = Settings::default();
        let mut app = App::new(settings).expect("create app");
        fn asset(name: &str) -> Asset {
            Asset {
                title: format!("File:{name}"),
                page_id: 1,
                original_name: name.to_string(),
                file_name: name.to_string(),
                ..Asset::default()
            }
        }
        app.assets = vec![asset("logo1.svg"), asset("logo2.svg")];
        app.screen = Screen::Results;
        app.list_state.select(Some(1));

        // Press Enter on item 1 -> opens Details
        app.on_key(KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Details);
        assert_eq!(app.detail_index, 1);

        // Press Up -> previous detail (index 0)
        app.on_key(KeyEvent::new(KeyCode::Up, crossterm::event::KeyModifiers::NONE));
        assert_eq!(app.detail_index, 0);

        // Press Down -> next detail (index 1)
        app.on_key(KeyEvent::new(KeyCode::Down, crossterm::event::KeyModifiers::NONE));
        assert_eq!(app.detail_index, 1);

        // Press Esc -> back to Results
        app.on_key(KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Results);
        assert_eq!(app.list_state.selected(), Some(1));

        // Re-enter Details screen and press 'n' -> back to Results
        app.on_key(KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Details);
        app.on_key(KeyEvent::new(KeyCode::Char('n'), crossterm::event::KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Results);
    }

    #[test]
    fn zip_confirm_prompt_handling() {
        let settings = Settings::default();
        let mut app = App::new(settings).expect("create app");
        fn asset(name: &str) -> Asset {
            Asset {
                title: format!("File:{name}"),
                page_id: 1,
                original_name: name.to_string(),
                file_name: name.to_string(),
                ..Asset::default()
            }
        }
        app.assets = vec![asset("logo1.svg")];
        app.screen = Screen::Results;

        // Press 'z' -> triggers zip_confirm dialog
        app.on_key(KeyEvent::new(KeyCode::Char('z'), crossterm::event::KeyModifiers::NONE));
        assert!(app.zip_confirm);

        // Press 'n' -> cancels zip_confirm dialog without starting download
        app.on_key(KeyEvent::new(KeyCode::Char('n'), crossterm::event::KeyModifiers::NONE));
        assert!(!app.zip_confirm);
        assert!(app.zip.is_none());

        // Press 'z' again then Esc -> also cancels
        app.on_key(KeyEvent::new(KeyCode::Char('z'), crossterm::event::KeyModifiers::NONE));
        assert!(app.zip_confirm);
        app.on_key(KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::NONE));
        assert!(!app.zip_confirm);
    }
}

