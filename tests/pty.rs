//! Headless PTY integration tests: real shell, no window. Every wait has a
//! deadline so a hung PTY fails instead of hanging CI.
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use egui_term::{BackendCommand, BackendSettings, PtyEvent, TerminalBackend};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

pub fn shell() -> String {
    if cfg!(windows) {
        "powershell.exe".into()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
}

/// Args that make the platform shell run `script` then keep the PTY open.
pub fn script_args(script: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            format!("{script}; Start-Sleep 300"),
        ]
    } else {
        vec!["-c".into(), format!("{script}; sleep 300")]
    }
}

pub fn spawn(
    id: u64,
    args: Vec<String>,
    cwd: &std::path::Path,
) -> (TerminalBackend, Receiver<(u64, PtyEvent)>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let backend = TerminalBackend::new(
        id,
        eframe::egui::Context::default(),
        tx,
        BackendSettings {
            shell: shell(),
            args,
            working_directory: Some(cwd.to_path_buf()),
        },
    )
    .expect("spawn");
    (backend, rx)
}

pub fn screen_text(backend: &mut TerminalBackend) -> String {
    let content = backend.sync();
    let grid = &content.grid;
    let mut out = String::new();
    for l in 0..grid.screen_lines() {
        let row = &grid[Line(l as i32)];
        for c in 0..grid.columns() {
            out.push(row[Column(c)].c);
        }
        out.push('\n');
    }
    out
}

pub fn wait_until(deadline: Duration, mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    f()
}

/// `egui_term::types::Size` is not re-exported upstream; `BackendCommand::Resize`
/// takes it. The vendored `lib.rs` re-exports it for these tests.
fn egui_term_size(w: f32, h: f32) -> egui_term::Size {
    egui_term::Size::new(w, h)
}

#[test]
fn shell_prints_marker_and_cwd() {
    let dir = tempfile::tempdir().unwrap();
    // PowerShell's table formatter (bare `pwd`) writes nothing through ConPTY,
    // so ask for the plain string instead.
    let script = if cfg!(windows) {
        "echo NANOMD_MARKER; (pwd).Path"
    } else {
        "echo NANOMD_MARKER; pwd"
    };
    let (mut b, _rx) = spawn(1, script_args(script), dir.path());
    let ok = wait_until(Duration::from_secs(10), || {
        let s = screen_text(&mut b);
        s.contains("NANOMD_MARKER") && s.contains(dir.path().file_name().unwrap().to_str().unwrap())
    });
    assert!(ok, "screen:\n{}", screen_text(&mut b));
    b.request_shutdown();
    assert!(
        b.wait_reader(Duration::from_secs(3)),
        "reader thread did not acknowledge shutdown"
    );
}

#[test]
fn input_reaches_the_shell() {
    let dir = tempfile::tempdir().unwrap();
    // `-NonInteractive` PowerShell refuses `Read-Host`, so read the line
    // through .NET; the shell echoes it back only if our bytes arrived.
    let script = if cfg!(windows) {
        "echo READY; $x = [Console]::In.ReadLine(); Write-Host GOT=$x"
    } else {
        "echo READY; read x; echo GOT=$x"
    };
    let (mut b, _rx) = spawn(1, script_args(script), dir.path());
    assert!(wait_until(Duration::from_secs(10), || screen_text(&mut b)
        .contains("READY")));
    b.process_command(BackendCommand::Write(b"PING\r".to_vec()));
    let ok = wait_until(Duration::from_secs(10), || {
        screen_text(&mut b).contains("GOT=PING")
    });
    assert!(ok, "screen:\n{}", screen_text(&mut b));
    b.request_shutdown();
    assert!(
        b.wait_reader(Duration::from_secs(3)),
        "reader thread did not acknowledge shutdown"
    );
}

#[test]
fn resize_is_visible_to_the_shell() {
    let dir = tempfile::tempdir().unwrap();
    // The shell reports its width once a second: ConPTY silently drops a
    // resize sent before the child has attached, so resize on first output.
    // 60 ticks, not 12: the two 10 s waits below can both run long on a cold
    // CI runner, and a probe that stops reporting is a flake, not a failure.
    let probe = if cfg!(windows) {
        "1..60 | ForEach-Object { Write-Host \"COLS=$($Host.UI.RawUI.WindowSize.Width)\"; Start-Sleep 1 }"
    } else {
        "for _ in $(seq 60); do echo \"COLS=$(stty size)\"; sleep 1; done"
    };
    let (mut b, _rx) = spawn(1, script_args(probe), dir.path());
    assert!(
        wait_until(Duration::from_secs(10), || screen_text(&mut b)
            .contains("COLS=")),
        "shell never reported its width:\n{}",
        screen_text(&mut b)
    );
    // 120 columns x 30 rows at an 8x16 cell.
    b.process_command(BackendCommand::Resize(
        egui_term_size(120.0 * 8.0, 30.0 * 16.0),
        egui_term_size(8.0, 16.0),
    ));
    let want = if cfg!(windows) {
        "COLS=120"
    } else {
        "COLS=30 120"
    };
    let ok = wait_until(Duration::from_secs(10), || {
        screen_text(&mut b).contains(want)
    });
    assert!(ok, "want {want}, screen:\n{}", screen_text(&mut b));
    b.request_shutdown();
    b.wait_reader(Duration::from_secs(3));
}

#[test]
fn late_events_from_an_exited_session_do_not_reach_a_new_one() {
    let dir = tempfile::tempdir().unwrap();
    let (mut old, old_rx) = spawn(1, script_args("echo OLD"), dir.path());
    assert!(wait_until(Duration::from_secs(10), || screen_text(
        &mut old
    )
    .contains("OLD")));
    old.request_shutdown();
    assert!(old.wait_reader(Duration::from_secs(3)));
    drop(old);
    let (mut new, new_rx) = spawn(2, script_args("echo NEW"), dir.path());
    assert!(wait_until(Duration::from_secs(10), || screen_text(
        &mut new
    )
    .contains("NEW")));
    // Whatever the old channel still holds is tagged with the old id and its
    // receiver is separate: nothing crosses over into the new session.
    while let Ok((id, _)) = old_rx.try_recv() {
        assert_eq!(id, 1, "new session's events reached the old receiver");
    }
    while let Ok((id, _)) = new_rx.try_recv() {
        assert_eq!(id, 2, "old session's events reached the new receiver");
    }
    new.request_shutdown();
    new.wait_reader(Duration::from_secs(3));
}

/// The subscription thread used to `panic!` when the application
/// dropped its receiver. Nothing else in this file drops `rx`, so this is the
/// only cover for that path.
#[test]
fn dropping_the_event_receiver_never_panics_the_subscription_thread() {
    static PANICKED: AtomicBool = AtomicBool::new(false);
    let dir = tempfile::tempdir().unwrap();
    // No trailing sleep: the shell exits by itself, so the reader really does
    // emit events (Wakeup, ChildExit) into the channel we are about to close.
    let args = if cfg!(windows) {
        vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            "echo BYE".into(),
        ]
    } else {
        vec!["-c".into(), "echo BYE".into()]
    };
    let (mut b, rx) = spawn(1, args, dir.path());
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|info| {
        PANICKED.store(true, Ordering::SeqCst);
        eprintln!("panic: {info}");
    }));
    drop(rx);
    let done = b.wait_reader(Duration::from_secs(5));
    b.request_shutdown();
    // Give the subscription thread time to hit the closed channel and unwind.
    std::thread::sleep(Duration::from_millis(200));
    std::panic::set_hook(prev);
    assert!(done, "reader thread did not acknowledge shutdown");
    assert!(
        !PANICKED.load(Ordering::SeqCst),
        "a thread panicked after the event receiver was dropped"
    );
}

/// The whole point of Task 8: closing the pane must take the shell *and* the
/// helper it left running in the foreground, inside a bounded budget.
#[test]
fn shutdown_ends_shell_and_foreground_helper_within_two_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let helper = if cfg!(windows) {
        "ping -t 127.0.0.1"
    } else {
        "sleep 300"
    };
    let (mut b, _rx) = spawn(
        1,
        script_args(&format!("echo HELPER_UP; {helper}")),
        dir.path(),
    );
    // From here on every early exit must `force_kill(shell_pid)` first: on Unix
    // the backend's own drop only SIGHUPs the shell, which orphans the helper.
    let shell_pid = b.pty_id();
    // The job now holds the shell, not the test process; the handle is left
    // open on purpose, so the kernel's close at test-binary exit is the
    // backstop that catches anything `shut_down` missed.
    #[cfg(windows)]
    {
        let job = nanomd_term_shutdown::JobGuard::new().unwrap();
        job.assign(shell_pid).unwrap();
    }
    if !wait_until(Duration::from_secs(10), || {
        screen_text(&mut b).contains("HELPER_UP")
    }) {
        force_kill(shell_pid);
        panic!(
            "shell {shell_pid} never printed HELPER_UP:\n{}",
            screen_text(&mut b)
        );
    }
    // The helper is a child of the shell, but the shell needs a moment to fork it.
    let mut helper_pid = None;
    wait_until(Duration::from_secs(5), || {
        helper_pid = first_child_of(shell_pid);
        helper_pid.is_some()
    });
    let Some(helper_pid) = helper_pid else {
        force_kill(shell_pid);
        panic!("shell {shell_pid} never spawned a child helper");
    };

    let started = Instant::now();
    let report = nanomd_term_shutdown::shut_down(&mut b, Duration::from_secs(1));
    let gone = wait_until(Duration::from_secs(2), || {
        !alive(shell_pid) && !alive(helper_pid)
    });
    // Belt and braces: never leak a helper on CI even when the assertion fails.
    if !gone {
        force_kill(helper_pid);
        force_kill(shell_pid);
    }
    eprintln!(
        "shutdown: shell {shell_pid} helper {helper_pid} gone after {:?}; report={report:?}",
        started.elapsed()
    );
    assert!(
        gone,
        "shell {shell_pid} alive={} helper {helper_pid} alive={} after {:?}; report={report:?}",
        alive(shell_pid),
        alive(helper_pid),
        started.elapsed()
    );
    assert!(
        report.acknowledged,
        "reader thread did not acknowledge within budget: {report:?}"
    );
}

fn first_child_of(pid: u32) -> Option<u32> {
    let out = if cfg!(windows) {
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "(Get-CimInstance Win32_Process -Filter 'ParentProcessId={pid}').ProcessId"
                ),
            ])
            .output()
            .unwrap()
    } else {
        std::process::Command::new("pgrep")
            .args(["-P", &pid.to_string()])
            .output()
            .unwrap()
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.trim().parse().ok())
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// `/T` kills the whole tree.
#[cfg(windows)]
fn force_kill(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status();
}

/// The shell is its own process-group leader (`setsid`), so signalling the
/// group is what reaches an orphaned helper; `killpg` on a non-leader pid just
/// returns ESRCH, so the plain `kill` covers the helper-pid callers.
#[cfg(unix)]
fn force_kill(pid: u32) {
    unsafe {
        libc::killpg(pid as i32, libc::SIGKILL);
        libc::kill(pid as i32, libc::SIGKILL);
    }
}

#[test]
fn scrollback_is_ten_thousand_lines_by_dependency_default() {
    // egui_term builds `term::Config::default()`; alacritty_terminal 0.26
    // hard-codes scrolling_history: 10000. nano.md uses that limit;
    // this pins it against a silent bump.
    assert_eq!(
        alacritty_terminal::term::Config::default().scrolling_history,
        10_000
    );
}
