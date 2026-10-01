//! Universal ANSI TrueColor half-block image renderer for terminal previews.
//!
//! Supports both direct vector SVG rendering (via `resvg`) and raster PNG/JPEG
//! thumbnails (via `image`).

use ratatui::layout::Alignment;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Convert image bytes (either SVG XML or raster PNG/JPEG) into ratatui Lines using Unicode half-blocks (▀ and ▄).
///
/// Each terminal character cell is 1 column wide and 1 row high.
/// With half-blocks (▀), 1 character cell renders 2 vertical pixels (top & bottom).
/// This provides crisp, high-density 24-bit TrueColor graphics on any terminal:
/// Antigravity IDE, VS Code, Ghostty, iTerm2, macOS Terminal, Windows Terminal, etc.
pub fn render_halfblocks(
    bytes: &[u8],
    max_cols: u16,
    max_rows: u16,
) -> Option<Vec<Line<'static>>> {
    // 1. Try rendering as SVG first (handles raw SVG from Wikimedia or disk)
    if let Some(lines) = render_svg(bytes, max_cols, max_rows) {
        return Some(lines);
    }
    // 2. Fall back to raster image (PNG, JPEG, WebP, etc.)
    render_raster(bytes, max_cols, max_rows)
}

fn render_svg(svg_bytes: &[u8], max_cols: u16, max_rows: u16) -> Option<Vec<Line<'static>>> {
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(svg_bytes, &opt).ok()?;
    let orig_size = tree.size();
    let (orig_w, orig_h) = (orig_size.width(), orig_size.height());
    if orig_w <= 0.0 || orig_h <= 0.0 {
        return None;
    }

    let max_cols = max_cols.max(6) as u32;
    let max_pixel_rows = (max_rows.max(4) as u32) * 2;

    let scale_x = max_cols as f32 / orig_w;
    let scale_y = max_pixel_rows as f32 / orig_h;
    let scale = scale_x.min(scale_y);

    let target_w = ((orig_w * scale).round() as u32).clamp(1, max_cols);
    let target_pixel_h = ((orig_h * scale).round() as u32).clamp(1, max_pixel_rows);

    let mut pixmap = resvg::tiny_skia::Pixmap::new(target_w, target_pixel_h)?;
    let transform = resvg::tiny_skia::Transform::from_scale(
        target_w as f32 / orig_w,
        target_pixel_h as f32 / orig_h,
    );

    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let mut pixels = Vec::with_capacity((target_w * target_pixel_h) as usize);
    for y in 0..target_pixel_h {
        for x in 0..target_w {
            if let Some(c) = pixmap.pixel(x, y) {
                let a = c.alpha();
                let (r, g, b) = if a > 0 {
                    (
                        (c.red() as u16 * 255 / a as u16).min(255) as u8,
                        (c.green() as u16 * 255 / a as u16).min(255) as u8,
                        (c.blue() as u16 * 255 / a as u16).min(255) as u8,
                    )
                } else {
                    (0, 0, 0)
                };
                pixels.push((r, g, b, a));
            } else {
                pixels.push((0, 0, 0, 0));
            }
        }
    }

    Some(pixels_to_halfblocks(&pixels, target_w, target_pixel_h, max_rows))
}

fn render_raster(bytes: &[u8], max_cols: u16, max_rows: u16) -> Option<Vec<Line<'static>>> {
    let img = image::load_from_memory(bytes).ok()?;
    let (orig_w, orig_h) = (img.width(), img.height());
    if orig_w == 0 || orig_h == 0 {
        return None;
    }

    let max_cols = max_cols.max(6) as u32;
    let max_pixel_rows = (max_rows.max(4) as u32) * 2;

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

    let mut pixels = Vec::with_capacity((target_w * target_pixel_h) as usize);
    for y in 0..target_pixel_h {
        for x in 0..target_w {
            let px = resized.get_pixel(x, y);
            pixels.push((px[0], px[1], px[2], px[3]));
        }
    }

    Some(pixels_to_halfblocks(&pixels, target_w, target_pixel_h, max_rows))
}

fn pixels_to_halfblocks(
    pixels: &[(u8, u8, u8, u8)],
    width: u32,
    pixel_height: u32,
    max_rows: u16,
) -> Vec<Line<'static>> {
    let char_rows = (pixel_height + 1) / 2;
    let vertical_pad = (max_rows as u32).saturating_sub(char_rows) / 2;

    let mut lines = Vec::with_capacity((char_rows + vertical_pad * 2) as usize);

    for _ in 0..vertical_pad {
        lines.push(Line::default());
    }

    for cy in 0..char_rows {
        let py_top = cy * 2;
        let py_bot = cy * 2 + 1;
        let mut spans = Vec::with_capacity(width as usize);

        for px in 0..width {
            let top_idx = (py_top * width + px) as usize;
            let top_rgba = pixels.get(top_idx).copied().unwrap_or((0, 0, 0, 0));

            let bot_rgba = if py_bot < pixel_height {
                let bot_idx = (py_bot * width + px) as usize;
                pixels.get(bot_idx).copied()
            } else {
                None
            };

            let top_vis = top_rgba.3 > 48;
            let bot_vis = bot_rgba.map(|p| p.3 > 48).unwrap_or(false);

            match (top_vis, bot_vis) {
                (true, true) => {
                    let fg = Color::Rgb(top_rgba.0, top_rgba.1, top_rgba.2);
                    let bot = bot_rgba.unwrap();
                    let bg = Color::Rgb(bot.0, bot.1, bot.2);
                    spans.push(Span::styled("▀", Style::default().fg(fg).bg(bg)));
                }
                (true, false) => {
                    let fg = Color::Rgb(top_rgba.0, top_rgba.1, top_rgba.2);
                    spans.push(Span::styled("▀", Style::default().fg(fg)));
                }
                (false, true) => {
                    let bot = bot_rgba.unwrap();
                    let fg = Color::Rgb(bot.0, bot.1, bot.2);
                    spans.push(Span::styled("▄", Style::default().fg(fg)));
                }
                (false, false) => {
                    spans.push(Span::raw(" "));
                }
            }
        }
        lines.push(Line::from(spans).alignment(Alignment::Center));
    }

    for _ in 0..vertical_pad {
        lines.push(Line::default());
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_halfblocks_generates_lines_for_valid_png() {
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
    fn render_halfblocks_generates_lines_for_valid_svg() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <rect width="100" height="100" fill="red" />
        </svg>"#;
        let lines = render_halfblocks(svg.as_bytes(), 20, 10);
        assert!(lines.is_some());
        let lines = lines.unwrap();
        assert!(!lines.is_empty());
    }

    #[test]
    fn render_halfblocks_returns_none_on_corrupt_data() {
        let corrupt = b"not a real image or svg";
        assert!(render_halfblocks(corrupt, 20, 10).is_none());
    }
}
