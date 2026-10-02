# Security

## Reporting a vulnerability

Please report security issues privately to floydianayhan@gmail.com. You can
also use GitHub's *Report a vulnerability* button if it is available on the
repository's Security tab. Include the affected version and enough detail
to reproduce the issue. Please avoid posting sensitive files in a public issue.

Only the latest release receives security fixes.

## Network behavior

nano.md has no telemetry, update checks, crash reporting, or built-in AI
service. The Markdown renderer does not fetch HTTP(S) images; it displays
them as links. Clicking a link opens it through the operating system's
default handler.

Processes started in the terminal pane can access the network and local
files with your user permissions. The pane is a regular shell, not a sandbox.
Browsers opened for links or printing have their own network behavior too.

## Local files and printing

Documents and relative images are read from disk. Paths on network shares
may involve network access through the operating system. Desktop libraries
may also use local sockets to communicate with the display server or file
dialogs; this is separate from fetching remote content.

Printing writes the current document, including unsaved edits, to a
`nanomd-print-*.html` file in the system temporary directory and opens it in
your browser. nano.md does not remove that file after printing. For sensitive
documents, remove the temporary copy when you no longer need it. Raw HTML in
the Markdown is displayed as text in the print page.

The app stores recent file paths and where each was left (its view and
scroll position), the selected theme and reading size, the window's size and
position, and the terminal pane height under `%APPDATA%\nanomd` on Windows,
or `$XDG_CONFIG_HOME/nanomd` (falling back to `~/.config/nanomd`) on macOS
and Linux.

Mermaid diagrams are drawn by the bundled renderer, which reads the system
fonts and keeps a copy of the font it picks in `~/.cache/mmdr/font-cache`
when `HOME` or `XDG_CACHE_HOME` is set (always on macOS and Linux).

## Verifying a download

Release archives are accompanied by `SHA256SUMS.txt`. Compare the checksum
for the archive you downloaded:

```sh
sha256sum nanomd-<version>-<target>.tar.gz        # Linux
shasum -a 256 nanomd-<version>-<target>.tar.gz    # macOS
```

```powershell
Get-FileHash .\nanomd-<version>-<target>.zip -Algorithm SHA256
```

Replace `<version>` and `<target>` with the downloaded filename's values.
The hash must match that file's entry in `SHA256SUMS.txt`. This checks file
integrity; it is not a code signature.

The release workflow does not Developer ID sign or notarize macOS builds,
or sign Windows builds. SmartScreen or Gatekeeper may warn on first launch.
