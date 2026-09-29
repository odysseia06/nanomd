use std::path::{Path, PathBuf};

/// PowerShell on Windows; the user's shell, or /bin/sh, elsewhere.
pub fn shell_program() -> String {
    #[cfg(windows)]
    return "powershell.exe".into();
    #[cfg(not(windows))]
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

/// Pure selection: first candidate that exists and is a directory, in
/// priority order (doc parent, home, process cwd). `None` means "refuse to
/// spawn" — the PTY layer must never be handed an invalid cwd.
pub fn choose_cwd(
    doc_path: Option<&Path>,
    home: Option<PathBuf>,
    process_cwd: Option<PathBuf>,
) -> Option<PathBuf> {
    doc_path
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .into_iter()
        .chain(home)
        .chain(process_cwd)
        .find(|d| d.is_dir())
}

/// Real lookups behind `choose_cwd`.
pub fn cwd_for(doc_path: Option<&Path>) -> Result<PathBuf, String> {
    #[allow(deprecated)] // std::env::home_dir is correct on Windows since 1.85
    let home = std::env::home_dir();
    choose_cwd(doc_path, home, std::env::current_dir().ok()).ok_or_else(|| {
        "no usable working directory (file dir, home, and cwd all missing)".to_owned()
    })
}

/// Shell args that run `cmd` at startup, empty for `None`. Only the
/// scripted-probe hook (`NANOMD_TERM_CMD`) ever passes `Some`, so a normal
/// spawn keeps sending the bare interactive shell.
pub fn command_args(cmd: Option<&str>) -> Vec<String> {
    let Some(cmd) = cmd else {
        return Vec::new();
    };
    if cfg!(windows) {
        vec![
            "-NoProfile".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-Command".into(),
            cmd.to_owned(),
        ]
    } else {
        vec!["-c".into(), cmd.to_owned()]
    }
}

pub fn settings_for(
    doc_path: Option<&Path>,
    cmd: Option<&str>,
) -> Result<egui_term::BackendSettings, String> {
    Ok(egui_term::BackendSettings {
        shell: shell_program(),
        args: command_args(cmd),
        working_directory: Some(cwd_for(doc_path)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_parent_wins_when_it_is_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        std::fs::write(&file, b"").unwrap();
        let got = choose_cwd(Some(&file), Some(PathBuf::from("/nope")), None);
        assert_eq!(got.as_deref(), Some(dir.path()));
    }

    #[test]
    fn missing_doc_parent_falls_back_to_existing_home() {
        let home = tempfile::tempdir().unwrap();
        let got = choose_cwd(
            Some(Path::new("/definitely/missing/x.md")),
            Some(home.path().to_path_buf()),
            None,
        );
        assert_eq!(got.as_deref(), Some(home.path()));
    }

    #[test]
    fn missing_home_falls_back_to_process_cwd() {
        let cwd = tempfile::tempdir().unwrap();
        let got = choose_cwd(
            None,
            Some(PathBuf::from("/no/such/home")),
            Some(cwd.path().to_path_buf()),
        );
        assert_eq!(got.as_deref(), Some(cwd.path()));
    }

    #[test]
    fn nothing_valid_yields_none_not_a_bogus_path() {
        let got = choose_cwd(
            None,
            Some(PathBuf::from("/no/such/home")),
            Some(PathBuf::from("/no/such/cwd")),
        );
        assert_eq!(got, None);
    }

    #[test]
    fn settings_carry_shell_and_validated_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.md");
        std::fs::write(&file, b"").unwrap();
        let s = settings_for(Some(&file), None).unwrap();
        assert_eq!(s.shell, shell_program());
        assert!(s.args.is_empty());
        assert_eq!(s.working_directory.as_deref(), Some(dir.path()));
    }

    #[test]
    fn the_probe_command_hook_only_adds_args_when_it_is_set() {
        assert!(command_args(None).is_empty(), "plain spawn is unchanged");
        let args = command_args(Some("echo hi"));
        assert_eq!(args.last().map(String::as_str), Some("echo hi"));
        assert_eq!(
            args.first().map(String::as_str),
            Some(if cfg!(windows) { "-NoProfile" } else { "-c" })
        );
    }
}
