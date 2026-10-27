//! The Mallet engine: a struck bar, bell or glass (modal synthesis). A
//! strike rings a handful of tuned resonators at the material's partials,
//! the higher ones dying away sooner, like a marimba, vibraphone, glass or
//! bell.

use std::f32::consts::TAU;

use super::play::MALLET;
use super::{choice, param, Module, ModuleInfo, Unit};

pub const MATERIAL: usize = 0;
pub const HARDNESS: usize = 1;
pub const DECAY: usize = 2;

pub const MATERIALS: [&str; 4] = ["Marimba", "Vibes", "Glass", "Bell"];
const MODES: usize = 6;
/// Each material's partials, as multiples of the note's pitch.
const RATIOS: [[f32; MODES]; 4] = [
    [1.0, 3.99, 10.65, 20.0, 31.0, 45.0],
    [1.0, 4.0, 10.0, 16.9, 25.3, 35.0],
    [1.0, 2.32, 4.25, 6.63, 9.38, 12.5],
    [1.0, 2.0, 2.4, 3.0, 4.0, 5.33],
];
/// How much faster each higher partial dies, per material.
const DAMPING: [f32; 4] = [1.6, 0.6, 0.4, 0.25];

pub const INFO: ModuleInfo = ModuleInfo {
    name: "Mallet",
    knobs: &[
        param("Material", 0.0, 3.0, 0.0, false, Unit::Choice(&MATERIALS)),
        param("Hardness", 0.0, 1.0, 0.5, false, Unit::Percent),
        param("Ring", 0.05, 8.0, 1.0, true, Unit::Seconds),
    ],
    rows: &[&[MATERIAL, HARDNESS, DECAY]],
    open: true,
    engine: Some(MALLET),
};

/// One voice's resonators.
pub struct Mallet {
    y1: [f32; MODES],
    y2: [f32; MODES],
    a1: [f32; MODES],
    a2: [f32; MODES],
    b: [f32; MODES],
    /// A strike waiting to go in on the next sample, per partial.
    strike: [f32; MODES],
    material: usize,
    hardness: f32,
    decay: f32,
    /// The pitch the coefficients were worked out for.
    tuned: f32,
}

impl Default for Mallet {
    fn default() -> Self {
        Self {
            y1: [0.0; MODES],
            y2: [0.0; MODES],
            a1: [0.0; MODES],
            a2: [0.0; MODES],
            b: [0.0; MODES],
            strike: [0.0; MODES],
            material: 0,
            hardness: 0.5,
            decay: 1.0,
            tuned: 0.0,
        }
    }
}

impl Module for Mallet {
    fn set_knobs(&mut self, k: &[f32], _sr: f32) {
        let material = choice(k[MATERIAL], MATERIALS.len());
        if material != self.material || k[DECAY] != self.decay {
            self.tuned = 0.0;
        }
        self.material = material;
        self.hardness = k[HARDNESS];
        self.decay = k[DECAY];
    }
}

impl Mallet {
    /// Strikes it: harder mallets bring out the higher partials.
    pub fn trigger(&mut self) {
        let hard = 0.15 + 0.75 * self.hardness;
        let mut g = 1.0;
        for s in &mut self.strike {
            *s = g;
            g *= hard;
        }
    }

    fn tune(&mut self, freq: f32, sr: f32) {
        for (m, ratio) in RATIOS[self.material].iter().enumerate() {
            let f = freq * ratio;
            if f > sr * 0.45 {
                // Too high to play; this partial stays silent.
                (self.a1[m], self.a2[m], self.b[m]) = (0.0, 0.0, 0.0);
                continue;
            }
            let w = TAU * f / sr;
            let seconds = self.decay / (1.0 + m as f32 * DAMPING[self.material]);
            let r = 0.001f32.powf(1.0 / (seconds * sr));
            self.a1[m] = 2.0 * r * w.cos();
            self.a2[m] = r * r;
            self.b[m] = w.sin();
        }
        self.tuned = freq;
    }

    /// The next sample at `freq` Hz.
    pub fn next(&mut self, freq: f32, sr: f32) -> f32 {
        if (freq - self.tuned).abs() > self.tuned * 0.0005 {
            self.tune(freq, sr);
        }
        let mut out = 0.0;
        for m in 0..MODES {
            let x = std::mem::take(&mut self.strike[m]) * self.b[m];
            let y = x + self.a1[m] * self.y1[m] - self.a2[m] * self.y2[m];
            self.y2[m] = self.y1[m];
            self.y1[m] = y;
            out += y;
        }
        out * 0.7
    }
}
