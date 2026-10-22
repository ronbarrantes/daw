//! A small mono synth: two detuned saws into a resonant low-pass filter,
//! with a filter envelope for a plucky, analog-ish sound.

use std::f32::consts::PI;

const ATTACK: f32 = 0.003;
const RELEASE: f32 = 0.15;
const FILTER_DECAY: f32 = 0.25;
const DETUNE: f32 = 1.006;

pub struct Voice {
    sr: f32,
    freq: f32,
    phase: [f32; 2],
    gate: bool,
    amp: f32,
    filter_env: f32,
    // State-variable filter memory.
    low: f32,
    band: f32,
    attack_coef: f32,
    release_coef: f32,
    filter_decay_coef: f32,
}

/// Per-sample smoothing factor that covers ~63% of the way in `seconds`.
fn coef(seconds: f32, sr: f32) -> f32 {
    1.0 - (-1.0 / (seconds * sr)).exp()
}

pub fn midi_to_hz(note: u8) -> f32 {
    440.0 * 2f32.powf((note as f32 - 69.0) / 12.0)
}

impl Voice {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            freq: 220.0,
            phase: [0.0; 2],
            gate: false,
            amp: 0.0,
            filter_env: 0.0,
            low: 0.0,
            band: 0.0,
            attack_coef: coef(ATTACK, sr),
            release_coef: coef(RELEASE, sr),
            filter_decay_coef: coef(FILTER_DECAY, sr),
        }
    }

    pub fn note_on(&mut self, note: u8) {
        self.freq = midi_to_hz(note);
        self.gate = true;
        self.filter_env = 1.0;
    }

    pub fn note_off(&mut self) {
        self.gate = false;
    }

    pub fn next_sample(&mut self) -> f32 {
        let (target, rate) = if self.gate {
            (1.0, self.attack_coef)
        } else {
            (0.0, self.release_coef)
        };
        self.amp += (target - self.amp) * rate;
        self.filter_env -= self.filter_env * self.filter_decay_coef;

        let mut osc = 0.0;
        for (i, phase) in self.phase.iter_mut().enumerate() {
            let freq = if i == 0 { self.freq } else { self.freq * DETUNE };
            *phase = (*phase + freq / self.sr).fract();
            osc += *phase * 2.0 - 1.0;
        }

        // Chamberlin state-variable filter; cutoff stays well below sr/6 so
        // it remains stable.
        let cutoff = 250.0 + 3500.0 * self.filter_env;
        let f = 2.0 * (PI * cutoff / self.sr).sin();
        let damping = 0.6; // lower = more resonance
        let high = osc * 0.5 - self.low - damping * self.band;
        self.band += f * high;
        self.low += f * self.band;

        self.low * self.amp * 0.35
    }
}
