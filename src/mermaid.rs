//! Mermaid diagrams in the preview, drawn natively on a background thread.
//!
//! [`Diagrams::rewrite`] turns each ```` ```mermaid ```` fence that has been
//! drawn into an image the preview loads from `mermaid://…`, which this
//! module's image loader serves. A fence not drawn yet, or one that can't
//! be drawn, stays a code block: the preview shows the source until the
//! picture is ready, or for good if it never is.

use eframe::egui::{self, ColorImage};
use egui::load::{ImageLoadResult, ImageLoader, ImagePoll, LoadError, SizeHint};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::{Arc, Mutex, mpsc};

const SCHEME: &str = "mermaid://";
/// Pixels per point the diagrams are drawn at, for sharp text on HiDPI.
const SCALE: f32 = 2.0;
/// Longest side, in pixels, a drawn diagram may have.
const MAX_SIDE: f32 = 4096.0;
/// Longest source drawn, in lines. Layout time grows fast with size (an
/// 800-edge flowchart takes 40 s), and diagrams are drawn one at a time.
const MAX_LINES: usize = 200;

/// A diagram: its source's hash, and whether it's drawn in dark colors.
type Key = (u64, bool);

fn uri((hash, dark): Key) -> String {
    format!(
        "{SCHEME}{hash:016x}-{}",
        if dark { "dark" } else { "light" }
    )
}

fn parse_uri(uri: &str) -> Option<Key> {
    let (hash, theme) = uri.strip_prefix(SCHEME)?.split_once('-')?;
    Some((u64::from_str_radix(hash, 16).ok()?, theme == "dark"))
}

fn hash(source: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    h.finish()
}

/// Where a diagram is.
#[derive(Clone)]
enum Slot {
    Drawing,
    Drawn(Arc<ColorImage>),
    Failed,
}

#[derive(Default)]
struct Shared {
    slots: HashMap<Key, Slot>,
    /// Hashes of the diagrams in the text last rewritten. Others are no
    /// longer on screen: skipped if queued, dropped if drawn.
    wanted: HashSet<u64>,
    /// A diagram was drawn since [`Diagrams::take_fresh`] last looked.
    fresh: bool,
}

/// A diagram for the worker to draw.
struct Job {
    key: Key,
    source: String,
}

/// The diagrams of the document on screen.
#[derive(Default)]
pub struct Diagrams {
    shared: Arc<Mutex<Shared>>,
    /// The drawing thread's queue, started with the first diagram.
    worker: Option<mpsc::Sender<Job>>,
    /// Repainted when a diagram is drawn; None in tests.
    ctx: Option<egui::Context>,
}

impl Diagrams {
    /// Serves drawn diagrams to `ctx`'s images, and repaints it when one is
    /// ready.
    pub fn install(&mut self, ctx: &egui::Context) {
        ctx.add_image_loader(Arc::new(Loader(self.shared.clone())));
        self.ctx = Some(ctx.clone());
    }

    /// Whether a diagram was drawn since the last call: the preview's text
    /// then needs rewriting.
    pub fn take_fresh(&self) -> bool {
        std::mem::take(&mut self.shared.lock().unwrap().fresh)
    }

    /// `text` with each drawn ```` ```mermaid ```` fence replaced by its
    /// picture, in `dark` or light colors (the other theme's while this
    /// one is drawn). Fences not drawn yet are queued.
    pub fn rewrite(&mut self, text: &str, dark: bool) -> String {
        let fences = fences(text);
        self.keep_only(fences.iter().map(|f| hash(&f.source)).collect());
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for f in fences {
            if let Some(key) = self.picture_or_queue(f.source, dark) {
                out.push_str(&text[at..f.range.start]);
                out.push_str(&picture(text, f.range.clone(), key, f.in_list));
                at = f.range.end;
            }
        }
        out.push_str(&text[at..]);
        out
    }

    /// The picture to show for `source`: drawn in the wanted theme, or
    /// in the other one while the wanted is drawn. Queues the wanted one
    /// (once, and only if it isn't too long). None while neither is drawn.
    fn picture_or_queue(&mut self, source: String, dark: bool) -> Option<Key> {
        let (want, other) = ((hash(&source), dark), (hash(&source), !dark));
        let mut shared = self.shared.lock().unwrap();
        let drawn = |s: &Shared, k: &Key| matches!(s.slots.get(k), Some(Slot::Drawn(_)));
        if drawn(&shared, &want) {
            return Some(want);
        }
        let fallback = drawn(&shared, &other).then_some(other);
        if shared.slots.contains_key(&want) || source.lines().count() > MAX_LINES {
            return fallback;
        }
        shared.slots.insert(want, Slot::Drawing);
        drop(shared);
        let job = Job { key: want, source };
        let _ = self.worker().send(job);
        fallback
    }

    /// Drops the diagrams not in `wanted` (their textures too), and tells
    /// the worker which ones still are.
    fn keep_only(&mut self, wanted: HashSet<u64>) {
        let mut gone = Vec::new();
        let mut shared = self.shared.lock().unwrap();
        shared.slots.retain(|&key, slot| {
            let keep = wanted.contains(&key.0) || matches!(slot, Slot::Drawing);
            if !keep && matches!(slot, Slot::Drawn(_)) {
                gone.push(uri(key));
            }
            keep
        });
        shared.wanted = wanted;
        drop(shared); // forgetting calls back into the loader
        if let Some(ctx) = &self.ctx {
            for uri in gone {
                ctx.forget_image(&uri);
            }
        }
    }

    /// The drawing thread's queue, starting the thread on first use.
    fn worker(&mut self) -> &mpsc::Sender<Job> {
        self.worker.get_or_insert_with(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            let (shared, ctx) = (self.shared.clone(), self.ctx.clone());
            let _ = std::thread::Builder::new()
                .name("mermaid".to_owned())
                .spawn(move || work(&rx, &shared, ctx.as_ref()));
            tx
        })
    }
}

/// The drawing thread: draws queued diagrams one at a time, skipping any
/// no longer on screen.
fn work(jobs: &mpsc::Receiver<Job>, shared: &Mutex<Shared>, ctx: Option<&egui::Context>) {
    // Loading the system fonts takes about a second: here, once, off the
    // UI thread.
    let mut fonts = None;
    while let Ok(job) = jobs.recv() {
        {
            let mut s = shared.lock().unwrap();
            if !s.wanted.contains(&job.key.0) {
                s.slots.remove(&job.key); // queued again if it comes back
                continue;
            }
        }
        let fonts = fonts.get_or_insert_with(|| {
            let mut db = resvg::usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        });
        // ponytail: release builds abort on a panic, so a renderer panic
        // would take the app down; 20k fuzzed diagrams found none. Draw in
        // a child process if one ever turns up.
        let image = draw(&job.source, job.key.1, fonts);
        let drawn = image.is_some();
        let mut s = shared.lock().unwrap();
        s.slots.insert(
            job.key,
            image.map_or(Slot::Failed, |i| Slot::Drawn(Arc::new(i))),
        );
        // A failure leaves the code block as it is: nothing to redraw.
        s.fresh |= drawn;
        drop(s);
        if let (true, Some(ctx)) = (drawn, ctx) {
            ctx.request_repaint();
        }
    }
}

/// A ```` ```mermaid ```` fence: its range in the text, its source, and
/// whether it sits in a list item.
struct Fence {
    range: Range<usize>,
    source: String,
    in_list: bool,
}

fn fences(text: &str) -> Vec<Fence> {
    let mut out = Vec::new();
    let mut open: Option<Fence> = None;
    let mut lists = 0;
    for (event, range) in Parser::new_ext(text, crate::doc::parser_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::List(_)) => lists += 1,
            Event::End(TagEnd::List(_)) => lists -= 1,
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                if info.split_whitespace().next() == Some("mermaid") =>
            {
                open = Some(Fence {
                    range,
                    source: String::new(),
                    in_list: lists > 0,
                });
            }
            Event::Text(t) => {
                if let Some(f) = open.as_mut() {
                    f.source.push_str(&t);
                }
            }
            Event::End(TagEnd::CodeBlock) => out.extend(open.take()),
            _ => {}
        }
    }
    out
}

/// The image paragraph that replaces the fence at `range`, built so the
/// blocks around it stay as they were: at the fence's indentation
/// (blockquote `>` kept, list markers as spaces), and with a line of that
/// indentation before and after it, so no paragraph runs into it.
fn picture(text: &str, range: Range<usize>, key: Key, in_list: bool) -> String {
    let line_start = text[..range.start].rfind('\n').map_or(0, |i| i + 1);
    let before = &text[line_start..range.start];
    let indent: String = before
        .chars()
        .map(|c| if c == '>' { '>' } else { ' ' })
        .collect();
    // A fence after a list marker starts the item: a newline there would
    // leave the marker alone on its line, which under a paragraph is a
    // heading underline.
    let open = if before.chars().all(|c| c == '>' || c.is_whitespace()) {
        format!("\n{indent}")
    } else {
        String::new()
    };
    // In a list item the viewer flows a paragraph on after the item's
    // text: a no-break space and a hard break put the picture on a row of
    // its own.
    let lead = if in_list {
        format!("\u{a0}  \n{indent}")
    } else {
        String::new()
    };
    // The fence's own line break usually follows it; a blank line of the
    // same indentation goes before that, not an empty line, which would
    // end a blockquote.
    let close = if text[range.end..].starts_with(['\n', '\r']) {
        format!("\n{indent}")
    } else {
        format!("\n{indent}\n")
    };
    format!("{open}{lead}![Mermaid diagram]({}){close}", uri(key))
}

/// `source` drawn as a picture: Mermaid to SVG, then SVG to pixels.
fn draw(
    source: &str,
    dark: bool,
    fonts: &Arc<resvg::usvg::fontdb::Database>,
) -> Option<ColorImage> {
    let theme = if dark {
        mermaid_rs_renderer::Theme::dark()
    } else {
        mermaid_rs_renderer::Theme::modern()
    };
    let options = mermaid_rs_renderer::RenderOptions {
        theme,
        ..Default::default()
    };
    let svg = mermaid_rs_renderer::render_with_options(source, options).ok()?;
    let tree = resvg::usvg::Tree::from_str(
        &svg,
        &resvg::usvg::Options {
            fontdb: fonts.clone(),
            ..Default::default()
        },
    )
    .ok()?;
    let size = tree.size();
    let scale = SCALE.min(MAX_SIDE / size.width().max(size.height()));
    let (w, h) = (
        (size.width() * scale).ceil() as u32,
        (size.height() * scale).ceil() as u32,
    );
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut image = ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data());
    image.source_size = egui::vec2(size.width(), size.height());
    Some(image)
}

/// Serves drawn diagrams to egui's images by their `mermaid://` URI.
struct Loader(Arc<Mutex<Shared>>);

impl ImageLoader for Loader {
    fn id(&self) -> &str {
        "nanomd::mermaid"
    }

    fn load(&self, _ctx: &egui::Context, uri: &str, _: SizeHint) -> ImageLoadResult {
        let Some(key) = parse_uri(uri) else {
            return Err(LoadError::NotSupported);
        };
        match self.0.lock().unwrap().slots.get(&key) {
            Some(Slot::Drawn(image)) => Ok(ImagePoll::Ready {
                image: image.clone(),
            }),
            _ => Err(LoadError::Loading("diagram not drawn".to_owned())),
        }
    }

    // Diagrams are dropped by `Diagrams::keep_only`, which forgets them.
    fn forget(&self, _uri: &str) {}

    fn forget_all(&self) {}

    fn byte_size(&self) -> usize {
        let shared = self.0.lock().unwrap();
        shared
            .slots
            .values()
            .map(|s| match s {
                Slot::Drawn(image) => image.pixels.len() * 4,
                _ => 0,
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Diagrams {
        fn busy(&self) -> bool {
            let shared = self.shared.lock().unwrap();
            shared.slots.values().any(|s| matches!(s, Slot::Drawing))
        }

        /// Marks `source` as drawn, as if the worker had finished it.
        fn pretend_drawn(&self, source: &str, dark: bool) {
            let image = Arc::new(ColorImage::filled([2, 2], egui::Color32::WHITE));
            let mut shared = self.shared.lock().unwrap();
            shared
                .slots
                .insert((hash(source), dark), Slot::Drawn(image));
        }
    }

    const FLOW: &str = "flowchart TD\n  A --> B\n";

    /// Headings, quotes, lists and items: what a rewrite must not change.
    fn shape(text: &str) -> Vec<String> {
        Parser::new_ext(text, crate::doc::parser_options())
            .filter_map(|e| match e {
                Event::Start(
                    t @ (Tag::Heading { .. } | Tag::BlockQuote(_) | Tag::List(_) | Tag::Item),
                ) => Some(format!("{t:?}")),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_drawn_fence_becomes_a_picture_and_a_broken_one_stays_code() {
        let mut d = Diagrams::default();
        let text = format!(
            "Before\n\n```mermaid\n{FLOW}```\n\n```mermaid\nflowchart TD\n  A -->\n```\nAfter\n"
        );
        // Not drawn yet: the source shows as before.
        assert_eq!(d.rewrite(&text, false), text);
        for _ in 0..500 {
            if !d.busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!d.busy(), "diagrams never finished drawing");

        let out = d.rewrite(&text, false);
        let pictures: Vec<&str> = out.lines().filter(|l| l.contains(SCHEME)).collect();
        assert_eq!(pictures.len(), 1, "{out}");
        assert!(
            out.contains("```mermaid\nflowchart TD\n  A -->\n```"),
            "{out}"
        );

        let link = pictures[0].split('(').nth(1).unwrap().trim_end_matches(')');
        let ctx = egui::Context::default();
        let loaded = Loader(d.shared.clone()).load(&ctx, link, SizeHint::default());
        let Ok(ImagePoll::Ready { image }) = loaded else {
            panic!("the loader serves the drawn picture");
        };
        assert!(image.source_size.x > 10.0 && image.size[0] as f32 >= image.source_size.x * 1.9);
    }

    #[test]
    fn pictures_keep_the_blocks_around_them() {
        let fence = |indent: &str| {
            let body: String = FLOW.lines().map(|l| format!("{indent}{l}\n")).collect();
            format!("```mermaid\n{body}{indent}```")
        };
        let cases = [
            format!("para\n{}\nafter\n", fence("")),
            format!("Steps:\n- {}\n- next\n", fence("  ")),
            format!("- parent\n  - {}\n- next\n", fence("    ")),
            format!("1. a\n2. {}\n3. c\n", fence("   ")),
            format!("> q\n>\n> {}\n> more\n", fence("> ")),
            format!("> 1. a\n> 2. {}\n> 3. c\n", fence(">    ")),
            format!("# Top\n\n{}\n## Next\n", fence("")).replace('\n', "\r\n"),
        ];
        for text in cases {
            let mut d = Diagrams::default();
            d.pretend_drawn(FLOW, false);
            let out = d.rewrite(&text, false);
            assert!(out.contains(SCHEME), "not replaced:\n{text}");
            assert_eq!(shape(&out), shape(&text), "\n{text}\n---\n{out}");
            assert_eq!(
                crate::doc::blocks(&out).len(),
                crate::doc::blocks(&text).len(),
                "\n{text}\n---\n{out}"
            );
        }
    }

    #[test]
    fn a_theme_switch_shows_the_other_picture_until_its_own_is_drawn() {
        let mut d = Diagrams::default();
        d.pretend_drawn(FLOW, false);
        let out = d.rewrite(&format!("```mermaid\n{FLOW}```\n"), true);
        assert!(out.contains(&uri((hash(FLOW), false))), "{out}");
    }

    #[test]
    fn diagrams_no_longer_on_screen_are_dropped() {
        let mut d = Diagrams::default();
        d.pretend_drawn(FLOW, false);
        d.pretend_drawn("pie\n  \"a\" : 1\n", false);
        d.rewrite(&format!("```mermaid\n{FLOW}```\n"), false);
        let shared = d.shared.lock().unwrap();
        assert_eq!(shared.slots.len(), 1);
        assert!(shared.slots.contains_key(&(hash(FLOW), false)));
    }
}
