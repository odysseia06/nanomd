//! Headless per-frame cost measurement for the two render paths in app.rs.
//! Usage: cargo run --example frame_bench -- <file.md>
//!            [--edit] [--no-code] [--scrollable] [--scroll] [--drag[=px_per_frame]]

use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

fn strip_code_fences(text: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("usage: frame_bench <file.md> [--edit] [--no-code]");
    let edit_mode = args.iter().any(|a| a == "--edit");
    let no_code = args.iter().any(|a| a == "--no-code");
    let scrollable = args.iter().any(|a| a == "--scrollable");
    let scroll = args.iter().any(|a| a == "--scroll");
    // --drag[=px_per_frame]: press the vertical scrollbar handle and drag it
    // down, the way a user drags the bar (offset jumps, no smooth animation).
    let drag_step: Option<f32> = args.iter().find_map(|a| {
        a.strip_prefix("--drag").map(|rest| {
            rest.strip_prefix('=')
                .and_then(|v| v.parse().ok())
                .unwrap_or(4.0)
        })
    });

    let mut text = std::fs::read_to_string(path).expect("read file");
    if no_code {
        text = strip_code_fences(&text);
    }

    let ctx = egui::Context::default();
    egui_extras::install_image_loaders(&ctx);
    let mut cache = CommonMarkCache::default();

    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 700.0),
        )),
        ..Default::default()
    };

    if scroll {
        // Pointer must hover the scroll area for wheel events to reach it.
        input
            .events
            .push(egui::Event::PointerMoved(egui::pos2(450.0, 350.0)));
    }
    let frames = if let Some(step) = drag_step {
        // Enough frames to drag the handle across the whole bar, plus warmup.
        if step > 0.0 {
            ((676.0 / step) as usize + 7).min(400)
        } else {
            120
        }
    } else if scroll {
        400
    } else {
        25
    };
    let mut times = Vec::new();
    for frame in 0..frames {
        let start = std::time::Instant::now();
        let mut frame_input = input.clone();
        if scroll {
            frame_input.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -600.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::default(),
            });
        }
        if let Some(step) = drag_step {
            // Panel inner rect right edge is ~892 (8px margin); the bar's
            // interact rect spans the last ~10px. Handle starts at y ∈ [8, 20].
            let y = (12.0 + step * frame.saturating_sub(1) as f32).min(688.0);
            let pos = egui::pos2(888.0, y);
            frame_input.events.clear();
            frame_input.events.push(egui::Event::PointerMoved(pos));
            if frame == 1 {
                frame_input.events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                });
            }
        }
        let mut out = ctx.run_ui(frame_input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                if scrollable {
                    // Brings its own ScrollArea; must not be nested in ours.
                    CommonMarkViewer::new()
                        .viewport_cache(true)
                        .show_scrollable(egui::Id::new("bench"), ui, &mut cache, &text);
                    return;
                }
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        if edit_mode {
                            ui.add(
                                egui::TextEdit::multiline(&mut text)
                                    .code_editor()
                                    .desired_width(ui.available_width())
                                    .desired_rows(30)
                                    .frame(egui::Frame::NONE),
                            );
                        } else {
                            CommonMarkViewer::new().show(ui, &mut cache, &text);
                        }
                    });
            });
        });
        // Include tessellation so the timing reflects a real painted frame.
        let _clipped = ctx.tessellate(std::mem::take(&mut out.shapes), 1.0);
        let elapsed = start.elapsed();
        out.textures_delta.clear();
        // Sanity check: dragged_id must be Some once the handle is grabbed,
        // otherwise the drag missed the bar and the run measures nothing.
        if drag_step.is_some() && frame % 20 == 0 {
            eprintln!(
                "frame {frame}: {:.1} ms dragged_id={:?}",
                elapsed.as_secs_f64() * 1000.0,
                ctx.dragged_id()
            );
        }
        if frame >= 5 {
            times.push(elapsed.as_secs_f64() * 1000.0);
        }
    }
    let avg = times.iter().sum::<f64>() / times.len() as f64;
    let min = times.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    println!(
        "mode={} no_code={} bytes={} | frame ms avg={avg:.1} min={min:.1} max={max:.1}",
        if edit_mode {
            "edit"
        } else if scrollable {
            "view-scrollable"
        } else {
            "view"
        },
        no_code,
        text.len(),
    );
}
