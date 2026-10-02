use eframe::egui::Theme;
use std::io;
use std::path::{Path, PathBuf};

/// One-line preference file inside [`config_dir`]; contains `dark` or `light`.
const THEME_FILE: &str = "theme";

fn theme_token(theme: Theme) -> &'static str {
    match theme {
        Theme::Dark => "dark",
        Theme::Light => "light",
    }
}

fn parse_theme(contents: &str) -> Option<Theme> {
    let token = contents.trim();
    if token.eq_ignore_ascii_case("dark") {
        Some(Theme::Dark)
    } else if token.eq_ignore_ascii_case("light") {
        Some(Theme::Light)
    } else {
        None
    }
}

pub fn load_theme_pref_from(dir: &Path) -> Option<Theme> {
    parse_theme(&std::fs::read_to_string(dir.join(THEME_FILE)).ok()?)
}

pub fn save_theme_pref_to(dir: &Path, theme: Theme) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(THEME_FILE), theme_token(theme))
}

/// One-line preference file inside [`config_dir`]: the terminal pane
/// height in egui logical points, a finite positive decimal.
const TERM_HEIGHT_FILE: &str = "term_height";

pub fn parse_term_height(contents: &str) -> Option<f32> {
    let v: f32 = contents.trim().parse().ok()?;
    (v.is_finite() && v > 0.0).then_some(v)
}

pub fn load_term_height_from(dir: &Path) -> Option<f32> {
    parse_term_height(&std::fs::read_to_string(dir.join(TERM_HEIGHT_FILE)).ok()?)
}

pub fn save_term_height_to(dir: &Path, height: f32) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(TERM_HEIGHT_FILE), height.to_string())
}

/// One-line preference file inside [`config_dir`]: the window zoom factor,
/// a decimal within `ZOOM_MIN..=ZOOM_MAX`.
const ZOOM_FILE: &str = "zoom";
pub const ZOOM_MIN: f32 = 0.5;
pub const ZOOM_MAX: f32 = 3.0;

pub fn load_zoom_from(dir: &Path) -> Option<f32> {
    let v: f32 = std::fs::read_to_string(dir.join(ZOOM_FILE))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    (ZOOM_MIN..=ZOOM_MAX).contains(&v).then_some(v)
}

pub fn save_zoom_to(dir: &Path, zoom: f32) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(ZOOM_FILE), zoom.to_string())
}

pub fn load_term_height() -> Option<f32> {
    load_term_height_from(&config_dir()?)
}

pub fn save_term_height(height: f32) -> io::Result<()> {
    let dir = config_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config directory"))?;
    save_term_height_to(&dir, height)
}

/// One path per line inside [`config_dir`]; most recent first.
const RECENT_FILE: &str = "recent";
pub const RECENT_CAP: usize = 8;

/// Dedupe, move to front, cap. Pure so it is trivially testable.
pub fn push_recent(recent: &mut Vec<PathBuf>, path: PathBuf) {
    recent.retain(|p| p != &path);
    recent.insert(0, path);
    recent.truncate(RECENT_CAP);
}

pub fn load_recent_from(dir: &Path) -> Vec<PathBuf> {
    // paths stored lossily as UTF-8 lines; a non-UTF-8 path would
    // round-trip wrong and simply fail to open from the menu. Fine for v0.1.
    let Ok(s) = std::fs::read_to_string(dir.join(RECENT_FILE)) else {
        return Vec::new();
    };
    // The file is a trust boundary: a hand-edited or oversized file must
    // not produce an unbounded or duplicated menu. Dedupe, stop at the cap.
    let mut out: Vec<PathBuf> = Vec::new();
    for line in s.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let p = PathBuf::from(line);
        if !out.contains(&p) {
            out.push(p);
            if out.len() == RECENT_CAP {
                break;
            }
        }
    }
    out
}

pub fn save_recent_to(dir: &Path, recent: &[PathBuf]) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut out = String::new();
    for p in recent.iter().take(RECENT_CAP) {
        out.push_str(&p.display().to_string());
        out.push('\n');
    }
    std::fs::write(dir.join(RECENT_FILE), out)
}

/// Empty when there is no config directory or no saved list.
pub fn load_recent() -> Vec<PathBuf> {
    config_dir()
        .map(|d| load_recent_from(&d))
        .unwrap_or_default()
}

/// Per-user config directory (`%APPDATA%\nanomd` on Windows,
/// `$XDG_CONFIG_HOME/nanomd` or `~/.config/nanomd` elsewhere).
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        });
    Some(base?.join("nanomd"))
}

/// `None` when there is no saved preference, the file is unreadable or
/// unrecognized, or no config directory exists: the app then keeps
/// following the OS theme.
pub fn load_theme_pref() -> Option<Theme> {
    load_theme_pref_from(&config_dir()?)
}

pub fn save_theme_pref(theme: Theme) -> io::Result<()> {
    let dir = config_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config directory"))?;
    save_theme_pref_to(&dir, theme)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Theme;

    #[test]
    fn round_trip_dark_and_light() {
        let dir = tempfile::tempdir().unwrap();
        save_theme_pref_to(dir.path(), Theme::Dark).unwrap();
        assert_eq!(load_theme_pref_from(dir.path()), Some(Theme::Dark));
        save_theme_pref_to(dir.path(), Theme::Light).unwrap();
        assert_eq!(load_theme_pref_from(dir.path()), Some(Theme::Light));
    }

    #[test]
    fn parsing_tolerates_case_and_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        // Deliberately writes the raw file name and contents (not the
        // constants) to pin the on-disk contract.
        std::fs::write(dir.path().join("theme"), "DARK\n").unwrap();
        assert_eq!(load_theme_pref_from(dir.path()), Some(Theme::Dark));
    }

    #[test]
    fn missing_empty_or_unrecognized_files_mean_no_preference() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_theme_pref_from(dir.path()), None);
        std::fs::write(dir.path().join("theme"), "").unwrap();
        assert_eq!(load_theme_pref_from(dir.path()), None);
        std::fs::write(dir.path().join("theme"), "purple").unwrap();
        assert_eq!(load_theme_pref_from(dir.path()), None);
    }

    #[test]
    fn save_creates_missing_config_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("does").join("not").join("exist");
        save_theme_pref_to(&nested, Theme::Dark).unwrap();
        assert_eq!(load_theme_pref_from(&nested), Some(Theme::Dark));
    }

    #[test]
    fn push_recent_dedupes_moves_to_front_and_caps() {
        let mut recent = Vec::new();
        for i in 0..10 {
            push_recent(&mut recent, PathBuf::from(format!("/f{i}.md")));
        }
        assert_eq!(recent.len(), RECENT_CAP);
        assert_eq!(recent[0], Path::new("/f9.md"));
        // Re-pushing an existing entry moves it to the front without growing.
        push_recent(&mut recent, PathBuf::from("/f5.md"));
        assert_eq!(recent.len(), RECENT_CAP);
        assert_eq!(recent[0], Path::new("/f5.md"));
        assert_eq!(
            recent.iter().filter(|p| *p == Path::new("/f5.md")).count(),
            1
        );
    }

    #[test]
    fn recent_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let list = vec![PathBuf::from("/a/one.md"), PathBuf::from("/b/two.md")];
        save_recent_to(dir.path(), &list).unwrap();
        assert_eq!(load_recent_from(dir.path()), list);
    }

    #[test]
    fn missing_recent_file_means_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_recent_from(dir.path()), Vec::<PathBuf>::new());
    }

    #[test]
    fn blank_lines_in_recent_file_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("recent"), "/a/one.md\n\n  \n/b/two.md\n").unwrap();
        assert_eq!(
            load_recent_from(dir.path()),
            vec![PathBuf::from("/a/one.md"), PathBuf::from("/b/two.md")]
        );
    }

    #[test]
    fn oversized_or_duplicated_recent_file_is_capped_and_deduped_on_load() {
        let dir = tempfile::tempdir().unwrap();
        // Simulates a stale or hand-edited file: 60 lines, half duplicates.
        let mut contents = String::new();
        for i in 0..30 {
            contents.push_str(&format!("/dup.md\n/f{i}.md\n"));
        }
        std::fs::write(dir.path().join("recent"), contents).unwrap();

        let loaded = load_recent_from(dir.path());

        assert_eq!(loaded.len(), RECENT_CAP, "restart must not exceed the cap");
        assert_eq!(loaded[0], Path::new("/dup.md"));
        assert_eq!(
            loaded.iter().filter(|p| *p == Path::new("/dup.md")).count(),
            1,
            "duplicates collapse to their first occurrence"
        );
        assert_eq!(loaded[1], Path::new("/f0.md"));
    }

    #[test]
    fn save_never_writes_more_than_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let long: Vec<PathBuf> = (0..20)
            .map(|i| PathBuf::from(format!("/f{i}.md")))
            .collect();
        save_recent_to(dir.path(), &long).unwrap();
        assert_eq!(load_recent_from(dir.path()).len(), RECENT_CAP);
    }

    #[test]
    fn term_height_parses_finite_positive_decimals_only() {
        assert_eq!(parse_term_height("240"), Some(240.0));
        assert_eq!(parse_term_height(" 187.5\n"), Some(187.5));
        assert_eq!(parse_term_height("0"), None);
        assert_eq!(parse_term_height("-3"), None);
        assert_eq!(parse_term_height("NaN"), None);
        assert_eq!(parse_term_height("inf"), None);
        assert_eq!(parse_term_height("tall"), None);
        assert_eq!(parse_term_height(""), None);
    }

    #[test]
    fn term_height_round_trips_and_missing_means_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_term_height_from(dir.path()), None);
        save_term_height_to(dir.path(), 301.25).unwrap();
        assert_eq!(load_term_height_from(dir.path()), Some(301.25));
        // Pins the on-disk contract: one plain decimal, file name term_height.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("term_height")).unwrap(),
            "301.25"
        );
    }

    #[test]
    fn zoom_round_trips_and_rejects_out_of_range_values() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_zoom_from(dir.path()), None);
        save_zoom_to(dir.path(), 1.25).unwrap();
        assert_eq!(load_zoom_from(dir.path()), Some(1.25));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("zoom")).unwrap(),
            "1.25"
        );
        for bad in ["0.1", "9", "NaN", "big", ""] {
            std::fs::write(dir.path().join("zoom"), bad).unwrap();
            assert_eq!(load_zoom_from(dir.path()), None, "{bad:?}");
        }
    }
}
