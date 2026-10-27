//! The Voice module: how the synth plays its notes. Engine picks what
//! makes the sound (Analog oscillators, a plucked string, a struck
//! mallet, FM, a wavetable). Poly plays chords; Mono and Legato play one note at a time,
//! and Glide slides between them.
//! Volume is the synth's own level (automation can move it; the track's
//! Mix volume comes after).

use super::{choice, param, ModuleInfo, Unit};

pub const VOICES: usize = 0;
pub const GLIDE: usize = 1;
pub const VOLUME: usize = 2;
pub const ENGINE: usize = 3;

pub const MODES: [&str; 3] = ["Poly", "Mono", "Legato"];
/// What makes the sound; each has its own panel section.
pub const ENGINES: [&str; 5] = ["Analog", "Pluck", "Mallet", "FM", "Wavetable"];
pub const ANALOG: usize = 0;
pub const PLUCK: usize = 1;
pub const MALLET: usize = 2;
pub const FM: usize = 3;
pub const WAVETABLE: usize = 4;

pub const INFO: ModuleInfo = ModuleInfo {
    name: "Voice",
    knobs: &[
        param("Voices", 0.0, 2.0, 0.0, false, Unit::Choice(&MODES)),
        param("Glide", 0.0, 1.0, 0.0, false, Unit::Seconds),
        param("Volume", 0.0, 1.0, 0.7, false, Unit::Percent),
        param("Engine", 0.0, 4.0, 0.0, false, Unit::Choice(&ENGINES)),
    ],
    rows: &[&[ENGINE, VOICES, GLIDE]],
    open: true,
    engine: None,
};

/// How a synth track plays overlapping notes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    /// Chords: each note gets its own voice.
    Poly,
    /// One note at a time; each new note restarts the envelope.
    Mono,
    /// One note at a time; a note played while another is held just
    /// changes pitch (gliding if Glide is up), without restarting.
    Legato,
}

/// The mode the Voices knob (`k[VOICES]`) picks.
pub fn mode(k: &[f32]) -> Mode {
    match choice(k[VOICES], MODES.len()) {
        1 => Mode::Mono,
        2 => Mode::Legato,
        _ => Mode::Poly,
    }
}
