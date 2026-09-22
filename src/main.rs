//! GET SVG binary entry point.

#[tokio::main]
async fn main() {
    // clap parses/prints help and exits inside `get_svg::run`.
    let code = get_svg::run().await;
    std::process::exit(code);
}
