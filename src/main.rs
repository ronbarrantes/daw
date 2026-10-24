mod audio;
mod drums;
mod midi;
mod synth;

use std::f32::consts::PI;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{self, Align, Color32, Layout, Pos2, Sense, Stroke, Vec2};

use audio::{PatternData, Shared, PATTERNS, SONG_SLOTS, STEPS, TRACKS};
use midi::Midi;
use synth::{Param, Unit, PARAMS};

const DRUM_ON: Color32 = Color32::from_rgb(255, 140, 40);
const NOTE_ON: Color32 = Color32::from_rgb(80, 180, 255);

fn main() -> eframe::Result<()> {
    let shared = Arc::new(Shared::new());
    let (midi_tx, midi_rx) = sync_channel(1024);
    let midi = Midi::new(shared.clone(), midi_rx);
    // Held until the window closes; dropping the stream stops audio.
    let stream = audio::start(shared.clone(), midi_tx);
    let error = stream.as_ref().err().cloned();

    // Sized to fit the 1366x768 screen of an 11" MacBook Air.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1000.0, 660.0]),
        ..Default::default()
    };
    eframe::run_native(
        "daw",
        options,
        Box::new(|_cc| {
            Ok(Box::new(App {
                shared,
                midi,
                error,
                clipboard: None,
            }))
        }),
    )
}

struct App {
    shared: Arc<Shared>,
    midi: Midi,
    error: Option<String>,
    clipboard: Option<PatternData>,
}

fn pattern_name(p: usize) -> String {
    char::from(b'A' + p as u8).to_string()
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let s = self.shared.clone();
        let playing = s.playing.load(Relaxed);

        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            s.playing.store(!playing, Relaxed);
        }

        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            ui.add_space(4.0);
            if let Some(err) = &self.error {
                ui.colored_label(Color32::RED, format!("Audio failed: {err}"));
            }
            if let Some(err) = &self.midi.error {
                ui.colored_label(Color32::RED, err);
            }
            self.transport_row(ui, playing);
            self.midi_row(ui);
            ui.add_space(4.0);
        });

        egui::SidePanel::right("synth")
            .resizable(false)
            .show(ctx, |ui| synth_panel(ui, &s));

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                song_row(ui, &s, playing);
                ui.add_space(6.0);
                self.pattern_row(ui);
                ui.add_space(6.0);
                let edit = s.edit_pattern.load(Relaxed) as usize;
                // Only show the playhead on the pattern that's actually playing.
                let current = (playing && s.play_pattern.load(Relaxed) as usize == edit)
                    .then(|| s.current_step.load(Relaxed));
                drum_grid(ui, &s, edit, current);
                ui.add_space(6.0);
                melody_grid(ui, &s, edit, current);
            });
        });

        // Only redraw continuously while playing, to keep CPU low when idle.
        if playing {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
}

impl App {
    fn transport_row(&mut self, ui: &mut egui::Ui, playing: bool) {
        let s = &self.shared;
        ui.horizontal(|ui| {
            let label = if playing { "■ Stop" } else { "▶ Play" };
            if ui.button(label).clicked() {
                s.playing.store(!playing, Relaxed);
            }
            let mut bpm = s.bpm();
            ui.label("BPM");
            if ui
                .add(
                    egui::DragValue::new(&mut bpm)
                        .range(40.0..=240.0)
                        .speed(0.5),
                )
                .changed()
            {
                s.set_bpm(bpm);
            }
            ui.separator();
            let song = s.song_mode.load(Relaxed);
            if ui
                .selectable_label(!song, "Pattern")
                .on_hover_text("Loop the pattern you're editing")
                .clicked()
            {
                s.song_mode.store(false, Relaxed);
            }
            if ui
                .selectable_label(song, "Song")
                .on_hover_text("Play the song row, bar by bar")
                .clicked()
            {
                s.song_mode.store(true, Relaxed);
            }
            ui.separator();
            ui.weak("Space = play/stop");
        });
    }

    fn midi_row(&mut self, ui: &mut egui::Ui) {
        let s = self.shared.clone();
        ui.horizontal(|ui| {
            if let Some(pick) =
                port_picker(ui, "MIDI out", &self.midi.out_ports, &self.midi.out_name)
            {
                self.midi.select_out(pick);
            }
            if let Some(pick) = port_picker(ui, "MIDI in", &self.midi.in_ports, &self.midi.in_name)
            {
                self.midi.select_in(pick);
            }
            if ui
                .button("⟳")
                .on_hover_text("Rescan MIDI devices")
                .clicked()
            {
                self.midi.refresh();
            }
            let mut notes = s.send_notes.load(Relaxed);
            if ui.checkbox(&mut notes, "Send notes (ch 10)").changed() {
                s.send_notes.store(notes, Relaxed);
            }
            let mut ext = s.external_sync.load(Relaxed);
            if ui.checkbox(&mut ext, "Follow MIDI clock").changed() {
                s.external_sync.store(ext, Relaxed);
            }
        });
    }

    fn pattern_row(&mut self, ui: &mut egui::Ui) {
        let s = &self.shared;
        let edit = s.edit_pattern.load(Relaxed) as usize;
        ui.horizontal(|ui| {
            ui.add_sized(
                [60.0, 20.0],
                egui::Label::new(egui::RichText::new("Pattern").strong()),
            );
            for p in 0..PATTERNS {
                let button = egui::Button::new(pattern_name(p))
                    .selected(p == edit)
                    .min_size(Vec2::new(28.0, 22.0));
                if ui.add(button).on_hover_text("Edit this pattern").clicked() {
                    s.edit_pattern.store(p as u8, Relaxed);
                }
            }
            ui.separator();
            if ui.button("Copy").clicked() {
                self.clipboard = Some(s.get_pattern(edit));
            }
            let paste = ui.add_enabled(self.clipboard.is_some(), egui::Button::new("Paste"));
            if paste.clicked() {
                if let Some(data) = &self.clipboard {
                    s.set_pattern(edit, data);
                }
            }
            if ui.button("Clear").clicked() {
                s.clear_pattern(edit);
            }
        });
    }
}

/// The song arrangement: one pattern per bar.
fn song_row(ui: &mut egui::Ui, s: &Shared, playing: bool) {
    let edit = s.edit_pattern.load(Relaxed) as usize;
    let song_mode = s.song_mode.load(Relaxed);
    let pos = s.song_pos.load(Relaxed);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.add_sized([62.0, 20.0], egui::Label::new(egui::RichText::new("Song").strong()))
            .on_hover_text("Click a bar to place the selected pattern.\nClick it again or right-click to empty it.");
        for bar in 0..SONG_SLOTS {
            let slot = s.song_slot(bar);
            let text = slot.map(pattern_name).unwrap_or_default();
            let fill = match (slot.is_some(), bar % 4 == 0) {
                (true, _) => Color32::from_rgb(70, 120, 70),
                (false, true) => Color32::from_gray(60),
                (false, false) => Color32::from_gray(45),
            };
            let stroke = if playing && song_mode && bar == pos {
                Stroke::new(2.0_f32, Color32::WHITE)
            } else {
                Stroke::NONE
            };
            let cell = egui::Button::new(text)
                .fill(fill)
                .stroke(stroke)
                .min_size(Vec2::new(20.0, 22.0));
            let resp = ui.add(cell);
            if resp.secondary_clicked() || (resp.clicked() && slot == Some(edit)) {
                s.set_song_slot(bar, None);
            } else if resp.clicked() {
                s.set_song_slot(bar, Some(edit));
            }
        }
    });
}

fn step_fill(on: bool, on_color: Color32, step: usize, dark: bool) -> Color32 {
    // Shade every other group of four so beats are easy to see.
    let light_group = (step / 4).is_multiple_of(2);
    match (on, light_group, dark) {
        (true, _, _) => on_color,
        (false, true, false) => Color32::from_gray(60),
        (false, false, false) => Color32::from_gray(45),
        (false, true, true) => Color32::from_gray(50),
        (false, false, true) => Color32::from_gray(37),
    }
}

fn playhead(current: Option<usize>, step: usize) -> Stroke {
    if current == Some(step) {
        Stroke::new(2.0_f32, Color32::WHITE)
    } else {
        Stroke::NONE
    }
}

fn drum_grid(ui: &mut egui::Ui, s: &Shared, pattern: usize, current: Option<usize>) {
    for track in 0..TRACKS {
        ui.horizontal(|ui| {
            let name = egui::Button::new(drums::NAMES[track]).min_size(Vec2::new(60.0, 30.0));
            if ui.add(name).on_hover_text("Click to hear it").clicked() {
                s.preview(track);
            }
            for step in 0..STEPS {
                let pad = egui::Button::new("")
                    .fill(step_fill(
                        s.is_on(pattern, track, step),
                        DRUM_ON,
                        step,
                        false,
                    ))
                    .stroke(playhead(current, step))
                    .min_size(Vec2::new(30.0, 30.0));
                if ui.add(pad).clicked() {
                    s.toggle(pattern, track, step);
                }
            }
        });
    }
}

/// A dropdown of MIDI ports. Returns `Some(choice)` when the user picks one,
/// where the choice is `None` for "Off".
fn port_picker(
    ui: &mut egui::Ui,
    label: &str,
    ports: &[String],
    current: &Option<String>,
) -> Option<Option<String>> {
    let mut picked = None;
    ui.label(label);
    egui::ComboBox::from_id_salt(label)
        .selected_text(current.as_deref().unwrap_or("Off"))
        .width(140.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), "Off").clicked() {
                picked = Some(None);
            }
            for name in ports {
                if ui
                    .selectable_label(current.as_ref() == Some(name), name)
                    .clicked()
                {
                    picked = Some(Some(name.clone()));
                }
            }
        });
    picked
}

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

fn melody_grid(ui: &mut egui::Ui, s: &Shared, pattern: usize, current: Option<usize>) {
    ui.horizontal(|ui| {
        ui.strong("Melody");
        let mut octave = s.octave.load(Relaxed);
        ui.label("Octave");
        if ui
            .add(egui::DragValue::new(&mut octave).range(1..=6))
            .changed()
        {
            s.octave.store(octave, Relaxed);
        }
        // Shown 1-16 like on hardware, stored 0-15.
        let mut channel = s.melody_channel.load(Relaxed) + 1;
        ui.label("MIDI ch");
        if ui
            .add(egui::DragValue::new(&mut channel).range(1..=16))
            .changed()
        {
            s.melody_channel.store(channel - 1, Relaxed);
        }
    });
    ui.add_space(4.0);
    ui.spacing_mut().item_spacing.y = 2.0;
    // Highest note on top, like a piano roll.
    for semitone in (0..12u8).rev() {
        let name = NOTE_NAMES[semitone as usize];
        let sharp = name.ends_with('#');
        ui.horizontal(|ui| {
            let key = egui::Button::new(name)
                .fill(if sharp {
                    Color32::from_gray(25)
                } else {
                    Color32::from_gray(90)
                })
                .min_size(Vec2::new(60.0, 17.0));
            if ui.add(key).clicked() {
                s.preview_note(semitone);
            }
            for step in 0..STEPS {
                let on = s.melody_note(pattern, step) == Some(semitone);
                let cell = egui::Button::new("")
                    .fill(step_fill(on, NOTE_ON, step, sharp))
                    .stroke(playhead(current, step))
                    .min_size(Vec2::new(30.0, 17.0));
                if ui.add(cell).clicked() {
                    s.toggle_melody(pattern, step, semitone);
                    if !on {
                        s.preview_note(semitone);
                    }
                }
            }
        });
    }
}

fn synth_panel(ui: &mut egui::Ui, s: &Shared) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.heading("Synth");
        let mut on = s.synth_on.load(Relaxed);
        if ui
            .checkbox(&mut on, "On")
            .on_hover_text("Turn off to hear only your MIDI gear")
            .changed()
        {
            s.synth_on.store(on, Relaxed);
        }
    });
    ui.weak("Drag knobs up/down.\nDouble-click to reset.");
    ui.add_space(8.0);
    // Three knobs per row: sound, filter, envelope.
    let rows = [
        [synth::WAVE, synth::DETUNE, synth::VOLUME],
        [synth::CUTOFF, synth::RESONANCE, synth::ENV_AMOUNT],
        [synth::ATTACK, synth::DECAY, synth::RELEASE],
    ];
    for row in rows {
        ui.horizontal(|ui| {
            for i in row {
                let mut value = s.synth_param(i);
                if knob(ui, &PARAMS[i], &mut value) {
                    s.set_synth_param(i, value);
                }
            }
        });
        ui.add_space(6.0);
    }
    ui.add_space(4.0);
    if ui.button("Play a note").clicked() {
        s.preview_note(0);
    }
}

fn to_unit(p: &Param, v: f32) -> f32 {
    if p.log {
        (v / p.min).ln() / (p.max / p.min).ln()
    } else {
        (v - p.min) / (p.max - p.min)
    }
}

fn from_unit(p: &Param, t: f32) -> f32 {
    if p.log {
        p.min * (p.max / p.min).powf(t)
    } else {
        p.min + (p.max - p.min) * t
    }
}

fn format_value(p: &Param, v: f32) -> String {
    match p.unit {
        Unit::Percent => format!("{:.0}%", v * 100.0),
        Unit::Hz if v >= 1000.0 => format!("{:.1} kHz", v / 1000.0),
        Unit::Hz => format!("{v:.0} Hz"),
        Unit::Seconds if v < 1.0 => format!("{:.0} ms", v * 1000.0),
        Unit::Seconds => format!("{v:.2} s"),
    }
}

fn hint(p: &Param) -> &'static str {
    match p.name {
        "Wave" => "0% = saw (buzzy), 100% = square (hollow)",
        "Detune" => "Pitch difference between the two oscillators; higher = thicker",
        "Cutoff" => "Filter brightness when the envelope is closed",
        "Reso" => "Filter peak; higher = squelchier",
        "Env amt" => "How far each note opens the filter",
        "Decay" => "How fast the filter closes after a note starts",
        "Attack" => "Fade-in time of each note",
        "Release" => "Fade-out time after a note ends",
        "Volume" => "Synth level",
        _ => "",
    }
}

/// A rotary knob: drag up/down to turn, double-click to reset.
/// Returns true when the value changed.
fn knob(ui: &mut egui::Ui, p: &Param, value: &mut f32) -> bool {
    let mut changed = false;
    ui.allocate_ui_with_layout(
        Vec2::new(64.0, 74.0),
        Layout::top_down(Align::Center),
        |ui| {
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::click_and_drag());
            let resp = resp.on_hover_text(hint(p));
            let mut t = to_unit(p, *value).clamp(0.0, 1.0);
            if resp.dragged() {
                t = (t - resp.drag_delta().y * 0.006).clamp(0.0, 1.0);
                *value = from_unit(p, t);
                changed = true;
            }
            if resp.double_clicked() {
                *value = p.default;
                t = to_unit(p, *value);
                changed = true;
            }

            // The arc sweeps 270°, from bottom-left clockwise to bottom-right.
            let painter = ui.painter();
            let center = rect.center();
            let radius = rect.width() / 2.0 - 3.0;
            let start = 0.75 * PI;
            let point = |a: f32, r: f32| Pos2::new(center.x + r * a.cos(), center.y + r * a.sin());
            painter.circle_filled(
                center,
                radius,
                Color32::from_gray(if resp.hovered() { 70 } else { 55 }),
            );
            let arc: Vec<Pos2> = (0..=24)
                .map(|i| point(start + 1.5 * PI * t * i as f32 / 24.0, radius + 1.5))
                .collect();
            painter.add(egui::Shape::line(arc, Stroke::new(3.0_f32, DRUM_ON)));
            let angle = start + 1.5 * PI * t;
            painter.line_segment(
                [center, point(angle, radius - 2.0)],
                Stroke::new(2.0_f32, Color32::WHITE),
            );

            ui.label(egui::RichText::new(p.name).small());
            ui.label(egui::RichText::new(format_value(p, *value)).small().weak());
        },
    );
    changed
}
