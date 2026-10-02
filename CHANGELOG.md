# Changelog

## Unreleased

## 0.4.0 — 2026-10-02

See what your agent changed.

- When the open file changes on disk, the headings, paragraphs, list items
  and code blocks that differ from what was on screen get a bar in the
  margin, in the preview and the editor; a bar above the document counts
  them and the removed blocks, and its arrows step through the changes

## 0.3.0 — 2026-10-02

Pick up where you left off.

- The window keeps its size and position between sessions, and a recent
  file reopens at the same scroll position, in the view it was left in

## 0.2.0 — 2026-10-02

Easier reading of long plans and specs.

- `Ctrl+F` in the preview searches the rendered text without switching to
  the editor: matches are highlighted and Enter / Shift+Enter scrolls to
  each one
- A headings menu on the toolbar lists the document's headings (levels
  1–3); click one to jump to it in the preview or the editor
- Starting without a file before any file has been opened shows a short
  welcome page with the key shortcuts and a link to the README
- The reading size (`Ctrl+=` / `Ctrl+-` / `Ctrl+0`) is remembered across
  restarts, and a toolbar menu shows it and lists the shortcuts
- Print renders definition lists the way the preview does

## 0.1.0 — 2026-09-29

Initial public release.

- Opens `.md` rendered; `Ctrl+E` toggles rendered view ↔ raw editor
- Open / save / undo, unsaved-changes guard, dark & light themes
- Icon toolbar (Phosphor): a View / Edit toggle that shows the current mode,
  Recent under Open, an unsaved-changes dot on Save, terminal and theme
  toggles, and tooltips with shortcuts; the find bar matches
- Print / Save as PDF (`Ctrl+P`): the document, unsaved edits included, opens
  as a print-styled page in the browser with its print dialog up
- App icon: a markdown `#` with its centre lit like a cursor, on the window,
  taskbar and Dock, and embedded in the Windows exe
- Drag-and-drop, file association (Windows + macOS), local images,
  fenced-block syntax highlighting with a copy button
- Find in the raw text (`Ctrl+F`) with wrap-around navigation
- Recent-files menu; scroll position syncs between preview and editor
- Auto-reloads clean files changed on disk; dirty buffers get Reload / Keep mine
  conflict handling, and saves refuse to overwrite unseen disk changes
- Built-in terminal pane (`` Ctrl+` ``): a real PTY running PowerShell on
  Windows or `$SHELL` elsewhere, opened in the file's folder; dark/light
  palettes, persisted height, copy/paste menu, and shutdown of the shell and
  its foreground process group when the window closes
- `--version` flag
