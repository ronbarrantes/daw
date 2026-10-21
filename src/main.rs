mod audio;
mod drums;

use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{self, Color32, Stroke, Vec2};

use audio::{Shared, STEPS, TRACKS};

fn main() -> eframe::Result<()> {
    let shared = Arc::new(Shared::new());
    // Held until the window closes; dropping the stream stops audio.
    let stream = audio::start(shared.clone());
    let error = stream.as_ref().err().cloned();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([760.0, 240.0]),
        ..Default::default()
    };
    eframe::run_native(
        "daw",
        options,
        Box::new(|_cc| Ok(Box::new(App { shared, error }))),
    )
}

struct App {
    shared: Arc<Shared>,
    error: Option<String>,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let s = &self.shared;
        let playing = s.playing.load(Relaxed);

        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            s.playing.store(!playing, Relaxed);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(err) = &self.error {
                ui.colored_label(Color32::RED, format!("Audio failed: {err}"));
            }

            ui.horizontal(|ui| {
                let label = if playing { "■ Stop" } else { "▶ Play" };
                if ui.button(label).clicked() {
                    s.playing.store(!playing, Relaxed);
                }
                let mut bpm = s.bpm();
                ui.label("BPM");
                if ui
                    .add(egui::DragValue::new(&mut bpm).range(40.0..=240.0).speed(0.5))
                    .changed()
                {
                    s.set_bpm(bpm);
                }
                if ui.button("Clear").clicked() {
                    s.clear();
                }
                ui.weak("Space = play/stop · click a name to hear it");
            });
            ui.add_space(8.0);

            let current = playing.then(|| s.current_step.load(Relaxed));
            for track in 0..TRACKS {
                ui.horizontal(|ui| {
                    let name = egui::Button::new(drums::NAMES[track]).min_size(Vec2::new(60.0, 32.0));
                    if ui.add(name).clicked() {
                        s.preview(track);
                    }
                    for step in 0..STEPS {
                        // Shade every other group of four so beats are easy to see.
                        let fill = match (s.is_on(track, step), (step / 4) % 2 == 0) {
                            (true, _) => Color32::from_rgb(255, 140, 40),
                            (false, true) => Color32::from_gray(60),
                            (false, false) => Color32::from_gray(45),
                        };
                        let stroke = if current == Some(step) {
                            Stroke::new(2.0_f32, Color32::WHITE)
                        } else {
                            Stroke::NONE
                        };
                        let pad = egui::Button::new("")
                            .fill(fill)
                            .stroke(stroke)
                            .min_size(Vec2::new(32.0, 32.0));
                        if ui.add(pad).clicked() {
                            s.toggle(track, step);
                        }
                    }
                });
            }
        });

        // Only redraw continuously while playing, to keep CPU low when idle.
        if playing {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
}
