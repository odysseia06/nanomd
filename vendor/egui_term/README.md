# egui_term as used by nano.md

This is a modified copy of [Harzu/egui_term](https://github.com/Harzu/egui_term)
at commit [`31bbc7ab8503c9518fcee5717cfa29011e59f451`](https://github.com/Harzu/egui_term/tree/31bbc7ab8503c9518fcee5717cfa29011e59f451).
The original author is Ilya Shvyryalkin; the [MIT license](LICENSE) is preserved.

nano.md keeps the library source and its build configuration. Upstream's
standalone demos, demo fonts and screenshots, and GitHub workflows are
omitted. See the upstream repository for those examples.

## Local changes

- Use egui 0.36.1 and omit the example workspace dependencies.
- Respect terminal cursor visibility.
- Keep keyboard input working when the widget has focus but the pointer
  moves away; route pointer input by hover.
- Send Ctrl-letter control bytes and let Shift override TUI mouse reporting
  for local selection.
- Stop the event subscription cleanly on disconnect, expose reader shutdown
  completion and the Unix PTY descriptor, and export the terminal size type.

The complete source and manifest changes are in
[`../patches/egui_term.patch`](../patches/egui_term.patch). Apply that file
**once** to a clean checkout of the pinned upstream commit. It replaces the
old overlapping patch files; it does not remove upstream's demo files or
regenerate its lockfile.

From the nano.md repository root, test this copy with:

```sh
cargo test --locked --manifest-path vendor/egui_term/Cargo.toml --lib
```

The separate lockfile is based on nano.md's dependency versions, with unused
packages removed. When updating the vendored library, update the pin and
patch together and run both the library tests and nano.md's tests.
