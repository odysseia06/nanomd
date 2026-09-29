//! Built-in terminal pane: session lifecycle, focus routing, layout,
//! palettes and shutdown. The widget itself is the vendored `egui_term`.
pub mod clipboard;
pub mod layout;
pub mod palette;
pub mod routing;
pub mod shutdown;
pub mod spawn;
pub mod stats;

use eframe::egui;
use egui_term::PtyEvent;
use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::Duration;

pub struct Session {
    pub id: u64,
    pub backend: egui_term::TerminalBackend,
    events: Receiver<(u64, PtyEvent)>,
    shut_down: bool,
    /// This session has produced output at least once, so the child has
    /// attached to the PTY.
    kicked: bool,
    /// One post-attach resize is owed to the PTY. ConPTY silently drops a
    /// resize sent before the child attaches (Task 7), and the backend's
    /// `resize` early-returns while `layout_size` is unchanged, so the
    /// view's per-frame resize cannot repair it on its own. The panel sends
    /// a deliberately-off-by-one size on the frame this is set; the view's
    /// own resize on that same frame then sends the true size.
    pub resize_kick_pending: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Drained {
    pub saw_output: bool,
    pub exited: bool,
}

/// Pure reducer: events tagged with a different session id are ignored,
/// so a late event from a dropped shell can never close a new one.
pub fn reduce(current_id: u64, events: impl IntoIterator<Item = (u64, PtyEvent)>) -> Drained {
    let mut d = Drained::default();
    for (id, ev) in events {
        if id != current_id {
            continue;
        }
        d.saw_output = true;
        if matches!(ev, PtyEvent::Exit) {
            d.exited = true;
        }
    }
    d
}

/// One-shot latch: true on the first drain of a session that saw output,
/// false ever after. Output means the child has attached to the PTY, which
/// is the earliest moment ConPTY will keep a resize (Task 7); repeating the
/// kick on every output frame would thrash the shell instead.
fn kick_on_first_output(kicked: &mut bool, saw_output: bool) -> bool {
    let owed = saw_output && !*kicked;
    *kicked |= saw_output;
    owed
}

pub struct Terminal {
    pub visible: bool,
    pub focused: bool,
    /// The pane was opened on this frame; the panel takes it and skips its
    /// click-outside test for one frame, because the toolbar click that
    /// opened the pane is still in the input queue and lands outside it.
    pub just_opened: bool,
    /// Requested pane height in points; `None` until first shown.
    pub height: Option<f32>,
    /// A drag is in flight or has just ended and its height is not saved
    /// yet. Consumed by `observe_height` on release.
    height_dirty: bool,
    /// A height whose save failed. Only `take_height_if_dirty` drains it, so
    /// the write is retried once at exit and never per frame.
    height_retry_at_exit: Option<f32>,
    next_id: u64,
    pub session: Option<Session>,
    #[cfg(windows)]
    job: Option<shutdown::JobGuard>,
    pub stats: Option<stats::Stats>,
}

impl Terminal {
    /// Scripted-probe hooks; no effect unless set. `NANOMD_TERM_OPEN=1`
    /// starts the pane visible and focused, so the first frame spawns a
    /// shell without a click. `NANOMD_TERM_CMD=<text>` makes [`spawn`]
    /// hand the shell args that run `<text>` at startup (read there, so it
    /// cannot change what a plain spawn sends when the variable is unset).
    ///
    /// [`spawn`]: Terminal::spawn
    pub fn new(saved_height: Option<f32>) -> Self {
        let open = std::env::var("NANOMD_TERM_OPEN").as_deref() == Ok("1");
        Terminal {
            visible: open,
            focused: open,
            just_opened: false,
            height: saved_height,
            height_dirty: false,
            height_retry_at_exit: None,
            next_id: 1,
            session: None,
            #[cfg(windows)]
            job: None,
            stats: stats::Stats::from_env(),
        }
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.focused = true;
        self.just_opened = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.focused = false;
    }

    pub fn toggle(&mut self) {
        if self.visible {
            self.close()
        } else {
            self.open()
        }
    }

    pub fn spawn(&mut self, ctx: &egui::Context, doc_path: Option<&Path>) -> Result<(), String> {
        if self.session.is_some() {
            return Ok(());
        }
        let cmd = std::env::var("NANOMD_TERM_CMD").ok();
        let settings = spawn::settings_for(doc_path, cmd.as_deref())?;
        self.spawn_with(ctx, settings)
    }

    pub fn spawn_with(
        &mut self,
        ctx: &egui::Context,
        settings: egui_term::BackendSettings,
    ) -> Result<(), String> {
        // Before the spawn: a job we cannot even create is a hard error, and
        // failing here costs nothing.
        #[cfg(windows)]
        if self.job.is_none() {
            self.job = Some(shutdown::JobGuard::new()?);
        }
        let id = self.next_id;
        self.next_id += 1;
        let (tx, rx) = std::sync::mpsc::channel();
        let backend = egui_term::TerminalBackend::new(id, ctx.clone(), tx, settings)
            .map_err(|e| e.to_string())?;
        #[cfg(windows)]
        if let Some(job) = self.job.as_ref()
            && let Err(e) = job.assign(backend.pty_id())
        {
            // A shell we cannot promise to clean up must not be kept: the
            // backend's drop requests shutdown, and the caller banners `e`.
            drop(backend);
            return Err(e);
        }
        self.session = Some(Session {
            id,
            backend,
            events: rx,
            shut_down: false,
            kicked: false,
            resize_kick_pending: false,
        });
        Ok(())
    }

    /// Every frame, including while hidden. A dead reader thread counts as
    /// an exit via the backend's `reader_finished` signal.
    pub fn drain(&mut self) -> Drained {
        let Some(s) = self.session.as_mut() else {
            return Drained::default();
        };
        let mut buf = Vec::new();
        while let Ok(ev) = s.events.try_recv() {
            buf.push(ev);
        }
        let mut d = reduce(s.id, buf);
        if s.backend.reader_finished() {
            d.exited = true;
        }
        s.resize_kick_pending |= kick_on_first_output(&mut s.kicked, d.saw_output);
        if let Some(st) = self.stats.as_mut() {
            st.tick(d.saw_output);
        }
        if d.exited {
            self.session = None;
            self.close();
        }
        d
    }

    /// Track the height the pane is actually shown at, and return it once
    /// when a *user-chosen* height is ready to persist. Only a change made
    /// while the pointer is down counts as chosen: the pane also follows the
    /// window when it is resized or clamped, and none of that is a
    /// preference worth writing to disk on every frame.
    pub fn observe_height(&mut self, shown: f32, pointer_down: bool) -> Option<f32> {
        if self.height != Some(shown) {
            self.height = Some(shown);
            self.height_dirty |= pointer_down;
        }
        if self.height_dirty && !pointer_down {
            self.height_dirty = false;
            return Some(shown);
        }
        None
    }

    /// Remember `height` for a single retry at exit after its save failed.
    /// Deliberately not `height_dirty`: that flag is exactly what
    /// `observe_height` releases on, so re-arming it would mean one failing
    /// write and one fresh banner per repaint.
    pub fn mark_height_dirty(&mut self, height: f32) {
        self.height_retry_at_exit = Some(height);
    }

    /// The one retry, for `on_exit`. A drag that never got released wins
    /// over an older failed write, and either way the value is handed out
    /// once.
    pub fn take_height_if_dirty(&mut self) -> Option<f32> {
        if self.height_dirty {
            self.height_dirty = false;
            self.height_retry_at_exit = None;
            return self.height;
        }
        self.height_retry_at_exit.take()
    }

    pub fn shutdown(&mut self, budget: Duration) -> Option<shutdown::Report> {
        let s = self.session.as_mut()?;
        if s.shut_down {
            return None;
        }
        s.shut_down = true;
        Some(shutdown::shut_down(&mut s.backend, budget))
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Backstop: on_exit normally ran first and made this a no-op.
        let _ = self.shutdown(Duration::from_millis(1000));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_term::PtyEvent;

    #[test]
    fn reduce_ignores_events_from_other_sessions() {
        let d = reduce(7, vec![(3, PtyEvent::Exit), (3, PtyEvent::Wakeup)]);
        assert!(!d.exited);
        assert!(!d.saw_output, "stale events are not output either");
    }

    #[test]
    fn reduce_reports_output_and_exit_for_the_current_session() {
        let d = reduce(7, vec![(7, PtyEvent::Wakeup), (7, PtyEvent::Exit)]);
        assert!(d.saw_output);
        assert!(d.exited);
        let d = reduce(7, vec![(7, PtyEvent::Wakeup)]);
        assert!(d.saw_output && !d.exited);
    }

    #[test]
    fn the_resize_kick_is_owed_once_per_session_on_first_output() {
        let mut kicked = false;
        assert!(
            !kick_on_first_output(&mut kicked, false),
            "no output yet: the child has not attached"
        );
        assert!(kick_on_first_output(&mut kicked, true), "first output");
        assert!(!kick_on_first_output(&mut kicked, true), "already kicked");
        assert!(!kick_on_first_output(&mut kicked, false));
        // A fresh session starts the latch over.
        let mut kicked = false;
        assert!(kick_on_first_output(&mut kicked, true));
    }

    #[test]
    fn open_close_toggle_drive_visibility_and_focus_together() {
        let mut t = Terminal::new(None);
        assert!(!t.visible && !t.focused, "pane starts hidden every launch");
        t.toggle();
        assert!(t.visible && t.focused);
        t.focused = false; // user clicked the editor
        t.toggle();
        assert!(!t.visible && !t.focused);
        t.open();
        assert!(t.visible && t.focused);
    }

    #[test]
    fn opening_flags_the_frame_so_the_opening_click_cannot_unfocus_the_pane() {
        let mut t = Terminal::new(None);
        assert!(!t.just_opened);
        t.open();
        assert!(t.just_opened);
        t.just_opened = false; // the panel consumed it
        t.close();
        t.toggle(); // hidden -> shown, the toolbar button's path
        assert!(t.just_opened, "toggling the pane open must flag it too");
    }

    #[test]
    fn height_persists_after_a_drag_ends_not_per_frame() {
        let mut t = Terminal::new(Some(200.0));
        assert_eq!(
            t.observe_height(200.0, false),
            None,
            "unchanged: nothing to save"
        );
        assert_eq!(t.observe_height(260.0, true), None, "dragging: not yet");
        assert_eq!(t.observe_height(262.0, true), None);
        assert_eq!(
            t.observe_height(262.0, false),
            Some(262.0),
            "drag ended: save once"
        );
        assert_eq!(t.observe_height(262.0, false), None);
        assert_eq!(t.take_height_if_dirty(), None);
        // The window shrinks (or the clamp bites) with the pointer up: the
        // pane follows, but the user chose nothing, so nothing is saved.
        assert_eq!(t.observe_height(180.0, false), None);
        assert_eq!(t.observe_height(150.0, false), None);
        assert_eq!(t.take_height_if_dirty(), None, "no drag, nothing to save");
        t.observe_height(300.0, true);
        assert_eq!(
            t.take_height_if_dirty(),
            Some(300.0),
            "orderly shutdown saves an unsaved drag"
        );
        // A failed write is retried once, at exit — not once per repaint.
        t.observe_height(320.0, true);
        assert_eq!(t.observe_height(320.0, false), Some(320.0), "one attempt");
        t.mark_height_dirty(320.0);
        for _ in 0..3 {
            assert_eq!(
                t.observe_height(320.0, false),
                None,
                "a failed save must not be retried on every frame"
            );
        }
        assert_eq!(
            t.take_height_if_dirty(),
            Some(320.0),
            "on_exit retries the failed write"
        );
        assert_eq!(t.take_height_if_dirty(), None, "exactly once");
    }

    #[test]
    fn spawn_failure_leaves_no_session_and_next_ids_advance() {
        let mut t = Terminal::new(None);
        let ctx = eframe::egui::Context::default();
        let bogus = std::path::Path::new("/definitely/missing/dir/x.md");
        // Force every cwd candidate invalid by pointing HOME nowhere is not
        // portable; instead exercise the shell-missing path.
        let err = t
            .spawn_with(
                &ctx,
                egui_term::BackendSettings {
                    shell: "nanomd-no-such-shell-xyz".into(),
                    args: vec![],
                    working_directory: Some(std::env::temp_dir()),
                },
            )
            .unwrap_err();
        assert!(!err.is_empty());
        assert!(t.session.is_none());
        let _ = bogus;
        assert_eq!(
            t.next_id, 2,
            "a failed spawn still burns its id so late events can never match"
        );
    }

    #[test]
    fn shutdown_is_idempotent_without_a_session() {
        let mut t = Terminal::new(None);
        assert!(t.shutdown(Duration::from_millis(10)).is_none());
        assert!(t.shutdown(Duration::from_millis(10)).is_none());
    }

    /// Real shell, no window — same trade as `tests/pty.rs`.
    #[test]
    fn shutdown_is_idempotent_with_a_live_session() {
        let mut t = Terminal::new(None);
        let ctx = eframe::egui::Context::default();
        t.spawn_with(&ctx, spawn::settings_for(None, None).unwrap())
            .unwrap();
        let r = t
            .shutdown(Duration::from_secs(1))
            .expect("the first call reports");
        assert!(
            r.acknowledged,
            "reader did not acknowledge in budget: {r:?}"
        );
        assert!(
            t.shutdown(Duration::from_secs(1)).is_none(),
            "a second call is a no-op, not a second teardown"
        );
        assert!(
            t.session.is_some(),
            "shutdown does not drop the session; drain sees the exit and does"
        );
    }

    #[test]
    fn dead_reader_surfaces_as_exit_and_hides_the_pane() {
        let mut t = Terminal::new(None);
        let ctx = eframe::egui::Context::default();
        let mut settings = spawn::settings_for(None, None).unwrap();
        // A shell that exits at once: the reader thread dies without anyone
        // asking it to, which is the path `reader_finished` exists for.
        settings.args = if cfg!(windows) {
            vec!["-NoProfile".into(), "-Command".into(), "exit".into()]
        } else {
            vec!["-c".into(), "exit".into()]
        };
        t.spawn_with(&ctx, settings).unwrap();
        t.open();
        let start = std::time::Instant::now();
        let mut exited = false;
        // Warm, the exit lands in ~200 ms. The first ConPTY + powershell.exe
        // launch on a fresh hosted Windows runner, alongside the rest of the
        // suite, took over 5 s; the loop breaks on exit, so a long budget
        // costs nothing when it passes.
        while start.elapsed() < Duration::from_secs(30) {
            if t.drain().exited {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(exited, "the shell exited but drain never reported it");
        assert!(t.session.is_none(), "an exit drops the session");
        assert!(!t.visible && !t.focused, "an exit hides the pane too");
    }
}
