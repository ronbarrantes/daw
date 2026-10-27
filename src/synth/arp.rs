//! The Arpeggiator module's knobs: it plays held notes one at a time in a
//! pattern, or a chord or scale built on one note. The engine runs it
//! (see `audio`), since it decides which notes the voices play.

use super::{param, ModuleInfo, Unit};

pub const MODE: usize = 0;
pub const RATE: usize = 1;
pub const OCTAVES: usize = 2;
pub const GATE: usize = 3;
pub const NOTES: usize = 4;
pub const LATCH: usize = 5;

/// How the arpeggiator walks through the held notes; Off plays them as
/// chords like normal.
pub const MODES: [&str; 6] = ["Off", "Up", "Down", "Up/Down", "Random", "As played"];
/// How often the arpeggiator plays a note, and how many 16th-note steps
/// that is.
pub const RATES: [&str; 6] = ["1/4", "1/8", "1/16", "1/32", "1/8 T", "1/16 T"];
pub const RATE_STEPS: [f32; 6] = [4.0, 2.0, 1.0, 0.5, 4.0 / 3.0, 2.0 / 3.0];
pub const OCTAVE_CHOICES: [&str; 4] = ["1", "2", "3", "4"];
pub const OFF_ON: [&str; 2] = ["Off", "On"];
/// What the arpeggiator plays: the notes held, or a chord or scale built
/// on the lowest held note, so one finger makes a whole pattern.
pub const NOTE_SETS: [&str; 10] = [
    "Held",
    "Major",
    "Minor",
    "7th",
    "Maj7",
    "Min7",
    "Sus4",
    "Major scale",
    "Minor scale",
    "Pentatonic",
];
/// The semitones above the root for each of `NOTE_SETS` after Held.
pub const INTERVALS: [&[u8]; 9] = [
    &[0, 4, 7],
    &[0, 3, 7],
    &[0, 4, 7, 10],
    &[0, 4, 7, 11],
    &[0, 3, 7, 10],
    &[0, 5, 7],
    &[0, 2, 4, 5, 7, 9, 11],
    &[0, 2, 3, 5, 7, 8, 10],
    &[0, 2, 4, 7, 9],
];

pub const INFO: ModuleInfo = ModuleInfo {
    name: "Arpeggiator",
    knobs: &[
        param("Arp", 0.0, 5.0, 0.0, false, Unit::Choice(&MODES)),
        param("Arp rate", 0.0, 5.0, 2.0, false, Unit::Choice(&RATES)),
        param(
            "Octaves",
            0.0,
            3.0,
            0.0,
            false,
            Unit::Choice(&OCTAVE_CHOICES),
        ),
        param("Arp gate", 0.05, 1.0, 0.5, false, Unit::Percent),
        param("Arp notes", 0.0, 9.0, 0.0, false, Unit::Choice(&NOTE_SETS)),
        param("Latch", 0.0, 1.0, 0.0, false, Unit::Choice(&OFF_ON)),
    ],
    rows: &[&[MODE, RATE, OCTAVES, GATE], &[NOTES, LATCH]],
    open: false,
};
