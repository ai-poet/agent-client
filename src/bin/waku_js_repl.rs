#[path = "../js_repl.rs"]
mod js_repl;
#[path = "../js_repl_cua.rs"]
mod js_repl_cua;
#[path = "../js_repl_image.rs"]
mod js_repl_image;

/// Run the dedicated stdio transport without initializing the Waku GUI.
fn main() {
    if let Err(error) = js_repl::serve_stdio() {
        eprintln!("JavaScript REPL: {error:#}");
        std::process::exit(1);
    }
}
