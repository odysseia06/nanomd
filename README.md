# <img src="assets/icon.svg" alt="" width="32" height="32" align="absmiddle"> nano.md

A small desktop app for reading and editing Markdown files.

[![CI](https://github.com/odysseia06/nanomd/actions/workflows/ci.yml/badge.svg)](https://github.com/odysseia06/nanomd/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

LLMs keep leaving me with Markdown files: plans, specs, reviews, notes. I
built nano.md because I wanted to open those files quickly, read through
them, and change a few things. Sometimes I'd rather ask the agent to review
or revise the file, so there's a terminal pane for that too.

Open a `.md` file and it shows up rendered. Press `Ctrl+E` to edit the source,
or open the terminal and work with your agent while keeping the document in
view. When the file changes on disk, the preview updates.

![nano.md showing a Markdown document alongside its terminal pane](assets/hero.gif)

Written in Rust with egui. Runs on Windows, macOS, and Linux. No account,
telemetry, or built-in AI service; you use whichever agent you already have
installed.

## Install

The first release is still being prepared. For now, [build from source](#build-from-source).
Packaged downloads will be listed on the [Releases page](https://github.com/odysseia06/nanomd/releases).

The release workflow packages these builds:

- **Windows (x86-64):** unzip and run `nanomd.exe`. The executable is unsigned,
  so SmartScreen may show a warning on first launch.
- **macOS (Apple Silicon and Intel):** extract the matching `aarch64` or
  `x86_64` archive and move `nanomd.app` to Applications. The app is not
  Developer ID signed or notarized. If macOS blocks it, follow
  [Apple's instructions for opening an unidentified app](https://support.apple.com/en-us/102445).
- **Linux (x86-64):** extract and run `nanomd`. The build targets Ubuntu 22.04
  and needs glibc 2.35 or newer, plus an X11 or Wayland desktop.

Releases include `SHA256SUMS.txt`; see [how to verify a download](SECURITY.md#verifying-a-download).

### Build from source

Install stable Rust and your platform's build tools. On Windows, use the
MSVC toolchain with Visual Studio C++ Build Tools; on macOS, install the
Xcode Command Line Tools. Linux packages are listed in
[CONTRIBUTING.md](CONTRIBUTING.md#build-and-test).

```sh
git clone https://github.com/odysseia06/nanomd.git
cd nanomd
cargo build --locked --release --features cjk-font
```

The executable is `target/release/nanomd` (`nanomd.exe` on Windows). The
`cjk-font` feature bundles a font for Chinese, Japanese, and Korean text in
the preview and terminal. Omit it for a smaller build if you don't need those
characters.

## Open a file

```sh
nanomd plan.md
```

You can also use `Ctrl+O`, drag a file onto the window, or associate `.md`
files with nano.md and double-click them. Starting without a file opens an
empty document. The Open menu keeps a list of recent files.

To make nano.md your default Markdown app:

- **Windows:** right-click a `.md` file, choose *Open with*, browse to
  `nanomd.exe`, and select *Always*.
- **macOS:** select a `.md` file in Finder, then *Get Info → Open with →
  nano.md → Change All…*.

## Work with an agent

Press `` Ctrl+` `` to open the terminal pane, then start `claude`, `codex`,
`aider`, or whichever CLI you use. The shell starts in the open file's folder.
You can ask the agent to review the document, rewrite a section, or make a
change while you read along.

If the agent changes the file while you have unsaved edits, nano.md offers
**Reload** or **Keep mine**. Reload uses the version on disk; Keep mine keeps
your edits for the next save. Saving also checks whether the file has changed
since you loaded it.

The terminal runs PowerShell on Windows and `$SHELL` (falling back to
`/bin/sh`) elsewhere. It keeps its session when hidden; opening another
document doesn't change an existing shell's working directory. Right-click
for Copy/Paste, or use `Ctrl+Shift+C` / `Ctrl+Shift+V` (`Cmd+C` / `Cmd+V` on
macOS).

## Shortcuts

Use `Cmd` instead of `Ctrl` on macOS.

| Shortcut | Action |
|----------|--------|
| `Ctrl+E` | Switch between preview and source editing |
| `Ctrl+S` | Save; choose a filename for an untitled document |
| `Ctrl+O` | Open a file |
| `Ctrl+F` | Find in the source; Enter / Shift+Enter goes to the next / previous match |
| `Ctrl+P` | Print or save as PDF using your browser's print dialog |
| `Ctrl+Z` | Undo in edit mode |
| `` Ctrl+` `` | Show or hide the terminal |

When the terminal has focus, shortcuts go to the shell except for the
terminal toggle. Click the document to use its shortcuts again.

A dot in the title bar means there are unsaved edits. Closing the window or
opening another file asks whether to save, discard, or cancel.

## A few details

- One file per window, with a rendered preview and a plain-text editor.
- Local images render relative to the document's folder. HTTP(S) images
  appear as links instead of being downloaded.
- Print includes unsaved edits. It creates a local HTML file and opens it
  in your browser; see [privacy details](SECURITY.md#local-files-and-printing).
- Files that aren't valid UTF-8 open with a warning. Saving writes UTF-8.
- `nanomd --version` prints the version. In a Windows terminal, pipe it to
  see the output: `nanomd --version | more`.

I'd like to keep this focused on opening, reading, and editing individual
Markdown files. WYSIWYG editing and a plugin system are outside that scope.
Bug reports and ideas are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE), at your option.

The bundled [Noto Sans Mono CJK font](assets/LICENSE-OFL-NotoSansCJK.txt) uses
the SIL Open Font License 1.1. The terminal uses a modified copy of
[egui_term](vendor/egui_term/README.md) (MIT), built on `alacritty_terminal`
(Apache-2.0).

See [font and icon licenses](assets/LICENSES.md) for the other bundled assets.
