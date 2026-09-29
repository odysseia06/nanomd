# Terminal pane: manual release checks

Run these checks on Windows, macOS, and Linux before tagging a release.
Record the commit, OS, machine, build command, and results with the release
notes. An automated test pass does not replace a manual rendering and input
check on each platform.

Build with `cargo build --locked --release --features cjk-font`. The CJK
checks need the bundled font. `tests/pty.rs` covers shell startup, working
directory, input, resize, session isolation, shutdown, and the scrollback cap.

## Interactive checks

- Open the pane with the toolbar and with Ctrl+Backtick (Cmd+Backtick on
  macOS). Confirm PowerShell starts on Windows and `$SHELL`, or `/bin/sh`,
  starts on Unix.
- Confirm a new shell starts in the open document's folder. With no file
  open, it should use the home directory, then the process working directory
  if no home directory is available.
- Hide and reopen the pane. The shell session should survive. Opening another
  document should not silently change an existing shell's directory.
- Drag the divider, resize the window, change display scale, and try the
  minimum window size. The terminal grid should resize and TUIs should reflow.
- Type `漢字 日本語 한글`, box-drawing characters, and emoji. Check alignment
  and missing glyphs. The bundled fonts do not cover every emoji.
- Click the terminal, then move the pointer outside without clicking. Typing
  should still reach the shell. Click the document to return focus there.
- Try a TUI with mouse reporting: clicks, drags, and wheel events should
  reach it. Shift+drag should select text locally.
- Check keyboard and context-menu Copy/Paste, including multiline and Unicode
  text. Use Ctrl+Shift+C/V on Windows/Linux and Cmd+C/V on macOS.
- Confirm scrolling retains up to 10,000 lines and older output is discarded.
- Check the ANSI palette, 256-color output, and truecolor in both app themes.
- With the terminal focused, check Ctrl+S/E/O/F/P/Q and Ctrl+Plus/Minus/0.
  They must not trigger document actions. On macOS, also check that Cmd+Q
  and Cmd+Plus/Minus/0 do not quit or zoom the app. The pane toggle should
  still work.
- Exit the shell and reopen the pane. A new shell should start with usable
  focus and no output from the old session.

## Performance

1. Start nano.md with `NANOMD_TERM_STATS=1` and a 900×700 window.
2. Run `scripts/term-perf.ps1` on Windows or `scripts/term-perf.sh` on Unix
   in the terminal pane. Each emits more than 5 MiB over roughly 30 seconds.
3. Type and scroll in the Markdown editor during the run.
4. Record p95, max, over250ms, and resident memory at the scrollback cap.
   Run the stream again and record the memory change.
5. Aim for p95 below 50 ms, no input stalls above 250 ms, and at most 20 MiB
   of memory growth after another 5 MiB at the scrollback cap.

The stats line samples consecutive frames that receive PTY events. It does
not measure every input event. `last_event_age_s` is the time since the last
received event, not a direct measurement of queue-drain latency. Verifying
that the queue drains within one second needs a separately recorded output
end time and final event time.

The previous Windows run (2026-09-29, i5-6500, release with `cjk-font`,
900×700) recorded 134 sampled frames, p95 17.1 ms, max 29.3 ms, and 0.2 MiB
additional working set at the scrollback cap. Typing and scrolling were
simulated. These numbers do not establish a full manual pass or validate
macOS and Linux.

## Shutdown

1. Note the shell PID (`$PID` in PowerShell, `echo $$` on Unix).
2. Start a foreground helper: `ping -t 127.0.0.1` on Windows or `sleep 9999`
   on Unix.
3. Close nano.md. Confirm the shell and foreground helper exit within two
   seconds.
4. Separately, open a preview link with no browser already running, then quit
   nano.md. The browser must remain open.

Windows uses a Job Object for the shell's process tree. Unix shutdown
signals the shell and foreground process groups; detached processes are
outside those groups.

## Automation hooks

- `NANOMD_TERM_STATS=1`: show frame statistics above the terminal.
- `NANOMD_TERM_OPEN=1`: open the terminal at startup.
- `NANOMD_TERM_CMD=<text>`: run a command when the terminal session starts.

These environment variables are for local test scripts. Set them only when
you intend to use them.
