//! The LFO module: a slow wobble on the pitch (vibrato), filter (wah),
//! volume (tremolo) or pan. One per track, shared by its voices so they
//! wobble together. Its speed is in Hz, or tied to the tempo.

use super::{choice, knobs, noise, param, ModuleInfo, Unit, Values};

pub const TARGET: usize = 0;
pub const SHAPE: usize = 1;
pub const AMOUNT: usize = 2;
pub const RATE: usize = 3;
pub const SYNC: usize = 4;

pub const SHAPES: [&str; 5] = ["Sine", "Tri", "Square", "Saw", "Random"];
/// `SHAPES` index of Random.
pub const RANDOM: usize = 4;
pub const TARGETS: [&str; 4] = ["Pitch", "Filter", "Volume", "Pan"];
/// `TARGETS` index of Pan, which the engine applies to the whole track.
pub const PAN: usize = 3;
/// LFO speeds tied to the tempo, and how many beats each cycle lasts.
pub const SYNCS: [&str; 8] = [
    "Off", "1 bar", "1/2", "1/4", "1/8", "1/16", "1/8 T", "1/16 T",
];
const SYNC_BEATS: [f32; 8] = [0.0, 4.0, 2.0, 1.0, 0.5, 0.25, 1.0 / 3.0, 1.0 / 6.0];

pub const INFO: ModuleInfo = ModuleInfo {
    name: "LFO",
    knobs: &[
        param("LFO to", 0.0, 3.0, 0.0, false, Unit::Choice(&TARGETS)),
        param("LFO wave", 0.0, 4.0, 0.0, false, Unit::Choice(&SHAPES)),
        param("LFO amt", 0.0, 1.0, 0.0, false, Unit::Percent),
        param("LFO rate", 0.05, 20.0, 4.0, true, Unit::Hz),
        param("LFO sync", 0.0, 7.0, 0.0, false, Unit::Choice(&SYNCS)),
    ],
    rows: &[&[TARGET, SHAPE, AMOUNT, RATE], &[SYNC]],
    open: true,
};

/// A track's LFO.
pub struct Lfo {
    phase: f32,
    /// The Random shape's current value, picked each cycle.
    held: f32,
    rng: u32,
}

impl Default for Lfo {
    fn default() -> Self {
        Self {
            phase: 0.0,
            held: 0.0,
            rng: 0x2f6b_1d93,
        }
    }
}

impl Lfo {
    /// How far through a cycle one sample goes, from the rate knob or,
    /// when synced, the tempo (`beat` is a beat's length in samples).
    pub fn step(v: &Values, sr: f32, beat: f32) -> f32 {
        let k = knobs(v, &INFO);
        Self::step_with(k[RATE], k[SYNC], sr, beat)
    }

    /// `step` for any LFO's Rate and Sync knob values.
    pub fn step_with(rate: f32, sync: f32, sr: f32, beat: f32) -> f32 {
        match choice(sync, SYNCS.len()) {
            0 => rate / sr,
            sync => 1.0 / (SYNC_BEATS[sync] * beat).max(1.0),
        }
    }

    /// The next value, -1 to 1.
    pub fn next(&mut self, v: &Values, step: f32) -> f32 {
        self.next_with(knobs(v, &INFO)[SHAPE], step)
    }

    /// `next` for any LFO's Wave knob value.
    pub fn next_with(&mut self, shape: f32, step: f32) -> f32 {
        self.phase += step;
        if self.phase >= 1.0 {
            self.phase = self.phase.fract();
            self.held = noise(&mut self.rng);
        }
        match choice(shape, SHAPES.len()) {
            0 => (self.phase * std::f32::consts::TAU).sin(),
            1 => 1.0 - 4.0 * (self.phase - 0.5).abs(),
            2 => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            3 => 1.0 - 2.0 * self.phase,
            _ => self.held,
        }
    }
}
