# Linux archive walkthrough: Omarchy 4.0.4

Checked on 22 October 2026 for [issue #31](https://github.com/odysseia06/nanomd/issues/31), after the maintainer agreed to this distribution. This records one desktop session, not general Linux compatibility.

## Release and desktop

- Release: [v0.6.2](https://github.com/odysseia06/nanomd/releases/tag/v0.6.2), the latest published release when this check started.
- Archive: `nanomd-v0.6.2-x86_64-unknown-linux-gnu.tar.gz` (21,607,345 bytes).
- SHA-256: `bd2d43e8eb9a52efb6bc2da35170ee4e87d50d86eac539b8d92a2e1cf2e48fcd`. Matched its entry in the published `SHA256SUMS.txt` and GitHub's release-asset digest. This is integrity checking, not a code signature.
- Extracted binary SHA-256: `131bba47a70e60286456afd5590d1d8a7c3369fd6cf0beed77050075dd29a34f`; `nanomd --version` printed `nanomd 0.6.2`.
- Omarchy 4.0.4 (`omarchy 4.0.4-1`), x86-64, glibc 2.44, Hyprland 0.56.2, native Wayland session. The managed command environment did not set `XDG_SESSION_TYPE`; the Wayland display socket was present and the native window was observed under Hyprland.
- One AcePC AK2 desktop. No X11 session or other machine was tested.

I build ibara and used ibara 0.3.1-1 to operate the separate desktop with native input. I checked the saved files independently of input-delivery receipts. No accounts were connected and no agent/model command was run in nano.md's terminal.

## Download, verify and launch

The archive and checksum file were downloaded from the release URLs below. Extraction and launch used a task-local directory, with a private home and XDG directories. The following reproduces that arrangement with a fresh directory:

```sh
check_dir=$(mktemp -d)
cd "$check_dir"
archive=nanomd-v0.6.2-x86_64-unknown-linux-gnu.tar.gz
release=https://github.com/odysseia06/nanomd/releases/download/v0.6.2
curl -fL "$release/$archive" -o "$archive"
curl -fL "$release/SHA256SUMS.txt" -o SHA256SUMS.txt
grep " $archive$" SHA256SUMS.txt | sha256sum -c -
mkdir app
# The checked archive contains nanomd and a licenses directory.
tar -xzf "$archive" -C app
chmod +x app/nanomd
./app/nanomd --version
mkdir -p profile/home profile/config profile/cache profile/data profile/state evidence
cat > sample.md <<'MARKDOWN'
# Fictional project

Find marker: amber-bicycle-42.

External line: original.

Native edit line: original.
MARKDOWN
HOME="$check_dir/profile/home" \
XDG_CONFIG_HOME="$check_dir/profile/config" \
XDG_CACHE_HOME="$check_dir/profile/cache" \
XDG_DATA_HOME="$check_dir/profile/data" \
XDG_STATE_HOME="$check_dir/profile/state" \
SHELL=/bin/sh ./app/nanomd "$check_dir/sample.md"
```

The actual task used Python's HTTP downloader, SHA-256 calculation, and `tarfile.extractall(filter="data")` for these same URLs and contents, rather than the shell download/extraction commands above. Its private XDG directory names were `.config`, `.cache`, `.local/share` and `.local/state`. The launch used the same environment isolation and absolute document path. Existing provider credential environment variables were removed from the launched process. This isolates app preferences and the shell's home; it is not a filesystem or network sandbox.

No runtime packages were installed. `ldd` reported resolved `libgcc_s.so.1`, `libm.so.6`, `libc.so.6` and the loader. Both launches exited with status 0. No launch errors or missing-library messages were observed. Development-package requirements were not inferred from this result.

## Observed checks

| Check | Result | Observation |
| --- | --- | --- |
| Open the sample | Pass | The native preview showed the heading and all three fictional text lines. |
| Find | Pass | `Ctrl+F`, then `amber-bicycle-42`, showed one highlighted match and `1/1`. |
| Edit and save | Pass, with focus caveat below | `Ctrl+E`, click the source text, `Ctrl+End`, append `Native saved marker: violet-compass-73.` on its own line, then `Ctrl+S`. The marker was present in the saved bytes. |
| Close and reopen | Pass | Closed the native window and relaunched the same archive binary with the same private profile and file. The saved marker appeared in preview. |
| External edit and reload | Pass | With preview open, replaced `External line: original.` on disk with `External line: teal-orbit-91.`. Preview updated, displayed `1 block changed on reload`, and showed a margin marker beside the changed paragraph. |
| Terminal open and command | Pass | `Ctrl+T` opened `/bin/sh`. A benign `printf` command ran, and `pwd` was the document's directory. Output files independently confirmed the command and working directory. |
| Terminal hide and reopen | Pass | Set a shell variable, hid and reopened with `Ctrl+T` twice, then wrote the variable to a file. Its value survived, confirming the same shell session. |
| Native close | Pass, no timing bound claimed | Closed the window normally. Both launches returned 0; the recorded terminal shell PID was absent at the later check. This did not measure the checklist's two-second shutdown target. |

The first semantic click on the source text was refused by the input harness as a stale target before execution. Subsequent keyboard-only edit input did not reach the editor: the saved file remained unchanged. A click inside the source text, using a fresh window image, established focus and the edit/save check then passed. Do not treat `Ctrl+E` alone as proof that subsequent text input reached the editor. No source fix was attempted.

For the external-edit step, use another terminal in the same directory:

```sh
python3 - <<'PYTHON'
from pathlib import Path
p = Path('sample.md')
s = p.read_text()
assert 'External line: original.' in s
p.write_text(s.replace('External line: original.', 'External line: teal-orbit-91.'))
PYTHON
```

For a harmless terminal persistence check, enter this in nano.md's terminal, hide and reopen the pane, then enter the second line:

```sh
export NANOMD_TEST_PERSIST=violet-73
printf '%s\n' "$NANOMD_TEST_PERSIST" > evidence/terminal-persist.txt
```

Read the file from another terminal and check for `violet-73`. In this run, the native-saved file had SHA-256 `e5c6ce389b3a68928bb354753ef9f4ee31ca45268e215f24455a5125cba900d4`; after the external edit it was `d1a5943799480b80a91a8d2fbb431e2add60aae649868b38f08209a29cbfe984`. These hashes describe the exact run's fixture, not every possible edit or newline arrangement.

## Not tested

X11, other distributions or hardware, system registration, source builds, unsaved-edit conflict resolution, printing and external-link behavior were not tested. The broader [terminal checklist](../../scripts/terminal-checklist.md)—including TUIs, pointer reporting, clipboard/Unicode/CJK, divider/scale changes, performance and timed process-tree shutdown—was not completed. No compatibility or performance result is implied for those checks.

The report and three native window images were retained with before/after fictional files and provenance in a 351,504-byte evidence archive, SHA-256 `5eab5bc40681677d64a5b823efbc226199abde6f79964835a52ccb72d87e8cdc`. The desktop was released with zero application windows. No system settings, file associations or ordinary user files were changed.
