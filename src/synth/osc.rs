//! The Oscillators module: two oscillators, each morphing from sine
//! through triangle, saw and square to noise. The second one can be mixed
//! in, tuned in semitones and detuned slightly for a thicker sound.

use super::play::ANALOG;
use super::{noise, param, Module, ModuleInfo, Unit};

pub const WAVE_1: usize = 0;
pub const WAVE_2: usize = 1;
pub const LEVEL_2: usize = 2;
pub const PITCH_2: usize = 3;
pub const DETUNE: usize = 4;

/// The shapes an oscillator morphs through, in order.
pub const WAVES: [&str; 5] = ["Sine", "Tri", "Saw", "Square", "Noise"];
/// Where `Unit::Wave` puts a plain saw, the sound before shapes could be picked.
pub const SAW: f32 = 2.0;

pub const INFO: ModuleInfo = ModuleInfo {
    name: "Oscillators",
    knobs: &[
        param("Wave 1", 0.0, 4.0, SAW, false, Unit::Wave),
        param("Wave 2", 0.0, 4.0, SAW, false, Unit::Wave),
        param("Level 2", 0.0, 1.0, 1.0, false, Unit::Percent),
        param("Pitch 2", -24.0, 24.0, 0.0, false, Unit::Semitones),
        param("Detune", 0.0, 1.0, 0.2, false, Unit::Percent),
    ],
    rows: &[&[WAVE_1, WAVE_2, LEVEL_2, PITCH_2], &[DETUNE]],
    open: true,
    engine: Some(ANALOG),
};

/// One of `WAVES` at `phase` (0-1).
fn shape(i: usize, phase: f32, rng: &mut u32) -> f32 {
    match i {
        0 => (phase * std::f32::consts::TAU).sin(),
        1 => 1.0 - 4.0 * (phase - 0.5).abs(),
        2 => phase * 2.0 - 1.0,
        3 => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        _ => noise(rng),
    }
}

/// An oscillator `wave` (0-4) of the way along `WAVES`, blending the two
/// shapes either side.
pub(super) fn morph(wave: f32, phase: f32, rng: &mut u32) -> f32 {
    let i = (wave.max(0.0) as usize).min(WAVES.len() - 2);
    let f = (wave - i as f32).clamp(0.0, 1.0);
    let a = if f < 1.0 { shape(i, phase, rng) } else { 0.0 };
    let b = if f > 0.0 {
        shape(i + 1, phase, rng)
    } else {
        0.0
    };
    a + (b - a) * f
}

/// One voice's oscillators.
pub struct Osc {
    phase: [f32; 2],
    rng: u32,
    waves: [f32; 2],
    level_2: f32,
    /// The second oscillator's pitch relative to the first: Detune and
    /// Pitch 2 together.
    ratio_2: f32,
}

impl Default for Osc {
    fn default() -> Self {
        Self {
            phase: [0.0; 2],
            rng: 0x1357_9bdf,
            waves: [SAW; 2],
            level_2: 1.0,
            ratio_2: 1.0,
        }
    }
}

impl Module for Osc {
    fn set_knobs(&mut self, k: &[f32], _sr: f32) {
        self.waves = [k[WAVE_1], k[WAVE_2]];
        self.level_2 = k[LEVEL_2];
        let semis = k[PITCH_2].round();
        self.ratio_2 = (1.0 + k[DETUNE] * 0.03) * 2f32.powf(semis / 12.0);
    }
}

impl Osc {
    /// The next sample at `freq` Hz.
    pub fn next(&mut self, freq: f32, sr: f32) -> f32 {
        self.phase[0] = (self.phase[0] + freq / sr).fract();
        let mut osc = morph(self.waves[0], self.phase[0], &mut self.rng);
        if self.level_2 > 0.0 {
            self.phase[1] = (self.phase[1] + freq * self.ratio_2 / sr).fract();
            osc += morph(self.waves[1], self.phase[1], &mut self.rng) * self.level_2;
        }
        osc
    }
}
