//! Universal ANSI TrueColor half-block image renderer for terminal previews.

use ratatui::layout::Alignment;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Convert decoded image bytes (e.g. PNG) into ratatui Lines using Unicode half-blocks (▀ and ▄).
///
/// Each terminal character cell is 1 column wide and 1 row high.
/// With half-blocks (▀), 1 character cell renders 2 vertical pixels (top & bottom).
/// This provides crisp, high-density 24-bit TrueColor graphics on any terminal:
/// Antigravity IDE, VS Code, Ghostty, iTerm2, macOS Terminal, Windows Terminal, etc.
pub fn render_halfblocks(
    img_bytes: &[u8],
    max_cols: u16,
    max_rows: u16,
) -> Option<Vec<Line<'static>>> {
    let img = image::load_from_memory(img_bytes).ok()?;
    let (orig_w, orig_h) = (img.width(), img.height());
    if orig_w == 0 || orig_h == 0 {
        return None;
    }

    let max_cols = max_cols.max(6) as u32;
    // Terminal character cells are roughly 1:2 aspect ratio (twice as tall as wide).
    // Because each character cell holds 2 vertical half-block pixels,
    // 1 horizontal cell = 2 vertical pixels, which makes the pixels square!
    let max_pixel_rows = (max_rows.max(4) as u32) * 2;

    // Calculate aspect-preserving scale
    let scale_x = max_cols as f32 / orig_w as f32;
    let scale_y = max_pixel_rows as f32 / orig_h as f32;
    let scale = scale_x.min(scale_y);

    let target_w = ((orig_w as f32 * scale).round() as u32).clamp(1, max_cols);
    let target_pixel_h = ((orig_h as f32 * scale).round() as u32).clamp(1, max_pixel_rows);

    let resized = image::imageops::resize(
        &img.to_rgba8(),
        target_w,
        target_pixel_h,
        image::imageops::FilterType::Triangle,
    );

    let char_rows = (target_pixel_h + 1) / 2;
    let vertical_pad = (max_rows as u32).saturating_sub(char_rows) / 2;

    let mut lines = Vec::with_capacity((char_rows + vertical_pad * 2) as usize);

    // Vertical centering top padding
    for _ in 0..vertical_pad {
        lines.push(Line::default());
    }

    for cy in 0..char_rows {
        let py_top = cy * 2;
        let py_bot = cy * 2 + 1;
        let mut spans = Vec::with_capacity(target_w as usize);

        for px in 0..target_w {
            let top_rgba = resized.get_pixel(px, py_top);
            let bot_rgba = if py_bot < target_pixel_h {
                Some(resized.get_pixel(px, py_bot))
            } else {
                None
            };

            let top_vis = top_rgba[3] > 48;
            let bot_vis = bot_rgba.map(|p| p[3] > 48).unwrap_or(false);

            match (top_vis, bot_vis) {
                (true, true) => {
                    let fg = Color::Rgb(top_rgba[0], top_rgba[1], top_rgba[2]);
                    let bot = bot_rgba.unwrap();
                    let bg = Color::Rgb(bot[0], bot[1], bot[2]);
                    spans.push(Span::styled("▀", Style::default().fg(fg).bg(bg)));
                }
                (true, false) => {
                    let fg = Color::Rgb(top_rgba[0], top_rgba[1], top_rgba[2]);
                    spans.push(Span::styled("▀", Style::default().fg(fg)));
                }
                (false, true) => {
                    let bot = bot_rgba.unwrap();
                    let fg = Color::Rgb(bot[0], bot[1], bot[2]);
                    spans.push(Span::styled("▄", Style::default().fg(fg)));
                }
                (false, false) => {
                    spans.push(Span::raw(" "));
                }
            }
        }
        lines.push(Line::from(spans).alignment(Alignment::Center));
    }

    // Vertical centering bottom padding
    for _ in 0..vertical_pad {
        lines.push(Line::default());
    }

    Some(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_halfblocks_generates_lines_for_valid_png() {
        // Create a 10x10 red PNG in memory
        let mut img = image::RgbaImage::new(10, 10);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgba([255, 0, 0, 255]);
        }
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let bytes = buf.into_inner();

        let lines = render_halfblocks(&bytes, 20, 10);
        assert!(lines.is_some());
        let lines = lines.unwrap();
        assert!(!lines.is_empty());
    }

    #[test]
    fn render_halfblocks_returns_none_on_corrupt_data() {
        let corrupt = b"not a real image";
        assert!(render_halfblocks(corrupt, 20, 10).is_none());
    }
}
