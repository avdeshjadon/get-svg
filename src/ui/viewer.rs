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

pub fn generate_viewer_html(_title: &str, svg_content: &str) -> String {
    format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>GET SVG</title>
<style>
  *, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}
  :root {{
    --bg: #ffffff;
    --grid: rgba(0, 0, 0, 0.04);
  }}
  body.theme-checker {{
    --bg: #f8fafc;
    background-image: 
      linear-gradient(45deg, #e2e8f0 25%, transparent 25%),
      linear-gradient(-45deg, #e2e8f0 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, #e2e8f0 75%),
      linear-gradient(-45deg, transparent 75%, #e2e8f0 75%) !important;
    background-size: 20px 20px !important;
    background-position: 0 0, 0 10px, 10px -10px, -10px 0px !important;
  }}
  body.theme-dark {{
    --bg: #090b10;
    --grid: rgba(255, 255, 255, 0.05);
  }}
  body {{
    width: 100vw; height: 100vh; overflow: hidden;
    background-color: var(--bg);
    background-image: radial-gradient(var(--grid) 1.5px, transparent 1.5px);
    background-size: 24px 24px;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
    user-select: none; -webkit-user-select: none;
    cursor: grab;
    transition: background-color 0.2s;
  }}
  body:active {{
    cursor: grabbing;
  }}
  svg {{
    width: 100vw !important;
    height: 100vh !important;
    max-width: 100vw !important;
    max-height: 100vh !important;
    display: block;
    shape-rendering: geometricPrecision;
    text-rendering: geometricPrecision;
  }}
  svg image {{
    image-rendering: high-quality;
    image-rendering: -webkit-optimize-contrast;
  }}
</style>
</head>
<body>
{svg_content}
<script>
const svg = document.querySelector('svg');
if (svg) {{
  let vb = svg.viewBox && svg.viewBox.baseVal;
  let hasValidVb = vb && vb.width > 0 && vb.height > 0;
  if (!hasValidVb) {{
    let bbox = null;
    try {{ bbox = svg.getBBox(); }} catch(e) {{}}
    let w = parseFloat(svg.getAttribute('width')) || (bbox && bbox.width > 0 ? bbox.width : 800);
    let h = parseFloat(svg.getAttribute('height')) || (bbox && bbox.height > 0 ? bbox.height : 600);
    let x = parseFloat(svg.getAttribute('x')) || (bbox ? bbox.x : 0);
    let y = parseFloat(svg.getAttribute('y')) || (bbox ? bbox.y : 0);
    svg.setAttribute('viewBox', `${{x}} ${{y}} ${{w}} ${{h}}`);
  }}
  svg.removeAttribute('width');
  svg.removeAttribute('height');
  if (!svg.getAttribute('preserveAspectRatio')) {{
    svg.setAttribute('preserveAspectRatio', 'xMidYMid meet');
  }}

  let initX = svg.viewBox.baseVal.x;
  let initY = svg.viewBox.baseVal.y;
  let initW = svg.viewBox.baseVal.width;
  let initH = svg.viewBox.baseVal.height;

  let curX = initX, curY = initY, curW = initW, curH = initH;
  let isDragging = false, startClientX = 0, startClientY = 0, startVbX = 0, startVbY = 0;
  const themes = ['', 'theme-checker', 'theme-dark'];
  let currentTheme = 0;

  function updateVb(x, y, w, h) {{
    curX = x; curY = y; curW = w; curH = h;
    svg.setAttribute('viewBox', `${{x}} ${{y}} ${{w}} ${{h}}`);
  }}

  function zoomAt(clientX, clientY, factor) {{
    const ctm = svg.getScreenCTM();
    if (!ctm) return;
    const pt = svg.createSVGPoint();
    pt.x = clientX;
    pt.y = clientY;
    const svgPt = pt.matrixTransform(ctm.inverse());

    const newW = curW * factor;
    const newH = curH * factor;
    if (newW < initW * 0.002 || newW > initW * 150) return;

    const newX = svgPt.x - (svgPt.x - curX) * factor;
    const newY = svgPt.y - (svgPt.y - curY) * factor;
    updateVb(newX, newY, newW, newH);
  }}

  window.addEventListener('wheel', (e) => {{
    e.preventDefault();
    const factor = e.deltaY < 0 ? 0.88 : 1.14;
    zoomAt(e.clientX, e.clientY, factor);
  }}, {{ passive: false }});

  window.addEventListener('mousedown', (e) => {{
    isDragging = true;
    startClientX = e.clientX;
    startClientY = e.clientY;
    startVbX = curX;
    startVbY = curY;
  }});

  window.addEventListener('mousemove', (e) => {{
    if (!isDragging) return;
    const ctm = svg.getScreenCTM();
    if (!ctm) return;
    const dx = (e.clientX - startClientX) / ctm.a;
    const dy = (e.clientY - startClientY) / ctm.d;
    updateVb(startVbX - dx, startVbY - dy, curW, curH);
  }});

  window.addEventListener('mouseup', () => isDragging = false);
  window.addEventListener('mouseleave', () => isDragging = false);

  window.addEventListener('keydown', (e) => {{
    if (e.key === 'Escape' || e.key === 'q' || e.key === 'Q') {{
      window.close();
    }} else if (e.key === '+' || e.key === '=') {{
      zoomAt(window.innerWidth / 2, window.innerHeight / 2, 0.85);
    }} else if (e.key === '-' || e.key === '_') {{
      zoomAt(window.innerWidth / 2, window.innerHeight / 2, 1.18);
    }} else if (e.key === '0' || e.key === 'r') {{
      updateVb(initX, initY, initW, initH);
    }} else if (e.key === 't' || e.key === 'T') {{
      currentTheme = (currentTheme + 1) % 3;
      document.body.className = themes[currentTheme];
    }}
  }});
}}
</script>
</body>
</html>"#)
}

pub fn open_vector_window(title: &str, svg_bytes: &[u8]) -> std::io::Result<PathBuf> {
    let svg_str = String::from_utf8_lossy(svg_bytes);
    let clean_svg = if let Some(idx) = svg_str.find("<svg") {
        &svg_str[idx..]
    } else {
        &svg_str
    };
    let html = generate_viewer_html(title, clean_svg);

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
