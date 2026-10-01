//! All terminal drawing for the interactive UI.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, List, ListItem, Padding, Paragraph, Wrap};
use ratatui::Frame;

use super::app::{App, BatchUi, Screen, MIN_HEIGHT, MIN_WIDTH};
use crate::models::format_size;
use crate::security;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Block-letter logo. All six lines are exactly 58 characters wide.
const LOGO: [&str; 6] = [
    " ██████╗ ███████╗ ████████╗     ██████╗ ██╗   ██╗ ██████╗ ",
    "██╔════╝ ██╔════╝ ╚══██╔══╝    ██╔═══╝  ██║   ██║██╔════╝ ",
    "██║  ███╗█████╗      ██║        ██████╗ ██║   ██║██║  ███╗",
    "██║   ██║██╔══╝      ██║       ╚═══███╗ ╚██╗ ██╔╝██║   ██║",
    "╚██████╔╝███████╗    ██║       ███████╗  ╚████╔╝ ╚██████╔╝",
    " ╚═════╝ ╚══════╝    ╚═╝       ╚══════╝   ╚═══╝   ╚═════╝ ",
];

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

    f.render_widget(Clear, area);

    let (body, footer_area) = match app.screen {
        Screen::SearchInput => {
            let layout = Layout::vertical([
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);
            (layout[0], layout[1])
        }
        _ => {
            let layout = Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);
            draw_header(f, layout[0], app);
            (layout[1], layout[2])
        }
    };

    match app.screen {
        Screen::SearchInput => draw_search_input(f, body, app),
        Screen::Searching => draw_searching(f, body, app),
        Screen::Results => draw_results(f, body, app),
        Screen::Details => draw_details(f, body, app),
        Screen::Downloading => draw_downloading(f, body, app),
    }
    draw_footer(f, footer_area, app);

    if let Some(err) = &app.error {
        draw_error(f, area, err, &app.theme);
    }
}

// ---------------------------------------------------------------------------
// Chrome
// ---------------------------------------------------------------------------

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let title = match app.screen {
        Screen::SearchInput => format!(
            "{} v{} — {}",
            crate::APP_NAME,
            crate::VERSION,
            crate::TAGLINE
        ),
        Screen::Searching => format!("Search: {}", app.search_query),
        Screen::Results => format!(
            "Search: {} — {} result(s)",
            app.search_query,
            app.assets.len()
        ),
        Screen::Details => {
            let name = app
                .assets
                .get(app.detail_index)
                .map(|a| a.original_name.as_str())
                .unwrap_or("File");
            format!("Details: {name}")
        }
        Screen::Downloading => "Downloading".to_string(),
    };
    let text = if app.screen == Screen::SearchInput {
        format!(" {title}")
    } else {
        format!(" {} › {title}", crate::APP_NAME)
    };

    let mut spans = vec![Span::styled(text, app.theme.bold_accent())];
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
        Screen::SearchInput => {
            s.extend(key_span(t, "Enter", "Search"));
            s.extend(key_span(t, "Esc", "Clear"));
            s.extend(key_span(t, "Ctrl+C", "Quit"));
        }
        Screen::Searching => {
            s.extend(key_span(t, "Esc", "Cancel"));
        }
        Screen::Results => {
            if app.zip_confirm {
                s.extend(key_span(t, "y / Enter", "Confirm ZIP Download"));
                s.extend(key_span(t, "n / Esc", "Cancel"));
            } else {
                s.extend(key_span(t, "↑↓", "Navigate"));
                s.extend(key_span(t, "Enter", "Details & Download"));
                s.extend(key_span(t, "z", "Download as ZIP"));
                s.extend(key_span(t, "Space", "Select"));
                s.extend(key_span(t, "Esc", "New search"));
                s.extend(key_span(t, "q", "Quit"));
            }
        }
        Screen::Details => {
            s.extend(key_span(t, "y / Enter", "Download file"));
            s.extend(key_span(t, "n / Esc", "Back to results"));
            s.extend(key_span(t, "↑↓", "Prev/Next file"));
            s.extend(key_span(t, "q", "Quit"));
        }
        Screen::Downloading => {
            s.extend(key_span(t, "Esc", "Back"));
            s.extend(key_span(t, "q", "Quit"));
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

fn draw_search_input(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let show_logo = area.height >= 17;
    let (logo_chunk, input_chunk, hint_chunk) = if show_logo {
        let chunks = Layout::vertical([
            Constraint::Length(11),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(area);
        (Some(chunks[0]), chunks[2], chunks[4])
    } else {
        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(area);
        (None, chunks[0], chunks[2])
    };

    if let Some(chunk) = logo_chunk {
        f.render_widget(Clear, chunk);
        let logo_block = Block::bordered().border_style(t.border_style());
        let mut logo_text = Text::default();
        logo_text.push_line(Line::default());
        for line in LOGO {
            logo_text.push_line(Line::from(Span::styled(line, t.accent_style())));
        }
        logo_text.push_line(Line::default());
        logo_text.push_line(
            Line::from(Span::styled(crate::TAGLINE, t.dim_style())).alignment(Alignment::Center),
        );
        let logo_para = Paragraph::new(logo_text)
            .block(logo_block)
            .alignment(Alignment::Center);
        f.render_widget(logo_para, chunk);
    }

    let display = if app.input.is_empty() {
        Line::from(Span::styled(
            "Type a keyword (e.g. Amazon, GitHub, React)…",
            t.dim_style(),
        ))
    } else {
        Line::from(Span::styled(app.input.clone(), t.text_style()))
    };
    let input = Paragraph::new(display).block(
        Block::bordered()
            .title(Span::styled(
                " Search SVGs on Wikimedia Commons ",
                t.title_style(),
            ))
            .border_style(t.border_active_style()),
    );
    f.render_widget(input, input_chunk);

    let cursor_line = input_chunk;
    let x = cursor_x(&app.input);
    f.set_cursor_position((cursor_line.x + x + 1, cursor_line.y + 1));

    let hint = Line::from(vec![
        Span::styled("Quick search: ", t.bold_accent()),
        Span::styled("Type any keyword like ", t.text_style()),
        Span::styled("Amazon", t.accent_style()),
        Span::styled(", ", t.text_style()),
        Span::styled("GitHub", t.accent_style()),
        Span::styled(", or ", t.text_style()),
        Span::styled("React", t.accent_style()),
        Span::styled(" and press Enter. All matching SVG icons will be shown.", t.text_style()),
    ]);
    f.render_widget(hint, hint_chunk);
}

fn cursor_x(input: &str) -> u16 {
    input.chars().count() as u16
}

fn draw_searching(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let s = spinner(t, app.frame);
    let text = Text::from(vec![
        Line::from(vec![
            Span::styled(s, t.accent_style()),
            Span::raw("  Searching Wikimedia Commons…"),
        ])
        .alignment(Alignment::Center),
        Line::from(Span::styled(
            format!("\"{}\"", app.search_query),
            t.dim_style(),
        ))
        .alignment(Alignment::Center),
        Line::default(),
        Line::styled("Only SVG files (image/svg+xml) are matched.", t.dim_style())
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
        let msg = Line::from(Span::styled("No SVG files matched.", t.warn_style()))
            .alignment(Alignment::Center);
        f.render_widget(Paragraph::new(msg), inner);
        return;
    }

    let (head_area, list_area) = {
        let c = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
        (c[0], c[1])
    };

    // Column header.
    let name_width = (inner.width as usize)
        .saturating_sub(4 + 6 + 10 + 28)
        .max(20) as u16;
    let size_hdr = format!("{:>9}", "SIZE");
    let lic_hdr = format!("{:<26}", "LICENSE");
    let header = Line::from(vec![
        Span::styled(
            format!("{:<11}{:<nw$}", "", "NAME", nw = name_width as usize),
            t.dim_style(),
        ),
        Span::raw(" "),
        Span::styled(format!("{size_hdr} "), t.dim_style()),
        Span::styled(lic_hdr, t.dim_style()),
    ]);
    f.render_widget(Paragraph::new(header), head_area);

    // Asset rows, then a spacer, then the two download actions.
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

    items.push(ListItem::new(Line::from(Span::styled(
        "─".repeat(32),
        t.dim_style(),
    ))));
    if !app.selected.is_empty() {
        items.push(ListItem::new(Line::from(vec![
            Span::raw("     "),
            Span::styled(
                format!("⬇ Download selected ({})", app.selected.len()),
                t.bold_accent(),
            ),
        ])));
    }
    items.push(ListItem::new(Line::from(vec![
        Span::raw("     "),
        Span::styled("⬇ Download all files as complete ZIP (or press 'z')", t.bold_accent()),
    ])));

    let list = List::new(items)
        .highlight_symbol("› ")
        .highlight_style(t.selected_style());
    let mut state = app.list_state.clone();
    f.render_stateful_widget(list, list_area, &mut state);

    if app.zip_confirm {
        draw_zip_confirm(f, area, app);
    }
}

fn draw_zip_confirm(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let width = 66u16.min(area.width.saturating_sub(4));
    let height = 9u16.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect {
        x,
        y,
        width,
        height,
    };

    f.render_widget(Clear, rect);
    let block = Block::bordered()
        .title(Span::styled(" Download All Files as ZIP ", t.title_style()))
        .border_style(t.border_active_style())
        .padding(Padding::uniform(1));

    let count = app.assets.len();
    let text = vec![
        Line::from(Span::styled(
            format!("Do you want to download all {count} files as a complete ZIP?"),
            t.title_style().add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center),
        Line::default(),
        Line::from(vec![
            Span::styled(" [Y] Yes (Download ZIP) ", t.selected_style()),
            Span::raw("    "),
            Span::styled(" [N] No (Cancel) ", t.border_style()),
        ])
        .alignment(Alignment::Center),
        Line::default(),
        Line::from(Span::styled(
            "Press 'y' or Enter to confirm  •  Press 'n' or Esc to cancel",
            t.dim_style(),
        ))
        .alignment(Alignment::Center),
    ];

    f.render_widget(Paragraph::new(text).block(block), rect);
}

fn draw_details(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let Some(asset) = app.assets.get(app.detail_index) else {
        f.render_widget(Paragraph::new("No file selected."), area);
        return;
    };

    let chunks = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(7),
    ])
    .split(area);

    let mut lines = Vec::new();
    let label = |k: &str| Span::styled(format!("{:<20}", k), t.bold_accent());

    lines.push(Line::from(vec![
        label("File Name:"),
        Span::styled(security::sanitize_text(&asset.original_name), t.title_style()),
    ]));
    lines.push(Line::default());

    lines.push(Line::from(vec![
        label("File Size:"),
        Span::styled(asset.size_human(), t.text_style()),
    ]));

    let dims = asset.dimensions_human();
    if !dims.is_empty() {
        lines.push(Line::from(vec![
            label("Dimensions:"),
            Span::styled(dims, t.text_style()),
        ]));
    }

    lines.push(Line::from(vec![
        label("License:"),
        Span::styled(
            security::sanitize_text(&asset.license_or_unknown()),
            t.success_style().add_modifier(Modifier::BOLD),
        ),
    ]));

    if asset.license.is_none() {
        lines.push(Line::from(Span::styled(
            "  ⚠ License unknown — verify on Wikimedia Commons before reuse.",
            t.warn_style(),
        )));
    }

    if let Some(author) = &asset.author {
        lines.push(Line::from(vec![
            label("Author:"),
            Span::styled(security::sanitize_text(author), t.text_style()),
        ]));
    }

    if let Some(uploader) = &asset.uploader {
        lines.push(Line::from(vec![
            label("Uploader:"),
            Span::styled(security::sanitize_text(uploader), t.text_style()),
        ]));
    }

    if let Some(date) = &asset.uploaded_at {
        lines.push(Line::from(vec![
            label("Uploaded:"),
            Span::styled(date.format("%Y-%m-%d").to_string(), t.text_style()),
        ]));
    }

    if !asset.categories.is_empty() {
        lines.push(Line::from(vec![
            label("Categories:"),
            Span::styled(
                security::sanitize_text(&asset.categories.join(", ")),
                t.dim_style(),
            ),
        ]));
    }

    let fallback_wiki = format!(
        "https://commons.wikimedia.org/wiki/{}",
        asset.title.replace(' ', "_")
    );
    let wiki_url = asset.description_url.as_deref().unwrap_or(&fallback_wiki);
    lines.push(Line::from(vec![
        label("Wikimedia Link:"),
        Span::styled(security::sanitize_text(wiki_url), t.accent_style()),
    ]));

    if let Some(url) = &asset.url {
        lines.push(Line::from(vec![
            label("Direct SVG URL:"),
            Span::styled(security::sanitize_text(url), t.dim_style()),
        ]));
    }

    let (meta_area, preview_area) = if chunks[0].width >= 80 {
        let cols = Layout::horizontal([
            Constraint::Min(44),
            Constraint::Length(chunks[0].width.min(44).max(34)),
        ])
        .split(chunks[0]);
        (cols[0], Some(cols[1]))
    } else if chunks[0].height >= 24 {
        let rows = Layout::vertical([
            Constraint::Length(10),
            Constraint::Min(0),
        ])
        .split(chunks[0]);
        (rows[1], Some(rows[0]))
    } else {
        (chunks[0], None)
    };

    let detail_block = Block::bordered()
        .title(Span::styled(
            format!(
                " SVG Details — {} of {} ",
                app.detail_index + 1,
                app.assets.len()
            ),
            t.title_style(),
        ))
        .border_style(t.border_active_style())
        .padding(Padding::horizontal(2));

    let details_p = Paragraph::new(lines)
        .block(detail_block)
        .wrap(Wrap { trim: true });
    f.render_widget(details_p, meta_area);

    if let Some(p_area) = preview_area {
        let p_block = Block::bordered()
            .title(Span::styled(" Visual Preview ", t.title_style()))
            .border_style(t.border_active_style())
            .padding(Padding::horizontal(1));

        let lines_opt = asset
            .thumb_url
            .as_deref()
            .and_then(|u| app.preview_cache.get(u))
            .or_else(|| {
                asset
                    .url
                    .as_deref()
                    .and_then(|u| app.preview_cache.get(u))
            });

        let is_loading = asset
            .thumb_url
            .as_deref()
            .map(|u| app.preview_loading.contains(u))
            .unwrap_or(false)
            || asset
                .url
                .as_deref()
                .map(|u| app.preview_loading.contains(u))
                .unwrap_or(false);

        let p_content = if let Some(lines) = lines_opt {
            Paragraph::new(lines.clone()).block(p_block)
        } else if is_loading {
            let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let s = spinner[(app.frame as usize / 2) % spinner.len()];
            Paragraph::new(vec![
                Line::default(),
                Line::default(),
                Line::default(),
                Line::from(vec![
                    Span::styled(format!("  {s} "), t.bold_accent()),
                    Span::styled("Loading logo preview…", t.dim_style()),
                ])
                .alignment(Alignment::Center),
            ])
            .block(p_block)
        } else {
            Paragraph::new(vec![
                Line::default(),
                Line::default(),
                Line::from(Span::styled("Preview unavailable", t.dim_style()))
                    .alignment(Alignment::Center),
            ])
            .block(p_block)
        };

        f.render_widget(p_content, p_area);
    }

    let action_block = Block::bordered()
        .title(Span::styled(" Download ", t.title_style()))
        .border_style(t.border_active_style())
        .padding(Padding::horizontal(1));
    let action_text = vec![
        Line::from(Span::styled(
            "Download this individual file?",
            t.title_style().add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center),
        Line::default(),
        Line::from(vec![
            Span::styled(" [Y] Yes (Download this file) ", t.selected_style()),
            Span::raw("    "),
            Span::styled(" [N] No (Back to results) ", t.border_style()),
        ])
        .alignment(Alignment::Center),
        Line::default(),
        Line::from(vec![
            Span::styled("[Y / Enter] Download file", t.bold_accent()),
            Span::raw("  •  "),
            Span::styled("[N / Esc] Back to results", t.dim_style()),
            Span::raw("  •  "),
            Span::styled("[←/→ / ↑/↓] Browse files", t.dim_style()),
            Span::raw("  •  "),
            Span::styled("[q] Quit", t.dim_style()),
        ])
        .alignment(Alignment::Center),
    ];
    let action_p = Paragraph::new(action_text)
        .block(action_block);
    f.render_widget(action_p, chunks[1]);
}



fn draw_downloading(f: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let mut text = Text::default();

    if let Some(batch) = &app.batch {
        draw_batch(&mut text, batch, t, app.frame);
    }
    if let Some(zip) = &app.zip {
        draw_zip(&mut text, zip, t, app.frame);
    }

    if text.lines.is_empty() {
        text.push_line(Line::from(Span::styled("No downloads yet.", t.dim_style())));
        text.push_line(Line::from(Span::styled(
            "Search, then choose “Download Manually” or “Download as ZIP”.",
            t.dim_style(),
        )));
    }

    let para = Paragraph::new(text)
        .block(
            Block::bordered()
                .title(Span::styled(" Downloading ", t.title_style()))
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
        Span::styled("  Download", t.bold_accent()),
        Span::styled(format!(" — {}", batch.dest.display()), t.dim_style()),
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
        Span::styled("  ZIP", t.bold_accent()),
        Span::styled(format!(" — {}", zip.phase), t.dim_style()),
        if let Some(name) = &zip.name {
            Span::styled(
                format!(" ({})", security::sanitize_text(name)),
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
    lines.push(Line::from(vec![
        Span::styled("[r] Retry   ", t.accent_style()),
        Span::styled("[q] Quit   ", t.dim_style()),
        Span::styled("[Esc] Back", t.dim_style()),
    ]));
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
