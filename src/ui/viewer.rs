//! Standalone interactive Vector SVG viewer window.
//!
//! Generates a self-contained, zero-dependency HTML5 viewer with:
//! - Exact mathematical vector rendering (zero pixelation, infinite retina zoom)
//! - Smooth mouse wheel & trackpad pinch-to-zoom (up to 3000%)
//! - Click-and-drag pan across the canvas
//! - Theme toggle (Dark Grid / Checkerboard for white logos / Light)
//! - Keyboard shortcuts (+, -, 0 to reset, Esc / q to close)
//! - Standalone borderless app window launch via Chrome / system viewer

use std::path::{Path, PathBuf};

pub fn generate_viewer_html(title: &str, svg_content: &str) -> String {
    let safe_title = title.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{safe_title} — GET SVG</title>
<style>
  *, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}
  :root {{
    --bg: #090b10;
    --grid: rgba(255, 255, 255, 0.05);
    --pill-bg: rgba(20, 24, 33, 0.85);
    --border: rgba(255, 255, 255, 0.12);
    --text: #f0f6fc;
    --text-dim: #8b949e;
    --accent: #58a6ff;
  }}
  body.theme-checker {{
    --bg: #1c2128;
    background-image: 
      linear-gradient(45deg, #161b22 25%, transparent 25%),
      linear-gradient(-45deg, #161b22 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, #161b22 75%),
      linear-gradient(-45deg, transparent 75%, #161b22 75%) !important;
    background-size: 20px 20px !important;
    background-position: 0 0, 0 10px, 10px -10px, -10px 0px !important;
  }}
  body.theme-light {{
    --bg: #f6f8fa;
    --grid: rgba(0, 0, 0, 0.06);
    --pill-bg: rgba(255, 255, 255, 0.9);
    --border: rgba(0, 0, 0, 0.15);
    --text: #1f2328;
    --text-dim: #656d76;
    --accent: #0969da;
  }}
  body {{
    width: 100vw; height: 100vh; overflow: hidden;
    background-color: var(--bg);
    background-image: radial-gradient(var(--grid) 1.5px, transparent 1.5px);
    background-size: 24px 24px;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
    color: var(--text);
    user-select: none; -webkit-user-select: none;
    transition: background-color 0.2s;
  }}
  #header {{
    position: fixed; top: 16px; left: 50%; transform: translateX(-50%);
    background: var(--pill-bg);
    backdrop-filter: blur(16px); -webkit-backdrop-filter: blur(16px);
    border: 1px solid var(--border);
    border-radius: 9999px;
    padding: 6px 14px;
    display: flex; align-items: center; gap: 10px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.35);
    z-index: 1000;
    font-size: 13px;
  }}
  .pill-title {{ font-weight: 600; color: var(--accent); max-width: 260px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
  .divider {{ width: 1px; height: 14px; background: var(--border); }}
  .zoom-val {{ font-variant-numeric: tabular-nums; min-width: 44px; text-align: center; color: var(--text); font-weight: 500; }}
  .btn {{
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid var(--border);
    color: var(--text);
    border-radius: 6px;
    padding: 3px 8px;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s ease;
  }}
  body.theme-light .btn {{ background: rgba(0, 0, 0, 0.05); }}
  .btn:hover {{ background: var(--accent); color: #fff; border-color: var(--accent); }}
  .hint {{ font-size: 11px; color: var(--text-dim); margin-left: 4px; }}
  #viewport {{
    width: 100%; height: 100%; display: flex; align-items: center; justify-content: center;
    cursor: grab;
  }}
  #viewport:active {{ cursor: grabbing; }}
  #canvas {{
    transform-origin: center center;
    display: flex; align-items: center; justify-content: center;
    will-change: transform;
    pointer-events: none;
  }}
  #canvas svg {{
    pointer-events: auto;
    max-width: 75vw;
    max-height: 75vh;
    filter: drop-shadow(0 16px 36px rgba(0, 0, 0, 0.3));
  }}
</style>
</head>
<body>
<div id="header">
  <span class="pill-title" title="{safe_title}">{safe_title}</span>
  <div class="divider"></div>
  <button class="btn" onclick="zoomRel(0.8)" title="Zoom Out (-)">−</button>
  <span class="zoom-val" id="zoom-text">100%</span>
  <button class="btn" onclick="zoomRel(1.25)" title="Zoom In (+)">+</button>
  <button class="btn" onclick="reset()" title="Reset (0)">Reset</button>
  <button class="btn" onclick="toggleTheme()" title="Toggle Canvas (T)">Theme</button>
  <div class="divider"></div>
  <span class="hint">Scroll to Zoom • Drag to Pan • Esc to Close</span>
</div>

<div id="viewport">
  <div id="canvas">
    {svg_content}
  </div>
</div>

<script>
let scale = 1, panX = 0, panY = 0, isDragging = false, startX = 0, startY = 0;
const canvas = document.getElementById('canvas');
const zoomText = document.getElementById('zoom-text');
const themes = ['', 'theme-checker', 'theme-light'];
let currentTheme = 0;

function render() {{
  canvas.style.transform = `translate(${{panX}}px, ${{panY}}px) scale(${{scale}})`;
  zoomText.textContent = `${{Math.round(scale * 100)}}%`;
}}

function zoomRel(factor) {{
  scale = Math.max(0.05, Math.min(30, scale * factor));
  render();
}}

function reset() {{
  scale = 1; panX = 0; panY = 0;
  render();
}}

function toggleTheme() {{
  currentTheme = (currentTheme + 1) % 3;
  document.body.className = themes[currentTheme];
}}

window.addEventListener('wheel', (e) => {{
  e.preventDefault();
  const delta = e.deltaY < 0 ? 1.15 : 0.87;
  zoomRel(delta);
}}, {{ passive: false }});

window.addEventListener('mousedown', (e) => {{
  if (e.target.closest('#header')) return;
  isDragging = true;
  startX = e.clientX - panX;
  startY = e.clientY - panY;
}});

window.addEventListener('mousemove', (e) => {{
  if (!isDragging) return;
  panX = e.clientX - startX;
  panY = e.clientY - startY;
  render();
}});

window.addEventListener('mouseup', () => isDragging = false);
window.addEventListener('mouseleave', () => isDragging = false);

window.addEventListener('keydown', (e) => {{
  if (e.key === 'Escape' || e.key === 'q' || e.key === 'Q') {{
    window.close();
  }} else if (e.key === '+' || e.key === '=') {{
    zoomRel(1.2);
  }} else if (e.key === '-' || e.key === '_') {{
    zoomRel(0.8);
  }} else if (e.key === '0' || e.key === 'r') {{
    reset();
  }} else if (e.key === 't' || e.key === 'T') {{
    toggleTheme();
  }}
}});
</script>
</body>
</html>"#)
}

pub fn open_vector_window(title: &str, svg_bytes: &[u8]) -> std::io::Result<PathBuf> {
    let svg_str = String::from_utf8_lossy(svg_bytes);
    let html = generate_viewer_html(title, &svg_str);

    let sanitized: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let temp_file = std::env::temp_dir().join(format!("getsvg_view_{}.html", sanitized));
    std::fs::write(&temp_file, html)?;

    launch_app_window(&temp_file);
    Ok(temp_file)
}

fn launch_app_window(path: &Path) {
    let path_str = path.to_string_lossy();
    let file_url = format!("file://{}", path_str);

    #[cfg(target_os = "macos")]
    {
        let chrome_path = Path::new("/Applications/Google Chrome.app");
        if chrome_path.exists() {
            let res = std::process::Command::new("open")
                .args([
                    "-na",
                    "Google Chrome",
                    "--args",
                    &format!("--app={file_url}"),
                    "--window-size=960,720",
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            if res.is_ok() {
                return;
            }
        }
        let _ = std::process::Command::new("open")
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    #[cfg(target_os = "linux")]
    {
        let res = std::process::Command::new("google-chrome")
            .args([&format!("--app={file_url}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if res.is_err() {
            let _ = std::process::Command::new("xdg-open")
                .arg(path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
    }

    #[cfg(target_os = "windows")]
    {
        let res = std::process::Command::new("cmd")
            .args(["/c", "start", "chrome", &format!("--app={file_url}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if res.is_err() {
            let _ = std::process::Command::new("cmd")
                .args(["/c", "start", "", &path_str])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
    }
}
