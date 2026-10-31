//! Small reusable UI pieces: knobs, step colors, dropdowns.

use std::f32::consts::PI;

use eframe::egui::{self, Align, Color32, Layout, Pos2, RichText, Sense, Stroke, Vec2};

use crate::project::SampleShape;
use crate::synth::{choice, Param, Source, Unit, WAVES};

pub use crate::theme::{drum_on, note_on};

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

/// A triangle pointing up (`up`) or down, like `disclosure_arrow`'s, for a
/// drawer along the bottom; `label` names it for screen readers (and tests).
pub fn drawer_arrow(ui: &mut egui::Ui, up: bool, label: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(18.0, 18.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let color = if resp.hovered() {
        Color32::WHITE
    } else {
        Color32::from_gray(180)
    };
    let c = rect.center();
    let y = if up { -1.0 } else { 1.0 };
    let points = vec![
        c + Vec2::new(-5.0, -3.0 * y),
        c + Vec2::new(5.0, -3.0 * y),
        c + Vec2::new(0.0, 4.0 * y),
    ];
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
    p.to_unit(v)
}

fn from_unit(p: &Param, t: f32) -> f32 {
    p.at_unit(t)
}

/// A knob's value as shown under it, e.g. "250 Hz" or "C".
pub fn format_value(p: &Param, v: f32) -> String {
    match p.unit {
        Unit::Percent => format!("{:.0}%", v * 100.0),
        Unit::Hz if v >= 1000.0 => format!("{:.1} kHz", v / 1000.0),
        Unit::Hz if v < 10.0 => format!("{v:.1} Hz"),
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
        Unit::Wave => {
            // A shape's name, or the two it's between.
            let i = (v.max(0.0) as usize).min(WAVES.len() - 1);
            let f = v - i as f32;
            if f < 0.05 {
                WAVES[i].into()
            } else if f > 0.95 {
                WAVES[(i + 1).min(WAVES.len() - 1)].into()
            } else {
                format!("{}/{}", WAVES[i], WAVES[i + 1])
            }
        }
        // Whole semitones as +7 or -12; in between as -0.25.
        Unit::Semitones if (v - v.round()).abs() >= 0.005 => format!("{v:+.2}"),
        Unit::Semitones => match v.round() as i32 {
            0 => "0".into(),
            n => format!("{n:+}"),
        },
        Unit::Ratio => {
            let snapped = (v * 2.0).round() / 2.0;
            if (v - snapped).abs() < 0.06 {
                format!("x{snapped}")
            } else {
                format!("x{v:.2}")
            }
        }
        Unit::Choice(names) => names[choice(v, names.len())].into(),
    }
}

fn hint(p: &Param) -> &'static str {
    match p.name {
        "Wave 1" | "Wave 2" => {
            "Oscillator shape: sine (smooth), triangle, saw (buzzy), square\n\
             (hollow) or noise (hiss, for shakers, hats and wind); in between blends"
        }
        "Level 2" => "How loud the second oscillator is; 0 = just oscillator 1",
        "Pitch 2" => "Second oscillator's pitch in semitones: +12 = an octave up, +7 = a fifth",
        "Detune" => "Fine pitch difference between the two oscillators; higher = thicker",
        "Engine" => {
            "What makes the sound: Analog (two oscillators), Pluck (a plucked\n\
             string), Mallet (marimba, vibes, glass, bell), FM (electric piano,\n\
             bells, basses) or Wavetable (morphing waves). Each has its own section."
        }
        "String tone" => "Dull (low) to bright (high), like plucking with a thumb or a pick",
        "String decay" => "How long the string rings",
        "Pick spot" => "Where it's plucked: near the end is bright, near the middle hollow",
        "Material" => "What's struck: marimba, vibraphone, glass or a bell",
        "Hardness" => "A soft or hard mallet: harder brings out the higher ringing",
        "Ring" => "How long it rings after the strike",
        "FM ratio" => {
            "The modulator's pitch, times the note's. Whole numbers (x1, x2, x3)\n\
             sound like instruments; odd ones like x3.5 sound like bells"
        }
        "FM depth" => "How much it changes the tone: 0 is a pure sine, more is brighter",
        "FM decay" => "How fast the bright start mellows",
        "FM feedback" => "Feeds the modulator back into itself: grittier, up to noisy",
        "Table" => {
            "Which set of waves: Basic (sine to square), Vocal (vowel-like),\n\
             Hollow (clarinet-like to bright) or Metal (clangy)"
        }
        "Position" => {
            "Where in the table: sweep it to morph the sound. Drag LFO 2 or the\n\
             mod envelope onto it to make it move by itself"
        }
        "Sub" => "A sine an octave below, for weight",
        "LFO retrig" | "LFO 2 retrig" => {
            "On: the LFO starts its cycle again at every new note, so each\n\
             note wobbles the same way. Off: it keeps running on its own"
        }
        "LFO 2 wave" | "LFO 2 rate" | "LFO 2 sync" => {
            "The second LFO's shape, speed and tempo sync. Drag its chip onto a\n\
             knob to make it move that knob"
        }
        "Mod attack" | "Mod decay" | "Mod sustain" => {
            "The mod envelope: rises over Attack when a note starts, falls over\n\
             Decay to Sustain. Drag its chip onto a knob to make it move that knob"
        }
        "LFO rate" => "How fast the LFO wobbles (when LFO sync is off)",
        "LFO amt" => "How much the LFO wobbles its target; 0 = off",
        "LFO wave" => {
            "The wobble's shape. Random picks a new value each cycle:\n\
                       on pitch, that's random tones"
        }
        "LFO to" => {
            "What the LFO wobbles: pitch (vibrato), filter (wah), volume (tremolo)\n\
                     or pan (left and right)"
        }
        "LFO sync" => "Tie the LFO's speed to the tempo, e.g. once per beat (1/4)",
        "Glide" => "Slide from one note's pitch to the next (in Mono and Legato)",
        "Arp" => {
            "Arpeggiator: play held notes one at a time in a pattern.\n\
                  Off plays them together, as chords"
        }
        "Arp rate" => "How often the arpeggiator plays a note, in time with the tempo",
        "Octaves" => "How many octaves the pattern climbs through",
        "Latch" => {
            "Keeps arpeggiating after you let go. Play a new chord to\n\
                    change it, or turn Latch off to stop"
        }
        "Arp notes" => {
            "Held: the notes you hold. A chord or scale: one held note\n\
                        becomes that chord or scale, so one finger plays a pattern"
        }
        "Arp gate" => "How long each arpeggiated note lasts: short is staccato",
        "Voices" => {
            "Poly: chords. Mono: one note at a time.\n\
                     Legato: one at a time, overlapping notes glide without restarting"
        }
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
        "Start" => "Where in the sample it starts playing",
        "End" => "Where in the sample it stops",
        "Fade in" => "How long the sample takes to fade in; 0 = full punch",
        "Humanize" => "Random small changes to each hit's timing and loudness",
        _ => "",
    }
}

/// A rotary knob: drag up/down to turn, double-click to reset.
/// Returns true when the value changed.
pub fn knob(ui: &mut egui::Ui, p: &Param, value: &mut f32) -> bool {
    mod_knob(ui, p, value, &[], None).0
}

/// A knob that also shows modulation: a coloured arc outside it for each
/// source moving it (`rings`: colour, amount (-1 to 1 of the sweep) and
/// whether it swings both ways, like an LFO),
/// and a dot where the sources have it right now (`live`, 0-1). Returns
/// whether the value changed, and the knob's response (for dropping
/// sources on it and its right-click menu).
pub fn mod_knob(
    ui: &mut egui::Ui,
    p: &Param,
    value: &mut f32,
    rings: &[(Color32, f32, bool)],
    live: Option<f32>,
) -> (bool, egui::Response) {
    let mut changed = false;
    let size = Vec2::new(50.0, 56.0);
    let inner = ui.allocate_ui_with_layout(size, Layout::top_down(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click_and_drag());
        let resp = if rings.is_empty() {
            resp.on_hover_text(hint(p))
        } else {
            resp.on_hover_text(format!(
                "{}\nRight-click to change or remove what moves it.",
                hint(p)
            ))
        };
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
        let radius = rect.width() / 2.0 - 4.0;
        let start = 0.75 * PI;
        let point = |a: f32, r: f32| Pos2::new(center.x + r * a.cos(), center.y + r * a.sin());
        let arc = |from: f32, to: f32, r: f32| -> Vec<Pos2> {
            (0..=24)
                .map(|i| point(start + 1.5 * PI * (from + (to - from) * i as f32 / 24.0), r))
                .collect()
        };
        let body = Color32::from_gray(if resp.hovered() { 70 } else { 55 });
        painter.circle_filled(center, radius, body);
        painter.add(egui::Shape::line(
            arc(0.0, t, radius + 1.5),
            Stroke::new(2.5_f32, drum_on()),
        ));
        // Each source's reach, one ring further out per source.
        for (n, (color, amount, both)) in rings.iter().enumerate() {
            let from = if *both {
                (t - amount).clamp(0.0, 1.0)
            } else {
                t
            };
            let to = (t + amount).clamp(0.0, 1.0);
            let r = radius + 4.0 + 2.0 * n as f32;
            painter.add(egui::Shape::line(
                arc(from, to, r),
                Stroke::new(2.0_f32, *color),
            ));
        }
        if let Some(live) = live.filter(|_| !rings.is_empty()) {
            let r = radius + 4.0;
            painter.circle_filled(
                point(start + 1.5 * PI * live.clamp(0.0, 1.0), r),
                3.0,
                Color32::WHITE,
            );
        }
        let angle = start + 1.5 * PI * t;
        let tip = point(angle, radius - 2.0);
        painter.line_segment([center, tip], Stroke::new(2.0_f32, Color32::WHITE));

        ui.label(RichText::new(p.name).size(10.0));
        ui.label(RichText::new(format_value(p, *value)).size(9.0).weak());
        resp
    });
    (changed, inner.inner)
}

/// Each modulation source's colour, the same everywhere it's shown.
pub fn source_color(s: Source) -> Color32 {
    match s {
        Source::Lfo1 => Color32::from_rgb(235, 200, 60),
        Source::Lfo2 => Color32::from_rgb(80, 205, 225),
        Source::ModEnv => Color32::from_rgb(235, 110, 200),
        Source::Velocity => Color32::from_rgb(120, 215, 120),
    }
}

/// A modulation source's chip: drag it onto a knob to connect it.
pub fn source_chip(ui: &mut egui::Ui, s: Source) {
    let id = egui::Id::new(("mod source", s.name()));
    let color = source_color(s);
    ui.dnd_drag_source(id, s, |ui| {
        egui::Frame::none()
            .fill(color.gamma_multiply(0.25))
            .stroke(Stroke::new(1.0_f32, color))
            .rounding(8.0)
            .inner_margin(egui::Margin::symmetric(7.0, 1.0))
            .show(ui, |ui| {
                ui.label(RichText::new(s.name()).small().color(color))
            });
    })
    .response
    .on_hover_text(format!(
        "Drag {} onto any knob to make it move that knob.",
        s.name()
    ));
}

/// A stereo level meter: two thin bars, green, then yellow near the top,
/// red at the limit. `levels` are peaks, 0-1. A red LIMIT light shows
/// while the limiter is turning the mix down noticeably.
pub fn meter(ui: &mut egui::Ui, levels: [f32; 2], limiting: f32) -> egui::Response {
    let size = Vec2::new(70.0, 14.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::from_gray(40));
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0_f32, Color32::from_gray(70)));
    let bar_h = (rect.height() - 3.0) / 2.0;
    for (i, level) in levels.iter().enumerate() {
        let top = rect.top() + 1.0 + i as f32 * (bar_h + 1.0);
        let width = level.clamp(0.0, 1.0) * rect.width();
        let color = level_color(*level);
        let bar = egui::Rect::from_min_size(Pos2::new(rect.left(), top), Vec2::new(width, bar_h));
        painter.rect_filled(bar, 1.0, color);
    }
    let limit = limiting > 0.05;
    ui.label(
        RichText::new("LIMIT")
            .small()
            .color(if limit { Color32::from_rgb(230, 70, 60) } else { Color32::from_gray(70) }),
    )
    .on_hover_text("Lights up when the mix is too loud and the limiter is turning it down.\nTurn Master or some tracks down a little.");
    resp.on_hover_text("Output level, left and right")
}

/// The color of a level, 0-1: green, then yellow near the top, red at it.
fn level_color(level: f32) -> Color32 {
    if level >= 0.94 {
        Color32::from_rgb(230, 70, 60)
    } else if level >= 0.7 {
        Color32::from_rgb(230, 200, 60)
    } else {
        Color32::from_rgb(80, 200, 110)
    }
}

/// One track's level, as a thin bar `width` wide (peak, 0-1).
pub fn level_bar(ui: &mut egui::Ui, level: f32, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 20.0), Sense::hover());
    let bar = egui::Rect::from_center_size(rect.center(), Vec2::new(width, 6.0));
    let painter = ui.painter_at(rect);
    painter.rect_filled(bar, 1.0, Color32::from_gray(40));
    let level = level.clamp(0.0, 1.0);
    let lit = egui::Rect::from_min_size(bar.min, Vec2::new(level * width, bar.height()));
    painter.rect_filled(lit, 1.0, level_color(level));
    resp.on_hover_text("How loud this track is right now")
}

/// A small slider for a track's row: a thin line with a handle, `width`
/// wide. Drag it, or double-click to reset; hover shows the value.
pub fn mini_slider(ui: &mut egui::Ui, p: &Param, value: &mut f32, width: f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 20.0), Sense::click_and_drag());
    let before = *value;
    let inner = rect.shrink2(Vec2::new(3.0, 0.0));
    if resp.double_clicked() {
        *value = p.default;
    } else if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.dragged()) {
        let t = ((pos.x - inner.left()) / inner.width()).clamp(0.0, 1.0);
        *value = from_unit(p, t);
    }
    let t = to_unit(p, *value).clamp(0.0, 1.0);
    let painter = ui.painter_at(rect);
    let y = rect.center().y;
    let line = if resp.hovered() || resp.dragged() {
        Color32::from_gray(150)
    } else {
        Color32::from_gray(100)
    };
    painter.line_segment(
        [Pos2::new(inner.left(), y), Pos2::new(inner.right(), y)],
        Stroke::new(2.0_f32, Color32::from_gray(55)),
    );
    // Pan fills from the middle; the rest from the left.
    let from: f32 = if p.min < 0.0 { 0.5 } else { 0.0 };
    let (a, b) = (from.min(t), from.max(t));
    painter.line_segment(
        [
            Pos2::new(inner.left() + a * inner.width(), y),
            Pos2::new(inner.left() + b * inner.width(), y),
        ],
        Stroke::new(2.0_f32, line),
    );
    let x = inner.left() + t * inner.width();
    let handle = egui::Rect::from_center_size(Pos2::new(x, y), Vec2::new(5.0, 11.0));
    painter.rect_filled(handle, 1.0, Color32::from_gray(210));
    let label = format!("{}: {}", p.name, format_value(p, *value));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, p.name));
    resp.on_hover_text(format!("{label}\nDrag to change, double-click to reset"));
    *value != before
}

/// A sample's waveform, with the parts outside `start`..`end` (0-1)
/// dimmed, so trimming shows what's left, and the fade out drawn as loud
/// as it plays. A label shows when it plays backwards.
pub fn waveform(ui: &mut egui::Ui, data: &[f32], shape: &SampleShape) {
    let SampleShape {
        start,
        end,
        fade_out,
        reverse,
        ..
    } = *shape;
    let size = Vec2::new(ui.available_width().min(270.0), 46.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_rgb(16, 22, 30));
    let columns = rect.width() as usize;
    if data.is_empty() || columns == 0 {
        return;
    }
    let (a, b) = (start.min(end), start.max(end));
    let mid = rect.center().y;
    let half = rect.height() / 2.0 - 2.0;
    for c in 0..columns {
        let from = c * data.len() / columns;
        let to = ((c + 1) * data.len() / columns)
            .max(from + 1)
            .min(data.len());
        let (lo, hi) = data[from..to]
            .iter()
            .fold((0.0_f32, 0.0_f32), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
        let t = c as f32 / columns as f32;
        // Inside the fade, drawn as loud as it plays.
        let left = if reverse { t - a } else { b - t };
        let fade = fade_out * (b - a);
        let gain = if (a..=b).contains(&t) && left < fade {
            (left / fade).powi(2)
        } else {
            1.0
        };
        let (lo, hi) = (lo * gain, hi * gain);
        let color = if (a..=b).contains(&t) {
            Color32::from_rgb(90, 170, 230)
        } else {
            Color32::from_gray(60)
        };
        let x = rect.left() + c as f32 + 0.5;
        painter.line_segment(
            [Pos2::new(x, mid - hi * half), Pos2::new(x, mid - lo * half)],
            Stroke::new(1.0_f32, color),
        );
    }
    for edge in [a, b] {
        let x = rect.left() + edge * rect.width();
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.5_f32, Color32::from_rgb(230, 200, 60)),
        );
    }
    if reverse {
        painter.text(
            rect.left_top() + Vec2::new(4.0, 2.0),
            egui::Align2::LEFT_TOP,
            "reversed",
            egui::FontId::proportional(10.0),
            Color32::from_gray(180),
        );
    }
}

/// How much time the scope shows, in milliseconds; click the scope to cycle.
pub const SCOPE_SPANS: [f32; 4] = [5.0, 10.0, 20.0, 50.0];

/// An oscilloscope: draws `samples` (oldest first) like a scope triggered
/// on a rising edge, so a steady note stands still. Returns true if clicked.
pub fn scope(
    ui: &mut egui::Ui,
    samples: &[f32],
    sample_rate: f32,
    span_ms: f32,
    size: Vec2,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_rgb(12, 20, 16));
    let grid = Stroke::new(1.0_f32, Color32::from_rgb(28, 48, 36));
    for i in 1..10 {
        let x = rect.left() + rect.width() * i as f32 / 10.0;
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            grid,
        );
    }
    for i in 1..4 {
        let y = rect.top() + rect.height() * i as f32 / 4.0;
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            grid,
        );
    }

    let window = ((span_ms / 1000.0 * sample_rate) as usize).clamp(16, samples.len());
    // Trigger: the newest rising zero crossing that still leaves a full
    // window after it.
    let latest = samples.len() - window;
    let start = (1..=latest)
        .rev()
        .find(|&i| samples[i - 1] < 0.0 && samples[i] >= 0.0)
        .unwrap_or(latest);
    let shown = &samples[start..start + window];
    let peak = shown.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    // Quiet sounds are scaled up so their shape is still visible.
    let scale = if peak > 0.0 {
        0.9 / peak.max(0.25)
    } else {
        1.0
    };
    let points: Vec<Pos2> = shown
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let x = rect.left() + rect.width() * i as f32 / (window - 1) as f32;
            let y = rect.center().y - s * scale * rect.height() / 2.0;
            Pos2::new(x, y.clamp(rect.top(), rect.bottom()))
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        Stroke::new(1.5_f32, Color32::from_rgb(90, 255, 140)),
    ));
    painter.text(
        rect.right_bottom() - Vec2::new(4.0, 2.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("{span_ms:.0} ms"),
        egui::FontId::proportional(10.0),
        Color32::from_rgb(90, 160, 110),
    );
    resp.on_hover_text("The selected track's sound, like an oscilloscope.\nClick to change how much time it shows.")
        .clicked()
}

/// The shape of an ADSR envelope: up over the attack, down to the sustain
/// level over the decay, held while the note is, then down over the release.
pub fn envelope(ui: &mut egui::Ui, attack: f32, decay: f32, sustain: f32, release: f32) {
    let size = Vec2::new(ui.available_width().min(270.0), 46.0);
    envelope_sized(ui, size, attack, decay, sustain, release);
}

/// `envelope` at `size`; small ones leave out the letters.
pub fn envelope_sized(
    ui: &mut egui::Ui,
    size: Vec2,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_gray(28));
    // Times are drawn on a square-root scale so short and long both fit;
    // the held part gets a fixed width.
    let held = 0.6;
    let widths = [attack.sqrt(), decay.sqrt(), held, release.sqrt()];
    let total: f32 = widths.iter().sum();
    let inner = rect.shrink(4.0);
    let x = |t: f32| inner.left() + inner.width() * t / total;
    let y = |level: f32| inner.bottom() - inner.height() * level;
    let mut t = 0.0;
    let mut points = vec![Pos2::new(x(0.0), y(0.0))];
    for (w, level) in widths.iter().zip([1.0, sustain, sustain, 0.0]) {
        t += w;
        points.push(Pos2::new(x(t), y(level)));
    }
    painter.add(egui::Shape::line(points, Stroke::new(1.5_f32, note_on())));
    let labels = if size.y >= 30.0 { 4 } else { 0 };
    for (label, at) in [
        ("A", widths[0] / 2.0),
        ("D", widths[0] + widths[1] / 2.0),
        ("S", widths[0] + widths[1] + held / 2.0),
        ("R", total - widths[3] / 2.0),
    ]
    .into_iter()
    .take(labels)
    {
        painter.text(
            Pos2::new(x(at), inner.bottom()),
            egui::Align2::CENTER_BOTTOM,
            label,
            egui::FontId::proportional(9.0),
            Color32::from_gray(140),
        );
    }
    resp.on_hover_text("The volume over one note: Attack, Decay, Sustain, Release");
}

/// The filter's shape: how loud each pitch comes through (low on the left,
/// high on the right), with the resonance bump at the cutoff. `live` is
/// where modulation has the cutoff now, drawn as a second, brighter line.
pub fn filter_curve(ui: &mut egui::Ui, cutoff: f32, reso: f32, live: Option<f32>) {
    let size = Vec2::new(ui.available_width().min(260.0), 46.0);
    filter_curve_sized(ui, size, cutoff, reso, live);
}

/// `filter_curve` at `size`.
pub fn filter_curve_sized(
    ui: &mut egui::Ui,
    size: Vec2,
    cutoff: f32,
    reso: f32,
    live: Option<f32>,
) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_gray(28));
    let inner = rect.shrink(3.0);
    // 30 Hz to 16 kHz across, -30 to +18 dB up.
    let (lo, hi) = (30.0_f32.ln(), 16_000.0_f32.ln());
    let x = |hz: f32| inner.left() + inner.width() * (hz.ln() - lo) / (hi - lo);
    let y = |db: f32| inner.bottom() - inner.height() * ((db + 30.0) / 48.0).clamp(0.0, 1.0);
    let zero = y(0.0);
    painter.line_segment(
        [
            Pos2::new(inner.left(), zero),
            Pos2::new(inner.right(), zero),
        ],
        Stroke::new(1.0_f32, Color32::from_gray(45)),
    );
    // A two-pole low-pass: what the synth's filter does, near enough.
    let q = 1.0 / (1.4 - reso.clamp(0.0, 1.0) * 1.3);
    let curve = |fc: f32| -> Vec<Pos2> {
        (0..=60)
            .map(|i| {
                let hz = (lo + (hi - lo) * i as f32 / 60.0).exp();
                let r = hz / fc.max(1.0);
                let mag = 1.0 / ((1.0 - r * r).powi(2) + (r / q).powi(2)).sqrt();
                Pos2::new(x(hz), y(20.0 * mag.log10()))
            })
            .collect()
    };
    let dim = if live.is_some() { 0.4 } else { 1.0 };
    painter.add(egui::Shape::line(
        curve(cutoff),
        Stroke::new(1.5_f32, note_on().gamma_multiply(dim)),
    ));
    if let Some(live) = live {
        painter.add(egui::Shape::line(
            curve(live),
            Stroke::new(1.5_f32, Color32::WHITE),
        ));
    }
    resp.on_hover_text(
        "The filter: pitches on the left come through, the right is cut.\n\
         Cutoff moves the slope, Reso makes the bump",
    );
}

/// One cycle of an LFO's shape (`shape`: an index into the LFO shapes),
/// with a dot where it is now (`phase`, 0-1) in its chip's colour.
pub fn lfo_shape(ui: &mut egui::Ui, shape: usize, phase: Option<f32>, color: Color32) {
    let size = Vec2::new(ui.available_width().min(260.0), 30.0);
    lfo_shape_sized(ui, size, shape, phase, color);
}

/// `lfo_shape` at `size`.
pub fn lfo_shape_sized(
    ui: &mut egui::Ui,
    size: Vec2,
    shape: usize,
    phase: Option<f32>,
    color: Color32,
) {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_gray(28));
    let inner = rect.shrink(4.0);
    let value = |t: f32| -> f32 {
        match shape {
            0 => (t * std::f32::consts::TAU).sin(),
            1 => 1.0 - 4.0 * (t - 0.5).abs(),
            2 => {
                if t < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            3 => 1.0 - 2.0 * t,
            // Random: steps, drawn as a made-up example.
            _ => [0.6, -0.3, 0.9, -0.8, 0.2, -0.5, 0.4, -0.1][((t * 8.0) as usize).min(7)],
        }
    };
    let at = |t: f32| {
        Pos2::new(
            inner.left() + inner.width() * t,
            inner.center().y - inner.height() / 2.0 * value(t),
        )
    };
    let points: Vec<Pos2> = (0..=64).map(|i| at(i as f32 / 64.0)).collect();
    painter.add(egui::Shape::line(
        points,
        Stroke::new(1.5_f32, color.gamma_multiply(0.7)),
    ));
    if let Some(p) = phase {
        painter.circle_filled(at(p.clamp(0.0, 1.0)), 3.5, color);
    }
    resp.on_hover_text("The LFO's shape over one cycle; the dot is where it is now");
}

/// How loud each pitch is in `samples` (newest last): low on the left,
/// high on the right, like an equalizer display.
pub fn spectrum(
    ui: &mut egui::Ui,
    samples: &[f32],
    sample_rate: f32,
    size: Vec2,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_rgb(12, 16, 24));
    const N: usize = 2048;
    if samples.len() >= N {
        let tail = &samples[samples.len() - N..];
        // A Hann window, so a steady tone shows as a clean peak.
        let mut re: Vec<f32> = tail
            .iter()
            .enumerate()
            .map(|(i, s)| s * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / N as f32).cos()))
            .collect();
        let mut im = vec![0.0_f32; N];
        fft(&mut re, &mut im);
        let inner = rect.shrink(3.0);
        let (lo, hi) = (30.0_f32.ln(), (sample_rate / 2.0).min(16_000.0).ln());
        let mut line = Vec::with_capacity(121);
        for i in 0..=120 {
            let hz = (lo + (hi - lo) * i as f32 / 120.0).exp();
            let bin = ((hz / sample_rate * N as f32) as usize).clamp(1, N / 2 - 1);
            let mag = (re[bin] * re[bin] + im[bin] * im[bin]).sqrt() / (N as f32 / 4.0);
            let db = 20.0 * mag.max(1e-6).log10();
            let level = ((db + 72.0) / 72.0).clamp(0.0, 1.0);
            line.push(Pos2::new(
                inner.left() + inner.width() * i as f32 / 120.0,
                inner.bottom() - inner.height() * level,
            ));
        }
        for w in line.windows(2) {
            // Filled under the line, one thin slice at a time (the shape
            // isn't convex).
            let quad = vec![
                w[0],
                w[1],
                Pos2::new(w[1].x, inner.bottom()),
                Pos2::new(w[0].x, inner.bottom()),
            ];
            painter.add(egui::Shape::convex_polygon(
                quad,
                Color32::from_rgb(40, 90, 140),
                Stroke::NONE,
            ));
        }
        painter.add(egui::Shape::line(
            line,
            Stroke::new(1.5_f32, Color32::from_rgb(120, 190, 255)),
        ));
    }
    resp
}

/// An in-place radix-2 FFT (`re.len()` a power of two).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -std::f32::consts::TAU / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (angle * k as f32).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fft_finds_a_tone() {
        let n = 1024;
        let mut re: Vec<f32> = (0..n)
            .map(|i| (std::f32::consts::TAU * 50.0 * i as f32 / n as f32).sin())
            .collect();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im);
        let mags: Vec<f32> = (0..n / 2)
            .map(|i| (re[i] * re[i] + im[i] * im[i]).sqrt())
            .collect();
        let peak = (0..n / 2)
            .max_by(|&a, &b| mags[a].total_cmp(&mags[b]))
            .unwrap();
        assert_eq!(peak, 50);
    }
}
