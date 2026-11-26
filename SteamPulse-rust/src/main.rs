#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod glass;
#[cfg(windows)]
mod native;
#[cfg(windows)]
mod widget_render;

#[cfg(windows)]
use app::Event;

#[cfg(windows)]
fn main() {
    if let Err(e) = app::run() {
        eprintln!("{e}");
        native::error(&e.to_string());
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Steam Pulse requires Windows.");
}
