mod app;
mod core;
mod system;
mod tray;
mod ui;

use gtk4::glib;
use tracing_subscriber::EnvFilter;

fn main() -> glib::ExitCode {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();

    app::run()
}
