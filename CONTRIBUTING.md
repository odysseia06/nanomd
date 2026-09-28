# Contributing

nano.md is for opening, reading, and editing Markdown files, often while an
agent works on them. Bug reports, documentation fixes, and small improvements
are welcome. For a larger feature, open an issue first so we can discuss
whether it fits.

## Build and test

Use stable Rust and your platform's native build tools. On Windows, install
Visual Studio C++ Build Tools and use Rust's MSVC toolchain. On macOS,
install the Xcode Command Line Tools.

On Ubuntu 22.04, CI installs these packages:

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libxkbcommon-dev libwayland-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
```

From the repository root:

```sh
cargo build --locked
cargo test --locked
cargo run --locked --features cjk-font -- sample.md
```

The `cjk-font` feature includes the bundled Chinese, Japanese, and Korean
font. Release builds enable it; default builds leave it out to reduce size.
Build from this checkout: publishing to crates.io is not currently supported
because nano.md depends on a patched local `egui_term` and a Git revision of
`egui_commonmark`.

Some Windows save/recovery tests return early when `CI` is set because they
depend on filesystem permissions and shell behavior. A green hosted Windows
run doesn't cover those cases. Run the suite locally as well. Set
`NANOMD_RUN_WINDOWS_SAVE_TESTS=1` to run them under CI anyway; `ci.yml` does
this in a non-blocking step on Windows. Two symlink tests need permission to
create symbolic links (Developer Mode or an elevated shell); without it they
skip themselves.

## Before opening a PR

```sh
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --manifest-path vendor/egui_term/Cargo.toml --lib
cargo build --locked --release --features cjk-font
```

These are also the CI checks on Linux, Windows, and macOS. Keep each PR
focused on one change, and add user-visible changes to [CHANGELOG.md](CHANGELOG.md).
The [terminal checklist](scripts/terminal-checklist.md) covers manual checks
that the automated suite cannot verify.

`sample.md` and `examples/copy-check.md` are rendering fixtures. The
`frame_bench` and `hl_check` Cargo examples help investigate rendering cost
and syntax highlighting.

## Licensing

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as MIT OR Apache-2.0, without any
additional terms or conditions.
