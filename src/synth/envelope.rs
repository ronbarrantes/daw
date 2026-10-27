//! The Envelope (ADSR) module: shapes each note's volume. Attack fades
//! in, Decay falls to the Sustain level while the note is held, Release
//! fades out after.

use super::{coef, param, Module, ModuleInfo, Unit};

pub const ATTACK: usize = 0;
pub const DECAY: usize = 1;
pub const SUSTAIN: usize = 2;
pub const RELEASE: usize = 3;

pub const INFO: ModuleInfo = ModuleInfo {
    name: "Envelope (ADSR)",
    knobs: &[
        param("Attack", 0.001, 1.0, 0.003, true, Unit::Seconds),
        param("Decay", 0.01, 2.0, 0.3, true, Unit::Seconds),
        param("Sustain", 0.0, 1.0, 0.8, false, Unit::Percent),
        param("Release", 0.01, 2.0, 0.15, true, Unit::Seconds),
    ],
    rows: &[&[ATTACK, DECAY, SUSTAIN, RELEASE]],
    open: true,
};

/// The attack aims past full volume so it actually gets there, then hands
/// over to the decay stage.
const ATTACK_TARGET: f32 = 1.2;

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
    attack_coef: f32,
    decay_coef: f32,
    release_coef: f32,
}

impl Default for Envelope {
    fn default() -> Self {
        Self {
            stage: Stage::Release,
            amp: 0.0,
            sustain: 0.0,
            attack_coef: 0.0,
            decay_coef: 0.0,
            release_coef: 0.0,
        }
    }
}

impl Module for Envelope {
    fn set_knobs(&mut self, k: &[f32], sr: f32) {
        self.sustain = k[SUSTAIN];
        // Reaching full volume takes ~1.8 time constants with ATTACK_TARGET,
        // so this makes the knob read as the actual attack time.
        self.attack_coef = coef(k[ATTACK] / 1.8, sr);
        self.decay_coef = coef(k[DECAY], sr);
        self.release_coef = coef(k[RELEASE], sr);
    }
}

impl Envelope {
    /// Starts the attack.
    pub fn trigger(&mut self) {
        self.stage = Stage::Attack;
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
                self.amp += (ATTACK_TARGET - self.amp) * self.attack_coef;
                if self.amp >= 1.0 {
                    self.amp = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => self.amp += (self.sustain - self.amp) * self.decay_coef,
            Stage::Release => self.amp -= self.amp * self.release_coef,
        }
        self.amp
    }
}
