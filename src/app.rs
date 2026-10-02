use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::doc::{DiskCheck, Doc, SaveOutcome, demote_remote_images, normalize_fence_langs};
use crate::settings::{ZOOM_MAX, ZOOM_MIN};
use crate::term::{self, Terminal, routing::Route};
use egui_phosphor::regular as icon;

const LOSSY_WARNING: &str =
    "File was not valid UTF-8 and was loaded lossily; saving will write UTF-8.";
const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Shown on first launch with no file to open.
const WELCOME: &str = include_str!("../assets/welcome.md");

#[derive(Clone, Copy)]
pub enum BannerKind {
    Error,
    Warning,
}

/// An action deferred until the unsaved-changes dialog is resolved.
enum Pending {
    Close,
    Open(PathBuf),
}

/// A detected external change awaiting the user's Reload / Keep mine
/// decision. Stores no bytes: both buttons read the disk at click time,
/// so nothing here can go stale.
struct Conflict {
    /// The conflict interrupted a Ctrl+S; Keep mine completes that save.
    resume_save: bool,
}

/// Which way a find jump moves. `Stay` lands on the current match without
/// stepping (used for the first jump after any recompute).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Jump {
    Next,
    Prev,
    Stay,
}

#[derive(Default)]
struct Find {
    open: bool,
    query: String,
    /// Ascending byte offsets of matches in doc.text.
    matches: Vec<usize>,
    /// Index into `matches`.
    current: usize,
    /// text_rev the matches were computed from.
    computed_rev: u64,
    /// Matches were recomputed since the last jump: nothing is selected yet,
    /// the counter shows a total instead of a position, and the next jump
    /// lands on `current` instead of stepping past it.
    fresh: bool,
    /// Focus the query box on the next frame (just opened).
    focus_field: bool,
    /// Char range to select in the editor, applied in central() *after*
    /// TextEdit::show() — the only point where editor state and galley are
    /// guaranteed to exist (a fresh generation has no stored TextEditState).
    pending_select: Option<(usize, usize)>,
    /// Preview mode's matches: where the query lands in the rendered text,
    /// relative to the document's top-left, in reading order. `current`
    /// indexes these instead of `matches` while the preview shows.
    hits: Vec<egui::Rect>,
    /// What `hits` was measured from: query, text_rev, and the viewer's
    /// laid-out page size (moves on resize, zoom and image loads).
    hits_key: Option<(String, u64, egui::Vec2)>,
    /// A preview jump for central() to apply once `hits` is measured.
    pending_hit: Option<Jump>,
    /// The view `current` and `fresh` belong to; a switch restarts the walk.
    for_editing: bool,
}

impl Find {
    /// Recompute matches against `text` and restart the walk. Selects
    /// nothing by itself; selection happens on the next jump.
    fn recompute(&mut self, text: &str, text_rev: u64) {
        self.matches = find_matches(text, &self.query);
        self.computed_rev = text_rev;
        self.current = 0;
        self.fresh = true;
    }

    /// The match `jump` lands on in a list of `len` (> 0) matches.
    fn land(&self, jump: Jump, len: usize) -> usize {
        match jump {
            Jump::Stay => self.current.min(len - 1),
            Jump::Next => step_match(self.current, len, true),
            Jump::Prev => step_match(self.current, len, false),
        }
    }

    /// Jump for Enter / the ◀ ▶ buttons: land on the current match right
    /// after a recompute, step otherwise.
    fn enter_jump(&self, backward: bool) -> Jump {
        if self.fresh {
            Jump::Stay
        } else if backward {
            Jump::Prev
        } else {
            Jump::Next
        }
    }
}

/// Base URI (implicit `file://` scheme) that makes relative image paths in a
/// document resolve against the document's directory instead of the process CWD.
/// Distinct per directory, so two documents with the same relative image name
/// can never collide in the image cache.
fn implicit_base(dir: &Path) -> String {
    #[cfg(windows)]
    let mut s = {
        // egui_extras accepts drive paths as `file:///C:/...` and UNC paths
        // as `file://server/share/...`.
        let path = dir.to_string_lossy().replace('\\', "/");
        let path = if let Some(rest) = path.strip_prefix("//?/UNC/") {
            format!("//{rest}")
        } else if let Some(rest) = path.strip_prefix("//?/") {
            rest.to_owned()
        } else {
            path
        };
        if let Some(unc) = path.strip_prefix("//") {
            format!("file://{unc}")
        } else if path.as_bytes().get(1) == Some(&b':') {
            format!("file:///{path}")
        } else {
            format!("file://{path}")
        }
    };
    #[cfg(not(windows))]
    let mut s = format!("file://{}", dir.display());
    if !s.ends_with('/') && !s.ends_with('\\') {
        s.push('/');
    }
    s
}

/// Byte offsets of ASCII-case-insensitive matches of `needle` in `haystack`,
/// ascending, overlaps included. Offsets are always char boundaries: a match
/// can't start on a UTF-8 continuation byte because the needle's first byte
/// is either ASCII (never a continuation byte) or the exact leading byte of
/// a multi-byte sequence.
/// O(n·m) scan; artifacts are small. Swap in memchr if a profile
/// ever says otherwise.
fn find_matches(haystack: &str, needle: &str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + n.len() <= h.len() {
        if h[i..i + n.len()].eq_ignore_ascii_case(n) {
            out.push(i);
        }
        i += 1;
    }
    out
}

/// Next/previous index into a match list, wrapping at both ends.
fn step_match(current: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        return 0;
    }
    if forward {
        (current + 1) % len
    } else {
        (current + len - 1) % len
    }
}

/// Preview find marks, painted over the text and translucent so it stays
/// readable in both themes (premultiplied: amber at 27%, orange at 55%).
const HIT: egui::Color32 = egui::Color32::from_rgba_premultiplied(70, 55, 0, 70);
const HIT_CURRENT: egui::Color32 = egui::Color32::from_rgba_premultiplied(140, 77, 0, 140);

/// Lays the whole preview out once, off-screen, and returns where `query`
/// lands in its rendered text: rects relative to the document's top-left,
/// in paint (reading) order. The viewer only draws the visible slice, so
/// this is the one way to know where every match is. Its widgets sit far
/// off-screen where nothing can hover them, and its shapes are dropped.
fn measure_hits(
    ui: &mut egui::Ui,
    viewer: CommonMarkViewer,
    cache: &mut CommonMarkCache,
    text: &str,
    query: &str,
    width: f32,
) -> Vec<egui::Rect> {
    let layer = egui::LayerId::new(egui::Order::Background, egui::Id::new("find_measure"));
    let origin = egui::pos2(-100_000.0, 0.0);
    let mut scratch = ui.new_child(
        egui::UiBuilder::new()
            .layer_id(layer)
            .id_salt("find_measure")
            .max_rect(egui::Rect::from_min_size(origin, egui::vec2(width, 1.0e7))),
    );
    // Labels paint only where the clip rect says they are visible.
    scratch.set_clip_rect(egui::Rect::EVERYTHING);
    viewer.show(&mut scratch, cache, text);
    let mut hits = Vec::new();
    ui.ctx().graphics_mut(|g| {
        let list = std::mem::take(g.entry(layer));
        for clipped in list.all_entries() {
            collect_hits(&clipped.shape, query, origin, &mut hits);
        }
    });
    hits
}

fn collect_hits(shape: &egui::Shape, query: &str, origin: egui::Pos2, out: &mut Vec<egui::Rect>) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for s in shapes {
                collect_hits(s, query, origin, out);
            }
        }
        egui::Shape::Text(t) => {
            let text = t.galley.text();
            for start in find_matches(text, query) {
                let a = text[..start].chars().count();
                let b = a + text[start..start + query.len()].chars().count();
                let ra = t.galley.pos_from_cursor(egui::text::CCursor::new(a));
                let rb = t.galley.pos_from_cursor(egui::text::CCursor::new(b));
                // A match broken by a line wrap is marked to its row's end.
                let right = if (ra.min.y - rb.min.y).abs() < 0.5 {
                    rb.max.x
                } else {
                    t.galley.rect.max.x
                };
                let r = egui::Rect::from_min_max(ra.min, egui::pos2(right, ra.max.y));
                out.push(r.translate(t.pos - origin));
            }
        }
        _ => {}
    }
}

/// Where the editor should scroll to mirror the preview: the preview's
/// pixel offset, clamped to the editor's own scrollable range.
/// Pixel mapping, not proportional — the preview's content height is not
/// reachable through public egui/egui_commonmark APIs, so a fraction has
/// no denominator; drift grows past content that renders taller than its
/// source. Fork the egui_commonmark pin to expose content height if that
/// ever matters.
fn edit_scroll_target(offset: f32, galley: f32, viewport: f32) -> f32 {
    offset.clamp(0.0, (galley - viewport).max(0.0))
}

/// Toolbar hit target, and the Phosphor glyph size inside it.
const TOOL_SIZE: egui::Vec2 = egui::Vec2::new(28.0, 24.0);
const ICON_SIZE: f32 = 16.0;

/// Borderless icon button whose frame appears on hover. `on` makes it a
/// toggle, raised while on. It skips `Button::selected`, whose accent fill is
/// loud for a toolbar, and reports the pressed state to screen readers itself.
fn tool_button(
    ui: &mut egui::Ui,
    glyph: &str,
    name: &str,
    tip: String,
    on: Option<bool>,
) -> egui::Response {
    let raised = on == Some(true);
    let mut text = egui::RichText::new(glyph).size(ICON_SIZE);
    if raised {
        text = text.color(ui.visuals().strong_text_color());
    }
    let mut button = egui::Button::new(text)
        .min_size(TOOL_SIZE)
        .frame_when_inactive(raised);
    if raised {
        button = button.fill(ui.visuals().widgets.inactive.weak_bg_fill);
    }
    let response = ui.add(button);
    label_for_a11y(&response, name, on);
    response.on_hover_text(tip)
}

/// An icon button's text is a private-use glyph; give screen readers a name.
fn label_for_a11y(response: &egui::Response, name: &str, on: Option<bool>) {
    let enabled = response.enabled();
    response.widget_info(|| match on {
        Some(on) => egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, name),
        None => egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name),
    });
}

/// Tooltip: the action, then its shortcut in the platform's notation.
fn tip(ctx: &egui::Context, action: &str, key: egui::Key) -> String {
    let shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key);
    format!("{action}  {}", ctx.format_shortcut(&shortcut))
}

pub struct App {
    doc: Doc,
    /// false = rendered view (default), true = raw text editor
    editing: bool,
    cache: CommonMarkCache,
    /// Bumped on every possible doc.text / saved-state change (edit, open,
    /// save); cache invalidation key for `filtered` and the window title.
    text_rev: u64,
    /// Value of `text_rev` that `filtered` was computed from.
    filter_rev: u64,
    /// Value of `text_rev` that `last_title` was computed from.
    title_rev: u64,
    /// doc.text with remote images demoted to links; what the viewer renders.
    filtered: String,
    last_title: String,
    /// Dismissible status layered over the warning derived from `doc.lossy`.
    transient_error: Option<String>,
    conflict: Option<Conflict>,
    /// Last poll instant; None polls on the next frame.
    last_poll: Option<Instant>,
    pending: Option<Pending>,
    force_close: bool,
    /// `file://<doc dir>/` â€” feeds the viewer's default_implicit_uri_scheme.
    image_base: Option<String>,
    /// Bumped on every open; part of the editor widget Id so undo history
    /// and cursor state can never leak across documents.
    generation: u64,
    /// Most-recent-first open history, capped at settings::RECENT_CAP.
    recent: Vec<PathBuf>,
    /// Where recents persist; None in tests so they never touch user config.
    config_dir: Option<PathBuf>,
    find: Find,
    /// Last-seen preview scroll offset, refreshed each preview frame.
    /// Captured after show_scrollable() under the correct ui-salted id.
    preview_scroll: Option<f32>,
    /// Preview pixel offset staged on rendered→raw switch, applied in
    /// central() after TextEdit::show() (same staging as find's
    /// pending_select).
    pending_edit_scroll: Option<f32>,
    /// Ordinal of a heading the preview should scroll to, applied in
    /// central() once the viewer has measured its blocks.
    pending_heading: Option<usize>,
    /// The built-in terminal pane: visibility, focus, height and session.
    term: Terminal,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        if let Some(theme) = crate::settings::load_theme_pref() {
            cc.egui_ctx.set_theme(theme);
        }
        let mut fonts = egui::FontDefinitions::default();
        // Toolbar icons: Phosphor Regular (MIT), a Proportional fallback right
        // after Ubuntu, so its private-use codepoints never shadow text.
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        #[cfg(feature = "cjk-font")]
        {
            // Release builds enable `cjk-font`; default builds omit the font.
            // Terminal font stack: Hack first, then the bundled Noto Sans
            // Mono CJK (OFL-1.1, assets/LICENSE-OFL-NotoSansCJK.txt), then egui's
            // built-in Noto Emoji — so Monospace takes it at index 1. Proportional
            // gets it appended instead, after egui's built-ins: Latin still
            // resolves to Ubuntu, but the preview's arrows and CJK render from
            // Noto CJK instead of tofu boxes.
            fonts.font_data.insert(
                "noto_cjk".into(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                    "../assets/NotoSansMonoCJKsc-Regular.otf"
                ))),
            );
            let mono = fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default();
            let at = mono.len().min(1); // right after Hack
            mono.insert(at, "noto_cjk".into());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push("noto_cjk".into());
        }
        cc.egui_ctx.set_fonts(fonts);
        // egui's built-in Ctrl+Plus/Minus/0 zoom stays off; term::routing
        // reimplements it so it only fires when the terminal is not focused.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let mut app = Self::bare();
        app.config_dir = crate::settings::config_dir();
        if let Some(zoom) = app
            .config_dir
            .as_deref()
            .and_then(crate::settings::load_zoom_from)
        {
            cc.egui_ctx.set_zoom_factor(zoom);
        }
        app.recent = crate::settings::load_recent();
        app.term = Terminal::new(crate::settings::load_term_height());
        app.open_initial(path);
        app
    }

    /// The startup document: `path` if given, else the welcome text on a
    /// first run (no recent files), else an empty buffer.
    fn open_initial(&mut self, path: Option<PathBuf>) {
        match path {
            Some(p) => self.open_path(p),
            None if self.recent.is_empty() => self.doc = Doc::untitled(WELCOME),
            None => {}
        }
    }

    /// Plain construction without egui side effects; shared by `new` and unit tests.
    fn bare() -> Self {
        Self {
            doc: Doc::empty(),
            editing: false,
            cache: CommonMarkCache::default(),
            text_rev: 1,
            filter_rev: 0,
            title_rev: 0,
            filtered: String::new(),
            last_title: String::new(),
            transient_error: None,
            conflict: None,
            last_poll: None,
            pending: None,
            force_close: false,
            image_base: None,
            generation: 0,
            recent: Vec::new(),
            config_dir: None,
            find: Find::default(),
            preview_scroll: None,
            pending_edit_scroll: None,
            pending_heading: None,
            term: Terminal::new(None),
        }
    }

    /// The raw editor's widget id. Salted by `generation` so undo history
    /// and cursor state can never leak across documents.
    fn editor_id(&self) -> egui::Id {
        egui::Id::new(("editor", self.generation))
    }

    fn open_path(&mut self, path: PathBuf) {
        match Doc::open(path.clone()) {
            Ok(doc) => {
                self.doc = doc;
                self.editing = false;
                self.preview_scroll = None;
                self.pending_edit_scroll = None;
                self.pending_heading = None;
                self.generation += 1;
                self.text_rev += 1;
                self.image_base = self
                    .doc
                    .path
                    .as_deref()
                    .and_then(|p| p.parent())
                    .map(implicit_base);
                self.transient_error = None;
                self.conflict = None;
                self.last_poll = None;
                if let Some(p) = self.doc.path.clone() {
                    self.record_recent(p);
                }
            }
            Err(e) => {
                self.show_error(format!("Could not open {}: {e}", path.display()));
            }
        }
    }

    fn record_recent(&mut self, path: PathBuf) {
        crate::settings::push_recent(&mut self.recent, path);
        if let Some(dir) = self.config_dir.clone()
            && let Err(e) = crate::settings::save_recent_to(&dir, &self.recent)
        {
            self.show_error(format!("Could not save recent files: {e}"));
        }
    }

    fn active_banner(&self) -> Option<(BannerKind, &str)> {
        if let Some(error) = self.transient_error.as_deref() {
            return Some((BannerKind::Error, error));
        }
        self.doc
            .lossy
            .then_some((BannerKind::Warning, LOSSY_WARNING))
    }

    fn show_error(&mut self, message: impl Into<String>) {
        self.transient_error = Some(message.into());
    }

    fn dismiss_banner(&mut self) {
        self.transient_error = None;
    }

    /// True while the unsaved-changes decision owns the UI.
    fn modal_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// One poll tick: route the disk state. Bar-open cases first — an open
    /// bar is only ever *closed* here (its claim went stale), never acted
    /// on: the buttons read the disk themselves at click time.
    fn poll_disk(&mut self) {
        match self.doc.external_change() {
            DiskCheck::Unchanged => {}
            DiskCheck::MatchesBaseline => {
                // Disk reverted to the baseline: the bar's claim is no
                // longer true.
                self.conflict = None;
            }
            DiskCheck::Changed(bytes) => {
                if self.conflict.is_some() {
                    // Bar already up; it already says "changed".
                } else if self.doc.dirty() {
                    self.conflict = Some(Conflict { resume_save: false });
                } else {
                    self.doc.accept_disk(bytes);
                    self.text_rev += 1;
                }
            }
        }
    }

    /// Open `p`, but if the current buffer has unsaved changes, ask first.
    fn request_open(&mut self, p: PathBuf) {
        if self.modal_pending() {
            return;
        }
        if self.doc.dirty() {
            self.pending = Some(Pending::Open(p));
        } else {
            self.open_path(p);
        }
    }

    fn proceed_pending(&mut self, ctx: &egui::Context) {
        match self.pending.take() {
            Some(Pending::Close) => {
                self.force_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(Pending::Open(p)) => self.open_path(p),
            None => {}
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        if self.pending.is_none() {
            return;
        }
        #[derive(Clone, Copy)]
        enum Choice {
            Save,
            Discard,
            Cancel,
        }
        let name = self.doc.file_name();
        let mut choice = None;
        egui::Modal::new(egui::Id::new("unsaved_changes")).show(ctx, |ui| {
            ui.heading("Unsaved changes");
            ui.add_space(8.0);
            ui.label(format!("Save changes to {name}?"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some(Choice::Save);
                }
                if ui.button("Discard").clicked() {
                    choice = Some(Choice::Discard);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
        });
        match choice {
            Some(Choice::Save) => {
                self.save();
                // Only proceed if the save actually landed (it can fail, or the
                // user can cancel the Save-As dialog for an untitled buffer).
                if !self.doc.dirty() {
                    self.proceed_pending(ctx);
                }
            }
            Some(Choice::Discard) => self.proceed_pending(ctx),
            Some(Choice::Cancel) => self.pending = None,
            None => {}
        }
    }

    fn open_dialog(&mut self) {
        if self.modal_pending() {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Markdown", &["md", "markdown", "txt"])
            .pick_file()
        {
            self.request_open(path);
        }
    }

    fn save_as_dialog(&mut self) {
        let suggested = if self.doc.path.is_none() {
            "untitled.md".to_owned()
        } else {
            self.doc.file_name()
        };
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Markdown", &["md", "markdown", "txt"])
            .set_file_name(suggested)
            .save_file()
        {
            if let Err(e) = self.doc.save_as(path) {
                self.show_error(format!("Save failed: {e}"));
            } else {
                self.transient_error = None;
                self.text_rev += 1;
                self.image_base = self
                    .doc
                    .path
                    .as_deref()
                    .and_then(|p| p.parent())
                    .map(implicit_base);
                if let Some(p) = self.doc.path.clone() {
                    self.record_recent(p);
                }
            }
        }
    }

    fn save(&mut self) {
        if self.doc.path.is_none() {
            self.save_as_dialog();
            return;
        }
        match self.doc.save() {
            Ok(SaveOutcome::Saved) => {
                self.transient_error = None;
                self.conflict = None; // a bar still open here was stale
                self.text_rev += 1;
            }
            Ok(SaveOutcome::Conflict) => {
                self.conflict = Some(Conflict { resume_save: true });
                // If the unsaved-changes modal triggered this save, let the
                // user resolve the bar first, then close/open again.
                self.pending = None;
            }
            Err(e) => self.show_error(format!("Save failed: {e}")),
        }
    }

    /// Print the buffer as it is now, unsaved edits included: write it as an
    /// HTML page and open that in the browser, whose print dialog appears on
    /// load and offers the printers and Save as PDF.
    fn print(&mut self, ctx: &egui::Context) {
        let dir = self.doc.path.as_deref().and_then(Path::parent);
        let title = self
            .doc
            .path
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(
                || "untitled".to_owned(),
                |s| s.to_string_lossy().into_owned(),
            );
        let page = crate::print::page(&self.doc.text, &title, dir.map(implicit_base).as_deref());
        // Keep the temporary HTML file so the browser can read it asynchronously.
        // nano.md does not currently clean up these files after printing.
        let written = tempfile::Builder::new()
            .prefix("nanomd-print-")
            .suffix(".html")
            .tempfile()
            .and_then(|mut file| {
                std::io::Write::write_all(&mut file, page.as_bytes())?;
                Ok(file.keep()?.1)
            });
        match written {
            Ok(path) => {
                let url = match (path.parent(), path.file_name()) {
                    (Some(parent), Some(name)) => {
                        format!("{}{}", implicit_base(parent), name.to_string_lossy())
                    }
                    _ => path.to_string_lossy().into_owned(),
                };
                ctx.open_url(egui::OpenUrl::new_tab(crate::print::encode(&url)));
            }
            Err(e) => self.show_error(format!("Could not print: {e}")),
        }
    }

    /// Reload button: adopt the disk as it is now.
    fn conflict_reload(&mut self) {
        match self.doc.reload_from_disk() {
            Ok(()) => {
                self.conflict = None;
                self.text_rev += 1;
            }
            Err(e) => {
                self.show_error(format!("Could not reload {}: {e}", self.doc.file_name()));
            }
        }
    }

    /// Keep mine button: the buffer wins over the disk read now; if the
    /// conflict interrupted a Ctrl+S, complete that save. A newer external
    /// write between this read and the save conflicts again — the bar
    /// simply returns.
    fn conflict_keep_mine(&mut self) {
        let resume = self.conflict.as_ref().is_some_and(|c| c.resume_save);
        match self.doc.rebase_to_disk() {
            Ok(()) => {
                self.conflict = None;
                if resume {
                    self.save();
                }
            }
            Err(e) => {
                self.show_error(format!("Could not read {}: {e}", self.doc.file_name()));
            }
        }
    }

    /// Switch to raw view, staging the preview's position so the editor
    /// opens where the reader was. No-op on the stage if already editing.
    fn enter_edit(&mut self) {
        if !self.editing {
            self.pending_edit_scroll = self.preview_scroll;
        }
        self.pending_heading = None; // a preview jump must not fire on return
        self.editing = true;
    }

    /// Jump to the `ordinal`-th heading: the editor puts the cursor on it,
    /// the preview stages a scroll for central().
    fn jump_to_heading(&mut self, ordinal: usize) {
        if !self.editing {
            self.pending_heading = Some(ordinal);
        } else if let Some(h) = crate::doc::headings(&self.doc.text).get(ordinal) {
            let c = self.doc.text[..h.start].chars().count();
            self.find.pending_select = Some((c, c));
        }
    }

    fn open_find(&mut self) {
        self.find.open = true;
        self.find.focus_field = true;
    }

    fn jump_to_match(&mut self, jump: Jump) {
        if !self.editing {
            self.find.pending_hit = Some(jump); // central() scrolls to it
            return;
        }
        if self.find.matches.is_empty() {
            return;
        }
        self.find.current = self.find.land(jump, self.find.matches.len());
        self.find.fresh = false;
        let start = self.find.matches[self.find.current];
        let end = start + self.find.query.len();
        // TextEdit cursors are char-indexed, not byte-indexed. The range is
        // only *staged* here; central() applies it after TextEdit::show(),
        // which works even when this generation's editor has never rendered.
        let start_c = self.doc.text[..start].chars().count();
        let end_c = start_c + self.doc.text[start..end].chars().count();
        self.find.pending_select = Some((start_c, end_c));
    }

    /// The routing table is the only source of app shortcuts: what it does
    /// not list this frame is left in egui's queue for the focused widget
    /// (the terminal, or the editor).
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let owner =
            term::routing::owner(self.modal_pending(), self.term.visible, self.term.focused);
        for (sc, route) in term::routing::table(owner, cfg!(target_os = "macos")) {
            if ctx.input_mut(|i| i.consume_shortcut(&sc)) {
                self.apply_route(route, ctx);
            }
        }
    }

    fn apply_route(&mut self, route: Route, ctx: &egui::Context) {
        match route {
            Route::Save => self.save(),
            Route::ToggleEdit => {
                if self.editing {
                    self.editing = false
                } else {
                    self.enter_edit()
                }
            }
            Route::Open => self.open_dialog(),
            Route::Print => self.print(ctx),
            Route::Find => self.open_find(),
            Route::ZoomIn => self.set_zoom(ctx, (ctx.zoom_factor() * 1.1).min(ZOOM_MAX)),
            Route::ZoomOut => self.set_zoom(ctx, (ctx.zoom_factor() / 1.1).max(ZOOM_MIN)),
            Route::ZoomReset => self.set_zoom(ctx, 1.0),
            Route::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Route::Swallow => {}
            Route::ToggleTerminal => {
                self.term.toggle();
                if !self.term.visible {
                    self.return_focus_from_terminal(ctx);
                }
            }
        }
    }

    fn set_zoom(&mut self, ctx: &egui::Context, zoom: f32) {
        ctx.set_zoom_factor(zoom);
        if let Some(dir) = self.config_dir.clone()
            && let Err(e) = crate::settings::save_zoom_to(&dir, zoom)
        {
            self.show_error(format!("Could not save reading size: {e}"));
        }
    }

    /// Closing the pane (toggle, child exit, spawn failure): the raw editor
    /// takes focus when visible; otherwise nothing keeps a stale focus.
    fn return_focus_from_terminal(&mut self, ctx: &egui::Context) {
        if self.editing {
            let id = self.editor_id();
            ctx.memory_mut(|m| m.request_focus(id));
        } else {
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
    }

    fn sync_title(&mut self, ctx: &egui::Context) {
        if self.title_rev == self.text_rev {
            return;
        }
        self.title_rev = self.text_rev;
        let title = self.doc.title();
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let modal_pending = self.modal_pending();
        let mut theme_save_error: Option<String> = None;
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_enabled_ui(!modal_pending, |ui| {
                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let ctx = ui.ctx().clone();
                    if tool_button(
                        ui,
                        icon::FOLDER_OPEN,
                        "Open",
                        tip(&ctx, "Open", egui::Key::O),
                        None,
                    )
                    .clicked()
                    {
                        self.open_dialog();
                    }
                    // Recent hangs off Open as the half of a split button.
                    let caret = egui::Button::new(egui::RichText::new(icon::CARET_DOWN).size(11.0))
                        .min_size(egui::vec2(14.0, TOOL_SIZE.y))
                        .frame_when_inactive(false);
                    let (recent, _) =
                        egui::containers::menu::MenuButton::from_button(caret).ui(ui, |ui| {
                            if self.recent.is_empty() {
                                ui.weak("No recent files");
                            }
                            let mut chosen: Option<PathBuf> = None;
                            for p in &self.recent {
                                let label = p
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| p.display().to_string());
                                if ui
                                    .button(label)
                                    .on_hover_text(p.display().to_string())
                                    .clicked()
                                {
                                    chosen = Some(p.clone());
                                    ui.close();
                                }
                            }
                            if let Some(p) = chosen {
                                self.request_open(p);
                            }
                        });
                    label_for_a11y(&recent, "Recent files", None);
                    recent.on_hover_text("Recent files");

                    let dirty = self.doc.dirty();
                    let save_name = if dirty {
                        "Save (unsaved changes)"
                    } else {
                        "Save"
                    };
                    let save = tool_button(
                        ui,
                        icon::FLOPPY_DISK,
                        save_name,
                        tip(&ctx, "Save", egui::Key::S),
                        None,
                    );
                    if dirty {
                        // Unsaved dot in the icon's empty top-right corner, the
                        // same mark the window title carries.
                        let at = save.rect.right_top() + egui::vec2(-4.5, 4.5);
                        ui.painter()
                            .circle_filled(at, 2.5, ui.visuals().strong_text_color());
                    }
                    if save.clicked() {
                        self.save();
                    }
                    let print_tip = tip(&ctx, "Print or save as PDF", egui::Key::P);
                    if tool_button(ui, icon::PRINTER, "Print", print_tip, None).clicked() {
                        self.print(&ctx);
                    }

                    ui.add_space(10.0);
                    if let Some(editing) = self.mode_switch(ui, &ctx) {
                        if editing {
                            self.enter_edit();
                        } else {
                            self.editing = false;
                        }
                    }
                    ui.add_space(4.0);
                    let list =
                        egui::Button::new(egui::RichText::new(icon::LIST_BULLETS).size(ICON_SIZE))
                            .min_size(TOOL_SIZE)
                            .frame_when_inactive(false);
                    let (outline, _) =
                        egui::containers::menu::MenuButton::from_button(list).ui(ui, |ui| {
                            ui.set_max_width(320.0);
                            // ponytail: reparsed every frame the menu is open;
                            // cache by text_rev if huge documents make it lag.
                            let headings = crate::doc::headings(&self.doc.text);
                            let listed = |h: &crate::doc::Heading| {
                                h.level <= 3 && !h.title.trim().is_empty()
                            };
                            let mut chosen = None;
                            egui::ScrollArea::vertical()
                                .max_height(400.0)
                                .show(ui, |ui| {
                                    for (k, h) in headings.iter().enumerate() {
                                        if !listed(h) {
                                            continue;
                                        }
                                        ui.horizontal(|ui| {
                                            ui.add_space((h.level - 1) as f32 * 12.0);
                                            let item = egui::Button::new(&h.title).truncate();
                                            if ui.add(item).clicked() {
                                                chosen = Some(k);
                                            }
                                        });
                                    }
                                });
                            if !headings.iter().any(listed) {
                                ui.weak("No headings");
                            }
                            if let Some(k) = chosen {
                                self.jump_to_heading(k);
                                ui.close();
                            }
                        });
                    label_for_a11y(&outline, "Headings", None);
                    outline.on_hover_text("Headings");

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let dark = ctx.theme() == egui::Theme::Dark;
                        let (glyph, name, next) = if dark {
                            (icon::SUN, "Switch to light mode", egui::Theme::Light)
                        } else {
                            (icon::MOON, "Switch to dark mode", egui::Theme::Dark)
                        };
                        if tool_button(ui, glyph, name, name.to_owned(), None).clicked() {
                            ctx.set_theme(next);
                            if let Err(e) = crate::settings::save_theme_pref(next) {
                                theme_save_error =
                                    Some(format!("Could not save theme preference: {e}"));
                            }
                        }
                        let term_name = if self.term.visible {
                            "Hide terminal"
                        } else {
                            "Show terminal"
                        };
                        let term_tip = tip(&ctx, term_name, term::routing::TERM_KEY);
                        if tool_button(
                            ui,
                            icon::TERMINAL_WINDOW,
                            "Terminal",
                            term_tip,
                            Some(self.term.visible),
                        )
                        .clicked()
                        {
                            self.apply_route(Route::ToggleTerminal, &ctx);
                        }
                        let size = format!("Reading size {:.0}%", ctx.zoom_factor() * 100.0);
                        let aa =
                            egui::Button::new(egui::RichText::new(icon::TEXT_AA).size(ICON_SIZE))
                                .min_size(TOOL_SIZE)
                                .frame_when_inactive(false);
                        let (zoom, _) =
                            egui::containers::menu::MenuButton::from_button(aa).ui(ui, |ui| {
                                ui.weak(&size);
                                for (label, key, route) in [
                                    ("Larger", egui::Key::Equals, Route::ZoomIn),
                                    ("Smaller", egui::Key::Minus, Route::ZoomOut),
                                    ("Actual size", egui::Key::Num0, Route::ZoomReset),
                                ] {
                                    let sc =
                                        egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key);
                                    let item = egui::Button::new(label)
                                        .shortcut_text(ctx.format_shortcut(&sc));
                                    if ui.add(item).clicked() {
                                        self.apply_route(route, &ctx);
                                    }
                                }
                            });
                        label_for_a11y(&zoom, "Reading size", None);
                        zoom.on_hover_text(size);
                    });
                });
                ui.add_space(3.0);
            });
        });
        if let Some(msg) = theme_save_error {
            self.show_error(msg);
        }
    }

    /// View/Edit as one segmented control, so the bar shows the current mode
    /// instead of naming the other one. Returns the mode clicked, if it is new.
    fn mode_switch(&self, ui: &mut egui::Ui, ctx: &egui::Context) -> Option<bool> {
        let visuals = ui.visuals().clone();
        let mut chosen = None;
        egui::Frame::new()
            .stroke(visuals.widgets.noninteractive.bg_stroke)
            .corner_radius(5)
            .inner_margin(2)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.spacing_mut().button_padding.x = 8.0;
                for (editing, glyph, label, hint) in [
                    (false, icon::EYE, "View", "Rendered preview"),
                    (true, icon::PENCIL_SIMPLE, "Edit", "Raw markdown"),
                ] {
                    let on = self.editing == editing;
                    let mut glyph_text = egui::RichText::new(glyph).size(ICON_SIZE);
                    let mut label_text = egui::RichText::new(label);
                    if on {
                        glyph_text = glyph_text.color(visuals.strong_text_color());
                        label_text = label_text.color(visuals.strong_text_color());
                    }
                    let mut segment = egui::Button::new((glyph_text, label_text))
                        .gap(5.0)
                        // Track: 2 margin + 1 stroke per side, so it stands TOOL_SIZE tall.
                        .min_size(egui::vec2(0.0, TOOL_SIZE.y - 6.0))
                        .corner_radius(3)
                        .frame_when_inactive(on);
                    if on {
                        segment = segment.fill(visuals.widgets.inactive.weak_bg_fill);
                    }
                    let response = ui.add(segment);
                    label_for_a11y(&response, label, Some(on));
                    if response
                        .on_hover_text(tip(ctx, hint, egui::Key::E))
                        .clicked()
                        && !on
                    {
                        chosen = Some(editing);
                    }
                }
            });
        chosen
    }

    fn banner_panel(&mut self, ui: &mut egui::Ui) {
        let modal_pending = self.modal_pending();
        let mut dismiss = false;
        if let Some((kind, msg)) = self.active_banner() {
            let fill = match kind {
                BannerKind::Error => egui::Color32::from_rgb(120, 40, 40),
                BannerKind::Warning => egui::Color32::from_rgb(130, 95, 20),
            };
            egui::Panel::top("banner")
                .frame(egui::Frame::default().fill(fill).inner_margin(8))
                .show(ui, |ui| {
                    ui.add_enabled_ui(!modal_pending, |ui| {
                        // Close button first, so a long message (full paths)
                        // wraps in the space left instead of pushing it off.
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if matches!(kind, BannerKind::Error) {
                                    // White glyph and a faint white wash on hover:
                                    // the grey tool frame is muddy on red.
                                    let close = egui::Button::new(
                                        egui::RichText::new(icon::X)
                                            .size(ICON_SIZE)
                                            .color(egui::Color32::WHITE),
                                    )
                                    .min_size(TOOL_SIZE)
                                    .frame_when_inactive(false)
                                    .fill(egui::Color32::from_white_alpha(28))
                                    .stroke(egui::Stroke::NONE);
                                    let response = ui.add(close);
                                    label_for_a11y(&response, "Dismiss", None);
                                    if response.on_hover_text("Dismiss").clicked() {
                                        dismiss = true;
                                    }
                                }
                                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(msg).color(egui::Color32::WHITE),
                                        )
                                        .wrap(),
                                    );
                                });
                            })
                        });
                    });
                });
        }
        if dismiss {
            self.dismiss_banner();
        }
    }

    fn conflict_bar(&mut self, ui: &mut egui::Ui) {
        if self.conflict.is_none() {
            return;
        }
        let modal_pending = self.modal_pending();
        let mut reload = false;
        let mut keep = false;
        egui::Panel::top("conflict_bar")
            .frame(
                egui::Frame::default()
                    .fill(egui::Color32::from_rgb(130, 95, 20))
                    .inner_margin(8),
            )
            .show(ui, |ui| {
                ui.add_enabled_ui(!modal_pending, |ui| {
                    // Buttons first, as in the banner: a long file name wraps
                    // instead of pushing the only way out of the conflict off.
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Keep mine").clicked() {
                                keep = true;
                            }
                            if ui.button("Reload").clicked() {
                                reload = true;
                            }
                            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(format!(
                                            "File changed on disk: {}",
                                            self.doc.file_name()
                                        ))
                                        .color(egui::Color32::WHITE),
                                    )
                                    .wrap(),
                                );
                            });
                        });
                    });
                });
            });
        if reload {
            self.conflict_reload();
        } else if keep {
            self.conflict_keep_mine();
        }
    }

    fn find_bar(&mut self, ui: &mut egui::Ui) {
        if self.find.for_editing != self.editing {
            // The other view has its own match list: restart the walk there.
            self.find.for_editing = self.editing;
            self.find.computed_rev = 0;
            self.find.hits_key = None;
            self.find.pending_hit = None;
        }
        if !self.find.open {
            return;
        }
        let total = if self.editing {
            self.find.matches.len()
        } else {
            self.find.hits.len()
        };
        let mut jump: Option<Jump> = None;
        let mut close = false;
        egui::Panel::top("find_bar").show(ui, |ui| {
            ui.add_enabled_ui(!self.modal_pending(), |ui| {
                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let visuals = ui.visuals().clone();
                    // Match count rides inside the field, right-aligned.
                    let count = (!self.find.query.is_empty()).then(|| {
                        if total == 0 {
                            egui::RichText::new("0/0").color(visuals.error_fg_color)
                        } else if self.find.fresh {
                            // Nothing is selected yet (e.g. document just
                            // opened with a retained query): show the total,
                            // never a position we haven't jumped to.
                            egui::RichText::new(format!("{total} found"))
                                .color(visuals.weak_text_color())
                        } else {
                            egui::RichText::new(format!("{}/{total}", self.find.current + 1))
                                .color(visuals.weak_text_color())
                        }
                    });
                    let mut field = egui::TextEdit::singleline(&mut self.find.query)
                        .id(egui::Id::new("find_query"))
                        .hint_text("Find")
                        .prefix(
                            egui::RichText::new(icon::MAGNIFYING_GLASS)
                                .size(ICON_SIZE - 2.0)
                                .color(visuals.weak_text_color()),
                        )
                        .margin(egui::Margin::symmetric(6, 2))
                        .min_size(egui::vec2(0.0, TOOL_SIZE.y))
                        .vertical_align(egui::Align::Center)
                        .desired_width(280.0);
                    if let Some(count) = count {
                        field = field.suffix(count);
                    }
                    let resp = ui.add(field);
                    if self.find.focus_field {
                        resp.request_focus();
                        self.find.focus_field = false;
                    }
                    let query_changed = resp.changed();
                    // The preview re-measures its hits in central(), keyed
                    // on the query; the editor recomputes here.
                    if self.editing && (query_changed || self.find.computed_rev != self.text_rev) {
                        self.find.recompute(&self.doc.text, self.text_rev);
                    }
                    if query_changed {
                        // Live preview while typing a query in the bar.
                        // Text-triggered recomputes deliberately do NOT
                        // auto-jump: that would yank the selection to the
                        // match on every keystroke in the editor.
                        jump = Some(Jump::Stay);
                    }
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if enter && (resp.lost_focus() || resp.has_focus()) {
                        let shift = ui.input(|i| i.modifiers.shift);
                        jump = Some(self.find.enter_jump(shift));
                        resp.request_focus(); // keep typing/searching from the bar
                    }
                    ui.add_space(6.0);
                    let prev = "Previous match  Shift+Enter".to_owned();
                    if tool_button(ui, icon::ARROW_UP, "Previous match", prev, None).clicked() {
                        jump = Some(self.find.enter_jump(true));
                    }
                    let next = "Next match  Enter".to_owned();
                    if tool_button(ui, icon::ARROW_DOWN, "Next match", next, None).clicked() {
                        jump = Some(self.find.enter_jump(false));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let esc = "Close  Esc".to_owned();
                        if tool_button(ui, icon::X, "Close find", esc, None).clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Escape))
                        {
                            close = true;
                        }
                    });
                });
                ui.add_space(3.0);
            });
        });
        if close {
            self.find.open = false;
            return;
        }
        if let Some(j) = jump {
            self.jump_to_match(j);
            // The count inside the field was drawn before this frame's
            // recompute and jump; one more frame shows the new position.
            ui.ctx().request_repaint();
        }
    }

    /// The terminal pane. Spawns on the first frame it is shown, so an
    /// unopened pane costs nothing; a spawn failure reports and closes.
    fn term_panel(&mut self, ui: &mut egui::Ui) {
        if !self.term.visible {
            return;
        }
        let ctx = ui.ctx().clone();
        if self.term.session.is_none()
            && let Err(e) = self.term.spawn(&ctx, self.doc.path.as_deref())
        {
            self.show_error(format!("Could not start the terminal: {e}"));
            self.term.close();
            self.return_focus_from_terminal(&ctx);
            return;
        }
        let modal = self.modal_pending();
        let font_measure = egui_term::TerminalFont::default().font_measure(&ctx);
        let available = ui.available_height();
        let b = term::layout::bounds(available, font_measure.height);
        let h = term::layout::clamp(self.term.height, available, font_measure.height);
        let theme = term::palette::theme_for(ctx.theme());
        // Consumed here, not in `open()`: the toolbar button toggles the
        // pane and this panel runs in the same frame, where `any_click()`
        // is still true for the release over the toolbar. Without this the
        // pane would open unfocused.
        let just_opened = std::mem::take(&mut self.term.just_opened);
        let mut clicked_view = false;
        let inner =
            egui::Panel::bottom("terminal")
                .resizable(true)
                .min_size(b.min.min(b.max.max(0.0)))
                .max_size(b.max.max(0.0))
                .default_size(h)
                .show(ui, |ui| {
                    if let Some(st) = &self.term.stats {
                        ui.weak(st.report());
                    }
                    let focused = self.term.focused && !modal;
                    // `drain` hides the pane when it clears the session, so
                    // this should be unreachable — but an empty panel beats a
                    // panic if that invariant ever slips.
                    let Some(session) = self.term.session.as_mut() else {
                        return;
                    };
                    if std::mem::take(&mut session.resize_kick_pending) {
                        // Deliberately one point short of what the view is about
                        // to ask for: the backend ignores a resize to the size it
                        // already has, and ConPTY dropped the pre-attach one.
                        let avail = ui.available_size();
                        session
                            .backend
                            .process_command(egui_term::BackendCommand::Resize(
                                egui_term::Size::new(avail.x, avail.y - 1.0),
                                font_measure,
                            ));
                    }
                    let view = egui_term::TerminalView::new(ui, &mut session.backend)
                        .set_focus(focused)
                        .set_theme(theme)
                        .set_size(ui.available_size());
                    let resp = ui.add(view);
                    if !modal {
                        clicked_view = resp.clicked();
                        resp.context_menu(|ui| {
                            if ui.button("Copy").clicked() {
                                // Empty selection: leave the clipboard alone
                                // rather than wiping it.
                                let text = session.backend.selectable_content();
                                if !text.is_empty() {
                                    ui.ctx().copy_text(text);
                                }
                                ui.close();
                            }
                            if ui.button("Paste").clicked() {
                                if let Some(text) = term::clipboard::read_text() {
                                    session.backend.process_command(
                                        egui_term::BackendCommand::Write(text.into_bytes()),
                                    );
                                }
                                ui.close();
                            }
                        });
                    }
                });
        // The whole panel, divider and padding included, counts as inside:
        // clicking the stats line or the drag handle must not unfocus.
        let panel_rect = inner.response.rect;
        if !modal {
            if clicked_view {
                self.term.focused = true;
            } else if !just_opened
                && ctx.input(|i| {
                    i.pointer.any_click()
                        && i.pointer
                            .interact_pos()
                            .is_some_and(|p| !panel_rect.contains(p))
                })
            {
                self.term.focused = false;
            }
        }
        let dragging = ctx.input(|i| i.pointer.primary_down());
        if let Some(h) = self.term.observe_height(panel_rect.height(), dragging)
            && let Err(e) = crate::settings::save_term_height(h)
        {
            self.show_error(format!("Could not save terminal height: {e}"));
            // One banner, one retry: this is remembered for on_exit only,
            // never re-offered to observe_height on the next frame.
            self.term.mark_height_dirty(h);
        }
    }

    /// Preview find, before the viewer draws: re-measure where the query
    /// lands when the query, text or layout changed, then apply a pending
    /// jump by scrolling its match into view this frame.
    fn update_hits(&mut self, ui: &mut egui::Ui, source_id: egui::Id, state_id: egui::Id) {
        if !self.find.open || self.find.query.is_empty() {
            self.find.hits.clear();
            self.find.hits_key = None;
            self.find.pending_hit = None;
            return;
        }
        let page =
            egui_commonmark_backend::misc::scroll_cache(&mut self.cache, &egui::Id::new(source_id))
                .page_size;
        let Some(page) = page else {
            ui.ctx().request_repaint(); // the viewer lays the page out this frame
            return;
        };
        let key = (self.find.query.clone(), self.text_rev, page);
        if self.find.hits_key.as_ref() != Some(&key) {
            let mut viewer = CommonMarkViewer::new();
            if let Some(base) = &self.image_base {
                viewer = viewer.default_implicit_uri_scheme(base.clone());
            }
            self.find.hits = measure_hits(
                ui,
                viewer,
                &mut self.cache,
                &self.filtered,
                &self.find.query,
                page.x,
            );
            self.find.hits_key = Some(key);
            // The walk restarts at the first match at or below the reader.
            let top = self.preview_scroll.unwrap_or(0.0);
            self.find.current = self
                .find
                .hits
                .iter()
                .position(|r| r.top() >= top)
                .unwrap_or(0);
            self.find.fresh = true;
        }
        let Some(jump) = self.find.pending_hit.take() else {
            return;
        };
        if self.find.hits.is_empty() {
            return;
        }
        self.find.current = self.find.land(jump, self.find.hits.len());
        self.find.fresh = false;
        let hit = self.find.hits[self.find.current];
        let view = ui.available_height();
        let top = self.preview_scroll.unwrap_or(0.0);
        if (hit.top() < top || hit.bottom() > top + view)
            && let Some(mut state) = egui::scroll_area::State::load(ui.ctx(), state_id)
        {
            state.offset.y = (hit.center().y - view / 2.0).max(0.0);
            state.store(ui.ctx(), state_id);
        }
    }

    /// Marks the preview's on-screen matches, the current one stronger.
    fn paint_hits(&self, ui: &egui::Ui, area: egui::Rect) {
        let origin = area.min - egui::vec2(0.0, self.preview_scroll.unwrap_or(0.0));
        let painter = ui.painter_at(area);
        // ponytail: scans every hit each frame; binary-search by y if a
        // document ever has enough matches for this to show up.
        for (i, hit) in self.find.hits.iter().enumerate() {
            let r = hit.translate(origin.to_vec2()).expand(1.0);
            if area.intersects(r) {
                let current = i == self.find.current && !self.find.fresh;
                painter.rect_filled(r, 2.0, if current { HIT_CURRENT } else { HIT });
            }
        }
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        let modal_pending = self.modal_pending();
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_enabled_ui(!modal_pending, |ui| {
                if self.editing {
                    let editor_id = self.editor_id();
                    egui::ScrollArea::vertical()
                        .auto_shrink(false)
                        .show(ui, |ui| {
                            let out = egui::TextEdit::multiline(&mut self.doc.text)
                                .id(editor_id)
                                .code_editor()
                                .desired_width(ui.available_width())
                                .desired_rows(30)
                                .frame(egui::Frame::NONE)
                                .show(ui);
                            if out.response.changed() {
                                self.text_rev += 1;
                            }
                            if let Some((start_c, end_c)) = self.find.pending_select.take() {
                                self.pending_edit_scroll = None; // find's jump owns the scroll
                                // Applied here, after show(): the output's
                                // state exists even on this generation's very
                                // first render, unlike TextEditState::load.
                                let mut state = out.state;
                                state
                                    .cursor
                                    .set_char_range(Some(egui::text::CCursorRange::two(
                                        egui::text::CCursor::new(start_c),
                                        egui::text::CCursor::new(end_c),
                                    )));
                                state.store(ui.ctx(), out.response.id);
                                let rect = out
                                    .galley
                                    .pos_from_cursor(egui::text::CCursor::new(start_c))
                                    .translate(out.galley_pos.to_vec2());
                                ui.scroll_to_rect(rect.expand(24.0), Some(egui::Align::Center));
                            } else if let Some(off) = self.pending_edit_scroll.take() {
                                // Viewport top lands at the preview's offset,
                                // clamped to the editor's own range.
                                let viewport_h = ui.clip_rect().height();
                                let y = out.galley_pos.y
                                    + edit_scroll_target(off, out.galley.size().y, viewport_h);
                                let target = egui::Rect::from_min_size(
                                    egui::pos2(out.galley_pos.x, y),
                                    egui::vec2(1.0, 1.0),
                                );
                                // Instant: opening the editor mid-animation
                                // reads as lag, not motion.
                                ui.scroll_to_rect_animation(
                                    target,
                                    Some(egui::Align::TOP),
                                    egui::style::ScrollAnimation::none(),
                                );
                            }
                        });
                } else {
                    if self.filter_rev != self.text_rev {
                        self.filter_rev = self.text_rev;
                        self.filtered =
                            normalize_fence_langs(&demote_remote_images(&self.doc.text));
                        // The viewer's cached element geometry is stale now.
                        self.cache.clear_scrollable();
                    }
                    // doc::headings assumes the viewer's default parser
                    // options: enabling math or scroll-to-heading here would
                    // shift the event indexes heading jumps rely on.
                    let mut viewer = CommonMarkViewer::new().viewport_cache(true);
                    if let Some(base) = &self.image_base {
                        viewer = viewer.default_implicit_uri_scheme(base.clone());
                    }
                    // Renders only the visible slice; brings its own ScrollArea.
                    // Keyed on generation so scroll state resets per document.
                    let source_id = egui::Id::new(("preview", self.generation));
                    // The preview's scroll state. egui_commonmark's public
                    // show_scrollable rehashes the caller's id through
                    // Id::new() before deriving its ScrollArea id, so the
                    // stored key is Id::new(source_id).with("_scroll_area"),
                    // salted by this ui — covered by the regression test
                    // preview_scroll_is_captured_from_real_scroll_state.
                    let state_id = ui.make_persistent_id(egui::IdSalt::new(
                        egui::Id::new(source_id).with("_scroll_area"),
                    ));
                    if let Some(ordinal) = self.pending_heading {
                        let sc = egui_commonmark_backend::misc::scroll_cache(
                            &mut self.cache,
                            &egui::Id::new(source_id),
                        );
                        if sc.page_size.is_none() {
                            // Blocks not measured yet (first frame, resize,
                            // images loading): the viewer measures this frame.
                            ui.ctx().request_repaint();
                        } else {
                            self.pending_heading = None;
                            // A heading inside a list has no position of its
                            // own: land where the block before the list ends.
                            let top =
                                crate::doc::headings(&self.filtered)
                                    .get(ordinal)
                                    .and_then(|h| {
                                        let p = sc
                                            .split_points
                                            .iter()
                                            .filter(|p| p.0 <= h.end_event)
                                            .max_by_key(|p| p.0)?;
                                        Some(if p.0 == h.end_event { p.1.y } else { p.2.y })
                                    });
                            if let Some(y) = top
                                && let Some(mut state) =
                                    egui::scroll_area::State::load(ui.ctx(), state_id)
                            {
                                state.offset.y = y;
                                state.store(ui.ctx(), state_id);
                            }
                        }
                    }
                    let area = ui.available_rect_before_wrap();
                    self.update_hits(ui, source_id, state_id);
                    viewer.show_scrollable(source_id, ui, &mut self.cache, &self.filtered);
                    self.preview_scroll =
                        egui::scroll_area::State::load(ui.ctx(), state_id).map(|s| s.offset.y);
                    self.paint_hits(ui, area);
                }
            });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.modal_pending() {
            let dropped = ctx.input(|i| {
                i.raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_path_buf())
                    .next()
            });
            if let Some(path) = dropped {
                self.request_open(path);
            }
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.force_close
            && (self.modal_pending() || self.doc.dirty())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.modal_pending() {
                self.pending = Some(Pending::Close);
            }
        }
        // macOS delivers the menu's Cmd+Q as a close request, not a key
        // event, so the routing table never sees it: cancel it here and
        // hand the shell the Ctrl+Q byte it was really aimed at.
        let cmd_q_down = ctx.input(|i| i.modifiers.mac_cmd && i.key_down(egui::Key::Q));
        if ctx.input(|i| i.viewport().close_requested())
            && term::routing::cancel_close_for_cmd_q(
                self.term.visible && self.term.focused,
                cfg!(target_os = "macos"),
                cmd_q_down,
            )
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if let Some(s) = self.term.session.as_mut() {
                s.backend
                    .process_command(egui_term::BackendCommand::Write(vec![0x11]));
            }
        }
        self.handle_shortcuts(&ctx);
        // Drain lifecycle events every frame, even when the pane is hidden.
        let drained = self.term.drain();
        if drained.exited {
            self.return_focus_from_terminal(&ctx);
        }
        // the stat runs on the UI thread like every other file
        // operation here; a hung network share would stall frames. Local
        // artifact files are the use case; upgrade path is a poll thread.
        if self.doc.path.is_some() && !self.modal_pending() {
            if self.last_poll.is_none_or(|t| t.elapsed() >= POLL_INTERVAL) {
                self.last_poll = Some(Instant::now());
                self.poll_disk();
            }
            // Keep ticking while idle (~2 cheap repaints/sec) so agent
            // writes appear without user input. Untitled buffers skip this.
            ctx.request_repaint_after(POLL_INTERVAL);
        }
        self.sync_title(&ctx);
        self.toolbar(ui);
        self.banner_panel(ui);
        self.conflict_bar(ui);
        self.find_bar(ui);
        self.term_panel(ui);
        self.central(ui);
        self.confirm_dialog(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(h) = self.term.take_height_if_dirty()
            && let Err(e) = crate::settings::save_term_height(h)
        {
            eprintln!("nanomd: could not save terminal height: {e}");
        }
        let _ = self.term.shutdown(std::time::Duration::from_millis(1000));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implicit_base_has_scheme_and_trailing_separator() {
        let b = implicit_base(Path::new("/tmp/docs"));
        assert_eq!(b, "file:///tmp/docs/");
    }

    #[test]
    fn implicit_base_differs_per_directory() {
        assert_ne!(
            implicit_base(Path::new("/a")),
            implicit_base(Path::new("/b"))
        );
    }

    #[test]
    fn request_open_defers_when_dirty() {
        let mut app = App::bare();
        app.doc.text.push_str("unsaved");
        app.request_open(PathBuf::from("other.md"));
        assert!(matches!(app.pending, Some(Pending::Open(_))));
        assert_eq!(
            app.doc.text, "unsaved",
            "buffer must survive until the user decides"
        );
    }

    #[test]
    fn request_open_opens_directly_when_clean() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.md");
        std::fs::write(&p, "# b").unwrap();
        let mut app = App::bare();
        let gen_before = app.generation;
        let rev_before = app.text_rev;
        app.request_open(p);
        assert!(app.pending.is_none());
        assert_eq!(app.doc.text, "# b");
        assert!(
            app.generation > gen_before,
            "editor undo state must reset per document"
        );
        assert!(
            app.text_rev > rev_before,
            "open must invalidate the title and filter caches"
        );
        assert!(
            app.image_base
                .as_deref()
                .is_some_and(|b| b.starts_with("file://"))
        );
    }

    #[test]
    fn pending_action_rejects_new_open_request() {
        let mut app = App::bare();
        app.doc.text.push_str("unsaved");
        app.pending = Some(Pending::Close);

        app.request_open(PathBuf::from("other.md"));

        assert!(matches!(app.pending, Some(Pending::Close)));
        assert_eq!(app.doc.text, "unsaved");
    }

    #[test]
    fn modal_pending_reflects_pending_action() {
        let mut app = App::bare();
        assert!(!app.modal_pending());

        app.pending = Some(Pending::Close);

        assert!(app.modal_pending());
    }

    #[test]
    fn transient_error_dismissal_reveals_lossy_warning() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("lossy.md");
        std::fs::write(&p, [b'h', b'i', 0xff]).unwrap();
        let mut app = App::bare();
        app.open_path(p);

        assert!(matches!(
            app.active_banner(),
            Some((BannerKind::Warning, message)) if message.contains("not valid UTF-8")
        ));

        std::fs::remove_file(dir.path().join("lossy.md")).unwrap();
        std::fs::remove_dir(dir.path()).unwrap();
        app.save();
        assert!(matches!(
            app.active_banner(),
            Some((BannerKind::Error, message)) if message.starts_with("Save failed:")
        ));
        assert!(app.doc.lossy);

        app.dismiss_banner();
        assert!(matches!(
            app.active_banner(),
            Some((BannerKind::Warning, message)) if message.contains("not valid UTF-8")
        ));

        app.dismiss_banner();
        assert!(matches!(
            app.active_banner(),
            Some((BannerKind::Warning, _))
        ));
    }

    #[test]
    fn successful_save_clears_lossy_warning_and_transient_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("lossy.md");
        std::fs::write(&p, [b'h', b'i', 0xff]).unwrap();
        let mut app = App::bare();
        app.open_path(p.clone());
        std::fs::remove_file(&p).unwrap();
        std::fs::remove_dir(dir.path()).unwrap();
        app.save();
        assert!(matches!(app.active_banner(), Some((BannerKind::Error, _))));

        std::fs::create_dir(dir.path()).unwrap();

        app.save();

        assert!(!app.doc.lossy);
        assert!(app.active_banner().is_none());
        assert_eq!(
            std::fs::read(&p).unwrap(),
            "hi\u{fffd}".as_bytes(),
            "the successful save writes the warned-about UTF-8 conversion"
        );
    }

    #[cfg(windows)]
    #[test]
    fn implicit_base_windows_drive_path_is_standard_file_uri() {
        assert_eq!(
            implicit_base(Path::new(r"C:\docs\images")),
            "file:///C:/docs/images/"
        );
    }

    #[cfg(windows)]
    #[test]
    fn implicit_base_windows_unc_path_keeps_host() {
        assert_eq!(
            implicit_base(Path::new(r"\\server\share\docs")),
            "file://server/share/docs/"
        );
    }

    #[test]
    fn opening_a_file_records_it_in_recents_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("cfg");
        let p = dir.path().join("a.md");
        std::fs::write(&p, "# a").unwrap();
        let mut app = App::bare();
        app.config_dir = Some(cfg.clone());

        app.open_path(p);

        let expected = app.doc.path.clone().unwrap(); // absolute form
        assert_eq!(app.recent.first(), Some(&expected));
        assert_eq!(
            crate::settings::load_recent_from(&cfg).first(),
            Some(&expected),
            "recents must survive a restart"
        );
    }

    #[test]
    fn bare_app_never_persists_recents() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        std::fs::write(&p, "# a").unwrap();
        let mut app = App::bare();

        app.open_path(p);

        assert_eq!(app.recent.len(), 1, "in-memory list still works");
        assert!(app.config_dir.is_none());
    }

    #[test]
    fn no_file_and_no_recents_shows_the_welcome_document() {
        let mut app = App::bare();

        app.open_initial(None);

        assert_eq!(app.doc.text, WELCOME);
        assert!(app.doc.path.is_none(), "welcome is untitled");
        assert!(!app.doc.dirty(), "closing must not prompt to save");
    }

    #[test]
    fn no_file_with_recents_shows_an_empty_document() {
        let mut app = App::bare();
        app.recent.push(PathBuf::from("/a.md"));

        app.open_initial(None);

        assert!(app.doc.text.is_empty());
        assert!(!app.doc.dirty());
    }

    #[test]
    fn poll_reloads_clean_buffer_silently() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        std::fs::write(&p, "original").unwrap();
        let mut app = App::bare();
        app.open_path(p.clone());
        let gen_before = app.generation;
        let rev_before = app.text_rev;
        std::fs::write(&p, "agent appended more content").unwrap();

        app.poll_disk();

        assert_eq!(app.doc.text, "agent appended more content");
        assert!(!app.doc.dirty());
        assert!(app.conflict.is_none());
        assert_eq!(
            app.generation, gen_before,
            "scroll/editor state must survive"
        );
        assert!(app.text_rev > rev_before, "preview and title must refresh");
    }

    #[test]
    fn poll_flags_conflict_on_dirty_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        std::fs::write(&p, "original").unwrap();
        let mut app = App::bare();
        app.open_path(p.clone());
        app.doc.text.push_str(" my unsaved edit");
        std::fs::write(&p, "external write").unwrap();

        app.poll_disk();

        assert!(app.doc.text.starts_with("original"), "buffer untouched");
        assert!(matches!(
            app.conflict,
            Some(Conflict { resume_save: false })
        ));
    }

    #[test]
    fn poll_ignores_further_changes_while_bar_open() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        std::fs::write(&p, "original").unwrap();
        let mut app = App::bare();
        app.open_path(p.clone());
        app.doc.text.push_str(" edit");
        app.conflict = Some(Conflict { resume_save: true });
        std::fs::write(&p, "even newer external write").unwrap();

        app.poll_disk();

        assert!(
            app.doc.text.starts_with("original"),
            "no reload under the bar"
        );
        assert!(
            matches!(app.conflict, Some(Conflict { resume_save: true })),
            "bar stays, resume flag preserved; buttons read fresh anyway"
        );
    }

    /// Review item 2: a disk reversion makes the bar's claim false; close it.
    #[test]
    fn poll_closes_bar_when_disk_matches_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        std::fs::write(&p, "original").unwrap();
        let mut app = App::bare();
        app.open_path(p);
        app.doc.text.push_str(" edit");
        app.conflict = Some(Conflict { resume_save: false });
        // Disk still equals the baseline; force the read that observes it.
        app.doc.disk_meta = None;

        app.poll_disk();

        assert!(app.conflict.is_none());
        assert!(app.doc.dirty(), "the buffer edit is untouched");
    }

    #[test]
    fn poll_without_path_does_nothing() {
        let mut app = App::bare();
        app.doc.text.push_str("untitled scratch");
        app.poll_disk();
        assert!(app.conflict.is_none());
        assert_eq!(app.doc.text, "untitled scratch");
    }

    #[test]
    fn open_path_clears_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        std::fs::write(&p, "# a").unwrap();
        let mut app = App::bare();
        app.conflict = Some(Conflict { resume_save: true });

        app.open_path(p);

        assert!(app.conflict.is_none());
    }

    #[test]
    fn save_conflict_raises_bar_and_drops_modal() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        std::fs::write(&p, "loaded").unwrap();
        let mut app = App::bare();
        app.open_path(p.clone());
        app.doc.text.push_str(" my edit");
        // The unsaved-changes dialog is mid-flight (user tried to close):
        app.pending = Some(Pending::Close);
        std::fs::write(&p, "agent write just before Ctrl+S").unwrap();

        app.save();

        assert!(matches!(app.conflict, Some(Conflict { resume_save: true })));
        assert!(
            app.pending.is_none(),
            "the user resolves the bar first, then re-closes"
        );
        assert!(app.doc.dirty(), "nothing was saved");
        assert!(app.active_banner().is_none(), "a conflict is not an error");
    }

    /// Review item 2: a save can succeed with the bar open only when the bar
    /// is already stale (disk matches the baseline) — clear it.
    #[test]
    fn successful_save_clears_stale_conflict_bar() {
        if crate::doc::skip_on_windows_ci() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        std::fs::write(&p, "loaded").unwrap();
        let mut app = App::bare();
        app.open_path(p.clone());
        app.doc.text.push_str(" my edit");
        app.conflict = Some(Conflict { resume_save: false });

        app.save();

        assert!(app.conflict.is_none());
        assert!(!app.doc.dirty());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "loaded my edit");
    }

    #[test]
    fn conflict_reload_reads_current_disk_and_closes_bar() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        std::fs::write(&p, "loaded").unwrap();
        let mut app = App::bare();
        app.open_path(p.clone());
        app.doc.text.push_str(" my edit");
        std::fs::write(&p, "state at detection").unwrap();
        app.poll_disk();
        assert!(app.conflict.is_some());
        // Disk moves again between detection and the click:
        std::fs::write(&p, "state at click time").unwrap();
        let rev_before = app.text_rev;

        app.conflict_reload();

        assert_eq!(
            app.doc.text, "state at click time",
            "fresh read, no snapshot"
        );
        assert!(!app.doc.dirty());
        assert!(app.conflict.is_none());
        assert!(app.text_rev > rev_before);
    }

    #[test]
    fn conflict_keep_mine_resumes_interrupted_save() {
        if crate::doc::skip_on_windows_ci() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        std::fs::write(&p, "loaded").unwrap();
        let mut app = App::bare();
        app.open_path(p.clone());
        app.doc.text.push_str(" my edit");
        std::fs::write(&p, "agent write just before Ctrl+S").unwrap();
        app.save();
        assert!(matches!(app.conflict, Some(Conflict { resume_save: true })));

        app.conflict_keep_mine();

        assert_eq!(std::fs::read_to_string(&p).unwrap(), "loaded my edit");
        assert!(!app.doc.dirty());
        assert!(app.conflict.is_none());
    }

    #[test]
    fn conflict_keep_mine_without_resume_only_rebases() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        std::fs::write(&p, "loaded").unwrap();
        let mut app = App::bare();
        app.open_path(p.clone());
        app.doc.text.push_str(" my edit");
        std::fs::write(&p, "external write").unwrap();
        app.poll_disk();
        assert!(matches!(
            app.conflict,
            Some(Conflict { resume_save: false })
        ));

        app.conflict_keep_mine();

        assert!(app.conflict.is_none());
        assert!(app.doc.dirty(), "kept, not saved");
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "external write",
            "no write happened"
        );
    }

    #[test]
    fn conflict_reload_failure_keeps_bar_and_shows_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        std::fs::write(&p, "loaded").unwrap();
        let mut app = App::bare();
        app.open_path(p.clone());
        app.doc.text.push_str(" my edit");
        app.conflict = Some(Conflict { resume_save: false });
        std::fs::remove_file(&p).unwrap();

        app.conflict_reload();

        assert!(app.conflict.is_some(), "no decision was made");
        assert!(matches!(
            app.active_banner(),
            Some((BannerKind::Error, m)) if m.starts_with("Could not reload")
        ));
        assert!(app.doc.text.starts_with("loaded"), "buffer untouched");
    }

    #[test]
    fn find_matches_is_ascii_case_insensitive_with_byte_offsets() {
        let hay = "Fox fox FOX";
        assert_eq!(find_matches(hay, "fox"), vec![0, 4, 8]);
        // Returned offsets slice cleanly to the needle's length.
        for o in find_matches(hay, "fox") {
            assert!(hay[o..o + 3].eq_ignore_ascii_case("fox"));
        }
    }

    #[test]
    fn find_matches_empty_needle_finds_nothing() {
        assert_eq!(find_matches("anything", ""), Vec::<usize>::new());
    }

    #[test]
    fn find_matches_handles_non_ascii_exactly_and_on_char_boundaries() {
        let hay = "café CAFÉ café";
        let offsets = find_matches(hay, "café");
        // 'é' is two bytes; only exact-non-ASCII matches count ("CAFÉ" differs).
        assert_eq!(offsets, vec![0, 12]);
        for o in &offsets {
            assert_eq!(&hay[*o..*o + "café".len()], "café");
        }
    }

    #[test]
    fn find_matches_includes_overlaps() {
        assert_eq!(find_matches("aaa", "aa"), vec![0, 1]);
    }

    #[test]
    fn step_match_wraps_both_directions_and_survives_empty() {
        assert_eq!(step_match(2, 3, true), 0);
        assert_eq!(step_match(0, 3, false), 2);
        assert_eq!(step_match(1, 3, true), 2);
        assert_eq!(step_match(0, 0, true), 0);
        assert_eq!(step_match(0, 0, false), 0);
    }

    #[test]
    fn find_opens_in_the_current_view() {
        let ctx = egui::Context::default();
        let mut app = App::bare();
        app.apply_route(crate::term::routing::Route::Find, &ctx);
        assert!(!app.editing, "Ctrl+F in the preview stays in the preview");
        assert!(app.find.open);
        assert!(app.find.focus_field);

        app.find.open = false;
        app.editing = true;
        app.apply_route(crate::term::routing::Route::Find, &ctx);
        assert!(app.editing);
        assert!(app.find.open);
    }

    /// One headless frame of the find bar and the document area, in the
    /// order ui() runs them.
    fn doc_frame(ctx: &egui::Context, app: &mut App) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 700.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            app.find_bar(ui);
            app.central(ui);
        });
        out.textures_delta.clear();
    }

    #[test]
    fn preview_find_counts_rendered_text_and_scrolls_to_each_match() {
        let ctx = egui::Context::default();
        let mut app = App::bare();
        let filler: String = (0..150).map(|i| format!("filler {i}\n\n")).collect();
        app.doc.text = format!(
            "**Alpha** one\n\n{filler}[alpha](https://alpha.example/alpha) two\n\n{filler}`alpha` three\n"
        );
        app.text_rev += 1;
        app.open_find();
        app.find.query = "alpha".to_owned();
        for _ in 0..3 {
            doc_frame(&ctx, &mut app);
        }
        assert!(!app.editing);
        assert_eq!(
            app.find.hits.len(),
            3,
            "rendered text only: the markup and the link's URL don't count"
        );

        for expect in 0..3 {
            app.jump_to_match(app.find.enter_jump(false));
            for _ in 0..2 {
                doc_frame(&ctx, &mut app);
            }
            assert_eq!(app.find.current, expect);
            let r = app.find.hits[expect];
            let off = app.preview_scroll.expect("preview scroll state");
            assert!(
                r.top() >= off && r.bottom() <= off + 650.0,
                "match {expect} at {r:?} is not in view at offset {off}"
            );
            if expect > 0 {
                assert!(off > 0.0, "match {expect} is below the first screen");
            }
        }
    }

    #[test]
    fn switching_to_the_editor_recounts_matches_in_the_source() {
        let ctx = egui::Context::default();
        let mut app = App::bare();
        app.doc.text = "[foo](https://foo.example)\n".to_owned();
        app.text_rev += 1;
        app.open_find();
        app.find.query = "foo".to_owned();
        for _ in 0..3 {
            doc_frame(&ctx, &mut app);
        }
        assert_eq!(app.find.hits.len(), 1, "the preview shows one foo");

        app.enter_edit();
        doc_frame(&ctx, &mut app);
        assert_eq!(app.find.matches.len(), 2, "the source has two");
        assert!(app.find.fresh);
    }

    #[test]
    fn jump_to_match_steps_wraps_and_stages_a_selection() {
        let mut app = App::bare();
        app.editing = true; // the editor's walk
        app.doc.text = "x foo y foo z FOO".to_owned();
        app.find.query = "foo".to_owned();
        app.find.recompute(&app.doc.text, app.text_rev);
        assert_eq!(app.find.matches.len(), 3);
        assert!(app.find.fresh);

        // First Enter after a recompute lands on match 1, not match 2.
        let j = app.find.enter_jump(false);
        assert_eq!(j, Jump::Stay);
        app.jump_to_match(j);
        assert_eq!(app.find.current, 0);
        assert!(!app.find.fresh);
        // "x foo…": the match spans chars 2..5; that exact range must be staged.
        assert_eq!(app.find.pending_select, Some((2, 5)));

        app.jump_to_match(Jump::Next);
        assert_eq!(app.find.current, 1);
        app.jump_to_match(Jump::Prev);
        assert_eq!(app.find.current, 0);
        app.jump_to_match(Jump::Prev);
        assert_eq!(app.find.current, 2, "prev from first wraps to last");

        // Empty matches: jumps are no-ops and stage nothing.
        app.find.matches.clear();
        app.find.current = 0;
        app.find.pending_select = None;
        app.jump_to_match(Jump::Next);
        assert_eq!(app.find.current, 0);
        assert_eq!(app.find.pending_select, None);
    }

    #[test]
    fn query_change_restarts_the_walk_at_the_first_match() {
        let mut app = App::bare();
        app.editing = true; // the editor's walk
        app.doc.text = "x foo y foo z FOO".to_owned();
        app.find.query = "foo".to_owned();
        app.find.recompute(&app.doc.text, app.text_rev);
        app.jump_to_match(Jump::Next);
        app.jump_to_match(Jump::Next);
        assert_eq!(app.find.current, 2, "navigated to the third match");

        // Typing a different query recomputes; even though the new query also
        // has at least three matches, the walk must restart at the first one.
        app.find.query = "o".to_owned();
        app.find.recompute(&app.doc.text, app.text_rev);
        assert_eq!(app.find.matches.len(), 6);
        assert_eq!(app.find.current, 0);
        assert!(app.find.fresh);
        app.jump_to_match(Jump::Stay); // what the bar schedules on query change
        assert_eq!(app.find.current, 0);
        assert_eq!(app.find.pending_select, Some((3, 4)), "first 'o' of 'foo'");
    }

    #[test]
    fn retained_query_resets_cleanly_across_document_opens() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        let b = dir.path().join("b.md");
        std::fs::write(&a, "foo foo foo foo").unwrap();
        std::fs::write(&b, "bar foo bar foo").unwrap();
        let mut app = App::bare();
        app.open_path(a);
        app.editing = true; // the editor's walk
        app.open_find();
        app.find.query = "foo".to_owned();
        app.find.recompute(&app.doc.text, app.text_rev);
        app.jump_to_match(app.find.enter_jump(false));
        app.jump_to_match(Jump::Next);
        assert_eq!(app.find.current, 1, "mid-walk in the first document");

        app.open_path(b);
        // The bar recomputes on its next frame because text_rev moved:
        assert_ne!(app.find.computed_rev, app.text_rev);
        app.find.recompute(&app.doc.text, app.text_rev);

        assert_eq!(app.find.matches.len(), 2);
        assert_eq!(app.find.current, 0, "the walk restarts in the new document");
        assert!(app.find.fresh, "no phantom position before the first jump");
        assert_eq!(
            app.find.enter_jump(false),
            Jump::Stay,
            "first Enter selects match 1/2, it does not skip to 2/2"
        );
        app.jump_to_match(app.find.enter_jump(false));
        assert_eq!(app.find.current, 0);
        // "bar foo…": chars 4..7 staged even though the new generation's editor
        // has never rendered (no TextEditState exists yet).
        assert_eq!(app.find.pending_select, Some((4, 7)));
    }

    #[test]
    fn edit_scroll_target_clamps_to_the_editor_range() {
        // Editor content fits its viewport: nothing to scroll, always top.
        assert_eq!(edit_scroll_target(120.0, 100.0, 300.0), 0.0);
        // Scrollable range is galley - viewport = 100: mid-range passes through.
        assert_eq!(edit_scroll_target(0.0, 400.0, 300.0), 0.0);
        assert_eq!(edit_scroll_target(50.0, 400.0, 300.0), 50.0);
        assert_eq!(edit_scroll_target(100.0, 400.0, 300.0), 100.0);
        // Beyond the editor's bottom, or negative bounce: clamps.
        assert_eq!(edit_scroll_target(250.0, 400.0, 300.0), 100.0);
        assert_eq!(edit_scroll_target(-10.0, 400.0, 300.0), 0.0);
    }

    #[test]
    fn terminal_starts_hidden_and_toggle_focuses_it() {
        let mut app = App::bare();
        assert!(!app.term.visible);
        app.apply_route(
            crate::term::routing::Route::ToggleTerminal,
            &egui::Context::default(),
        );
        assert!(app.term.visible && app.term.focused);
    }

    /// The toolbar button and the panel run in the same frame, and the
    /// click that opened the pane is still in the input queue with its
    /// position over the toolbar. term_panel skips its click-outside test
    /// when this flag is set, so the pane opens focused and typing works
    /// without a second click into it.
    #[test]
    fn the_toolbar_toggle_flags_the_frame_it_opens_the_pane_on() {
        let mut app = App::bare();
        let ctx = egui::Context::default();
        assert!(!app.term.just_opened);
        app.apply_route(crate::term::routing::Route::ToggleTerminal, &ctx);
        assert!(app.term.focused);
        assert!(
            app.term.just_opened,
            "the panel must know the opening click is not an outside click"
        );
    }

    #[test]
    fn closing_the_pane_while_editing_hands_focus_to_the_editor() {
        let mut app = App::bare();
        let ctx = egui::Context::default();
        app.editing = true;
        app.term.open();
        app.apply_route(crate::term::routing::Route::ToggleTerminal, &ctx);
        assert!(!app.term.visible);
        assert_eq!(ctx.memory(|m| m.focused()), Some(app.editor_id()));
    }

    #[test]
    fn zoom_routes_change_the_zoom_factor_and_reset() {
        let mut app = App::bare();
        let ctx = egui::Context::default();
        // set_zoom_factor only stages the value; egui applies it at the
        // start of the next pass, so each assertion needs a pass first.
        let pass = || {
            let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
            out.textures_delta.clear();
        };
        app.apply_route(crate::term::routing::Route::ZoomIn, &ctx);
        pass();
        assert!((ctx.zoom_factor() - 1.1).abs() < 1e-4);
        app.apply_route(crate::term::routing::Route::ZoomReset, &ctx);
        pass();
        assert_eq!(ctx.zoom_factor(), 1.0);
    }

    #[test]
    fn zoom_level_persists_to_the_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::bare();
        app.config_dir = Some(dir.path().to_owned());
        let ctx = egui::Context::default();

        app.apply_route(crate::term::routing::Route::ZoomIn, &ctx);

        let saved = crate::settings::load_zoom_from(dir.path()).unwrap();
        assert!((saved - 1.1).abs() < 1e-4, "saved {saved}");
    }

    #[test]
    fn preview_scroll_is_captured_from_real_scroll_state() {
        // Drives the real central() headlessly: render the preview, wheel-
        // scroll it, and assert the capture under the commonmark-derived id
        // sees the moved offset. Guards against the id-derivation drift that
        // once made the whole feature a silent no-op.
        let ctx = egui::Context::default();
        let mut app = App::bare();
        app.doc.text = (0..300).map(|i| format!("line {i}\n\n")).collect();
        app.text_rev += 1;
        let frame = |events: Vec<egui::Event>| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 700.0),
            )),
            events,
            ..Default::default()
        };
        let run = |app: &mut App, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(frame(events), |ui| app.central(ui));
            out.textures_delta.clear();
        };
        run(&mut app, vec![]);
        run(
            &mut app,
            vec![
                egui::Event::PointerMoved(egui::pos2(450.0, 350.0)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -300.0),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                },
            ],
        );
        // Smooth scrolling animates over a few frames; let it settle.
        for _ in 0..5 {
            run(&mut app, vec![]);
        }
        let captured = app
            .preview_scroll
            .expect("preview scroll state must be found under the commonmark-derived id");
        assert!(captured > 0.0, "wheel must move the offset, got {captured}");
        app.enter_edit();
        assert_eq!(
            app.pending_edit_scroll,
            Some(captured),
            "entering edit stages the captured offset"
        );
    }

    #[test]
    fn heading_jump_in_the_editor_selects_the_heading_by_char_index() {
        let mut app = App::bare();
        app.doc.text = "# é\n\n## Target\n".into();
        app.editing = true;

        app.jump_to_heading(1);

        let c = "# é\n\n".chars().count();
        assert_eq!(app.find.pending_select, Some((c, c)));
    }

    #[test]
    fn heading_jump_in_the_preview_scrolls_to_the_heading() {
        let ctx = egui::Context::default();
        let mut app = App::bare();
        app.doc.text = (0..300)
            .map(|i| format!("## Section {i}\n\nbody {i}\n\n"))
            .collect();
        app.text_rev += 1;
        let run = |app: &mut App| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 700.0),
                )),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| app.central(ui));
            out.textures_delta.clear();
        };

        // Requested before the first frame: it waits for the measured layout.
        app.jump_to_heading(150);
        for _ in 0..3 {
            run(&mut app);
        }

        let end_event = crate::doc::headings(&app.filtered)[150].end_event;
        let id = egui::Id::new(egui::Id::new(("preview", app.generation)));
        let top = egui_commonmark_backend::misc::scroll_cache(&mut app.cache, &id)
            .split_points
            .iter()
            .find(|p| p.0 == end_event)
            .expect("heading position is measured")
            .1
            .y;
        assert!(top > 0.0);
        let y = app.preview_scroll.expect("preview scroll state");
        assert!((y - top).abs() < 1.0, "scrolled to {y}, heading at {top}");
        assert!(app.pending_heading.is_none());
    }
}
