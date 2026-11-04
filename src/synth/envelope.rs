//! The Envelope (ADSR) module: shapes each note's volume. Attack fades
//! in, Decay falls to the Sustain level while the note is held, Release
//! fades out after.
//!
//! The attack rises on a curve that starts slow (the volume is the
//! square of how far through it is), so a long one sounds like a fade-in
//! instead of the note being there at once.

use super::{coef, param, Module, ModuleInfo, Unit};

pub const ATTACK: usize = 0;
pub const DECAY: usize = 1;
pub const SUSTAIN: usize = 2;
pub const RELEASE: usize = 3;

pub const INFO: ModuleInfo = ModuleInfo {
    name: "Envelope (ADSR)",
    knobs: &[
        param("Attack", 0.001, 10.0, 0.003, true, Unit::Seconds),
        param("Decay", 0.01, 2.0, 0.3, true, Unit::Seconds),
        param("Sustain", 0.0, 1.0, 0.8, false, Unit::Percent),
        param("Release", 0.01, 2.0, 0.15, true, Unit::Seconds),
    ],
    rows: &[&[ATTACK, DECAY, SUSTAIN, RELEASE]],
    open: true,
    engine: None,
};

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Attack,
    Decay,
    Release,
}

/// One voice's envelope.
pub struct Envelope {
    stage: Stage,
    /// The volume right now, 0-1.
    pub amp: f32,
    sustain: f32,
    /// How far through the attack, 0-1; the volume is this squared.
    rise: f32,
    attack_step: f32,
    decay_coef: f32,
    release_coef: f32,
}

impl Default for Envelope {
    fn default() -> Self {
        Self {
            stage: Stage::Release,
            amp: 0.0,
            sustain: 0.0,
            rise: 0.0,
            attack_step: 1.0,
            decay_coef: 0.0,
            release_coef: 0.0,
        }
    }
}

impl Module for Envelope {
    fn set_knobs(&mut self, k: &[f32], sr: f32) {
        self.sustain = k[SUSTAIN];
        self.attack_step = 1.0 / (k[ATTACK] * sr).max(1.0);
        self.decay_coef = coef(k[DECAY], sr);
        self.release_coef = coef(k[RELEASE], sr);
    }
}

impl Envelope {
    /// Starts the attack, from the volume it's at now (a retriggered note
    /// carries on rising rather than jumping).
    pub fn trigger(&mut self) {
        self.stage = Stage::Attack;
        self.rise = self.amp.sqrt();
    }

    /// Starts the release.
    pub fn release(&mut self) {
        self.stage = Stage::Release;
    }

    pub fn is_releasing(&self) -> bool {
        self.stage == Stage::Release
    }

    /// The next volume.
    pub fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Attack => {
                self.rise += self.attack_step;
                if self.rise >= 1.0 {
                    self.amp = 1.0;
                    self.stage = Stage::Decay;
                } else {
                    self.amp = self.rise * self.rise;
                }
            }
            Stage::Decay => self.amp += (self.sustain - self.amp) * self.decay_coef,
            Stage::Release => self.amp -= self.amp * self.release_coef,
        }
        self.amp
    }
}
