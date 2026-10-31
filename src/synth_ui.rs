//! Instrument panel and its pop-out windows: synth controls, modulation,
//! scope, drone, theremin and A/B morphing, plus shared sample/mix sections.
//!
//! App owns the state and publishes edits; this module draws and edits it.
//! Keep panel/window IDs stable so existing layouts continue to work.

use std::sync::atomic::Ordering::Relaxed;

use eframe::egui::{self, Color32, RichText, Sense, Vec2};

use crate::audio::{self, Event, SCOPE_LEN};
use crate::project::{Kind, NORMAL};
use crate::synth::{self, Param, Source, Unit, PARAMS};
use crate::widgets::{
    envelope, filter_curve, knob, lfo_shape, mod_knob, scope, source_chip, source_color, spectrum,
    waveform, SCOPE_SPANS,
};
use crate::{file, notes, App, DELAY_SEND, PAN, REVERB_SEND, TRACK_VOLUME};

const SAMPLE_START: Param = Param {
    name: "Start",
    min: 0.0,
    max: 1.0,
    default: 0.0,
    log: false,
    unit: Unit::Percent,
};

const SAMPLE_END: Param = Param {
    name: "End",
    default: 1.0,
    ..SAMPLE_START
};

const SAMPLE_FADE_OUT: Param = Param {
    name: "Fade out",
    default: 0.0,
    ..SAMPLE_START
};

const SAMPLE_ATTACK: Param = Param {
    name: "Fade in",
    min: 0.0,
    max: 1.0,
    default: 0.0,
    log: false,
    unit: Unit::Seconds,
};

const SAMPLE_RELEASE: Param = Param {
    name: "Release",
    min: 0.01,
    max: 3.0,
    default: 0.1,
    log: true,
    unit: Unit::Seconds,
};

const HUMANIZE: Param = Param {
    name: "Humanize",
    min: 0.0,
    max: 1.0,
    default: 0.0,
    log: false,
    unit: Unit::Percent,
};

/// The preset whose sound the knobs are set to, or "Custom" once any
/// knob differs from every preset.
pub(super) fn preset_name(values: &synth::Values) -> &'static str {
    if *values == PARAMS.map(|p| p.default) {
        return "Default";
    }
    (0..synth::PRESETS.len())
        .find(|&i| synth::preset(i) == *values)
        .map_or("Custom", |i| synth::PRESETS[i].name)
}

/// A pitch in Hz as a MIDI note to hold and how far to bend it (0-1
/// semitones, or further past the lowest and highest notes), for pitches
/// between the usual notes.
fn free_pitch(hz: f32) -> (u8, f32) {
    let exact = 69.0 + 12.0 * (hz / 440.0).log2();
    // Past the ends of MIDI's notes, the bend does the rest.
    let note = exact.floor().clamp(0.0, 127.0);
    (note as u8, exact - note)
}

/// The Drone's range: free in Hz, or note by note (C0 to C8).
const DRONE_LOW: f32 = 0.1;
const DRONE_HIGH: f32 = 20_000.0;
const NOTE_LOW: f32 = 12.0;
const NOTE_HIGH: f32 = 108.0;

/// Two sounds remembered for a synth track (by `Track::id`), and where
/// the morph between them is (0 = A, 1 = B).
pub(super) struct Snapshot {
    track: u32,
    a: Option<synth::Values>,
    b: Option<synth::Values>,
    t: f32,
}

/// Every section the instrument panel can show, in order: a section per
/// synth module (the sound first), then the rest.
pub(super) fn section_titles() -> impl Iterator<Item = &'static str> {
    SYNTH_ORDER
        .into_iter()
        .chain(["Sample", "Scope", "Mix", "Theremin"])
}

/// The synth's sections in the panel, which isn't their order in
/// `synth::MODULES`: the LFOs sit by the sound they wobble and the mod
/// envelope by the envelope, so they read as connected.
const SYNTH_ORDER: [&str; 12] = [
    "Voice",
    "Oscillators",
    "Pluck",
    "Mallet",
    "FM",
    "Wavetable",
    "LFO",
    "LFO 2",
    "Filter",
    "Envelope (ADSR)",
    "Mod envelope",
    "Arpeggiator",
];

/// The narrowest a section's column gets in the drawer.
const SECTION_WIDTH: f32 = 210.0;

/// Whether a section starts unfolded.
fn starts_open(title: &str) -> bool {
    match synth::MODULES.iter().find(|m| m.name == title) {
        Some(m) => m.open,
        None => title != "Theremin",
    }
}

/// Synth knob `k`, showing the modulation on it: a ring per source, a
/// dot where they have it now, a drop target for source chips, and a
/// right-click menu to change or remove them. Returns whether anything
/// changed.
fn synth_knob(
    ui: &mut egui::Ui,
    k: usize,
    value: &mut f32,
    mods: &mut Vec<synth::Link>,
    shared: &audio::Shared,
) -> bool {
    let rings: Vec<(Color32, f32, bool)> = mods
        .iter()
        .filter(|l| l.knob == k)
        .map(|l| (source_color(l.source), l.amount, l.source.bipolar()))
        .collect();
    let (mut changed, resp) = mod_knob(ui, &PARAMS[k], value, &rings, shared.mod_live(k));
    if !synth::movable(k) {
        return changed;
    }
    if let Some(source) = resp.dnd_hover_payload::<Source>() {
        let r = resp.rect.expand(4.0);
        ui.painter().circle_stroke(
            r.center(),
            r.width() / 2.0,
            egui::Stroke::new(2.0_f32, source_color(*source)),
        );
    }
    if let Some(source) = resp.dnd_release_payload::<Source>() {
        if !mods.iter().any(|l| l.knob == k && l.source == *source) {
            mods.push(synth::Link {
                source: *source,
                knob: k,
                amount: 0.25,
            });
            changed = true;
        }
    }
    if !rings.is_empty() {
        resp.context_menu(|ui| {
            ui.label(RichText::new(format!("What moves {}", PARAMS[k].name)).strong());
            let mut remove = None;
            for (n, l) in mods.iter_mut().enumerate().filter(|(_, l)| l.knob == k) {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(l.source.name()).color(source_color(l.source)));
                    let mut percent = l.amount * 100.0;
                    let slider = egui::Slider::new(&mut percent, -100.0..=100.0).suffix("%");
                    if ui
                        .add(slider)
                        .on_hover_text("How far it moves the knob, either way")
                        .changed()
                    {
                        l.amount = percent / 100.0;
                        changed = true;
                    }
                    if ui.small_button("Remove").clicked() {
                        remove = Some(n);
                    }
                });
            }
            if let Some(n) = remove {
                mods.remove(n);
                changed = true;
                ui.close_menu();
            }
        });
    }
    changed
}

/// Whether a panel section belongs on a synth track (`synth`) or a sample
/// track.
fn section_fits(title: &str, synth: bool) -> bool {
    match title {
        "Sample" => !synth,
        "Scope" | "Mix" => true,
        _ => synth,
    }
}

/// The instrument panel's sections, while it's being drawn: each one can
/// be folded (click its heading; egui remembers that while the app runs),
/// popped out into its own window with ⬈, or closed with × and brought
/// back from + Section or the View menu.
struct Sections {
    /// Closed sections, by title.
    closed: Vec<String>,
    /// A section whose × was clicked this frame.
    close: Option<String>,
    /// A section whose ⬈ was clicked this frame.
    pop: Option<String>,
}

impl Sections {
    fn is_open(&self, title: &str) -> bool {
        !self.closed.iter().any(|c| c == title)
    }

    /// Shows a section as a box, unless it's closed: its name (with the
    /// modulation source it offers, to drag onto knobs), then `body`.
    /// Click the name to fold it to just the name (some start folded);
    /// right-click it to pop it out or close it.
    fn show(&mut self, ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
        if !self.is_open(title) {
            return;
        }
        let line = ui.visuals().widgets.noninteractive.bg_stroke.color;
        egui::Frame::none()
            .stroke(egui::Stroke::new(1.0_f32, line))
            .inner_margin(egui::Margin::same(6.0))
            .rounding(2.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 3.0;
                let head = ui
                    .horizontal(|ui| {
                        if let Some(source) = offered_source(title) {
                            source_chip(ui, source);
                        }
                        ui.label(RichText::new(title).size(11.0).strong());
                    })
                    .response
                    .interact(Sense::click())
                    .on_hover_text(
                        "Click to fold or unfold · right-click to pop it out or close it",
                    );
                let id = ui.make_persistent_id(("section unfolded", title));
                let unfolded = ui.data_mut(|d| *d.get_persisted_mut_or(id, starts_open(title)));
                if head.clicked() {
                    ui.data_mut(|d| d.insert_persisted(id, !unfolded));
                }
                head.context_menu(|ui| {
                    if ui.button("Pop out into its own window").clicked() {
                        self.pop = Some(title.to_string());
                        ui.close_menu();
                    }
                    if ui
                        .button("Close (More > + Section brings it back)")
                        .clicked()
                    {
                        self.close = Some(title.to_string());
                        ui.close_menu();
                    }
                });
                if unfolded {
                    body(ui);
                }
            });
    }
}

/// The modulation source a section offers, to drag onto knobs. Each sits
/// with what shapes it; Velocity has none, so it sits with how notes play.
fn offered_source(title: &str) -> Option<Source> {
    match title {
        "LFO" => Some(Source::Lfo1),
        "LFO 2" => Some(Source::Lfo2),
        "Mod envelope" => Some(Source::ModEnv),
        "Voice" => Some(Source::Velocity),
        _ => None,
    }
}

impl App {
    /// Reads the latest scope samples and draws them at `size`. Returns
    /// true if clicked (to show more or less time).
    fn draw_scope(&mut self, ui: &mut egui::Ui, size: Vec2) -> bool {
        let mut samples = [0.0_f32; SCOPE_LEN];
        self.shared.read_scope(&mut samples);
        self.scope_busy = samples.iter().any(|s| s.abs() > 1e-4);
        let rate = self.shared.sample_rate.load(Relaxed) as f32;
        if self.spectrum {
            spectrum(ui, &samples, rate, size).on_hover_text("How loud each pitch is");
            return false;
        }
        scope(ui, &samples, rate, SCOPE_SPANS[self.scope_span], size)
    }

    /// The engine a section belongs to, if the selected track has picked
    /// a different one (so the section is hidden).
    fn other_engine(&self, title: &str) -> Option<&'static str> {
        let m = synth::MODULES.iter().find(|m| m.name == title)?;
        match &self.project.tracks.get(self.selected)?.kind {
            Kind::Synth(v) if !synth::shown(m, v) => m.engine.map(|e| synth::ENGINES[e]),
            _ => None,
        }
    }

    pub(super) fn is_popped(&self, title: &str) -> bool {
        self.popped.iter().any(|(t, _)| t == title)
    }

    /// The sections popped out of the instrument panel, each in its own
    /// window that can be moved (and for the scope and theremin, resized).
    /// They show the selected track, like the panel. Closing one puts it
    /// back in the panel.
    pub(super) fn popped_windows(&mut self, ctx: &egui::Context) {
        let name = self
            .project
            .tracks
            .get(self.selected)
            .map(|t| t.name.clone());
        let synth = matches!(
            self.project.tracks.get(self.selected).map(|t| &t.kind),
            Some(Kind::Synth(_))
        );
        let mut close = None;
        for i in 0..self.popped.len() {
            let (title, rect) = self.popped[i].clone();
            if title == "Scope" && !self.show_scope {
                continue;
            }
            let big = matches!(title.as_str(), "Scope" | "Theremin");
            let heading = match &name {
                Some(name) => format!("{title} · {name}"),
                None => title.clone(),
            };
            let mut open = true;
            let mut window = egui::Window::new(heading)
                .id(egui::Id::new(("section window", &title)))
                .open(&mut open)
                .collapsible(true)
                .resizable(big);
            window = match rect {
                // Only the scope and theremin have a size to keep; the
                // others fit their knobs.
                // (Capped, in case an older version saved a runaway size.)
                Some([x, y, w, h]) if big => window
                    .default_pos([x, y])
                    .default_size([w.min(900.0), h.min(600.0)]),
                Some([x, y, ..]) => window.default_pos([x, y]),
                None if big => window.default_size([480.0, 240.0]),
                None => window,
            };
            if big {
                window = window.min_size([200.0, 80.0]);
            }
            let mut inner = None;
            let shown = window.show(ctx, |ui| {
                inner = Some(ui.available_size());
                if name.is_none() {
                    ui.weak("Add a track to get started.");
                } else if let Some(engine) = self.other_engine(&title) {
                    ui.weak(format!(
                        "Pick the {engine} engine (in the Voice section) to use this."
                    ));
                } else if !section_fits(&title, synth) {
                    let kind = if title == "Sample" { "sample" } else { "synth" };
                    ui.weak(format!("Select a {kind} track to see its {title}."));
                    if big {
                        // Keeps its size for when it's back.
                        ui.allocate_space(ui.available_size());
                    }
                } else {
                    self.section(ui, &title, true);
                }
            });
            if !open {
                close = Some(i);
            } else if let (Some(shown), Some(size)) = (shown, inner) {
                // Remember where it is (not while it's folded).
                let at = shown.response.rect.min;
                let now = [at.x, at.y, size.x, size.y];
                let moved =
                    rect.is_none_or(|a| a.iter().zip(now).any(|(p, q)| (p - q).abs() > 0.5));
                if moved {
                    self.popped[i].1 = Some(now);
                    self.popped_changed = true;
                }
            }
        }
        if let Some(i) = close {
            self.popped.remove(i);
            self.popped_changed = true;
        }
        if self.popped_changed && !ctx.input(|i| i.pointer.any_down()) {
            self.popped_changed = false;
            file::remember_popped_sections(&self.popped);
        }
    }

    /// A pad to play the selected synth like a theremin: hold the mouse
    /// down and move it. Left to right slides the pitch smoothly over four
    /// octaves (no steps), bottom to top opens the filter.
    fn theremin_pad(&mut self, ui: &mut egui::Ui, size: Vec2) {
        const LOW: f32 = 36.0; // C2
        const HIGH: f32 = 84.0; // C6
        const NOTE: u8 = 60;
        let (rect, resp) = ui.allocate_exact_size(size, Sense::drag());
        let resp = resp.on_hover_text(
            "Hold the mouse down and move: left to right is pitch (smooth,\n\
             between notes too), bottom to top opens the filter",
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::from_rgb(20, 18, 32));
        // A line at each C, so you can find your way.
        for c in (LOW as u8..=HIGH as u8).step_by(12) {
            let x = rect.left() + (c as f32 - LOW) / (HIGH - LOW) * rect.width();
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                egui::Stroke::new(1.0_f32, Color32::from_gray(60)),
            );
            painter.text(
                egui::pos2(x + 2.0, rect.bottom() - 2.0),
                egui::Align2::LEFT_BOTTOM,
                notes::note_name(c),
                egui::FontId::proportional(9.0),
                Color32::from_gray(140),
            );
        }
        let i = self.selected;
        let pointer = resp
            .is_pointer_button_down_on()
            .then(|| resp.interact_pointer_pos())
            .flatten();
        match (pointer, self.theremin) {
            (Some(pos), held) => {
                let x = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                let y = ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0);
                let pitch = LOW + x * (HIGH - LOW);
                if held.is_none() {
                    self.theremin = Some((i, NOTE));
                    let _ = self.events.try_send(Event::Hold {
                        track: i,
                        note: NOTE,
                        velocity: NORMAL,
                    });
                }
                let (track, note) = self.theremin.unwrap_or((i, NOTE));
                let _ = self.events.try_send(Event::Bend {
                    track,
                    note,
                    semis: pitch - note as f32,
                    filter: y * 6.0 - 2.0,
                });
                painter.circle_filled(pos, 6.0, Color32::from_rgb(180, 140, 255));
                let hz = 440.0 * 2f32.powf((pitch - 69.0) / 12.0);
                painter.text(
                    rect.left_top() + Vec2::new(4.0, 2.0),
                    egui::Align2::LEFT_TOP,
                    format!("{hz:.0} Hz"),
                    egui::FontId::proportional(11.0),
                    Color32::WHITE,
                );
                ui.ctx().request_repaint();
            }
            (None, Some((track, note))) => {
                self.theremin = None;
                let _ = self.events.try_send(Event::Release { track, note });
            }
            (None, None) => {}
        }
    }

    /// The Drone button and its pitch slider: holds a note on the selected
    /// synth nonstop, at any pitch, to shape the sound by ear.
    fn drone_controls(&mut self, ui: &mut egui::Ui) {
        let i = self.selected;
        let on = self.drone.is_some_and(|(t, _)| t == i);
        let mut toggle = on;
        let hint = "Play a note nonstop, so you can turn knobs and hear\n\
                                the sound change. Click again to stop.";
        let flip = ui
            .horizontal(|ui| {
                let flip = ui.toggle_value(&mut toggle, "Drone").on_hover_text(hint);
                let pitch = free_pitch(self.drone_hz);
                let note = (pitch.0 as f32 + pitch.1).round().clamp(0.0, 127.0) as u8;
                let tap = "Play one short note at this pitch, to hear its\n\
                           attack and release";
                if ui.small_button("Tap").on_hover_text(tap).clicked() {
                    let _ = self.events.try_send(Event::Preview { track: i, note });
                }
                ui.separator();
                ui.selectable_value(&mut self.drone_free, false, "Notes")
                    .on_hover_text("The slider moves note by note");
                ui.selectable_value(&mut self.drone_free, true, "Free")
                    .on_hover_text("The slider moves smoothly, between notes too");
                flip
            })
            .inner;
        // Any pitch: the note it's held on is bent the rest of the way.
        let mut moved = false;
        ui.horizontal(|ui| {
            let step = |hz: f32, by: f32| (hz * 2f32.powf(by / 12.0)).clamp(DRONE_LOW, DRONE_HIGH);
            if ui
                .small_button("-")
                .on_hover_text("Down a semitone")
                .clicked()
            {
                self.drone_hz = step(self.drone_hz, -1.0);
                moved = true;
            }
            // As wide as the panel allows, leaving room for + and the readout.
            ui.spacing_mut().slider_width = 140.0;
            if self.drone_free {
                let pick = egui::Slider::new(&mut self.drone_hz, DRONE_LOW..=DRONE_HIGH)
                    .logarithmic(true)
                    .show_value(false);
                let hint = "The drone's pitch: slide anywhere, between notes too";
                moved |= ui.add(pick).on_hover_text(hint).changed();
            } else {
                let mut n = (free_pitch(self.drone_hz).0 as f32 + free_pitch(self.drone_hz).1)
                    .round()
                    .clamp(NOTE_LOW, NOTE_HIGH);
                let pick = egui::Slider::new(&mut n, NOTE_LOW..=NOTE_HIGH)
                    .step_by(1.0)
                    .show_value(false);
                if ui
                    .add(pick)
                    .on_hover_text("The drone's note, C0 to C8")
                    .changed()
                {
                    self.drone_hz = synth::midi_to_hz(n as u8);
                    moved = true;
                }
            }
            if ui
                .small_button("+")
                .on_hover_text("Up a semitone")
                .clicked()
            {
                self.drone_hz = step(self.drone_hz, 1.0);
                moved = true;
            }
            if moved && !self.drone_free {
                // Land on the nearest note.
                let (base, semis) = free_pitch(self.drone_hz);
                let n = (base as f32 + semis).round().clamp(NOTE_LOW, NOTE_HIGH);
                self.drone_hz = synth::midi_to_hz(n as u8);
            }
            let (base, semis) = free_pitch(self.drone_hz);
            let pitch = base as f32 + semis;
            let hz = self.drone_hz;
            // Named down to MIDI's lowest note (C-1, about 8 Hz).
            let note = if (-0.5..=127.5).contains(&pitch) {
                notes::note_name(pitch.round() as u8)
            } else {
                String::new()
            };
            let hz = if hz < 10.0 {
                format!("{hz:.1} Hz")
            } else {
                format!("{hz:.0} Hz")
            };
            ui.label(RichText::new(format!("{note}  {hz}")).small());
        });
        let (base, semis) = free_pitch(self.drone_hz);
        if flip.clicked() {
            if let Some((t, note)) = self.drone.take() {
                let _ = self.events.try_send(Event::Release { track: t, note });
            }
            if toggle {
                self.drone = Some((i, base));
                let _ = self.events.try_send(Event::Hold {
                    track: i,
                    note: base,
                    velocity: NORMAL,
                });
            }
        }
        if let Some((t, note)) = self.drone.filter(|_| moved || flip.clicked()) {
            let semis = semis + base as f32 - note as f32;
            let _ = self.events.try_send(Event::Bend {
                track: t,
                note,
                semis,
                filter: 0.0,
            });
        }
    }

    /// One section's knobs for the selected track, in the instrument panel
    /// or in its own window (`big`: the scope and theremin fill it).
    fn section(&mut self, ui: &mut egui::Ui, title: &str, big: bool) {
        let fill = |ui: &egui::Ui, height: f32| {
            if big {
                ui.available_size().max(Vec2::new(200.0, 80.0))
            } else {
                Vec2::new(ui.available_width().min(270.0), height)
            }
        };
        match title {
            "Scope" => {
                // The switch first, so the scope can take exactly the room
                // that's left: asking for more makes a popped-out window
                // grow every frame.
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.spectrum, false, "Wave")
                        .on_hover_text("The wave's shape over time");
                    ui.selectable_value(&mut self.spectrum, true, "Spectrum")
                        .on_hover_text(
                            "How loud each pitch is: low on the left, high on the right",
                        );
                });
                if self.draw_scope(ui, fill(ui, 46.0)) {
                    self.scope_span = (self.scope_span + 1) % SCOPE_SPANS.len();
                }
                return;
            }
            "Theremin" => {
                self.theremin_pad(ui, fill(ui, 110.0));
                return;
            }
            _ => {}
        }
        let show_scope = self.show_scope;
        let shared = self.shared.clone();
        let Some(track) = self.project.tracks.get_mut(self.selected) else {
            return;
        };
        let mut changed = false;
        match (title, &mut track.kind) {
            (_, Kind::Synth(values)) if synth::MODULES.iter().any(|m| m.name == title) => {
                let m = synth::MODULES.iter().find(|m| m.name == title).unwrap();
                let at = synth::first_knob(m);
                // Drawings of what the knobs do (View > Scope and drawings).
                if show_scope {
                    match title {
                        "Envelope (ADSR)" => envelope(
                            ui,
                            values[synth::ATTACK],
                            values[synth::DECAY],
                            values[synth::SUSTAIN],
                            values[synth::RELEASE],
                        ),
                        "Filter" => {
                            let live = shared
                                .mod_live(synth::CUTOFF)
                                .map(|u| PARAMS[synth::CUTOFF].at_unit(u));
                            filter_curve(ui, values[synth::CUTOFF], values[synth::RESONANCE], live);
                        }
                        "LFO" => lfo_shape(
                            ui,
                            synth::choice(values[synth::LFO_SHAPE], synth::LFO_SHAPES.len()),
                            Some(shared.lfo_phase(0)),
                            source_color(Source::Lfo1),
                        ),
                        "LFO 2" => lfo_shape(
                            ui,
                            synth::choice(values[synth::LFO2_SHAPE], synth::LFO_SHAPES.len()),
                            synth::modulation::uses(&track.mods, Source::Lfo2)
                                .then(|| shared.lfo_phase(1)),
                            source_color(Source::Lfo2),
                        ),
                        _ => {}
                    }
                }
                // All the knobs in one row, wrapping only if they must.
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for &k in m.rows.iter().flat_map(|r| r.iter()) {
                        let k = at + k;
                        changed |= synth_knob(ui, k, &mut values[k], &mut track.mods, &shared);
                    }
                    // Humanize is the track's, not the synth's, but it's
                    // about how notes play.
                    if title == "Voice" {
                        changed |= knob(ui, &HUMANIZE, &mut track.humanize);
                    }
                });
            }
            (
                "Sample",
                Kind::Sampler {
                    sample,
                    pitched,
                    shape,
                },
            ) => {
                ui.label(format!("Sample: {}", sample.name));
                ui.weak(format!("{:.2} s, {} Hz", sample.seconds(), sample.rate));
                waveform(ui, &sample.data, shape);
                ui.horizontal(|ui| {
                    let hint = "Off: plays as recorded, like a drum.\n\
                                On: plays at the pitch of each note, like an instrument (C4 = as recorded).";
                    changed |= ui.checkbox(pitched, "Pitched").on_hover_text(hint).changed();
                    changed |= ui
                        .checkbox(&mut shape.reverse, "Reverse")
                        .on_hover_text("Play the sample backwards")
                        .changed();
                    changed |= ui
                        .checkbox(&mut shape.ring, "Ring out")
                        .on_hover_text(
                            "On: plays to the end however short the note, like a drum.\n\
                             Off: fades out (Release) when the note ends.",
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    changed |= knob(ui, &SAMPLE_START, &mut shape.start);
                    changed |= knob(ui, &SAMPLE_END, &mut shape.end);
                    changed |= knob(ui, &SAMPLE_FADE_OUT, &mut shape.fade_out);
                    changed |= knob(ui, &SAMPLE_ATTACK, &mut shape.attack);
                    ui.add_enabled_ui(!shape.ring, |ui| {
                        changed |= knob(ui, &SAMPLE_RELEASE, &mut shape.release);
                    });
                });
                changed |= knob(ui, &HUMANIZE, &mut track.humanize);
            }
            ("Mix", _) => {
                ui.horizontal(|ui| {
                    changed |= knob(ui, &TRACK_VOLUME, &mut track.volume);
                    changed |= knob(ui, &PAN, &mut track.pan);
                    changed |= knob(ui, &REVERB_SEND, &mut track.reverb);
                    changed |= knob(ui, &DELAY_SEND, &mut track.delay);
                });
            }
            _ => {}
        }
        if changed {
            self.dirty = true;
        }
    }

    /// The instrument drawer's insides: the track's own controls in a
    /// column on the left, then its sections side by side, wrapping onto
    /// more rows when the window is narrow.
    pub(super) fn instrument_panel(&mut self, ui: &mut egui::Ui) {
        if self.project.tracks.get(self.selected).is_none() {
            ui.weak("Add a track to get started.");
            return;
        }
        let synth = matches!(self.project.tracks[self.selected].kind, Kind::Synth(_));
        let mut sections = Sections {
            closed: std::mem::take(&mut self.closed_sections),
            close: None,
            pop: None,
        };
        ui.horizontal_wrapped(|ui| self.instrument_head(ui, &mut sections, synth));
        ui.separator();
        self.instrument_sections(ui, &mut sections, synth);
        if let Some(t) = sections.close.take() {
            sections.closed.push(t);
            self.sections_changed = true;
        }
        if let Some(t) = sections.pop.take() {
            self.popped.push((t, None));
            self.popped_changed = true;
        }
        self.closed_sections = sections.closed;
        if std::mem::take(&mut self.sections_changed) {
            file::remember_closed_sections(&self.closed_sections);
        }
    }

    /// Whether a section shows in the drawer: it fits this kind of track
    /// and isn't popped out into its own window.
    fn shows_here(&self, title: &str, synth: bool) -> bool {
        section_fits(title, synth)
            && !self.is_popped(title)
            && (title != "Scope" || self.show_scope)
            && self.other_engine(title).is_none()
    }

    /// The sections, as many side by side as fit, all the same width.
    fn instrument_sections(&mut self, ui: &mut egui::Ui, sections: &mut Sections, synth: bool) {
        let layout = egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true);
        let gap = 10.0;
        let room = ui.available_width();
        let across = ((room + gap) / (SECTION_WIDTH + gap)).floor().max(1.0);
        let width = ((room - gap * (across - 1.0)) / across - 3.0).floor();
        ui.with_layout(layout, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, 6.0);
            // Bodies sit under their heading without the usual indent.
            ui.spacing_mut().indent = 4.0;
            for title in section_titles() {
                if !self.shows_here(title, synth) || !sections.is_open(title) {
                    continue;
                }
                let size = Vec2::new(width, 0.0);
                ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(width);
                    sections.show(ui, title, |ui| self.section(ui, title, false));
                });
            }
        });
    }

    /// The drawer's top line: the track's name, Drone and Tap, MIDI
    /// channel, preset, Mutate and A/B, and bringing back closed sections.
    fn instrument_head(&mut self, ui: &mut egui::Ui, sections: &mut Sections, synth: bool) {
        let track = &mut self.project.tracks[self.selected];
        let mut changed = false;
        let name = egui::TextEdit::singleline(&mut track.name)
            .font(egui::TextStyle::Body)
            .desired_width(110.0);
        changed |= ui.add(name).changed();
        ui.add_space(4.0);
        // Hear the track: a quick note, or a drone that keeps playing.
        if synth {
            self.drone_controls(ui);
        } else if ui
            .small_button("Tap")
            .on_hover_text("Play the sound once")
            .clicked()
        {
            self.preview(self.selected, 0);
        }
        ui.add_space(4.0);
        if !synth {
            if let Some(sound) = self.drum_browser.show(ui, &self.project.tracks) {
                self.dirty |=
                    crate::drum_browser::replace(&mut self.project.tracks[self.selected], sound);
            }
        }
        let track = &mut self.project.tracks[self.selected];

        ui.add_space(8.0);

        if let Kind::Synth(values) = &mut track.kind {
            // A starting sound, adjusted from there with the knobs.
            ui.horizontal(|ui| {
                ui.label("Preset");
                egui::ComboBox::from_id_salt("preset")
                    .selected_text(preset_name(values))
                    .width(150.0)
                    .height(500.0)
                    .show_ui(ui, |ui| {
                        for group in synth::PRESET_GROUPS {
                            ui.label(RichText::new(group).small().strong().weak());
                            let presets = synth::PRESETS.iter().enumerate();
                            for (i, p) in presets.filter(|(_, p)| p.group == group) {
                                if ui.selectable_label(false, p.name).clicked() {
                                    *values = synth::preset(i);
                                    // A preset is a whole sound, modulation too.
                                    track.mods = p.links();
                                    changed = true;
                                }
                            }
                        }
                    })
                    .response
                    .on_hover_text(
                        "Start from an instrument sound, then adjust any knob.\n\
                         Ctrl+Z undoes it.",
                    );
            });
            // Happy accidents: nudge everything a little.
            ui.horizontal(|ui| {
                let mutate = ui.button("Mutate").on_hover_text(
                    "Nudges every knob a little, for a new take on the sound.\n\
                     Ctrl+Z undoes it. Right-click to leave some sections alone.",
                );
                if mutate.clicked() {
                    synth::mutate(
                        values,
                        self.mutate_amount,
                        &mut self.rng,
                        &self.mutate_locked,
                    );
                    changed = true;
                }
                mutate.context_menu(|ui| {
                    ui.label(RichText::new("Mutate leaves these alone:").strong());
                    for m in synth::MODULES
                        .iter()
                        .filter(|m| m.rows.iter().any(|r| !r.is_empty()))
                    {
                        let mut locked = self.mutate_locked.contains(&m.name);
                        if ui.checkbox(&mut locked, m.name).changed() {
                            self.mutate_locked.retain(|n| *n != m.name);
                            if locked {
                                self.mutate_locked.push(m.name);
                            }
                        }
                    }
                });
                let mut percent = self.mutate_amount * 100.0;
                let drag = egui::DragValue::new(&mut percent)
                    .range(1.0..=60.0)
                    .suffix("%");
                if ui
                    .add(drag)
                    .on_hover_text("How far Mutate moves each knob")
                    .changed()
                {
                    self.mutate_amount = percent / 100.0;
                }
            });
            // Two sounds to slide between.
            let id = track.id;
            let at = match self.snapshots.iter().position(|s| s.track == id) {
                Some(at) => at,
                None => {
                    self.snapshots.push(Snapshot {
                        track: id,
                        a: None,
                        b: None,
                        t: 0.0,
                    });
                    self.snapshots.len() - 1
                }
            };
            let snap = &mut self.snapshots[at];
            ui.horizontal(|ui| {
                if ui
                    .button("Keep A")
                    .on_hover_text("Remember the sound as it is now, as A")
                    .clicked()
                {
                    snap.a = Some(*values);
                    snap.t = 0.0;
                }
                if ui
                    .button("Keep B")
                    .on_hover_text("Remember the sound as it is now, as B")
                    .clicked()
                {
                    snap.b = Some(*values);
                    snap.t = 1.0;
                }
                if let (Some(a), Some(b)) = (snap.a, snap.b) {
                    ui.label("A");
                    let slider = egui::Slider::new(&mut snap.t, 0.0..=1.0).show_value(false);
                    if ui
                        .add(slider)
                        .on_hover_text("Slide between sound A and sound B")
                        .changed()
                    {
                        *values = synth::morph(&a, &b, snap.t);
                        changed = true;
                    }
                    ui.label("B");
                }
            });
            ui.add_space(4.0);
        }
        if changed {
            self.dirty = true;
        }
        ui.add_space(8.0);
        // The rest, out of the way: MIDI channel, closed sections, delete.
        ui.menu_button("More", |ui| {
            let track = &mut self.project.tracks[self.selected];
            let mut changed = false;
            ui.horizontal(|ui| {
                // Shown 1-16 like on hardware, stored 0-15; 0 means off.
                let mut ch = track.midi_channel.map_or(0, |c| c + 1);
                ui.label("MIDI ch");
                let drag = egui::DragValue::new(&mut ch)
                    .range(0..=16)
                    .custom_formatter(|v, _| {
                        if v == 0.0 {
                            "Off".into()
                        } else {
                            format!("{v}")
                        }
                    });
                if ui
                    .add(drag)
                    .on_hover_text("Send this track's notes to MIDI gear")
                    .changed()
                {
                    track.midi_channel = (ch > 0).then(|| ch - 1);
                    changed = true;
                }
                if !track.is_melodic() {
                    ui.label("note");
                    let drag = egui::DragValue::new(&mut track.midi_note).range(0..=127);
                    changed |= ui
                        .add(drag)
                        .on_hover_text("MIDI note this track sends")
                        .changed();
                }
            });
            if changed {
                self.dirty = true;
            }
            ui.separator();
            ui.horizontal(|ui| {
                // Closed sections that belong on this kind of track.
                let reopen: Vec<&str> = section_titles()
                    .filter(|t| section_fits(t, synth) && !sections.is_open(t))
                    .collect();
                if !reopen.is_empty() {
                    ui.menu_button("+ Section", |ui| {
                        for t in reopen {
                            if ui.button(t).clicked() {
                                sections.closed.retain(|c| c != t);
                                self.sections_changed = true;
                                ui.close_menu();
                            }
                        }
                    })
                    .response
                    .on_hover_text("Bring back a section you closed");
                }
            });
            ui.add_space(8.0);
            if ui
                .button(RichText::new("Delete track").color(Color32::LIGHT_RED))
                .clicked()
            {
                self.delete_selected();
            }
        });
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;

    #[test]
    fn the_panel_shows_every_synth_section_once() {
        assert_eq!(SYNTH_ORDER.len(), synth::MODULES.len());
        for m in synth::MODULES {
            let n = SYNTH_ORDER.iter().filter(|&&t| t == m.name).count();
            assert_eq!(n, 1, "{}", m.name);
        }
    }
}
