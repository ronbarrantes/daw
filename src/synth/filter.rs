//! The Filter module: a resonant low-pass filter, with its own decay
//! envelope that sweeps it down after each note starts, for plucky,
//! analog-ish sounds.

use std::f32::consts::PI;

use super::{coef, param, Module, ModuleInfo, Unit};

pub const CUTOFF: usize = 0;
pub const RESONANCE: usize = 1;
pub const ENV_AMOUNT: usize = 2;
pub const DECAY: usize = 3;

pub const INFO: ModuleInfo = ModuleInfo {
    name: "Filter",
    knobs: &[
        param("Cutoff", 60.0, 8000.0, 250.0, true, Unit::Hz),
        param("Reso", 0.0, 1.0, 0.35, false, Unit::Percent),
        param("Env amt", 0.0, 1.0, 0.65, false, Unit::Percent),
        param("F decay", 0.02, 2.0, 0.25, true, Unit::Seconds),
    ],
    rows: &[&[CUTOFF, RESONANCE, ENV_AMOUNT, DECAY]],
    open: true,
};

/// One voice's filter.
pub struct Filter {
    // State-variable filter memory.
    low: f32,
    band: f32,
    /// The decay envelope, 1 at the start of a note, falling to 0.
    env: f32,
    cutoff: f32,
    damping: f32,
    env_octaves: f32,
    decay_coef: f32,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            low: 0.0,
            band: 0.0,
            env: 0.0,
            cutoff: 0.0,
            damping: 1.0,
            env_octaves: 0.0,
            decay_coef: 0.0,
        }
    }
}

impl Module for Filter {
    fn set_knobs(&mut self, k: &[f32], sr: f32) {
        self.cutoff = k[CUTOFF];
        // Less damping = more resonance. Never reaches 0, so it can't self-oscillate wildly.
        self.damping = 1.4 - k[RESONANCE] * 1.3;
        self.env_octaves = k[ENV_AMOUNT] * 6.0;
        self.decay_coef = coef(k[DECAY], sr);
    }
}

impl Filter {
    /// Restarts the decay envelope, for a new note.
    pub fn trigger(&mut self) {
        self.env = 1.0;
    }

    /// How many octaves the envelope lifts the cutoff right now.
    pub fn env_octaves(&self) -> f32 {
        self.env_octaves * self.env
    }

    /// Moves the envelope on by a sample.
    pub fn decay(&mut self) {
        self.env -= self.env * self.decay_coef;
    }

    /// Filters a sample, with the cutoff moved by `octaves`.
    pub fn process(&mut self, input: f32, octaves: f32, sr: f32) -> f32 {
        // Chamberlin state-variable filter. Keeping the cutoff below sr/6.5
        // keeps it stable.
        let cutoff = (self.cutoff * 2f32.powf(octaves)).min(sr / 6.5);
        let f = 2.0 * (PI * cutoff / sr).sin();
        let high = input * 0.5 - self.low - self.damping * self.band;
        self.band += f * high;
        self.low += f * self.band;
        self.low
    }
}
