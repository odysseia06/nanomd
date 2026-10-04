//! LaTeX math rendered locally, using the same glyph outlines for preview and print.

use eframe::egui::{self, Color32};
use latex_rust::{Color, Dim, MathFont, SvgOptions};
use std::{collections::HashMap, sync::OnceLock};

// Keep a single expression from creating an enormous layout or texture.
const MAX_SOURCE: usize = 4096;
const MAX_SIDE: f32 = 4096.0;
const MAX_PIXELS: f32 = 1_048_576.0;

/// Self-contained SVG: paths only, with no external fonts or resources.
pub fn svg(source: &str, inline: bool, font_size: f32, color: Color32) -> Result<String, String> {
    if source.len() > MAX_SOURCE {
        return Err("Equation is too long to render".into());
    }
    static FONT: OnceLock<Option<MathFont>> = OnceLock::new();
    let font = FONT
        .get_or_init(|| MathFont::stix_two_math().ok())
        .as_ref()
        .ok_or("Could not load the math font")?;
    latex_rust::latex_to_svg(
        source,
        font,
        &SvgOptions {
            font_size_pt: Dim::from_ieee32_bits(font_size.to_bits()),
            color: Color::rgb(color.r(), color.g(), color.b()),
            display: !inline,
        },
    )
    .map_err(|e| e.to_string())
}

struct Equation {
    texture: egui::TextureHandle,
    size: egui::Vec2,
}

/// Lives for the current document; clearing drops its GPU textures as well.
#[derive(Default)]
pub struct Math {
    entries: HashMap<egui::Id, Result<Equation, String>>,
}

impl Math {
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn show(&mut self, ui: &mut egui::Ui, source: &str, inline: bool) {
        let font_size = egui::TextStyle::Body.resolve(ui.style()).size;
        let color = ui.visuals().text_color();
        let scale = ui.ctx().pixels_per_point().max(2.0);
        let key = egui::Id::new((source, inline, font_size.to_bits(), scale.to_bits(), color));
        let equation = self.entries.entry(key).or_insert_with(|| {
            let svg = svg(source, inline, font_size, color)?;
            let image = rasterize(&svg, scale)?;
            let size = image.source_size;
            let texture =
                ui.ctx()
                    .load_texture("LaTeX equation", image, egui::TextureOptions::LINEAR);
            Ok(Equation { texture, size })
        });
        if !inline {
            egui_commonmark_backend::elements::newline(ui);
        }
        match equation {
            Ok(equation) => {
                // Wide equations shrink to the page, never force horizontal scrolling.
                let size = equation.size * (ui.max_rect().width() / equation.size.x).min(1.0);
                if inline {
                    ui.add(egui::Image::new((equation.texture.id(), size)))
                        .on_hover_text(source);
                } else {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), size.y + 8.0),
                        egui::Sense::hover(),
                    );
                    let picture = egui::Rect::from_center_size(rect.center(), size);
                    ui.painter().image(
                        equation.texture.id(),
                        picture,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    response.on_hover_text(source);
                }
            }
            Err(error) => {
                let delimiter = if inline { "$" } else { "$$" };
                ui.label(
                    egui::RichText::new(format!("{delimiter}{source}{delimiter}")).monospace(),
                )
                .on_hover_text(format!("Could not render this equation: {error}"));
            }
        }
        if !inline {
            egui_commonmark_backend::elements::newline(ui);
        }
    }
}

fn rasterize(svg: &str, scale: f32) -> Result<egui::ColorImage, String> {
    // SVG uses physical points. Here one point is one egui logical point.
    let tree = resvg::usvg::Tree::from_str(
        svg,
        &resvg::usvg::Options {
            dpi: 72.0,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;
    let size = tree.size();
    let (w, h) = (
        (size.width() * scale).ceil(),
        (size.height() * scale).ceil(),
    );
    if w > MAX_SIDE || h > MAX_SIDE || w * h > MAX_PIXELS {
        return Err("Equation is too large to render".into());
    }
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(w as u32, h as u32).ok_or("Equation has no drawable area")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut image =
        egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data());
    image.source_size = egui::vec2(size.width(), size.height());
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_equations_render_in_both_themes_and_at_high_dpi() {
        for source in [
            r"E = mc^2",
            r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
            r"\sum_{n=1}^{\infty} \frac{1}{n^2} = \frac{\pi^2}{6}",
            r"\int_0^1 x^2\,dx = \frac{1}{3}",
            r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
            r"\begin{aligned} a &= b+c \\ d &= e+f \end{aligned}",
        ] {
            for inline in [true, false] {
                for color in [Color32::BLACK, Color32::WHITE] {
                    let svg = svg(source, inline, 16.0, color).unwrap();
                    let image = rasterize(&svg, 2.0).unwrap();
                    assert!(
                        image.source_size.x > 5.0 && image.source_size.y > 5.0,
                        "{source}"
                    );
                    assert!(image.size[0] as f32 >= image.source_size.x * 2.0);
                    let ink = image.pixels.iter().find(|p| p.a() > 128).unwrap();
                    assert_eq!(ink.r() > 128, color == Color32::WHITE, "{source}");
                }
            }
        }
    }

    #[test]
    fn invalid_and_oversized_equations_have_a_fallback() {
        for source in [
            r"\frac{1}{",
            r"\notacommand{x}",
            &"x".repeat(MAX_SOURCE + 1),
        ] {
            assert!(svg(source, true, 16.0, Color32::BLACK).is_err());
        }
        let huge = svg("x", true, 5000.0, Color32::BLACK).unwrap();
        assert!(rasterize(&huge, 2.0).is_err());
    }
}
