//! Opt-in frame statistics, enabled with `NANOMD_TERM_STATS=1`.
//! The report includes the age of the last received PTY event. Measuring
//! queue-drain latency also requires knowing when output stopped.
//!
//! A frame delta is recorded only when this frame's PTY-event drain and the
//! previous frame's drain both saw >= 1 event — see `tick` for why.
use std::time::Instant;

pub struct Stats {
    pub recording: bool,
    frames: Vec<f32>,
    last_frame: Option<Instant>,
    prev_saw_output: bool,
    /// When a drain last saw a PTY event. `report` prints its age.
    last_event: Option<Instant>,
}

impl Stats {
    pub fn new() -> Self {
        Stats {
            recording: false,
            frames: Vec::new(),
            last_frame: None,
            prev_saw_output: false,
            last_event: None,
        }
    }

    /// `Some` (with `recording: true`) only when the operator opted in via
    /// `NANOMD_TERM_STATS=1`; `None` otherwise so the app carries no
    /// instrumentation overhead by default.
    pub fn from_env() -> Option<Stats> {
        if std::env::var("NANOMD_TERM_STATS").as_deref() == Ok("1") {
            let mut s = Stats::new();
            s.recording = true;
            Some(s)
        } else {
            None
        }
    }

    /// Call once per update() frame, after the PTY-event drain, passing
    /// whether that drain received >= 1 PTY event this frame
    /// (`saw_output_now`). Records a frame delta only when `recording` is
    /// on and both this frame and the previous frame saw output: a stall
    /// during streaming still leaves events queued, so the stalled frame's
    /// drain sees output and the previous frame did too, and the delta is
    /// recorded. The frame after output stops sees nothing (excluded); the
    /// first frame after output resumes has `prev_saw_output == false`
    /// (excluded). So idle-poll and output-gap frames never masquerade as
    /// stalls, while real stalls are kept.
    pub fn tick(&mut self, saw_output_now: bool) {
        let now = Instant::now();
        if let Some(prev) = self.last_frame.replace(now)
            && self.recording
            && self.prev_saw_output
            && saw_output_now
        {
            self.frames.push(now.duration_since(prev).as_secs_f32());
        }
        self.prev_saw_output = saw_output_now;
        if saw_output_now {
            self.last_event = Some(now);
        }
    }

    pub fn report(&self) -> String {
        let mut f = self.frames.clone();
        if f.is_empty() {
            return "no frames recorded".into();
        }
        f.sort_by(|a, b| a.total_cmp(b));
        let pct = |p: f32| f[(((f.len() as f32) * p) as usize).min(f.len() - 1)] * 1000.0;
        format!(
            "frames={} p50={:.1}ms p90={:.1}ms p95={:.1}ms p99={:.1}ms max={:.1}ms \
             over50ms={} over250ms={} last_event_age_s={:.1}",
            f.len(),
            pct(0.50),
            pct(0.90),
            pct(0.95),
            pct(0.99),
            f.last().unwrap() * 1000.0,
            f.iter().filter(|d| **d > 0.050).count(),
            f.iter().filter(|d| **d > 0.250).count(),
            // -1 = no PTY event has ever been drained.
            self.last_event.map_or(-1.0, |t| t.elapsed().as_secs_f32()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    /// Pulls one `key=value` field out of `Stats::report`'s output.
    fn field<'a>(report: &'a str, key: &str) -> &'a str {
        let after = report.split(key).nth(1).unwrap();
        after.split([' ', 'm']).next().unwrap()
    }

    #[test]
    fn report_records_a_stall_via_tick_but_not_idle_or_gap_frames() {
        let mut stats = Stats::new();
        stats.recording = true;

        // Idle-poll frame before anything streams: no prev frame yet, so no
        // delta either way.
        stats.tick(false);
        // Prime prev_saw_output=true with no measurable delta.
        stats.tick(true);
        // 20 fast frames while output streams continuously.
        for _ in 0..20 {
            sleep(Duration::from_millis(10));
            stats.tick(true);
        }
        // A stall: the drain still sees output this frame and saw it last
        // frame too, so this delta must be recorded despite the long gap.
        sleep(Duration::from_millis(300));
        stats.tick(true);
        // Output stops: this frame's drain is empty, so no delta is
        // recorded even though real time passed.
        sleep(Duration::from_millis(300));
        stats.tick(false);

        let report = stats.report();
        assert_eq!(field(&report, "frames="), "21", "report: {report}");
        assert_eq!(field(&report, "over250ms="), "1", "report: {report}");
        let max_ms: f32 = field(&report, "max=").parse().unwrap();
        // >= the requested sleep, bounded loosely to tolerate scheduler
        // jitter on slow CI runners.
        assert!((295.0..500.0).contains(&max_ms), "max_ms={max_ms}");
        let p95_ms: f32 = field(&report, "p95=").parse().unwrap();
        // The one 300ms outlier must not pollute p95 across 21 samples.
        assert!(p95_ms < 250.0, "p95_ms={p95_ms}");
        // Perf target 3's readout: output stopped ~300ms ago (the last tick
        // that saw output was one 300ms sleep back).
        let age: f32 = field(&report, "last_event_age_s=").parse().unwrap();
        assert!((0.2..1.5).contains(&age), "report: {report}");
    }
}
