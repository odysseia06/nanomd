#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod doc;
mod print;
mod settings;
mod term;

use std::path::PathBuf;

/// Window, taskbar and (on macOS) Dock icon. eframe sets the macOS Dock icon
/// from this at runtime, so macOS gets the version drawn on Apple's grid,
/// with its transparent margin; elsewhere the tile fills the square.
#[cfg(target_os = "macos")]
const APP_ICON: &[u8] = include_bytes!("../assets/icon-512.png");
#[cfg(not(target_os = "macos"))]
const APP_ICON: &[u8] = include_bytes!("../assets/icon-48.png");

#[derive(Debug, PartialEq)]
enum Cli {
    Version,
    Open(Option<PathBuf>),
}

/// First CLI argument (after the exe name): version flag or a path to open.
fn parse_cli(mut args: impl Iterator<Item = std::ffi::OsString>) -> Cli {
    match args.nth(1) {
        Some(a) if a == "--version" || a == "-V" => Cli::Version,
        Some(a) => Cli::Open(Some(PathBuf::from(a))),
        None => Cli::Open(None),
    }
}

fn main() -> eframe::Result {
    let path = match parse_cli(std::env::args_os()) {
        Cli::Version => {
            // Release builds are windows_subsystem = "windows": this prints
            // when stdout is piped/redirected (the CI smoke test), not in an
            // interactive Windows console. AttachConsole via
            // windows-sys if interactive output ever matters.
            println!("nanomd {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Cli::Open(p) => p,
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([900.0, 700.0])
            .with_min_inner_size([320.0, 240.0])
            .with_title("nano.md")
            .with_icon(
                eframe::icon_data::from_png_bytes(APP_ICON).expect("bundled icon is a valid PNG"),
            ),
        // eframe restores the window's size and position from here, and
        // pulls it back on screen if its monitor is gone. App::save writes
        // the entry itself; settings::WindowGeometry says why.
        persistence_path: settings::window_state_path(),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "nano.md",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc, path)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn version_flags_are_detected() {
        let long = vec![OsString::from("nanomd.exe"), OsString::from("--version")];
        assert_eq!(parse_cli(long.into_iter()), Cli::Version);
        let short = vec![OsString::from("nanomd.exe"), OsString::from("-V")];
        assert_eq!(parse_cli(short.into_iter()), Cli::Version);
    }

    #[test]
    fn first_arg_is_a_path_when_not_a_flag() {
        let args = vec![OsString::from("nanomd.exe"), OsString::from("notes.md")];
        assert_eq!(
            parse_cli(args.into_iter()),
            Cli::Open(Some(PathBuf::from("notes.md")))
        );
    }

    #[test]
    fn no_args_means_empty_buffer() {
        let args = vec![OsString::from("nanomd.exe")];
        assert_eq!(parse_cli(args.into_iter()), Cli::Open(None));
    }
}
