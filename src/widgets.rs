//! Small reusable UI pieces: knobs, step colors, dropdowns.

use std::f32::consts::PI;

use eframe::egui::{self, Align, Color32, Layout, Pos2, RichText, Sense, Stroke, Vec2};

use crate::synth::{Param, Unit};

pub const DRUM_ON: Color32 = Color32::from_rgb(255, 140, 40);
pub const NOTE_ON: Color32 = Color32::from_rgb(80, 180, 255);
pub const NOTE_HELD: Color32 = Color32::from_rgb(45, 110, 160);

pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

pub fn playhead(on: bool) -> Stroke {
    if on {
        Stroke::new(2.0_f32, Color32::WHITE)
    } else {
        Stroke::NONE
    }
}

/// Background of an empty step. Every other group of four is lighter so
/// beats are easy to see; `dark` rows (black piano keys) are darker.
pub fn empty_fill(step: usize, dark: bool) -> Color32 {
    let light_group = (step / 4).is_multiple_of(2);
    match (light_group, dark) {
        (true, false) => Color32::from_gray(60),
        (false, false) => Color32::from_gray(45),
        (true, true) => Color32::from_gray(50),
        (false, true) => Color32::from_gray(37),
    }
}

/// A lit step, dimmer for quieter velocities so ghost notes stand out.
pub fn lit_fill(color: Color32, velocity: u8) -> Color32 {
    let t = 0.3 + 0.7 * velocity as f32 / 127.0;
    Color32::from_gray(40).lerp_to_gamma(color, t)
}

/// A ▶/▼ expand button, drawn rather than typed since not every font has
/// the down-pointing triangle.
pub fn disclosure_arrow(ui: &mut egui::Ui, collapsed: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(18.0, 26.0), Sense::click());
    let color = if resp.hovered() {
        Color32::WHITE
    } else {
        Color32::from_gray(180)
    };
    let c = rect.center();
    let points = if collapsed {
        vec![
            c + Vec2::new(-3.0, -5.0),
            c + Vec2::new(4.0, 0.0),
            c + Vec2::new(-3.0, 5.0),
        ]
    } else {
        vec![
            c + Vec2::new(-5.0, -3.0),
            c + Vec2::new(5.0, -3.0),
            c + Vec2::new(0.0, 4.0),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
    resp
}

/// A dropdown of MIDI ports. Returns `Some(choice)` when the user picks one,
/// where the choice is `None` for "Off".
pub fn port_picker(
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
        Unit::Pan if v.abs() < 0.005 => "C".into(),
        Unit::Pan if v < 0.0 => format!("L{:.0}", -v * 100.0),
        Unit::Pan => format!("R{:.0}", v * 100.0),
        Unit::Steps => {
            // Reduce n/16, so 4 steps reads 1/4.
            let mut n = v.round().max(1.0) as u32;
            let mut d = 16;
            while n.is_multiple_of(2) && d > 1 {
                n /= 2;
                d /= 2;
            }
            format!("{n}/{d}")
        }
    }
}

fn hint(p: &Param) -> &'static str {
    match p.name {
        "Wave" => "0% = saw (buzzy), 100% = square (hollow)",
        "Detune" => "Pitch difference between the two oscillators; higher = thicker",
        "Cutoff" => "Filter brightness when the envelope is closed",
        "Reso" => "Filter peak; higher = squelchier",
        "Env amt" => "How far each note opens the filter",
        "F decay" => "How fast the filter closes after a note starts",
        "Attack" => "Fade-in time of each note",
        "Decay" => "Time to fall from full volume to the sustain level",
        "Sustain" => "Volume while a note is held",
        "Release" => "Fade-out time after a note ends",
        "Volume" => "Track level",
        "Pan" => "Left or right in the stereo mix",
        "Reverb" => "How much of this track goes to the reverb (FX)",
        "Delay" => "How much of this track goes to the delay (FX)",
        "Size" => "How long the reverb rings",
        "Tone" => "Reverb brightness: low is muffled, high is airy",
        "Time" => "Time between echoes, in 16th notes",
        "Feedback" => "How many times each echo repeats",
        "Humanize" => "Random small changes to each hit's timing and loudness",
        _ => "",
    }
}

/// A rotary knob: drag up/down to turn, double-click to reset.
/// Returns true when the value changed.
pub fn knob(ui: &mut egui::Ui, p: &Param, value: &mut f32) -> bool {
    let mut changed = false;
    let size = Vec2::new(60.0, 74.0);
    ui.allocate_ui_with_layout(size, Layout::top_down(Align::Center), |ui| {
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
        let body = Color32::from_gray(if resp.hovered() { 70 } else { 55 });
        painter.circle_filled(center, radius, body);
        let arc: Vec<Pos2> = (0..=24)
            .map(|i| point(start + 1.5 * PI * t * i as f32 / 24.0, radius + 1.5))
            .collect();
        painter.add(egui::Shape::line(arc, Stroke::new(3.0_f32, DRUM_ON)));
        let angle = start + 1.5 * PI * t;
        let tip = point(angle, radius - 2.0);
        painter.line_segment([center, tip], Stroke::new(2.0_f32, Color32::WHITE));

        ui.label(RichText::new(p.name).small());
        ui.label(RichText::new(format_value(p, *value)).small().weak());
    });
    changed
}
