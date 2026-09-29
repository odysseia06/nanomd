//! One-off check: does the cached egui_extras highlighter still colorize the
//! fence languages used in real docs? Prints distinct color count per language.

use eframe::egui;

fn main() {
    let ctx = egui::Context::default();
    let samples = [
        ("rs", "fn main() { let x: u32 = 1; println!(\"{x}\"); }"),
        ("py", "def f(x):\n    return x + 1"),
        ("js", "function f(x) { return x + 1; }"),
        ("sh", "for f in *.md; do echo \"$f\"; done"),
        ("rb", "def f(x)\n  x + 1\nend"),
        ("pl", "sub f { my $x = shift; return $x + 1; }"),
        ("hs", "f :: Int -> Int\nf x = x + 1"),
        ("erl", "f(X) -> X + 1."),
        ("go", "func f(x int) int { return x + 1 }"),
        ("clj", "(defn f [x] (+ x 1))"),
        ("tex", "\\section{Intro} $x^2$"),
        ("ml", "let f x = x + 1"),
        ("cs", "int F(int x) { return x + 1; }"),
        ("bash", "for f in *.md; do echo \"$f\"; done"),
        ("markdown", "# Title\n*em* [link](https://x.y)"),
    ];
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 700.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| {
        let theme = egui_extras::syntax_highlighting::CodeTheme::from_style(ui.style());
        for (lang, code) in samples {
            let job = egui_extras::syntax_highlighting::highlight(
                ui.ctx(),
                ui.style(),
                &theme,
                code,
                lang,
            );
            let colors: std::collections::HashSet<[u8; 4]> = job
                .sections
                .iter()
                .map(|s| s.format.color.to_array())
                .collect();
            println!(
                "{lang}: {} sections, {} distinct colors",
                job.sections.len(),
                colors.len()
            );
        }
    });
    out.textures_delta.clear();
}
