//! Micro-benchmarks for the security-critical and hot text paths.
//!
//! Run with: `cargo bench`

use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};

fn bench_sanitize_filename(c: &mut Criterion) {
    let cases = [
        "GitHub_Logo.svg",
        "../../secret.svg",
        "C:\\Windows\\System32\\..\\evil.svg",
        "日本語のとても長いファイル名で絵文字も含むのはどうでしょう.svg",
        "CON.svg",
        "a/b/c/d/e/f/g/h/i/j/k.svg",
    ];
    c.bench_function("sanitize_filename (short)", |b| {
        b.iter(|| {
            for case in &cases {
                black_box(get_svg::security::sanitize_filename(case));
            }
        })
    });

    let long_name = format!("{}.svg", "x".repeat(4000));
    c.bench_function("sanitize_filename (4000 chars)", |b| {
        // This case is unaffected by criterion's black_box in place of the name.
        let _ = &long_name;
        // Manually bench with a heavy loop to avoid a huge long_name in each iter.
        b.iter_custom(|iters| {
            let start = std::time::Instant::now();
            for _ in 0..iters {
                let _ = black_box(get_svg::security::sanitize_filename(&long_name));
            }
            start.elapsed()
        });
    });
}

fn bench_terminal_sanitization(c: &mut Criterion) {
    let evil = "\x1b[2J\x1b[1;31mGitHub\x1b]8;;http://evil\x07Logo\x07.svg";
    c.bench_function("sanitize_text (escapes)", |b| {
        b.iter(|| {
            black_box(get_svg::security::sanitize_text(evil));
        })
    });
    let plain = "A fairly long but entirely normal file name for the benchmark.svg";
    c.bench_function("sanitize_text (plain)", |b| {
        b.iter(|| {
            black_box(get_svg::security::sanitize_text(plain));
        })
    });
}

fn bench_asset_parse(c: &mut Criterion) {
    let value = serde_json::json!({
        "pageid": 123456,
        "ns": 6,
        "title": "File:GitHub_Logo.svg",
        "index": 3,
        "imageinfo": {
            "url": "https://upload.wikimedia.org/wikipedia/commons/5/5a/GitHub_Logo.svg",
            "size": 12700,
            "mime": "image/svg+xml",
            "width": 1024,
            "height": 1024,
            "timestamp": "2023-08-01T00:00:00Z",
            "extmetadata": {
                "LicenseShortName": {"value": "CC BY-SA 4.0", "source": "commons"},
                "Artist": {"value": "GitHub Inc.", "source": "infobox"},
                "Categories": {"value": "* Logos\n* GitHub", "source": "parser"}
            }
        }
    });
    c.bench_function("parse asset from api json", |b| {
        b.iter(|| {
            let a = get_svg::models::Asset::from_api_json(black_box(&value));
            assert!(a.is_some());
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(50).warm_up_time(Duration::from_millis(300)).measurement_time(Duration::from_secs(1));
    targets = bench_sanitize_filename, bench_terminal_sanitization, bench_asset_parse
}
criterion_main!(benches);
