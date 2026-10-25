//! A small mono synth: two oscillators (saw-to-square morph) into a resonant
//! low-pass filter. An ADSR envelope shapes the volume, and a separate decay
//! envelope sweeps the filter for plucky, analog-ish sounds.

use std::f32::consts::PI;

/// A user-facing synth control.
pub struct Param {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// Knob moves exponentially (for frequencies and times).
    pub log: bool,
    pub unit: Unit,
}

#[derive(PartialEq)]
pub enum Unit {
    Percent,
    Hz,
    Seconds,
    /// -1 to 1, shown as L50, C or R50.
    Pan,
}

pub const WAVE: usize = 0;
pub const DETUNE: usize = 1;
pub const CUTOFF: usize = 2;
pub const RESONANCE: usize = 3;
pub const ENV_AMOUNT: usize = 4;
pub const FILTER_DECAY: usize = 5;
pub const ATTACK: usize = 6;
pub const RELEASE: usize = 7;
pub const VOLUME: usize = 8;
pub const DECAY: usize = 9;
pub const SUSTAIN: usize = 10;

const fn param(
    name: &'static str,
    min: f32,
    max: f32,
    default: f32,
    log: bool,
    unit: Unit,
) -> Param {
    Param {
        name,
        min,
        max,
        default,
        log,
        unit,
    }
}

pub const PARAMS: [Param; 11] = [
    param("Wave", 0.0, 1.0, 0.0, false, Unit::Percent),
    param("Detune", 0.0, 1.0, 0.2, false, Unit::Percent),
    param("Cutoff", 60.0, 8000.0, 250.0, true, Unit::Hz),
    param("Reso", 0.0, 1.0, 0.35, false, Unit::Percent),
    param("Env amt", 0.0, 1.0, 0.65, false, Unit::Percent),
    param("F decay", 0.02, 2.0, 0.25, true, Unit::Seconds),
    param("Attack", 0.001, 1.0, 0.003, true, Unit::Seconds),
    param("Release", 0.01, 2.0, 0.15, true, Unit::Seconds),
    param("Volume", 0.0, 1.0, 0.7, false, Unit::Percent),
    param("Decay", 0.01, 2.0, 0.3, true, Unit::Seconds),
    param("Sustain", 0.0, 1.0, 0.8, false, Unit::Percent),
];

/// The attack aims past full volume so it actually gets there, then hands
/// over to the decay stage.
const ATTACK_TARGET: f32 = 1.2;

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Attack,
    Decay,
    Release,
}

pub type Values = [f32; PARAMS.len()];

/// Per-sample smoothing factor that covers ~63% of the way in `seconds`.
fn coef(seconds: f32, sr: f32) -> f32 {
    1.0 - (-1.0 / (seconds * sr)).exp()
}

pub fn midi_to_hz(note: u8) -> f32 {
    440.0 * 2f32.powf((note as f32 - 69.0) / 12.0)
}

pub struct Voice {
    sr: f32,
    freq: f32,
    phase: [f32; 2],
    stage: Stage,
    amp: f32,
    filter_env: f32,
    // State-variable filter memory.
    low: f32,
    band: f32,
    // Derived from the knob values by `set_params`.
    wave: f32,
    detune: f32,
    cutoff: f32,
    damping: f32,
    env_octaves: f32,
    volume: f32,
    sustain: f32,
    attack_coef: f32,
    decay_coef: f32,
    release_coef: f32,
    filter_decay_coef: f32,
}

impl Voice {
    pub fn new(sr: f32) -> Self {
        let mut v = Self {
            sr,
            freq: 220.0,
            phase: [0.0; 2],
            stage: Stage::Release,
            amp: 0.0,
            filter_env: 0.0,
            low: 0.0,
            band: 0.0,
            wave: 0.0,
            detune: 1.0,
            cutoff: 0.0,
            damping: 1.0,
            env_octaves: 0.0,
            volume: 0.0,
            sustain: 0.0,
            attack_coef: 0.0,
            decay_coef: 0.0,
            release_coef: 0.0,
            filter_decay_coef: 0.0,
        };
        v.set_params(&PARAMS.map(|p| p.default));
        v
    }

    pub fn set_params(&mut self, v: &Values) {
        self.wave = v[WAVE];
        self.detune = 1.0 + v[DETUNE] * 0.03;
        self.cutoff = v[CUTOFF];
        // Less damping = more resonance. Never reaches 0, so it can't self-oscillate wildly.
        self.damping = 1.4 - v[RESONANCE] * 1.3;
        self.env_octaves = v[ENV_AMOUNT] * 6.0;
        self.volume = v[VOLUME];
        self.sustain = v[SUSTAIN];
        // Reaching full volume takes ~1.8 time constants with ATTACK_TARGET,
        // so this makes the knob read as the actual attack time.
        self.attack_coef = coef(v[ATTACK] / 1.8, self.sr);
        self.decay_coef = coef(v[DECAY], self.sr);
        self.release_coef = coef(v[RELEASE], self.sr);
        self.filter_decay_coef = coef(v[FILTER_DECAY], self.sr);
    }

    pub fn note_on(&mut self, note: u8) {
        self.freq = midi_to_hz(note);
        self.stage = Stage::Attack;
        self.filter_env = 1.0;
    }

    /// Finished and silent, so it can be skipped to save CPU.
    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Release && self.amp < 1e-5
    }

    pub fn note_off(&mut self) {
        self.stage = Stage::Release;
    }

    pub fn next_sample(&mut self) -> f32 {
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
        self.filter_env -= self.filter_env * self.filter_decay_coef;

        let mut osc = 0.0;
        for (i, phase) in self.phase.iter_mut().enumerate() {
            let freq = if i == 0 {
                self.freq
            } else {
                self.freq * self.detune
            };
            *phase = (*phase + freq / self.sr).fract();
            let saw = *phase * 2.0 - 1.0;
            let square = if *phase < 0.5 { 1.0 } else { -1.0 };
            osc += saw + (square - saw) * self.wave;
        }

        // Chamberlin state-variable filter. Keeping the cutoff below sr/6.5
        // keeps it stable.
        let cutoff =
            (self.cutoff * 2f32.powf(self.env_octaves * self.filter_env)).min(self.sr / 6.5);
        let f = 2.0 * (PI * cutoff / self.sr).sin();
        let high = osc * 0.5 - self.low - self.damping * self.band;
        self.band += f * high;
        self.low += f * self.band;

        self.low * self.amp * self.volume * 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adsr_shapes_the_volume() {
        let sr = 48_000.0;
        let mut values = PARAMS.map(|p| p.default);
        values[ATTACK] = 0.1;
        values[DECAY] = 0.05;
        values[SUSTAIN] = 0.5;
        values[RELEASE] = 0.05;
        let mut v = Voice::new(sr);
        v.set_params(&values);
        v.note_on(60);
        let run = |v: &mut Voice, seconds: f32| {
            for _ in 0..(seconds * sr) as usize {
                v.next_sample();
            }
            v.amp
        };
        let rising = run(&mut v, 0.05);
        assert!(
            rising > 0.2 && rising < 0.9,
            "halfway through attack: {rising}"
        );
        let peak = run(&mut v, 0.06);
        assert!(peak > 0.8, "attack done near 0.1 s: {peak}");
        let held = run(&mut v, 0.5);
        assert!((held - 0.5).abs() < 0.01, "settles at sustain: {held}");
        v.note_off();
        let released = run(&mut v, 0.5);
        assert!(released < 0.001, "fades after release: {released}");
    }
}
