//! The Mod envelope module: a second envelope, one per voice, that rises
//! when a note starts and falls back. It has no target of its own: drag it
//! onto any knob (see `modulation`), e.g. the cutoff for a filter sweep or
//! Pitch 2 for a blip at the start of each note.

use super::{coef, param, ModuleInfo, Unit};

pub const ATTACK: usize = 0;
pub const DECAY: usize = 1;
pub const SUSTAIN: usize = 2;

pub const INFO: ModuleInfo = ModuleInfo {
    name: "Mod envelope",
    knobs: &[
        param("Mod attack", 0.001, 2.0, 0.01, true, Unit::Seconds),
        param("Mod decay", 0.01, 4.0, 0.4, true, Unit::Seconds),
        param("Mod sustain", 0.0, 1.0, 0.0, false, Unit::Percent),
    ],
    rows: &[&[ATTACK, DECAY, SUSTAIN]],
    open: true,
};

/// One voice's mod envelope, 0-1. It moves in steps of `n` samples, as
/// often as the engine updates modulation, not every sample.
#[derive(Clone, Copy, Default)]
pub struct ModEnv {
    pub value: f32,
    rising: bool,
    held: bool,
}

impl ModEnv {
    pub fn trigger(&mut self) {
        self.rising = true;
        self.held = true;
    }

    pub fn release(&mut self) {
        self.held = false;
        self.rising = false;
    }

    /// Moves on by `n` samples with the module's knobs `k`.
    pub fn advance(&mut self, k: &[f32], n: u32, sr: f32) {
        let n = n as f32;
        if self.rising {
            self.value += n / (k[ATTACK] * sr).max(1.0);
            if self.value >= 1.0 {
                self.value = 1.0;
                self.rising = false;
            }
        } else {
            let to = if self.held { k[SUSTAIN] } else { 0.0 };
            let c = coef(k[DECAY], sr);
            // `c` per sample, `n` times over.
            let keep = (1.0 - c).powf(n);
            self.value = to + (self.value - to) * keep;
        }
    }
}
