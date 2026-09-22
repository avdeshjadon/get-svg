//! Application state machine and event loop.

use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::widgets::ListState;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;

use super::keymap::Keymap;
use super::render;
use super::theme::Theme;
use crate::api::{AssetProvider, WikimediaClient};
use crate::cache::Cache;
use crate::config::Settings;
use crate::download::{estimated_bytes, BatchContext, BatchEvent, DownloadStats};
use crate::error::{Error, Result};
use crate::metadata;
use crate::models::{format_size, Asset, SearchPage};
use crate::search;
use crate::security;

/// Minimum usable terminal size.
pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;

/// Home menu entries.
pub const MENU: [&str; 8] = [
    "Search",
    "Download a file",
    "Browse Categories",
    "My Downloads",
    "Recent Searches",
    "Settings",
    "Help",
    "Quit",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    SearchInput,
    DownloadInput,
    Searching,
    Results,
    Details,
    Downloads,
    Recent,
    SettingsInfo,
    Help,
}

/// Messages posted by background jobs.
pub enum AppEvent {
    SearchDone {
        query: String,
        offset: u64,
        result: Result<(SearchPage, bool)>,
    },
    SingleProgress {
        id: u64,
        done: u64,
        total: u64,
    },
    SingleDone {
        id: u64,
        name: String,
        result: Result<PathBuf>,
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
    FileResolved {
        result: Result<Box<Asset>>,
    },
}

#[derive(Debug, Clone)]
pub struct UiJob {
    pub id: u64,
    pub name: String,
    pub done: u64,
    pub total: u64,
    pub status: JobStatus,
}

#[derive(Debug, Clone)]
pub enum JobStatus {
    Active,
    Done(PathBuf),
    Failed(String),
    Skipped,
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

/// Action executed after the user confirms a destructive/bulk operation.
pub enum PendingAction {
    DownloadSelected(Vec<Asset>, PathBuf),
    DownloadAll,
    ZipFiles(Vec<Asset>, String),
}

pub struct ConfirmState {
    pub message: String,
    pub action: PendingAction,
}

pub struct ErrorState {
    pub title: String,
    pub message: String,
    pub can_retry: bool,
    pub can_cache: bool,
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

    // Home
    pub menu_index: usize,

    // Search
    pub input: String,
    pub search_query: String,

    // Results
    pub assets: Vec<Asset>,
    pub total_hits: Option<u64>,
    pub list_state: ListState,
    pub selected: HashSet<usize>,
    pub next_offset: u64,
    pub loading_more: bool,
    pub from_cache: bool,
    pub searching: bool,

    // Details
    pub detail_index: usize,
    pub show_attribution: bool,

    // Downloads
    pub jobs: Vec<UiJob>,
    pub next_job_id: u64,
    pub batch: Option<BatchUi>,
    pub zip: Option<ZipUi>,

    // Recent searches
    pub recents: Vec<String>,
    pub recent_index: usize,

    // Overlays / status
    pub confirm: Option<ConfirmState>,
    pub error: Option<ErrorState>,
    pub status: String,

    // pending search bookkeeping
    pending_search_query: String,
    pending_refresh: bool,
}

impl App {
    /// True when the typed query is more likely a file name than a search
    /// phrase: a `File:...` title, or a single token ending in `.svg`.
    fn looks_like_file(raw: &str) -> bool {
        let t = raw.trim();
        if t.is_empty() {
            return false;
        }
        let lower = t.to_lowercase();
        lower.starts_with("file:") || (lower.ends_with(".svg") && !t.contains(char::is_whitespace))
    }

    pub fn new(settings: Settings) -> Result<App> {
        let provider = WikimediaClient::new(&settings)?;
        let cache = Cache::new(&settings);
        let theme = Theme::load(&settings);
        let (tx, rx) = unbounded_channel();
        let recents = load_recents(&settings);

        let mut list_state = ListState::default();
        list_state.select(Some(0));

        Ok(App {
            provider,
            cache,
            theme,
            keymap: Keymap::default(),
            tx,
            rx,
            screen: Screen::Home,
            should_quit: false,
            cancel: CancellationToken::new(),
            frame: 0,
            menu_index: 0,
            input: String::new(),
            search_query: String::new(),
            assets: Vec::new(),
            total_hits: None,
            list_state,
            selected: HashSet::new(),
            next_offset: 0,
            loading_more: false,
            from_cache: false,
            searching: false,
            detail_index: 0,
            show_attribution: false,
            jobs: Vec::new(),
            next_job_id: 1,
            batch: None,
            zip: None,
            recents,
            recent_index: 0,
            confirm: None,
            error: None,
            status: format!(
                "{} v{} — {}",
                crate::APP_NAME,
                crate::VERSION,
                crate::TAGLINE
            ),
            pending_search_query: String::new(),
            pending_refresh: false,
            settings,
        })
    }

    // ------------------------------------------------------------------
    // Event loop
    // ------------------------------------------------------------------

    pub fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<i32> {
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
        // Confirm/error overlays capture input first.
        if self.confirm.is_some() {
            self.on_key_confirm(key);
            return;
        }
        if self.error.is_some() {
            self.on_key_error(key);
            return;
        }

        match self.screen {
            Screen::Home => self.on_key_home(key),
            Screen::SearchInput => self.on_key_search_input(key),
            Screen::DownloadInput => self.on_key_download_input(key),
            Screen::Searching => {
                if key.code == KeyCode::Esc {
                    self.cancel.cancel();
                    self.screen = if self.assets.is_empty() {
                        Screen::Home
                    } else {
                        Screen::Results
                    };
                    self.searching = false;
                }
            }
            Screen::Results => self.on_key_results(key),
            Screen::Details => self.on_key_details(key),
            Screen::Downloads => self.on_key_downloads(key),
            Screen::Recent => self.on_key_recent(key),
            Screen::SettingsInfo | Screen::Help => {
                if key.code == KeyCode::Esc
                    || self.keymap.matches(&key, self.keymap.back)
                    || self.keymap.matches(&key, self.keymap.quit)
                    || key.code == KeyCode::Enter
                {
                    self.screen = Screen::Home;
                }
            }
        }
    }

    fn on_key_home(&mut self, key: KeyEvent) {
        if self.keymap.matches(&key, self.keymap.quit) {
            self.should_quit = true;
            return;
        }
        if self.keymap.matches(&key, self.keymap.search) {
            self.screen = Screen::SearchInput;
            self.input = String::new();
            return;
        }
        if self.keymap.matches(&key, self.keymap.help) {
            self.screen = Screen::Help;
            return;
        }
        match key.code {
            KeyCode::Up => {
                self.menu_index = self.menu_index.saturating_sub(1);
            }
            KeyCode::Down if self.menu_index + 1 < MENU.len() => {
                self.menu_index += 1;
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                self.screen = Screen::DownloadInput;
                self.input = String::new();
            }
            KeyCode::Enter => self.activate_menu(),
            _ => {}
        }
    }

    fn activate_menu(&mut self) {
        match self.menu_index {
            0 => {
                self.screen = Screen::SearchInput;
                self.input = String::new();
            }
            1 => {
                self.screen = Screen::DownloadInput;
                self.input = String::new();
            }
            2 => {
                self.screen = Screen::SearchInput;
                self.input = "incategory:".to_string();
            }
            3 => self.screen = Screen::Downloads,
            4 => {
                self.recent_index = 0;
                self.screen = Screen::Recent;
            }
            5 => self.screen = Screen::SettingsInfo,
            6 => self.screen = Screen::Help,
            7 => self.should_quit = true,
            _ => {}
        }
    }

    fn on_key_download_input(&mut self, key: KeyEvent) {
        if self.keymap.matches(&key, self.keymap.back)
            || self.keymap.matches(&key, self.keymap.quit)
        {
            self.screen = Screen::Home;
            return;
        }
        match key.code {
            KeyCode::Enter => {
                let name = self.input.trim().to_string();
                if name.is_empty() {
                    return;
                }
                self.start_download_file(name);
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

    fn on_key_search_input(&mut self, key: KeyEvent) {
        if self.keymap.matches(&key, self.keymap.back) {
            self.screen = Screen::Home;
            return;
        }
        match key.code {
            KeyCode::Enter => {
                let query = self.input.trim().to_string();
                if query.is_empty() {
                    return;
                }
                if Self::looks_like_file(&query) {
                    self.start_download_file(query);
                } else {
                    self.start_search(query, false);
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => {
                // Ignore the bare `/` shortcut only when the field is empty? No:
                // typing must always win inside the input field.
                self.input.push(c);
            }
            _ => {}
        }
    }

    fn on_key_results(&mut self, key: KeyEvent) {
        let count = self.assets.len();
        if self.keymap.matches(&key, self.keymap.quit) {
            self.should_quit = true;
            return;
        }
        if self.keymap.matches(&key, self.keymap.back) {
            self.screen = Screen::Home;
            return;
        }
        if self.keymap.matches(&key, self.keymap.help) {
            self.screen = Screen::Help;
            return;
        }
        if self.keymap.matches(&key, self.keymap.search) {
            self.screen = Screen::SearchInput;
            self.input = self.search_query.clone();
            return;
        }
        if self.keymap.matches(&key, self.keymap.refresh) {
            if !self.search_query.is_empty() {
                self.start_search(self.search_query.clone(), true);
            }
            return;
        }
        if self.keymap.matches(&key, self.keymap.toggle_select) {
            if let Some(i) = self.list_state.selected() {
                if !self.selected.remove(&i) {
                    self.selected.insert(i);
                }
            }
            return;
        }
        if self.keymap.matches(&key, self.keymap.select_all) {
            self.selected = (0..self.assets.len()).collect();
            self.status = format!("Selected all {} visible results", self.assets.len());
            return;
        }
        if self.keymap.matches(&key, self.keymap.select_none) {
            self.selected.clear();
            self.status = "Selection cleared".into();
            return;
        }
        if self.keymap.matches(&key, self.keymap.download) {
            self.action_download_selection();
            return;
        }
        if self.keymap.matches(&key, self.keymap.download_all) {
            let total = self.total_hits.unwrap_or(self.assets.len() as u64);
            let msg = format!(
                "Download all matching results?\n\nQuery: {}\nAvailable: {} file(s)\n\nThis may consume significant bandwidth and disk space.",
                self.search_query, total
            );
            self.confirm = Some(ConfirmState {
                message: msg,
                action: PendingAction::DownloadAll,
            });
            return;
        }
        if self.keymap.matches(&key, self.keymap.zip) {
            self.action_zip_selection();
            return;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::PageUp => {
                let sel = self.list_state.selected().unwrap_or(0);
                let target = sel.saturating_sub(20);
                self.list_state.select(Some(target));
                self.maybe_load_more();
            }
            KeyCode::PageDown => {
                let sel = self.list_state.selected().unwrap_or(0);
                let target = (sel + 20).min(count.saturating_sub(1));
                self.list_state.select(Some(target));
                self.maybe_load_more();
            }
            KeyCode::Home => self.list_state.select(Some(0)),
            KeyCode::End if count > 0 => {
                self.list_state.select(Some(count - 1));
                self.maybe_load_more();
            }
            KeyCode::Enter => {
                if let Some(i) = self.list_state.selected() {
                    if i < self.assets.len() {
                        self.detail_index = i;
                        self.show_attribution = false;
                        self.screen = Screen::Details;
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
        if self.keymap.matches(&key, self.keymap.back) || key.code == KeyCode::Char('b') {
            self.screen = Screen::Results;
            return;
        }
        if self.keymap.matches(&key, self.keymap.help) {
            self.screen = Screen::Help;
            return;
        }
        if self.keymap.matches(&key, self.keymap.download) {
            if let Some(asset) = self.assets.get(self.detail_index).cloned() {
                self.spawn_single_download(asset);
            }
            return;
        }
        if self.keymap.matches(&key, self.keymap.zip) {
            if let Some(asset) = self.assets.get(self.detail_index).cloned() {
                let name = asset.original_name.clone();
                self.confirm = Some(ConfirmState {
                    message: format!("Create a ZIP archive containing {name}?"),
                    action: PendingAction::ZipFiles(vec![asset], name),
                });
            }
            return;
        }
        if self.keymap.matches(&key, self.keymap.copy) {
            if let Some(asset) = self.assets.get(self.detail_index) {
                let url = asset
                    .description_url
                    .clone()
                    .or_else(|| asset.url.clone())
                    .unwrap_or_default();
                if url.is_empty() {
                    self.status = "No source URL available for this file.".into();
                } else {
                    match copy_osc52(&url) {
                        Ok(()) => self.status = format!("Copied: {url}"),
                        Err(_) => self.status = format!("Copy unavailable — source: {url}"),
                    }
                }
            }
            return;
        }
        if self.keymap.matches(&key, self.keymap.open) {
            if let Some(asset) = self.assets.get(self.detail_index) {
                if let Some(url) = asset.description_url.clone().or_else(|| asset.url.clone()) {
                    match open_url(&url) {
                        Ok(()) => self.status = format!("Opening {url}"),
                        Err(e) => self.status = format!("Could not open browser: {e}"),
                    }
                }
            }
            return;
        }
        if self.keymap.matches(&key, self.keymap.attribution) {
            self.show_attribution = !self.show_attribution;
            return;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.detail_index = self.detail_index.saturating_sub(1);
                self.show_attribution = false;
            }
            KeyCode::Down | KeyCode::Char('j') if self.detail_index + 1 < self.assets.len() => {
                self.detail_index += 1;
                self.show_attribution = false;
            }
            _ => {}
        }
    }

    fn on_key_downloads(&mut self, key: KeyEvent) {
        if self.keymap.matches(&key, self.keymap.quit) {
            self.should_quit = true;
            return;
        }
        if self.keymap.matches(&key, self.keymap.back) {
            self.screen = Screen::Home;
            return;
        }
        if self.keymap.matches(&key, self.keymap.help) {
            self.screen = Screen::Help;
            return;
        }
        if key.code == KeyCode::Char('c') {
            self.jobs.retain(|j| matches!(j.status, JobStatus::Active));
            self.status = "Cleared finished downloads".into();
        }
    }

    fn on_key_recent(&mut self, key: KeyEvent) {
        if self.keymap.matches(&key, self.keymap.quit) {
            self.should_quit = true;
            return;
        }
        if self.keymap.matches(&key, self.keymap.back) {
            self.screen = Screen::Home;
            return;
        }
        if self.keymap.matches(&key, self.keymap.help) {
            self.screen = Screen::Help;
            return;
        }
        match key.code {
            KeyCode::Up => self.recent_index = self.recent_index.saturating_sub(1),
            KeyCode::Down if self.recent_index + 1 < self.recents.len() => self.recent_index += 1,
            KeyCode::Enter => {
                if let Some(q) = self.recents.get(self.recent_index).cloned() {
                    self.start_search(q, false);
                }
            }
            _ => {}
        }
    }

    fn on_key_confirm(&mut self, key: KeyEvent) {
        if self.keymap.matches(&key, self.keymap.cancel) {
            self.confirm = None;
            self.status = "Cancelled".into();
            return;
        }
        if self.keymap.matches(&key, self.keymap.confirm)
            || key.code == KeyCode::Enter
            || key.code == KeyCode::Char('Y')
        {
            let confirm = self.confirm.take();
            if let Some(c) = confirm {
                self.execute_confirmed(c.action);
            }
        } else {
            self.confirm = None;
            self.status = "Cancelled".into();
        }
    }

    fn on_key_error(&mut self, key: KeyEvent) {
        let can_retry = self.error.as_ref().map(|e| e.can_retry).unwrap_or(false);
        let can_cache = self.error.as_ref().map(|e| e.can_cache).unwrap_or(false);
        if self.keymap.matches(&key, self.keymap.quit)
            || self.keymap.matches(&key, self.keymap.back)
        {
            self.error = None;
            if self.screen == Screen::DownloadInput {
                return;
            }
            if self.assets.is_empty() {
                self.screen = Screen::Home;
            } else {
                self.screen = Screen::Results;
            }
            return;
        }
        match key.code {
            KeyCode::Char('r') if can_retry => {
                let q = self.pending_search_query.clone();
                self.error = None;
                if !q.is_empty() {
                    self.start_search(q, self.pending_refresh);
                }
            }
            KeyCode::Char('c') if can_cache => {
                self.error = None;
                // Show whatever we already have; cached pages still work offline.
                if self.assets.is_empty() {
                    self.status = "No cached results are available yet.".into();
                } else {
                    self.screen = Screen::Results;
                    self.from_cache = true;
                }
            }
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let count = self.assets.len();
        if count == 0 {
            return;
        }
        let current = self.list_state.selected().unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, count as i32 - 1);
        self.list_state.select(Some(next as usize));
        self.maybe_load_more();
    }

    // ------------------------------------------------------------------
    // Actions
    // ------------------------------------------------------------------

    fn action_download_selection(&mut self) {
        let chosen: Vec<Asset> = if self.selected.is_empty() {
            self.list_state
                .selected()
                .and_then(|i| self.assets.get(i).cloned())
                .into_iter()
                .collect()
        } else {
            let mut idx: Vec<usize> = self.selected.iter().copied().collect();
            idx.sort_unstable();
            idx.into_iter()
                .filter_map(|i| self.assets.get(i).cloned())
                .collect()
        };
        if chosen.is_empty() {
            self.status = "Nothing selected.".into();
            return;
        }
        let bytes = estimated_bytes(&chosen);
        let msg = format!(
            "Download {} file(s) ({}) to:\n{}\n\nContinue?",
            chosen.len(),
            format_size(bytes),
            self.settings.download_dir.display()
        );
        self.confirm = Some(ConfirmState {
            message: msg,
            action: PendingAction::DownloadSelected(chosen, self.settings.download_dir.clone()),
        });
    }

    fn action_zip_selection(&mut self) {
        let chosen: Vec<Asset> = if self.selected.is_empty() {
            self.assets.clone()
        } else {
            let mut idx: Vec<usize> = self.selected.iter().copied().collect();
            idx.sort_unstable();
            idx.into_iter()
                .filter_map(|i| self.assets.get(i).cloned())
                .collect()
        };
        if chosen.is_empty() {
            self.status = "Nothing to archive.".into();
            return;
        }
        let bytes = estimated_bytes(&chosen);
        let msg = format!(
            "Create a ZIP archive?\n\n{} file(s)\nEstimated download size: {}\n\nContinue?",
            chosen.len(),
            format_size(bytes)
        );
        self.confirm = Some(ConfirmState {
            message: msg,
            action: PendingAction::ZipFiles(chosen, self.search_query.clone()),
        });
    }

    fn execute_confirmed(&mut self, action: PendingAction) {
        match action {
            PendingAction::DownloadSelected(assets, dest) => {
                self.spawn_batch_download(assets, dest)
            }
            PendingAction::DownloadAll => {
                // Paginate through every matching result, downloading as we go.
                let query = self.search_query.clone();
                let all = self.assets.clone();
                self.spawn_download_all(query, all);
            }
            PendingAction::ZipFiles(assets, query) => self.spawn_zip(assets, query),
        }
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

    /// Resolve a user-supplied file name (or `File:Title`) to an asset, then
    /// download it straight into the configured download directory.
    fn start_download_file(&mut self, raw: String) {
        self.search_query = format!("file: {raw}");
        self.searching = true;
        self.screen = Screen::Searching;
        self.error = None;

        let provider = self.provider.clone();
        let cache = self.cache.clone();
        let tx = self.tx.clone();
        let cancel = self.cancel.child_token();

        tokio::spawn(async move {
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(Error::Cancelled),
                res = resolve_file(&provider, &cache, &raw) => res,
            };
            let _ = tx.send(AppEvent::FileResolved {
                result: result.map(Box::new),
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

    fn spawn_single_download(&mut self, asset: Asset) {
        let id = self.next_job_id;
        self.next_job_id += 1;
        let name = asset.original_name.clone();
        self.jobs.push(UiJob {
            id,
            name: name.clone(),
            done: 0,
            total: asset.size_bytes.unwrap_or(0),
            status: JobStatus::Active,
        });
        self.screen = Screen::Downloads;
        self.status = format!("Downloading {name}");

        let dest = self.settings.download_dir.clone();
        let client = self.provider.client();
        let tx = self.tx.clone();
        let cancel = self.cancel.child_token();
        let overwrite = false;

        tokio::spawn(async move {
            let dir = match std::fs::create_dir_all(&dest) {
                Ok(()) => dest,
                Err(e) => {
                    let _ = tx.send(AppEvent::SingleDone {
                        id,
                        name,
                        result: Err(Error::Io(e)),
                    });
                    return;
                }
            };

            let mut allocator = crate::download::NameAllocator::new();
            let Some(path) = allocator.allocate(&dir, &asset.file_name, overwrite) else {
                let _ = tx.send(AppEvent::SingleDone {
                    id,
                    name: asset.original_name.clone(),
                    result: Err(Error::Other(format!(
                        "{} already exists (use --overwrite to replace)",
                        asset.file_name
                    ))),
                });
                return;
            };

            let Some(url) = asset.url.clone() else {
                let _ = tx.send(AppEvent::SingleDone {
                    id,
                    name: asset.original_name.clone(),
                    result: Err(Error::Download("no download URL available".into())),
                });
                return;
            };

            let tx2 = tx.clone();
            let cb = move |done: u64, total: u64| {
                let _ = tx2.send(AppEvent::SingleProgress { id, done, total });
            };

            let result = crate::download::file::download_one(
                &client,
                &url,
                &path,
                &cancel,
                Some(Box::new(cb)),
            )
            .await;

            let outcome = match result {
                Ok(crate::download::DownloadOutcome::Written { path, .. }) => Ok(path),
                Ok(crate::download::DownloadOutcome::Skipped) => {
                    Err(Error::Other(format!("{} already exists", asset.file_name)))
                }
                Err(e) => Err(e),
            };

            if outcome.is_ok() {
                let _ =
                    metadata::write_metadata_dir(&dir, &asset.title, std::slice::from_ref(&asset));
            }

            let _ = tx.send(AppEvent::SingleDone {
                id,
                name: asset.original_name.clone(),
                result: outcome,
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
        self.screen = Screen::Downloads;
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

    fn spawn_download_all(&mut self, query: String, known: Vec<Asset>) {
        self.batch = Some(BatchUi {
            dest: self.settings.download_dir.clone(),
            stats: DownloadStats {
                total: known.len(),
                ..DownloadStats::default()
            },
            finished: false,
        });
        self.screen = Screen::Downloads;
        self.status = "Downloading all matching results…".into();

        let provider = self.provider.clone();
        let cache = self.cache.clone();
        let dest = self.settings.download_dir.clone();
        let concurrency = self.settings.max_concurrency;
        let client = self.provider.client();
        let tx = self.tx.clone();
        let cancel = self.cancel.child_token();

        tokio::spawn(async move {
            let _ = std::fs::create_dir_all(&dest);
            let mut collected = known;
            let mut offset = collected.len() as u64;

            // Paginate until exhausted or cancelled.
            loop {
                if cancel.is_cancelled() {
                    break;
                }
                let page = match search::fetch_page(&provider, &cache, &query, offset, 50).await {
                    Ok(p) => p.page,
                    Err(e) => {
                        let _ = tx.send(AppEvent::BatchDone {
                            stats: DownloadStats::default(),
                            dest,
                            result: Err(e),
                        });
                        return;
                    }
                };
                if page.assets.is_empty() {
                    break;
                }
                let hits = page.total_hits;
                let n = page.assets.len() as u64;
                collected.extend(page.assets);
                offset += n;
                if let Some(h) = hits {
                    if offset >= h {
                        break;
                    }
                }
                if n < 25 {
                    break;
                }
            }

            let (etx, mut erx) = tokio::sync::mpsc::unbounded_channel::<BatchEvent>();
            let ctx = std::sync::Arc::new(BatchContext {
                client,
                dest: dest.clone(),
                overwrite: false,
                concurrency,
                cancel: cancel.clone(),
                events: Some(etx),
            });
            let for_meta = collected.clone();
            let download = crate::download::download_many(ctx, collected);
            let forward = async {
                while let Some(ev) = erx.recv().await {
                    if let BatchEvent::Batch { stats } = ev {
                        let _ = tx.send(AppEvent::BatchTick { stats });
                    }
                }
            };
            let (stats, _) = tokio::join!(download, forward);
            match stats {
                Ok(stats) => {
                    let _ = metadata::write_metadata_dir(&dest, &query, &for_meta);
                    let _ = tx.send(AppEvent::BatchDone {
                        stats,
                        dest,
                        result: Ok(()),
                    });
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::BatchDone {
                        stats: DownloadStats::default(),
                        dest,
                        result: Err(e),
                    });
                }
            }
        });
    }

    fn spawn_zip(&mut self, assets: Vec<Asset>, query: String) {
        self.zip = Some(ZipUi {
            phase: "Preparing".into(),
            done: 0,
            total: assets.len(),
            name: None,
            result: None,
            running: true,
        });
        self.screen = Screen::Downloads;
        self.status = format!("Creating ZIP of {} file(s)…", assets.len());

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
                        crate::archive::ZipPhase::Downloading => "Downloading".to_string(),
                        crate::archive::ZipPhase::Packing => "Packing".to_string(),
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
                            self.push_recent(query.clone());
                            self.status = if from_cache {
                                format!("Showing cached results for \"{query}\"")
                            } else {
                                match self.total_hits {
                                    Some(t) => format!(
                                        "{} of {t} results for \"{query}\"",
                                        self.assets.len()
                                    ),
                                    None => {
                                        format!("{} results for \"{}\"", self.assets.len(), query)
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
                        let can_cache = cached.is_some() || !self.assets.is_empty();
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
                            can_retry: true,
                            can_cache,
                        });
                    }
                }
            }
            AppEvent::SingleProgress { id, done, total } => {
                if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                    job.done = done;
                    if total > 0 {
                        job.total = total;
                    }
                }
            }
            AppEvent::SingleDone { id, name, result } => {
                if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                    match result {
                        Ok(path) => {
                            job.status = JobStatus::Done(path.clone());
                            job.done = job.total;
                            self.status = format!("Saved {}", path.display());
                        }
                        Err(Error::Cancelled) => {
                            job.status = JobStatus::Skipped;
                            self.status = format!("Cancelled: {name}");
                        }
                        Err(e) => {
                            if matches!(e, Error::Other(ref m) if m.contains("already exists")) {
                                job.status = JobStatus::Skipped;
                                self.status = e.to_string();
                            } else {
                                job.status = JobStatus::Failed(e.to_string());
                                self.status = format!("Failed: {name}");
                            }
                        }
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
                            "Batch finished — completed {}, failed {}, skipped {} → {}",
                            stats.completed,
                            stats.failed,
                            stats.skipped,
                            dest.display()
                        );
                    }
                    Err(e) => {
                        self.status = format!("Batch failed: {e}");
                        self.error = Some(ErrorState {
                            title: "Download failed".into(),
                            message: e.friendly(),
                            can_retry: false,
                            can_cache: false,
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
                            "Created {} ({files} files, {})",
                            path.display(),
                            format_size(*bytes)
                        );
                    }
                    Err(e) => {
                        self.status = format!("ZIP failed: {e}");
                        if !matches!(e, Error::Cancelled) {
                            self.error = Some(ErrorState {
                                title: "ZIP creation failed".into(),
                                message: e.friendly(),
                                can_retry: false,
                                can_cache: false,
                            });
                        }
                    }
                }
                if let Some(zip) = &mut self.zip {
                    zip.running = false;
                    zip.result = Some(result);
                }
            }
            AppEvent::FileResolved { result } => {
                self.searching = false;
                match result {
                    Ok(asset) => {
                        self.spawn_single_download(*asset);
                    }
                    Err(Error::Cancelled) => {
                        self.status = "Download cancelled".into();
                        self.screen = if self.assets.is_empty() {
                            Screen::Home
                        } else {
                            Screen::Results
                        };
                    }
                    Err(e) => {
                        self.screen = Screen::DownloadInput;
                        self.error = Some(ErrorState {
                            title: "File not found".into(),
                            message: e.friendly(),
                            can_retry: false,
                            can_cache: false,
                        });
                    }
                }
            }
        }
    }

    fn push_recent(&mut self, query: String) {
        self.recents.retain(|q| q != &query);
        self.recents.insert(0, query);
        self.recents.truncate(20);
        save_recents(&self.settings, &self.recents);
    }

    /// Snapshot of the home menu selection (for rendering).
    pub fn list_state_menu(&self) -> ListState {
        let mut s = ListState::default();
        s.select(Some(self.menu_index));
        s
    }

    /// Snapshot of the recent-searches selection (for rendering).
    pub fn recents_state(&self) -> ListState {
        let mut s = ListState::default();
        s.select(Some(self.recent_index));
        s
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_recents(settings: &Settings) -> Vec<String> {
    std::fs::read_to_string(settings.recent_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_recents(settings: &Settings, recents: &[String]) {
    if let Some(parent) = settings.recent_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(raw) = serde_json::to_string(recents) {
        let _ = std::fs::write(settings.recent_path(), raw);
    }
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

/// Copy text to the system clipboard via the OSC 52 terminal escape.
///
/// This requires no clipboard library and works over SSH. We write the escape
/// ourselves; the payload is base64 so no user-controlled bytes reach the
/// terminal unescaped.
fn copy_osc52(text: &str) -> Result<()> {
    if !security::validate_https_url(text) && !text.starts_with("https://") {
        return Err(Error::InvalidUrl(text.to_string()));
    }
    let b64 = base64_encode(text.as_bytes());
    let payload = format!("\x1b]52;c;{b64}\x07");
    let mut stdout = std::io::stdout();
    stdout.write_all(payload.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Open an HTTPS URL in the user's browser without involving a shell.
fn open_url(url: &str) -> Result<()> {
    if !security::validate_https_url(url) {
        return Err(Error::InvalidUrl(url.to_string()));
    }
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut cmd = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut cmd = std::process::Command::new("explorer");
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return Err(Error::Other(
        "opening URLs is not supported on this platform".into(),
    ));

    cmd.arg(url);
    #[cfg(target_os = "windows")]
    let spawn = cmd.spawn().map(|_| ());
    #[cfg(not(target_os = "windows"))]
    let spawn = cmd.spawn().map(|_| ());
    spawn.map_err(|e| Error::Other(format!("could not launch browser: {e}")))?;
    Ok(())
}

/// Resolve a user-supplied file name / `File:Title` to an `Asset`, mirroring
/// the `download` subcommand: exact lookup first, title search as a fallback
/// so `Giraffe-logo.svg` and `github-logo` both work.
async fn resolve_file(provider: &WikimediaClient, cache: &Cache, raw: &str) -> Result<Asset> {
    let title = if raw.starts_with("File:") || raw.starts_with("file:") {
        raw.to_string()
    } else if raw.ends_with(".svg") && !raw.contains('/') {
        format!("File:{raw}")
    } else {
        raw.to_string()
    };

    if let Some(asset) = provider.get_asset(&title).await? {
        return Ok(asset);
    }

    let found = search::fetch_page(provider, cache, &title, 0, 5).await?;
    found
        .page
        .assets
        .into_iter()
        .find(|a| {
            a.original_name.eq_ignore_ascii_case(&title) || a.title.eq_ignore_ascii_case(&title)
        })
        .ok_or_else(|| {
            Error::Other(format!(
                "no file named \"{title}\" was found on Wikimedia Commons"
            ))
        })
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
    fn base64_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"https://x.test"), "aHR0cHM6Ly94LnRlc3Q=");
    }

    #[test]
    fn recents_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings {
            config_dir: dir.path().to_path_buf(),
            ..Settings::default()
        };
        save_recents(&s, &["github".into(), "icons".into()]);
        let loaded = load_recents(&s);
        assert_eq!(loaded, vec!["github".to_string(), "icons".to_string()]);
    }

    #[test]
    fn zip_stem_sanitizes_query() {
        let s = zip_stem("../../etc/passwd");
        assert!(!s.contains('/'));
        assert!(!s.contains(".."));
    }

    #[test]
    fn min_terminal_size_is_standard() {
        assert_eq!(MIN_WIDTH, 80);
        assert_eq!(MIN_HEIGHT, 24);
    }

    #[test]
    fn open_url_rejects_non_https() {
        assert!(open_url("javascript:alert(1)").is_err());
        assert!(open_url("http://x.test").is_err());
    }
}
