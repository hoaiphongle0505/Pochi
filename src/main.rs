#![cfg_attr(windows, windows_subsystem = "windows")]
#![cfg_attr(not(windows), allow(dead_code))]
mod anim;
mod app;
mod data;
mod gfx;
mod icons;
mod settings;
mod time;
mod ui;
#[cfg(windows)]
mod media_win;
#[cfg(windows)]
mod win;
#[cfg(not(windows))]
mod preview;

fn main() {
    #[cfg(windows)]
    win::run();
    #[cfg(not(windows))]
    preview::run();
}
