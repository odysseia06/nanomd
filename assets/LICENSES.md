# Font and icon licenses

The release binary includes these fonts and icons. Their license notices
are copied into each release archive's `licenses` directory (inside
`nanomd.app/Contents/Resources` on macOS).

| Asset | Source | Notice |
|-------|--------|--------|
| Noto Sans Mono CJK | `NotoSansMonoCJKsc-Regular.otf` in this directory, enabled by `cjk-font` | [OFL 1.1](LICENSE-OFL-NotoSansCJK.txt) |
| STIX Two Math | Bundled by `latex-rust` 2.1.0 | [OFL 1.1](LICENSE-OFL-STIXTwoMath.txt) |
| Hack | `epaint_default_fonts` 0.36.1 | [MIT and Bitstream Vera](LICENSE-Hack.txt) |
| Noto Emoji | `epaint_default_fonts` 0.36.1 | [OFL 1.1](LICENSE-OFL-NotoEmoji.txt) |
| Ubuntu Light | `epaint_default_fonts` 0.36.1 | [Ubuntu Font License](LICENSE-Ubuntu.txt) |
| Emoji Icon Font | `epaint_default_fonts` 0.36.1 | [MIT](LICENSE-MIT-EmojiIcons.txt) |
| Phosphor Regular icons | `egui-phosphor` 0.14.0; [upstream license](https://github.com/phosphor-icons/web/blob/master/LICENSE) | [MIT](LICENSE-MIT-Phosphor.txt) |

The egui font notices were copied unchanged from the pinned
`epaint_default_fonts` crate. Recheck them when updating that dependency.

LaTeX-Rust is Copyright 2026 Jeffrey S Carr, licensed under
[MIT](LICENSE-MIT-LaTeX-Rust.txt) OR Apache-2.0. It embeds STIX Two Math 2.13.
STIX Fonts is a trademark of The Institute of Electrical and Electronics
Engineers, Inc.
