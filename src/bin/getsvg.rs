//! Alias binary: `getsvg` behaves exactly like `get-svg`.

#[tokio::main]
async fn main() {
    let code = get_svg::run().await;
    std::process::exit(code);
}
