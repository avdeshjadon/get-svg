//! All terminal drawing for the interactive UI.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap};
use ratatui::{style::Style, Frame};

use super::app::{App, BatchUi, JobStatus, Screen, MIN_HEIGHT, MIN_WIDTH};
use crate::models::{format_size, Asset};
use crate::security;

/// Block-letter logo. All six lines are exactly 58 characters wide.
const LOGO: [&str; 6] = [
    " ██████╗ ███████╗ ████████╗     ██████╗ ██╗   ██╗ ██████╗ ",
    "██╔════╝ ██╔════╝ ╚══██╔══╝    ██╔═══╝  ██║   ██║██╔════╝ ",
    "██║  ███╗█████╗      ██║        ██████╗ ██║   ██║██║  ███╗",
    "██║   ██║██╔══╝      ██║       ╚═══███╗ ╚██╗ ██╔╝██║   ██║",
    "╚██████╔╝███████╗    ██║       ███████╗  ╚████╔╝ ╚██████╔╝",
    " ╚═════╝ ╚══════╝    ╚═╝       ╚══════╝   ╚═══╝   ╚═════╝ ",
];

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Spinner glyph for the current frame (static "/" when animations are off).
fn spinner(t: &super::theme::Theme, frame: u64) -> &'static str {
    if t.animations {
        SPINNER[(frame / 6) as usize % SPINNER.len()]
    } else {
        "/"
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        draw_too_small(f, area);
        return;
    }

    let layout = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    draw_header(f, layout[0], app);
    let body = layout[1];
    match app.screen {
        Screen::Home => draw_home(f, body, app),
        Screen::SearchInput => draw_search_input(f, body, app),
        Screen::DownloadInput => draw_download_input(f, body, app),
        Screen::Searching => draw_searching(f, body, app),
        Screen::Results => draw_results(f, body, app),
        Screen::Details => draw_details(f, body, app),
        Screen::Downloads => draw_downloads(f, body, app),
        Screen::Recent => draw_recent(f, body, app),
        Screen::SettingsInfo => draw_settings(f, body, app),
        Screen::Help => draw_help(f, body),
    }
    draw_footer(f, layout[2], app);

    if let Some(confirm) = &app.confirm {
        draw_confirm(f, area, confirm, &app.theme);
    }
    if let Some(err) = &app.error {
        draw_error(f, area, err, &app.theme);
    }
}

// ---------------------------------------------------------------------------
// Chrome
// ---------------------------------------------------------------------------

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let title = match app.screen {
        Screen::Home => format!(
            "{} v{} — {}",
            crate::APP_NAME,
            crate::VERSION,
            crate::TAGLINE
        ),
        Screen::SearchInput => "Search Wikimedia Commons".to_string(),
        Screen::DownloadInput => "Download a file".to_string(),
        Screen::Searching => format!("Search: {}", app.search_query),
        Screen::Results | Screen::Details => format!("Search: {}", app.search_query),
        Screen::Downloads => "Downloads".to_string(),
        Screen::Recent => "Recent Searches".to_string(),
        Screen::SettingsInfo => "Settings".to_string(),
        Screen::Help => "Help".to_string(),
    };
    let style = app.theme.bold_accent();
    // Home already shows the big block logo, so the chrome row carries the
    // version line instead of repeating the name.
    let text = if app.screen == Screen::Home {
        title
    } else {
        format!(" {} › {title}", crate::APP_NAME)
    };

    let mut spans = vec![Span::styled(text, style)];
    if app.from_cache && !app.assets.is_empty() && matches!(app.screen, Screen::Results) {
        spans.push(Span::styled(
            "  [cached]".to_string(),
            app.theme.warn_style(),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let spans = footer_hint(app);
    let line = Line::from(limit_spans(Line::from(spans), area.width as usize));
    f.render_widget(line, area);
}

fn key_span(t: &super::theme::Theme, keys: &str, desc: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!("{keys} "), t.bold_accent()),
        Span::styled(desc.to_string(), t.dim_style()),
        Span::raw("  "),
    ]
}

fn footer_hint(app: &App) -> Vec<Span<'static>> {
    let t = &app.theme;
    if !app.status.is_empty() {
        return vec![Span::styled(app.status.clone(), t.text_style())];
    }
    let mut s: Vec<Span<'static>> = Vec::new();
    match app.screen {
        Screen::Home => {
            s.extend(key_span(t, "↑↓", "Navigate"));
            s.extend(key_span(t, "d", "Download a file"));
            s.extend(key_span(t, "Enter", "Select"));
            s.extend(key_span(t, "/", "Search"));
            s.extend(key_span(t, "?", "Help"));
            s.extend(key_span(t, "q", "Quit"));
        }
        Screen::SearchInput => {
            s.extend(key_span(t, "Enter", "Search"));
            s.extend(key_span(t, "Esc", "Cancel"));
        }
        Screen::DownloadInput => {
            s.extend(key_span(t, "Enter", "Download"));
            s.extend(key_span(t, "Esc", "Cancel"));
        }
        Screen::Searching => {
            s.extend(key_span(t, "Esc", "Cancel"));
        }
        Screen::Results => {
            s.extend(key_span(t, "↑↓", "Navigate"));
            s.extend(key_span(t, "Space", "Select"));
            s.extend(key_span(t, "d", "Download"));
            s.extend(key_span(t, "A", "Download all"));
            s.extend(key_span(t, "z", "ZIP"));
            s.extend(key_span(t, "Enter", "Details"));
            s.extend(key_span(t, "? ", "Help"));
            s.extend(key_span(t, "q", "Quit"));
        }
        Screen::Details => {
            s.extend(key_span(t, "d", "Download"));
            s.extend(key_span(t, "z", "ZIP"));
            s.extend(key_span(t, "c", "Copy URL"));
            s.extend(key_span(t, "o", "Open page"));
            s.extend(key_span(t, "a", "Attribution"));
            s.extend(key_span(t, "↑↓", "Prev/Next"));
            s.extend(key_span(t, "b", "Back"));
            s.extend(key_span(t, "q", "Quit"));
        }
        Screen::Downloads => {
            s.extend(key_span(t, "Esc", "Back"));
            s.extend(key_span(t, "c", "Clear finished"));
            s.extend(key_span(t, "?", "Help"));
            s.extend(key_span(t, "q", "Quit"));
        }
        Screen::Recent => {
            s.extend(key_span(t, "↑↓", "Navigate"));
            s.extend(key_span(t, "Enter", "Search"));
            s.extend(key_span(t, "Esc", "Back"));
        }
        Screen::SettingsInfo | Screen::Help => {
            s.extend(key_span(t, "Esc", "Back"));
        }
    }
    s
}

fn limit_spans(line: Line<'static>, max: usize) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut seen = 0usize;
    for span in line.into_iter() {
        let width = span.content.chars().count();
        if seen + width > max {
            let room = max.saturating_sub(seen);
            if room > 1 {
                let text: String = span.content.chars().take(room - 1).collect();
                out.push(Span::styled(text, span.style));
                out.push(Span::raw("…"));
            }
            break;
        }
        out.push(span);
        seen += width;
    }
    out
}

// ---------------------------------------------------------------------------
// Screens
// ---------------------------------------------------------------------------

fn draw_too_small(f: &mut Frame, area: Rect) {
    let block = Block::bordered().title("Terminal too small");
    let text = Paragraph::new(Text::from(Line::from(vec![
        Span::raw("Please resize to at least "),
        Span::styled(
            format!("{MIN_WIDTH}×{MIN_HEIGHT}"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" columns×rows to use GET SVG."),
    ])))
    .block(block)
    .alignment(Alignment::Center)
    .wrap(Wrap { trim: true });
    f.render_widget(text, area);
}

fn draw_home(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    // Split vertically: logo block / menu / tip
    let chunks = Layout::vertical([
        Constraint::Length(13),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(area);

    let logo_block = Block::bordered().border_style(t.border_style());
    let mut logo_text = Text::default();
    for line in LOGO {
        logo_text.push_line(Line::from(Span::styled(line, t.accent_style())));
    }
    logo_text.push_line(Line::default());
    logo_text.push_line(Line::default());
    logo_text.push_line(
        Line::from(Span::styled(crate::TAGLINE, t.dim_style())).alignment(Alignment::Center),
    );
    let logo_para = Paragraph::new(logo_text)
        .block(logo_block)
        .alignment(Alignment::Center);
    f.render_widget(logo_para, chunks[0]);

    // Menu
    let mut items: Vec<ListItem> = Vec::new();
    for (i, label) in super::app::MENU.iter().enumerate() {
        let marker = if i == app.menu_index { "› " } else { "  " };
        let prefix: char = match i {
            0 => 'S',
            1 => 'D',
            2 => 'C',
            3 => 'M',
            4 => 'R',
            5 => 'T',
            6 => 'H',
            _ => 'Q',
        };
        let text = Line::from(vec![
            Span::styled(
                format!("{marker} "),
                if i == app.menu_index {
                    t.bold_accent()
                } else {
                    t.dim_style()
                },
            ),
            Span::styled(
                format!("{prefix}"),
                if i == app.menu_index {
                    t.bold_accent()
                } else {
                    t.dim_style()
                },
            ),
            Span::raw("  "),
            Span::styled(
                *label,
                if i == app.menu_index {
                    t.selected_style()
                } else {
                    t.text_style()
                },
            ),
        ]);
        items.push(ListItem::new(text));
    }
    let list = List::new(items).block(
        Block::bordered()
            .title(" Menu ")
            .border_style(t.border_style()),
    );
    let mut state = app.list_state_menu();
    f.render_stateful_widget(list, chunks[2], &mut state);

    // Download tip row, kept short so it fits narrow terminals.
    let tip = Line::from(vec![
        Span::styled("Tip: ", t.bold_accent()),
        Span::styled("Search → results me ", t.dim_style()),
        Span::styled("d", t.accent_style()),
        Span::styled(" = Download, ", t.dim_style()),
        Span::styled("A", t.accent_style()),
        Span::styled(" = all, ", t.dim_style()),
        Span::styled("z", t.accent_style()),
        Span::styled(" = ZIP.", t.dim_style()),
    ])
    .alignment(Alignment::Left);
    let tip_para = Paragraph::new(tip).style(t.dim_style());
    f.render_widget(tip_para, chunks[3]);
}

fn draw_download_input(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(0),
    ])
    .split(area);

    let display = if app.input.is_empty() {
        Line::from(Span::styled(
            "Enter a file name (e.g. Flag_of_India.svg)…",
            t.dim_style(),
        ))
    } else {
        Line::from(Span::styled(app.input.clone(), t.text_style()))
    };
    let input = Paragraph::new(display).block(
        Block::bordered()
            .title(Span::styled(
                "Download a file from Wikimedia Commons",
                t.title_style(),
            ))
            .border_style(t.border_active_style()),
    );
    f.render_widget(input, chunks[0]);

    let cursor_line = chunks[0];
    let x = cursor_x(&app.input);
    f.set_cursor_position((cursor_line.x + x + 1, cursor_line.y + 1));

    let hint = Line::from(vec![
        Span::styled("Download tip: ", t.dim_style()),
        Span::styled("A name like ", t.text_style()),
        Span::styled("\"GitHub-logo.svg\"", t.accent_style()),
        Span::styled(" or a ", t.text_style()),
        Span::styled("File:Title", t.accent_style()),
        Span::styled(" works. Press Enter to download.", t.text_style()),
    ]);
    f.render_widget(hint, chunks[2]);
}

fn draw_search_input(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(0),
    ])
    .split(area);

    let title = if app.input.starts_with("incategory:") {
        "Browse a Wikimedia Commons category"
    } else {
        "Search Wikimedia Commons for SVGs"
    };

    let display = if app.input.is_empty() {
        Line::from(Span::styled("Type a query…", t.dim_style()))
    } else {
        Line::from(Span::styled(
            app.input.clone(),
            style_if_incategory(&app.input, &t.bold_accent(), &t.text_style()),
        ))
    };
    let input = Paragraph::new(display).block(
        Block::bordered()
            .title(Span::styled(title, t.title_style()))
            .border_style(t.border_active_style()),
    );
    f.render_widget(input, chunks[0]);

    // Cursor — render a block cursor at the current position.
    let cursor_line = chunks[0];
    let x = cursor_x(&app.input);
    f.set_cursor_position((cursor_line.x + x + 1, cursor_line.y + 1));

    let hint = if app.input.starts_with("incategory:") {
        Line::from(vec![
            Span::styled("Search tip: ", t.dim_style()),
            Span::styled("Type a category name, e.g. ", t.text_style()),
            Span::styled("Logos", t.accent_style()),
        ])
    } else {
        Line::from(vec![
            Span::styled("Search tip: ", t.dim_style()),
            Span::styled("Phrases work too — ", t.text_style()),
            Span::styled("\"arrow icons\"", t.accent_style()),
            Span::styled(" finds SVG files only.", t.text_style()),
        ])
    };
    f.render_widget(hint, chunks[2]);
}

fn style_if_incategory(
    input: &str,
    accent: &ratatui::style::Style,
    text: &ratatui::style::Style,
) -> ratatui::style::Style {
    if input.starts_with("incategory:") {
        *accent
    } else {
        *text
    }
}

/// Byte offset for placing the block cursor (single-line grapheme caveat
/// accepted; normal filenames/queries are fine).
fn cursor_x(input: &str) -> u16 {
    input.chars().count() as u16
}

fn draw_searching(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let spinner = spinner(t, app.frame);
    let text = Text::from(vec![
        Line::from(vec![
            Span::styled(spinner, t.accent_style()),
            Span::raw("  Searching Wikimedia Commons…"),
        ])
        .alignment(Alignment::Center),
        Line::from(Span::styled(
            format!("{spinner} \"{}\"", app.search_query),
            t.dim_style(),
        ))
        .alignment(Alignment::Center),
        Line::default(),
        Line::styled(
            "This only matches SVG files (image/svg+xml).",
            t.dim_style(),
        )
        .alignment(Alignment::Center),
    ]);
    let para = Paragraph::new(text)
        .block(
            Block::bordered()
                .title(" Search ")
                .border_style(t.border_active_style()),
        )
        .alignment(Alignment::Center);
    f.render_widget(para, area);
}

fn draw_results(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let block = Block::bordered()
        .title(Span::styled(
            format!(" Results — {}", app.assets.len()),
            t.title_style(),
        ))
        .border_style(t.border_style())
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.assets.is_empty() {
        let msg = if app.loading_more {
            Line::from(Span::styled("Loading…", t.dim_style())).alignment(Alignment::Center)
        } else {
            Line::from(Span::styled("No SVG files matched.", t.warn_style()))
                .alignment(Alignment::Center)
        };
        f.render_widget(Paragraph::new(msg), inner);
        return;
    }

    // Column budget: name gets everything left over.
    let name_width = (inner.width as usize).saturating_sub(4 + 6 + 10 + 28);
    let name_width = name_width.max(20) as u16;

    let (head_area, list_area) = {
        let c = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
        (c[0], c[1])
    };

    // Column header + pagination summary share the top row of the list.
    let total = app.total_hits.unwrap_or(app.assets.len() as u64);
    let summary = if app.loading_more {
        "loading more…".to_string()
    } else {
        format!("{}–{} of {}", 1, app.assets.len(), total)
    };
    let size_hdr = format!("{:>9}", "SIZE");
    let lic_hdr = format!("{:<26}", "LICENSE");
    let mut header_spans = vec![
        Span::styled(
            format!("{:<11}{:<nw$}", "", "NAME", nw = name_width as usize),
            t.dim_style(),
        ),
        Span::raw(" "),
        Span::styled(format!("{size_hdr} "), t.dim_style()),
        Span::styled(lic_hdr, t.dim_style()),
    ];
    let used: usize = 11 + name_width as usize + 1 + 9 + 1 + 26;
    if head_area.width as usize > used + summary.chars().count() {
        header_spans.push(Span::raw(
            " ".repeat(head_area.width as usize - used - summary.chars().count()),
        ));
        header_spans.push(Span::styled(summary, t.dim_style()));
    }
    f.render_widget(Paragraph::new(Line::from(header_spans)), head_area);

    let mut items: Vec<ListItem> = Vec::new();
    for (i, asset) in app.assets.iter().enumerate() {
        let marker = if app.selected.contains(&i) {
            "[x]"
        } else {
            "[ ]"
        };
        let index = format!("{:>4}", i + 1);
        let name = security::sanitize_text(&asset.original_name);
        let name = truncate_chars(&name, name_width as usize);
        let size = format!("{:>9}", asset.size_human());
        let license = security::sanitize_text(&asset.license_or_unknown());
        let license = truncate_chars(&license, 26);

        let name_style = if app.selected.contains(&i) {
            t.success_style().add_modifier(Modifier::BOLD)
        } else {
            t.text_style()
        };
        let pad = pad_to(name_width as usize, &name);
        let row = Line::from(vec![
            Span::styled(marker, t.dim_style()),
            Span::raw(" "),
            Span::styled(index, t.dim_style()),
            Span::raw("  "),
            Span::styled(name, name_style),
            Span::styled(pad, t.text_style()),
            Span::styled(format!(" {size} "), t.dim_style()),
            Span::styled(license, t.dim_style()),
        ]);
        items.push(ListItem::new(row));
    }

    let list = List::new(items)
        .highlight_symbol("› ")
        .highlight_style(t.selected_style());
    let mut state = app.list_state.clone();
    f.render_stateful_widget(list, list_area, &mut state);
}

fn draw_details(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(asset) = app.assets.get(app.detail_index) else {
        f.render_widget(Paragraph::new("No file selected."), area);
        return;
    };

    let index_line = Line::from(Span::styled(
        format!(
            "  {} / {} — press ↑/↓ to navigate",
            app.detail_index + 1,
            app.assets.len()
        ),
        t.dim_style(),
    ));

    let lines = detail_lines(app, asset);
    let para = Paragraph::new(lines)
        .block(
            Block::bordered()
                .title(Span::styled(" SVG Details ", t.title_style()))
                .border_style(t.border_active_style())
                .padding(Padding::horizontal(2)),
        )
        .wrap(Wrap { trim: true })
        .scroll((0, 0));
    let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);
    f.render_widget(para, chunks[0]);
    f.render_widget(index_line, chunks[1]);
}

fn detail_lines<'a>(app: &'a App, asset: &'a Asset) -> Vec<Line<'a>> {
    let t = &app.theme;
    let mut out: Vec<Line<'a>> = Vec::new();

    let field: fn(&str, &str) -> Vec<Span<'static>> = |k, v| {
        vec![
            Span::styled(
                format!("{k:<16}"),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(crate::security::sanitize_text(v)),
        ]
    };

    out.push(Line::from(field("Name:", &asset.original_name)));
    out.push(Line::from(field("Size:", &asset.size_human())));
    out.push(Line::from(field(
        "Format:",
        asset.mime.as_deref().unwrap_or("SVG"),
    )));
    out.push(Line::from(field("Dimensions:", &asset.dimensions_human())));
    if let Some(a) = &asset.author {
        out.push(Line::from(field(
            "Author:",
            &crate::security::sanitize_text(a),
        )));
    }
    if let Some(u) = &asset.uploader {
        out.push(Line::from(field("Uploader:", u)));
    }
    out.push(Line::from(field("License:", &asset.license_or_unknown())));
    if asset.license.is_none() {
        out.push(Line::from(Span::styled(
            "  ⚠ License unknown — verify on the Wikimedia Commons page before reuse.",
            t.warn_style(),
        )));
    }
    if let Some(lu) = &asset.license_url {
        out.push(Line::from(field("License URL:", lu)));
    }
    if let Some(u) = &asset.uploaded_at {
        let uploaded = u.format("%Y-%m-%d").to_string();
        out.push(Line::from(field("Uploaded:", &uploaded)));
    }
    if !asset.categories.is_empty() {
        out.push(Line::from(field(
            "Categories:",
            &asset.categories.join(", "),
        )));
    }
    if let Some(d) = &asset.description {
        out.push(Line::default());
        out.push(Line::from(Span::styled("Description:", t.title_style())));
        out.push(Line::from(crate::security::sanitize_text(d)));
    }
    if let Some(u) = &asset.url {
        out.push(Line::default());
        out.push(Line::from(Span::styled("Source:", t.title_style())));
        out.push(Line::from(crate::security::sanitize_text(u)));
    }
    if app.show_attribution {
        out.push(Line::default());
        out.push(Line::from(Span::styled("Attribution:", t.title_style())));
        let attribution = asset
            .attribution
            .as_deref()
            .unwrap_or("Not provided by the API; see the Commons page.");
        out.push(Line::from(crate::security::sanitize_text(attribution)));
    }
    out.push(Line::default());
    out
}

fn draw_downloads(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let mut text = Text::default();

    if let Some(batch) = &app.batch {
        draw_batch(&mut text, batch, t, app.frame);
    }
    if let Some(zip) = &app.zip {
        draw_zip(&mut text, zip, t, app.frame);
    }

    if !app.jobs.is_empty() {
        text.push_line(Line::from(Span::styled(
            "Single file downloads",
            t.title_style(),
        )));
        for job in &app.jobs {
            let (marker, bar) = match &job.status {
                JobStatus::Active => {
                    let pct = if job.total > 0 {
                        (job.done as f64 / job.total as f64 * 100.0).clamp(0.0, 100.0)
                    } else {
                        50.0
                    };
                    let bar = progress_text(&t.bar_chars(), pct);
                    (spinner(t, app.frame).to_string(), bar)
                }
                JobStatus::Done(path) => ("✓".to_string(), format!("Saved: {}", path.display())),
                JobStatus::Skipped => ("–".to_string(), "Skipped".to_string()),
                JobStatus::Failed(e) => ("✗".to_string(), format!("Failed: {e}")),
            };
            let done = format_size(job.done);
            let total = if job.total > 0 {
                format_size(job.total)
            } else {
                "unknown".to_string()
            };
            text.push_line(Line::from(vec![
                Span::styled(marker, t.accent_style()),
                Span::raw("  "),
                Span::styled(crate::security::sanitize_text(&job.name), t.text_style()),
            ]));
            if matches!(job.status, JobStatus::Active) {
                text.push_line(Line::from(Span::styled(
                    format!("    {bar}  {done} / {total}"),
                    t.dim_style(),
                )));
            }
        }
        text.push_line(Line::default());
    }

    if text.lines.is_empty() {
        text.push_line(Line::from(Span::styled("No downloads yet.", t.dim_style())));
        text.push_line(Line::default());
        text.push_line(Line::from(Span::styled(
            "Press d in the results list, or download a ZIP with z.",
            t.dim_style(),
        )));
    }

    let para = Paragraph::new(text)
        .block(
            Block::bordered()
                .title(Span::styled(" Downloads ", t.title_style()))
                .border_style(t.border_style())
                .padding(Padding::horizontal(2)),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(para, area);
}

fn draw_batch<'a>(text: &mut Text<'a>, batch: &BatchUi, t: &super::theme::Theme, frame: u64) {
    let s = &batch.stats;
    let pct = if s.total > 0 {
        ((s.completed + s.failed + s.skipped) as f64 / s.total as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let bar = progress_text(&t.bar_chars(), pct);
    let marker = if batch.finished {
        "✓"
    } else {
        spinner(t, frame)
    };
    text.push_line(Line::from(vec![
        Span::styled(marker, t.accent_style()),
        Span::styled("  Batch download — ", t.bold_accent()),
        Span::styled(batch.dest.display().to_string(), t.dim_style()),
    ]));
    text.push_line(Line::from(Span::styled(
        format!(
            "    {bar}  {} / {}  ·  completed {}  failed {}  skipped {}",
            s.completed + s.failed + s.skipped,
            s.total,
            s.completed,
            s.failed,
            s.skipped
        ),
        t.dim_style(),
    )));
    text.push_line(Line::default());
}

fn draw_zip<'a>(text: &mut Text<'a>, zip: &super::app::ZipUi, t: &super::theme::Theme, frame: u64) {
    let marker = if !zip.running {
        "✓"
    } else {
        spinner(t, frame)
    };
    text.push_line(Line::from(vec![
        Span::styled(marker, t.accent_style()),
        Span::styled("  ZIP archive — ", t.bold_accent()),
        Span::styled(zip.phase.clone(), t.dim_style()),
        if let Some(name) = &zip.name {
            Span::styled(
                format!(" ({})", crate::security::sanitize_text(name)),
                t.dim_style(),
            )
        } else {
            Span::raw("")
        },
    ]));
    if zip.running {
        let pct = if zip.total > 0 {
            (zip.done as f64 / zip.total as f64 * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
        let bar = progress_text(&t.bar_chars(), pct);
        text.push_line(Line::from(Span::styled(
            format!("    {bar}  {} / {}", zip.done, zip.total),
            t.dim_style(),
        )));
    } else if let Some(Ok((path, files, bytes))) = &zip.result {
        text.push_line(Line::from(Span::styled(
            format!(
                "    {files} files, {} → {}",
                format_size(*bytes),
                path.display()
            ),
            t.success_style(),
        )));
    } else if let Some(Err(e)) = &zip.result {
        text.push_line(Line::from(Span::styled(
            format!("    Failed: {e}"),
            t.error_style(),
        )));
    }
    text.push_line(Line::default());
}

fn draw_recent(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    if app.recents.is_empty() {
        let msg = Paragraph::new(Line::from(Span::styled(
            "No recent searches yet. Press Esc to go back.",
            t.dim_style(),
        )))
        .block(
            Block::bordered()
                .title(" Recent Searches ")
                .border_style(t.border_style()),
        )
        .alignment(Alignment::Center);
        f.render_widget(msg, area);
        return;
    }
    let items: Vec<ListItem> = app
        .recents
        .iter()
        .enumerate()
        .map(|(i, q)| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    if i == app.recent_index { "› " } else { "  " },
                    t.accent_style(),
                ),
                Span::styled(
                    crate::security::sanitize_text(q),
                    if i == app.recent_index {
                        t.selected_style()
                    } else {
                        t.text_style()
                    },
                ),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::bordered()
                .title(" Recent Searches ")
                .border_style(t.border_style()),
        )
        .highlight_style(t.selected_style());
    let mut state = app.recents_state();
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_settings(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let settings = &app.settings;
    let mut text = Vec::new();
    let kv = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!("{k:<24}"), t.dim_style()),
            Span::styled(v, t.text_style()),
        ])
    };
    text.push(kv(
        "Config file",
        settings.config_path.display().to_string(),
    ));
    text.push(Line::default());
    text.push(kv(
        "Download directory",
        settings.download_dir.display().to_string(),
    ));
    text.push(kv("Max concurrency", settings.max_concurrency.to_string()));
    text.push(kv(
        "Cache",
        if settings.cache_enabled {
            "enabled"
        } else {
            "disabled"
        }
        .into(),
    ));
    text.push(kv("Cache TTL", format!("{}h", settings.cache_ttl_hours)));
    text.push(kv("Theme", settings.theme.clone()));
    text.push(kv(
        "Animations",
        if settings.animations { "on" } else { "off" }.into(),
    ));
    text.push(Line::default());
    text.push(Line::from(Span::styled(
        "Edit the config file listed above and relaunch to apply changes.\nRun `get-svg config` to print these values.",
        t.dim_style(),
    )));

    let para = Paragraph::new(text)
        .block(
            Block::bordered()
                .title(Span::styled(" Settings ", t.title_style()))
                .border_style(t.border_style())
                .padding(Padding::horizontal(2)),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(para, area);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let content = Text::from(vec![
        Line::from(Span::styled(
            "Navigation",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  ↑ ↓ / j k      Move through lists"),
        Line::from("  Enter           Select / open details / confirm"),
        Line::from("  Esc             Back"),
        Line::from("  q / Ctrl+C      Quit"),
        Line::default(),
        Line::from(Span::styled(
            "Search & results",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  /               New search"),
        Line::from("  Space           Toggle selection"),
        Line::from("  a               Select all visible"),
        Line::from("  n               Clear selection"),
        Line::from("  d               Download (selected, else highlighted)"),
        Line::from("  A               Download all matching results"),
        Line::from("  z               Download ZIP"),
        Line::from("  r               Refresh from network"),
        Line::from("  PgUp/PgDn       Page through results"),
        Line::default(),
        Line::from(Span::styled(
            "Details",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  c               Copy source URL (OSC 52 clipboard)"),
        Line::from("  o               Open the Wikimedia page"),
        Line::from("  a               Toggle attribution"),
        Line::from("  ↑ ↓             Prev / next result"),
        Line::default(),
        Line::from(Span::styled(
            "Tips",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  Results are SVG-only and fetched from the official MediaWiki API."),
        Line::from("  Licensing is per-file; verify before reuse when marked Unknown."),
        Line::from("  Run `get-svg --help` for scripting commands and `--json` output."),
    ])
    .into_iter()
    .map(|l| l.alignment(Alignment::Left))
    .collect::<Text>();

    let para = Paragraph::new(content)
        .block(
            Block::bordered()
                .title(" Help ")
                .borders(Borders::ALL)
                .padding(Padding::horizontal(2)),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(para, area);
}

// ---------------------------------------------------------------------------
// Overlays
// ---------------------------------------------------------------------------

fn overlay_area(area: Rect) -> Rect {
    let width = (area.width as usize).min(72) as u16;
    let height = ((area.height as usize).min(18)) as u16;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn draw_confirm(
    f: &mut Frame,
    area: Rect,
    confirm: &super::app::ConfirmState,
    t: &super::theme::Theme,
) {
    let rect = overlay_area(area);
    f.render_widget(Clear, rect);
    let block = Block::bordered()
        .title(Span::styled(" Confirm ", t.title_style()))
        .border_style(t.border_active_style())
        .padding(Padding::horizontal(2));
    let mut body: Vec<Line> = confirm
        .message
        .split('\n')
        .map(|l| Line::from(String::from(l)))
        .collect();
    body.push(Line::default());
    body.push(Line::from(Span::styled(
        "[y] Yes    [any other key] No",
        t.warn_style(),
    )));
    let para = Paragraph::new(Text::from(body))
        .block(block)
        .wrap(Wrap { trim: true });
    f.render_widget(para, rect);
}

fn draw_error(f: &mut Frame, area: Rect, err: &super::app::ErrorState, t: &super::theme::Theme) {
    let rect = overlay_area(area);
    f.render_widget(Clear, rect);
    let block = Block::bordered()
        .title(Span::styled(
            format!(" {} ", err.title),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::Red))
        .padding(Padding::horizontal(2));
    let mut lines: Vec<Line> = err
        .message
        .split('\n')
        .map(|l| Line::from(String::from(l)))
        .collect();
    lines.push(Line::default());
    let mut keys = Vec::new();
    if err.can_cache {
        keys.push(Span::styled("[c] Use cache   ", t.warn_style()));
    }
    if err.can_retry {
        keys.push(Span::styled("[r] Retry   ", t.accent_style()));
    }
    keys.push(Span::styled("[q] Quit   ", t.dim_style()));
    keys.push(Span::styled("[Esc] Back", t.dim_style()));
    lines.push(Line::from(keys));
    let para = Paragraph::new(Text::from(lines))
        .block(block)
        .wrap(Wrap { trim: true });
    f.render_widget(para, rect);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn progress_text(chars: &(&'static str, &'static str), pct: f64) -> String {
    let width = 30usize;
    let filled = ((pct / 100.0) * width as f64).round() as usize;
    format!(
        "{}{} {:>3.0}%",
        chars.0.repeat(filled),
        chars.1.repeat(width - filled),
        pct
    )
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn pad_to(width: usize, s: &str) -> String {
    let count = s.chars().count();
    if count >= width {
        return String::new();
    }
    " ".repeat(width - count)
}
