#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod doc;
mod print;
mod register;
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
    Register,
    Unregister,
    Open(Option<PathBuf>),
}

/// First CLI argument (after the exe name): a flag or a path to open.
fn parse_cli(mut args: impl Iterator<Item = std::ffi::OsString>) -> Cli {
    match args.nth(1) {
        Some(a) if a == "--version" || a == "-V" => Cli::Version,
        Some(a) if a == "--register" => Cli::Register,
        Some(a) if a == "--unregister" => Cli::Unregister,
        Some(a) => Cli::Open(Some(PathBuf::from(a))),
        None => Cli::Open(None),
    }
}

/// Release builds on Windows are GUI programs, which start without a
/// console: borrow the one they were run from, so flags can print.
/// Piped or redirected output (the CI smoke test) works either way.
fn attach_console() {
    #[cfg(windows)]
    // SAFETY: no arguments to check; failing (no parent console) is fine.
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(
            windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
        );
    }
}

/// Prints what a flag did, or its error with exit status 1.
fn report(done: std::io::Result<String>) -> eframe::Result {
    attach_console();
    match done {
        Ok(message) => println!("{message}"),
        Err(e) => {
            eprintln!("nanomd: {e}");
            std::process::exit(1);
        }
    }
    Ok(())
}

fn main() -> eframe::Result {
    let path = match parse_cli(std::env::args_os()) {
        Cli::Version => {
            attach_console();
            println!("nanomd {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Cli::Register => return report(register::register()),
        Cli::Unregister => return report(register::unregister()),
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
    fn register_flags_are_detected() {
        let args = |flag: &str| vec![OsString::from("nanomd.exe"), OsString::from(flag)];
        assert_eq!(parse_cli(args("--register").into_iter()), Cli::Register);
        assert_eq!(parse_cli(args("--unregister").into_iter()), Cli::Unregister);
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
