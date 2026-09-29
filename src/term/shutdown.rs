//! Bounded, idempotent terminal session teardown.
//!
//! Windows: each spawned shell is assigned to a kill-on-close Job Object,
//! so processes in that job are terminated when nano.md exits. The job
//! handle is kept open until process exit, when the kernel closes it.
//! Unix: SIGHUP → SIGTERM to the shell's process group and to the
//! foreground group captured with `tcgetpgrp` while the PTY is still open,
//! then SIGKILL to survivors after the polite phase.
//!
//! This file is both a module of the binary (`term::shutdown`) and the root
//! of a tiny library target, so the integration tests can drive it; it must
//! therefore never reach outside itself with a `crate::` path.
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub acknowledged: bool,
    pub elapsed: Duration,
}

/// Polite phase is 75 % of the budget, capped at 750 ms; the rest is the
/// force phase. Integer maths, so the split is exact (and `/ 4` first cannot
/// overflow the way `* 3` could).
pub fn phases(budget: Duration) -> (Duration, Duration) {
    let polite = (budget / 4 * 3).min(Duration::from_millis(750));
    (polite, budget.saturating_sub(polite))
}

/// A kill-on-close Job Object that holds the spawned shells — and nothing
/// else. The handle is deliberately never closed by us: the kernel closes
/// it at process exit, and that close is what kills any survivor.
///
/// The app process is intentionally *not* a member. If it were, every later
/// child of nano.md would die with it, including a browser cold-started by
/// eframe's `links` feature when the user follows a preview link.
#[cfg(windows)]
pub struct JobGuard(pub windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl JobGuard {
    /// Create the job. Call once, then [`assign`](JobGuard::assign) every
    /// shell you spawn.
    pub fn new() -> Result<JobGuard, String> {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(format!(
                    "CreateJobObjectW failed (error {})",
                    GetLastError()
                ));
            }
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                let e = GetLastError();
                CloseHandle(job);
                return Err(format!("SetInformationJobObject failed (error {e})"));
            }
            Ok(JobGuard(job))
        }
    }

    /// Put `pid` — and everything it spawns afterwards — into the job.
    ///
    /// Alacritty's ConPTY layer creates the shell before this assignment,
    /// so assignment is not atomic with process creation. Scoping the job
    /// to the shell lets unrelated processes, such as a browser opened
    /// from a preview link, outlive the app.
    pub fn assign(&self, pid: u32) -> Result<(), String> {
        use windows_sys::Win32::Foundation::{CloseHandle, FALSE, GetLastError};
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
        };
        unsafe {
            let proc = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, FALSE, pid);
            if proc.is_null() {
                return Err(format!(
                    "OpenProcess({pid}) failed (error {}); terminal cannot guarantee cleanup",
                    GetLastError()
                ));
            }
            let assigned = AssignProcessToJobObject(self.0, proc) != 0;
            let e = GetLastError();
            CloseHandle(proc);
            if !assigned {
                return Err(format!(
                    "AssignProcessToJobObject failed (error {e}); terminal cannot guarantee cleanup"
                ));
            }
            Ok(())
        }
    }
}

/// Idempotent: a second call finds the reader already finished and returns
/// immediately with `acknowledged = true`.
pub fn shut_down(backend: &mut egui_term::TerminalBackend, budget: Duration) -> Report {
    let start = Instant::now();
    let (polite, force) = phases(budget);
    #[cfg(unix)]
    let groups = unix::capture_groups(backend);
    backend.request_shutdown(); // reader thread drops the PTY: ConPTY close / SIGHUP+wait
    #[cfg(unix)]
    unix::signal(&groups, libc::SIGHUP);
    #[cfg(unix)]
    unix::signal(&groups, libc::SIGTERM);
    let mut acknowledged = backend.wait_reader(polite);
    if !acknowledged {
        #[cfg(unix)]
        unix::signal(&groups, libc::SIGKILL);
        // Windows: nothing polite is left to do; the Job Object kills any
        // survivor when the process exits.
        acknowledged = backend.wait_reader(force);
    }
    Report {
        acknowledged,
        elapsed: start.elapsed(),
    }
}

#[cfg(unix)]
mod unix {
    pub struct Groups {
        pub shell: libc::pid_t,
        pub foreground: Option<libc::pid_t>,
    }

    /// The shell was spawned with `setsid`, so its pid is its process
    /// group; the foreground group is whatever `tcgetpgrp` reports on the
    /// master while it is still open (a job-control shell puts a running
    /// helper in its own group).
    pub fn capture_groups(backend: &egui_term::TerminalBackend) -> Groups {
        let shell = backend.pty_id() as libc::pid_t;
        let fg = unsafe { libc::tcgetpgrp(backend.master_fd()) };
        Groups {
            shell,
            foreground: (fg > 0 && fg != shell).then_some(fg),
        }
    }

    pub fn signal(groups: &Groups, sig: libc::c_int) {
        unsafe {
            libc::killpg(groups.shell, sig);
            if let Some(fg) = groups.foreground {
                libc::killpg(fg, sig);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_split_gives_polite_phase_750ms_and_never_exceeds_total() {
        let (polite, force) = phases(Duration::from_millis(1000));
        assert_eq!(polite, Duration::from_millis(750));
        assert_eq!(force, Duration::from_millis(250));
        let (polite, force) = phases(Duration::from_millis(400));
        assert_eq!(polite, Duration::from_millis(300));
        assert_eq!(force, Duration::from_millis(100));
    }

    #[cfg(windows)]
    #[test]
    fn job_assigns_a_child_and_kills_it_on_close() {
        /// Never leak the probe onto CI, whatever the assertions do.
        struct Reaper(std::process::Child);
        impl Drop for Reaper {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let job = JobGuard::new().unwrap();
        // `cmd` outlives the assignment and then forks `ping`, so this also
        // covers "descendants spawned later are in the job too".
        let mut child = Reaper(
            std::process::Command::new("cmd")
                .args(["/c", "ping", "-n", "30", "127.0.0.1"])
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        job.assign(child.0.id()).unwrap();
        // The one place we close the handle: production leaves that to the
        // kernel at process exit, which is exactly what this simulates.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(job.0) };
        let start = Instant::now();
        let mut gone = false;
        while start.elapsed() < Duration::from_secs(2) {
            if child.0.try_wait().unwrap().is_some() {
                gone = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(gone, "closing the job did not kill the assigned child");
    }
}
