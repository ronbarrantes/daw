//! The FM engine: one sine wave (the modulator) wobbles another's pitch
//! (the carrier) so fast that it turns into new tones. Bright at the start
//! of a note and mellowing as it rings, it makes electric pianos, bells,
//! glassy keys and punchy basses.

use std::f32::consts::TAU;

use super::play::FM;
use super::{coef, param, Module, ModuleInfo, Unit};

pub const RATIO: usize = 0;
pub const DEPTH: usize = 1;
pub const DECAY: usize = 2;
pub const FEEDBACK: usize = 3;

pub const INFO: ModuleInfo = ModuleInfo {
    name: "FM",
    knobs: &[
        param("FM ratio", 0.5, 8.0, 2.0, false, Unit::Ratio),
        param("FM depth", 0.0, 1.0, 0.4, false, Unit::Percent),
        param("FM decay", 0.02, 4.0, 0.6, true, Unit::Seconds),
        param("FM feedback", 0.0, 1.0, 0.0, false, Unit::Percent),
    ],
    rows: &[&[RATIO, DEPTH, DECAY, FEEDBACK]],
    open: true,
    engine: Some(FM),
};

/// One voice's two operators.
pub struct Fm {
    carrier: f32,
    modulator: f32,
    /// The modulator's last output, fed back into itself.
    last: f32,
    /// How bright the note still is: 1 when it starts, falling to a
    /// quarter of the depth.
    env: f32,
    ratio: f32,
    depth: f32,
    feedback: f32,
    decay_coef: f32,
}

impl Default for Fm {
    fn default() -> Self {
        Self {
            carrier: 0.0,
            modulator: 0.0,
            last: 0.0,
            env: 0.0,
            ratio: 2.0,
            depth: 0.4,
            feedback: 0.0,
            decay_coef: 0.0,
        }
    }
}

impl Module for Fm {
    fn set_knobs(&mut self, k: &[f32], sr: f32) {
        // Whole and half-way ratios sound musical; snap close ones there.
        let r = k[RATIO];
        let snapped = (r * 2.0).round() / 2.0;
        self.ratio = if (r - snapped).abs() < 0.06 {
            snapped
        } else {
            r
        };
        self.depth = k[DEPTH];
        self.feedback = k[FEEDBACK];
        self.decay_coef = coef(k[DECAY], sr);
    }
}

impl Fm {
    pub fn trigger(&mut self) {
        self.env = 1.0;
        // Start both waves at the top of their cycle, so every note
        // begins the same.
        self.carrier = 0.0;
        self.modulator = 0.0;
    }

    /// The next sample at `freq` Hz.
    pub fn next(&mut self, freq: f32, sr: f32) -> f32 {
        self.env -= self.env * self.decay_coef;
        let index = self.depth * 8.0 * (0.25 + 0.75 * self.env);
        self.modulator = (self.modulator + freq * self.ratio / sr).fract();
        let m = (self.modulator * TAU + self.feedback * 1.5 * self.last).sin();
        self.last = m;
        self.carrier = (self.carrier + freq / sr).fract();
        (self.carrier * TAU + index * m).sin()
    }
}
