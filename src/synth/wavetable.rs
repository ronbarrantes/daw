//! The Wavetable engine: a table of single-cycle waves that it sweeps
//! through, so turning Position (or dragging an LFO or the mod envelope
//! onto it) makes the sound move and morph: from smooth to buzzy, through
//! vowel-like sounds, or bright metallic tones.

use std::sync::LazyLock;

use super::play::WAVETABLE;
use super::{choice, param, Module, ModuleInfo, Unit};

pub const TABLE: usize = 0;
pub const POSITION: usize = 1;
pub const SUB: usize = 2;

pub const TABLES: [&str; 4] = ["Basic", "Vocal", "Hollow", "Metal"];

pub const INFO: ModuleInfo = ModuleInfo {
    name: "Wavetable",
    knobs: &[
        param("Table", 0.0, 3.0, 0.0, false, Unit::Choice(&TABLES)),
        param("Position", 0.0, 1.0, 0.0, false, Unit::Percent),
        param("Sub", 0.0, 1.0, 0.0, false, Unit::Percent),
    ],
    rows: &[&[TABLE, POSITION, SUB]],
    open: true,
    engine: Some(WAVETABLE),
};

/// Samples in one cycle.
const SIZE: usize = 2048;
/// Waves in a table, swept through by Position.
const FRAMES: usize = 8;
/// The most harmonics a wave has; higher notes use copies with fewer, so
/// nothing goes above what the sample rate can play (no aliasing).
const MAX_HARMONICS: usize = 64;
/// Copies with 64, 32, 16, 8, 4, 2 and 1 harmonics.
const LEVELS: usize = 7;

type Wave = [f32; SIZE];

/// Every wave of every table, built once (before the engine starts, see
/// `warm_up`); `wave(table, level, frame)` finds one.
static WAVES: LazyLock<Vec<Wave>> = LazyLock::new(build);

fn wave(table: usize, level: usize, frame: usize) -> &'static Wave {
    &WAVES[(table * LEVELS + level) * FRAMES + frame]
}

/// Builds the tables now, so the audio thread never has to (building
/// allocates).
pub fn warm_up() {
    LazyLock::force(&WAVES);
}

/// Harmonic `k`'s (1-based) strength in frame `t` (0-1) of `table`.
fn harmonic(table: usize, k: usize, t: f32) -> f32 {
    let kf = k as f32;
    let odd = k % 2 == 1;
    match table {
        // Sine, then triangle, saw and square, blending in between.
        0 => {
            let sine = if k == 1 { 1.0 } else { 0.0 };
            let tri = if odd {
                let sign = if (k / 2).is_multiple_of(2) { 1.0 } else { -1.0 };
                sign / (kf * kf)
            } else {
                0.0
            };
            let saw = 1.0 / kf;
            let square = if odd { 1.0 / kf } else { 0.0 };
            let s = t * 3.0;
            let (a, b, f) = match s as usize {
                0 => (sine, tri, s),
                1 => (tri, saw, s - 1.0),
                _ => (saw, square, (s - 2.0).min(1.0)),
            };
            a + (b - a) * f
        }
        // Two vowel-like peaks moving up the harmonics.
        1 => {
            let f1 = 3.0 + 5.0 * t;
            let f2 = 10.0 + 10.0 * (1.0 - t);
            let peak = |c: f32, w: f32| (-(kf - c) * (kf - c) / w).exp();
            (peak(f1, 4.0) + 0.7 * peak(f2, 8.0)) / kf.sqrt() + 0.05 / kf
        }
        // Odd harmonics only (hollow, like a clarinet), with the even ones
        // and more brightness coming in.
        2 => {
            let even = if odd { 1.0 } else { t };
            even / kf.powf(2.0 - 1.4 * t)
        }
        // A cluster of high harmonics sliding up: clangy and bright.
        _ => {
            let fundamental = if k == 1 { 0.6 } else { 0.0 };
            let c = 4.0 + 40.0 * t;
            fundamental + (-(kf - c) * (kf - c) / 6.0).exp() * if odd { 1.0 } else { 0.6 }
        }
    }
}

fn build() -> Vec<Wave> {
    let sine: Vec<f32> = (0..SIZE)
        .map(|n| (std::f32::consts::TAU * n as f32 / SIZE as f32).sin())
        .collect();
    let mut waves = vec![[0.0_f32; SIZE]; TABLES.len() * LEVELS * FRAMES];
    for table in 0..TABLES.len() {
        for f in 0..FRAMES {
            let t = f as f32 / (FRAMES - 1) as f32;
            for level in 0..LEVELS {
                let wave = &mut waves[(table * LEVELS + level) * FRAMES + f];
                for k in 1..=MAX_HARMONICS >> level {
                    let a = harmonic(table, k, t);
                    if a.abs() < 1e-4 {
                        continue;
                    }
                    for (n, s) in wave.iter_mut().enumerate() {
                        *s += a * sine[(k * n) % SIZE];
                    }
                }
            }
            // The same loudness at every level: scale by the full wave's
            // peak.
            let full = (table * LEVELS) * FRAMES + f;
            let peak = waves[full].iter().fold(0.0_f32, |m, s| m.max(s.abs()));
            for level in 0..LEVELS {
                for s in waves[(table * LEVELS + level) * FRAMES + f].iter_mut() {
                    *s /= peak.max(1e-6);
                }
            }
        }
    }
    waves
}

/// One voice's wavetable oscillator.
#[derive(Default)]
pub struct Wavetable {
    phase: f32,
    sub_phase: f32,
    table: usize,
    position: f32,
    sub: f32,
}

impl Module for Wavetable {
    fn set_knobs(&mut self, k: &[f32], _sr: f32) {
        self.table = choice(k[TABLE], TABLES.len());
        self.position = k[POSITION].clamp(0.0, 1.0);
        self.sub = k[SUB];
    }
}

impl Wavetable {
    /// The next sample at `freq` Hz.
    pub fn next(&mut self, freq: f32, sr: f32) -> f32 {
        // The copy with as many harmonics as fit below half the sample rate.
        let fit = (sr * 0.5 / freq.max(1.0)) as usize;
        let level = (0..LEVELS)
            .find(|&l| MAX_HARMONICS >> l <= fit)
            .unwrap_or(LEVELS - 1);
        let at = self.position * (FRAMES - 1) as f32;
        let f = (at as usize).min(FRAMES - 2);
        let mix = at - f as f32;
        self.phase = (self.phase + freq / sr).fract();
        let x = self.phase * SIZE as f32;
        let i = x as usize % SIZE;
        let j = (i + 1) % SIZE;
        let frac = x - x.floor();
        let read = |w: &Wave| w[i] + (w[j] - w[i]) * frac;
        let a = read(wave(self.table, level, f));
        let b = read(wave(self.table, level, f + 1));
        let mut out = a + (b - a) * mix;
        if self.sub > 0.0 {
            self.sub_phase = (self.sub_phase + freq * 0.5 / sr).fract();
            out += (self.sub_phase * std::f32::consts::TAU).sin() * self.sub;
        }
        out
    }
}
