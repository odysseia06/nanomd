//! Print by handing the browser a standalone HTML page of the buffer: its
//! print dialog already covers every printer and Save as PDF, on every OS,
//! so nano.md needs no PDF writer or print backend of its own.

use pulldown_cmark::{Event, Parser, Tag, TagEnd, html};

use crate::doc::{demote_remote_images, parser_options};

/// The page for `src`. `base` is the document folder's `file://` URL, so
/// relative images resolve as they do in the preview.
pub fn page(src: &str, title: &str, base: Option<&str>) -> String {
    let src = crate::doc::literal_label_math(&demote_remote_images(src));
    // Raw HTML stays text, as in the preview: the file may be agent-written
    // and this page opens in a real browser.
    let events = Parser::new_ext(&src, parser_options()).flat_map(|event| match event {
        Event::Start(Tag::HtmlBlock) => vec![Event::Start(Tag::Paragraph)],
        Event::End(TagEnd::HtmlBlock) => vec![Event::End(TagEnd::Paragraph)],
        Event::Html(line) => vec![
            Event::Text(line.trim_end().to_owned().into()),
            Event::HardBreak,
        ],
        Event::InlineHtml(text) => vec![Event::Text(text)],
        Event::InlineMath(source) => vec![math_html(&source, true)],
        Event::DisplayMath(source) => vec![math_html(&source, false)],
        other => vec![other],
    });
    let mut body = String::new();
    html::push_html(&mut body, events);
    let base = base
        .map(|b| format!("<base href=\"{}\">", escape(&encode(b))))
        .unwrap_or_default();
    let title = escape(title);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src file: data:; style-src 'unsafe-inline'; script-src 'nonce-nanomd'">
{base}
<title>{title}</title>
<style>{STYLE}</style>
<script nonce="nanomd">addEventListener("load", () => print());</script>
</head>
<body>
{body}</body>
</html>
"#
    )
}

fn math_html(source: &str, inline: bool) -> Event<'static> {
    match crate::math::svg(source, inline, 11.0, eframe::egui::Color32::from_gray(27)) {
        Ok(svg) => {
            // An embedded SVG has no XML declaration. All markup here comes
            // from the renderer; document HTML still takes the escaped path.
            let svg = &svg[svg.find("<svg").unwrap_or(0)..];
            let class = if inline {
                "math-inline"
            } else {
                "math-display"
            };
            Event::InlineHtml(
                format!(
                    "<span class=\"{class}\" role=\"math\" aria-label=\"{}\">{svg}</span>",
                    escape(source)
                )
                .into(),
            )
        }
        Err(_) => {
            let delimiter = if inline { "$" } else { "$$" };
            Event::Code(format!("{delimiter}{source}{delimiter}").into())
        }
    }
}

/// Percent-encodes everything but unreserved characters and the `/` and `:`
/// of a `file://` URL, so spaces, `#` and non-ASCII paths survive.
pub fn encode(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for b in url.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Light on paper whatever the app theme: a dark page wastes ink.
const STYLE: &str = r#"
@page { margin: 18mm 16mm; }
:root { color-scheme: light; }
body {
  margin: 0 auto; padding: 32px 24px; max-width: 46em;
  background: #fff; color: #1b1b1b;
  font: 11pt/1.55 system-ui, -apple-system, "Segoe UI", Ubuntu, sans-serif;
}
@media print { body { padding: 0; max-width: none; } }
h1, h2, h3, h4, h5, h6 { line-height: 1.25; margin: 1.4em 0 .5em; break-after: avoid; }
h1 { font-size: 1.8em; } h2 { font-size: 1.4em; } h3 { font-size: 1.15em; }
body > :first-child { margin-top: 0; }
p, ul, ol, table, pre, blockquote { margin: 0 0 .8em; }
a { color: #0b57d0; }
code, pre {
  font-family: ui-monospace, "Cascadia Mono", Consolas, "SF Mono", Menlo, monospace;
  font-size: .9em;
}
code { background: #f1f1f1; border-radius: 3px; padding: .1em .3em; }
pre {
  background: #f6f6f6; border: 1px solid #e2e2e2; border-radius: 4px;
  padding: .7em .9em; white-space: pre-wrap; overflow-wrap: anywhere; break-inside: avoid;
}
pre code { background: none; padding: 0; font-size: 1em; }
blockquote { margin-left: 0; padding-left: 1em; border-left: 3px solid #d0d0d0; color: #555; }
table { border-collapse: collapse; }
th, td { border: 1px solid #d0d0d0; padding: .3em .6em; text-align: left; }
th { background: #f6f6f6; }
tr { break-inside: avoid; }
img { max-width: 100%; }
.math-inline svg { max-width: 100%; height: auto; vertical-align: middle; }
.math-display { display: block; text-align: center; margin: .8em 0; break-inside: avoid; }
.math-display svg { max-width: 100%; height: auto; }
li:has(> input[type=checkbox]) { list-style: none; margin-left: -1.3em; }
input[type=checkbox] { margin: 0 .4em 0 0; vertical-align: -1px; }
hr { border: 0; border-top: 1px solid #d0d0d0; margin: 1.5em 0; }
.footnote-definition { font-size: .9em; margin: .3em 0; }
.footnote-definition-label { margin-right: .4em; }
.footnote-definition p { display: inline; }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latex_math_prints_as_self_contained_equations() {
        let page = page("Inline $x^2$ here.\n\n$$\n\\frac{1}{2}\n$$\n", "Math", None);
        assert_eq!(page.matches("<svg").count(), 2, "{page}");
        assert!(page.contains("math-inline"));
        assert!(page.contains("math-display"));
        assert!(!page.contains("src=\"http"));
    }

    #[test]
    fn latex_in_code_and_escaped_dollars_stays_literal() {
        let page = page("`$x$`\n\n```tex\n$$x$$\n```\n\n\\$5 and \\$10\n", "", None);
        assert!(!page.contains("<svg"));
        assert!(page.contains("<code>$x$</code>"));
        assert!(page.contains("$$x$$"));
        assert!(page.contains("$5 and $10"));
    }

    #[test]
    fn latex_in_image_descriptions_stays_source() {
        let page = page(r"![Energy $E=mc^2$](local.png)", "", None);
        assert!(page.contains(r#"alt="Energy $E=mc^2$""#), "{page}");
        assert!(!page.contains("<svg"));
        assert!(!page.contains("&lt;svg"));
    }

    #[test]
    fn unsupported_latex_prints_as_escaped_source() {
        let page = page(r"$\notacommand{<script>}$", "", None);
        assert!(page.contains(r"\notacommand{&lt;script&gt;}"));
        assert!(!page.contains("<svg"));
    }

    #[test]
    fn page_escapes_raw_html_title_and_demotes_remote_images() {
        let page = page(
            "# T\n\n<script>alert(1)</script>\n\nhi <b>x</b> ![p](https://e.com/p.png)\n",
            "a<b>&c",
            Some("file:///C:/my docs/#1/"),
        );
        assert!(page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!page.contains("<script>alert"));
        assert!(page.contains("&lt;b&gt;x&lt;/b&gt;"));
        assert!(page.contains("<title>a&lt;b&gt;&amp;c</title>"));
        assert!(page.contains(r#"<a href="https://e.com/p.png">p</a>"#));
        assert!(!page.contains("<img"));
        assert!(page.contains(r#"<base href="file:///C:/my%20docs/%231/">"#));
    }

    #[test]
    fn encode_keeps_url_structure_and_escapes_the_rest() {
        assert_eq!(encode("file:///C:/a b/ç.md"), "file:///C:/a%20b/%C3%A7.md");
    }
}
