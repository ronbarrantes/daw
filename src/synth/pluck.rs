//! The Pluck engine: a plucked string (Karplus-Strong). A burst of noise
//! goes round a short delay line, one trip per cycle of the note, and is
//! softened a little each trip, so it rings like a guitar, harp or koto
//! string and dies away by itself.

use super::play::PLUCK;
use super::{noise, param, Module, ModuleInfo, Unit};

pub const TONE: usize = 0;
pub const DECAY: usize = 1;
pub const POSITION: usize = 2;

pub const INFO: ModuleInfo = ModuleInfo {
    name: "Pluck",
    knobs: &[
        param("String tone", 0.0, 1.0, 0.6, false, Unit::Percent),
        param("String decay", 0.1, 10.0, 2.0, true, Unit::Seconds),
        param("Pick spot", 0.0, 0.5, 0.15, false, Unit::Percent),
    ],
    rows: &[&[TONE, DECAY, POSITION]],
    open: true,
    engine: Some(PLUCK),
};

/// The longest string, in samples: below about 23 Hz at 96 kHz.
const LEN: usize = 4096;

/// One voice's string.
pub struct Pluck {
    /// Inline, not boxed: voices are made on the audio thread, which
    /// mustn't allocate.
    line: [f32; LEN],
    pos: usize,
    /// The last sample read, for the softening filter.
    last: f32,
    rng: u32,
    /// How much each trip softens the sound (0 = bright, 0.5 = soft).
    damp: f32,
    /// The noise burst's brightness, as a one-pole low-pass amount.
    burst: f32,
    position: f32,
    decay: f32,
    /// The loss per trip for `gain_freq`, worked out again when the pitch
    /// moves.
    gain: f32,
    gain_freq: f32,
}

impl Default for Pluck {
    fn default() -> Self {
        Self {
            line: [0.0; LEN],
            pos: 0,
            last: 0.0,
            rng: 0x6d2b_79f5,
            damp: 0.3,
            burst: 0.6,
            position: 0.15,
            decay: 2.0,
            gain: 0.0,
            gain_freq: 0.0,
        }
    }
}

impl Module for Pluck {
    fn set_knobs(&mut self, k: &[f32], _sr: f32) {
        self.damp = 0.05 + 0.45 * (1.0 - k[TONE]);
        self.burst = 0.1 + 0.9 * k[TONE];
        self.position = k[POSITION];
        if self.decay != k[DECAY] {
            self.decay = k[DECAY];
            self.gain_freq = 0.0;
        }
    }
}

impl Pluck {
    fn period(freq: f32, sr: f32) -> f32 {
        (sr / freq.max(1.0)).clamp(2.0, (LEN - 2) as f32)
    }

    /// Plucks the string: fills one cycle's worth of the line with a burst
    /// of noise, softened by Tone and shaped by where it's picked.
    pub fn trigger(&mut self, freq: f32, sr: f32) {
        let period = Self::period(freq, sr) as usize;
        let mut lp = 0.0;
        let mut burst = [0.0_f32; LEN];
        for b in burst.iter_mut().take(period) {
            lp += (noise(&mut self.rng) - lp) * self.burst;
            *b = lp;
        }
        // Picking away from the end cancels some harmonics, like a real
        // string: the burst minus itself a little later.
        let gap = (self.position * period as f32) as usize;
        for i in 0..period {
            let back = if i >= gap { burst[i - gap] } else { 0.0 };
            let at = (self.pos + LEN - period + i) % LEN;
            self.line[at] += burst[i] - back;
        }
    }

    /// The next sample at `freq` Hz.
    pub fn next(&mut self, freq: f32, sr: f32) -> f32 {
        // The softening filter delays by about `damp`; take it off so the
        // string stays in tune.
        let delay = (Self::period(freq, sr) - self.damp).max(1.0);
        let back = self.pos as f32 + LEN as f32 - delay;
        let i = back as usize;
        let f = back - i as f32;
        let a = self.line[i % LEN];
        let b = self.line[(i + 1) % LEN];
        let y = a + (b - a) * f;
        if (freq - self.gain_freq).abs() > self.gain_freq * 0.001 {
            // Down 60 dB after `decay` seconds, `freq` trips a second.
            self.gain = 0.001f32.powf(1.0 / (self.decay * freq.max(1.0)));
            self.gain_freq = freq;
        }
        let soft = y * (1.0 - self.damp) + self.last * self.damp;
        self.last = y;
        self.line[self.pos] = soft * self.gain;
        self.pos = (self.pos + 1) % LEN;
        y
    }
}
