//! The Pluck engine: a plucked or struck string (Karplus-Strong). A burst
//! goes round a short delay line, one trip per cycle of the note, and is
//! softened a little each trip, so it rings like a guitar, harp or koto
//! string and dies away by itself.
//!
//! For pianos and their kin it can also: strike with a soft felt hammer
//! instead of a pick, sound two or three strings a few cents apart (their
//! slow beating is much of a piano's sound), stretch the overtones sharp
//! like a stiff string, and let high notes die away sooner than low ones.
//! All of those start off, so the plain pluck sounds as it always has.

use super::play::PLUCK;
use super::{noise, param, Module, ModuleInfo, Unit};

pub const TONE: usize = 0;
pub const DECAY: usize = 1;
pub const POSITION: usize = 2;
pub const HAMMER: usize = 3;
pub const STRINGS: usize = 4;
pub const DETUNE: usize = 5;
pub const STRETCH: usize = 6;
pub const KEY_DECAY: usize = 7;

pub const STRING_COUNTS: [&str; 3] = ["One", "Two", "Three"];

pub const INFO: ModuleInfo = ModuleInfo {
    name: "Pluck",
    knobs: &[
        param("String tone", 0.0, 1.0, 0.6, false, Unit::Percent),
        param("String decay", 0.1, 10.0, 2.0, true, Unit::Seconds),
        param("Pick spot", 0.0, 0.5, 0.15, false, Unit::Percent),
        param("Hammer", 0.0, 1.0, 0.0, false, Unit::Percent),
        param(
            "Strings",
            0.0,
            2.0,
            0.0,
            false,
            Unit::Choice(&STRING_COUNTS),
        ),
        param("String detune", 0.0, 1.0, 0.3, false, Unit::Percent),
        param("Stretch", 0.0, 1.0, 0.0, false, Unit::Percent),
        param("Key decay", 0.0, 1.0, 0.0, false, Unit::Percent),
    ],
    rows: &[
        &[TONE, DECAY, POSITION, HAMMER],
        &[STRINGS, DETUNE, STRETCH, KEY_DECAY],
    ],
    open: true,
    engine: Some(PLUCK),
};

/// The longest string, in samples: below about 23 Hz at 96 kHz.
const LEN: usize = 4096;
/// The extra strings are shorter: the lowest notes get just one string,
/// as on a piano.
const EXTRA_LEN: usize = 2048;
/// How far apart the outer strings are at full String detune, in cents.
const MAX_DETUNE_CENTS: f32 = 6.0;
/// Allpass stages that bend the overtones sharp (Stretch).
const STAGES: usize = 3;

/// One string: a delay line and what it carries round each trip.
#[derive(Clone, Copy)]
struct Line<const N: usize> {
    data: [f32; N],
    pos: usize,
    /// The last sample read, for the softening filter.
    last: f32,
    /// The allpass stages' memory (input, output).
    ap: [(f32, f32); STAGES],
}

impl<const N: usize> Line<N> {
    const fn new() -> Self {
        Self {
            data: [0.0; N],
            pos: 0,
            last: 0.0,
            ap: [(0.0, 0.0); STAGES],
        }
    }

    /// Adds `burst` (one period long) to the line, ending where it reads
    /// next.
    fn excite(&mut self, burst: &[f32], scale: f32) {
        let period = burst.len().min(N - 2);
        for (i, b) in burst.iter().take(period).enumerate() {
            let at = (self.pos + N - period + i) % N;
            self.data[at] += b * scale;
        }
    }

    /// The next sample of a string `delay` samples round, with the
    /// softening `damp`, the loss `gain` and the stretch allpass `a`.
    fn next(&mut self, delay: f32, damp: f32, gain: f32, a: f32) -> f32 {
        let back = self.pos as f32 + N as f32 - delay.clamp(1.0, (N - 2) as f32);
        let i = back as usize;
        let f = back - i as f32;
        let x = self.data[i % N];
        let y = x + (self.data[(i + 1) % N] - x) * f;
        let mut soft = y * (1.0 - damp) + self.last * damp;
        self.last = y;
        if a != 0.0 {
            for (x1, y1) in &mut self.ap {
                let out = a * soft + *x1 - a * *y1;
                *x1 = soft;
                *y1 = out;
                soft = out;
            }
        }
        self.data[self.pos] = soft * gain;
        self.pos = (self.pos + 1) % N;
        y
    }
}

/// One voice's strings.
pub struct Pluck {
    /// Inline, not boxed: voices are made on the audio thread, which
    /// mustn't allocate.
    main: Line<LEN>,
    extra: [Line<EXTRA_LEN>; 2],
    rng: u32,
    /// How much each trip softens the sound (0 = bright, 0.5 = soft).
    damp: f32,
    /// The noise burst's brightness, as a one-pole low-pass amount.
    burst: f32,
    position: f32,
    decay: f32,
    hammer: f32,
    strings: usize,
    detune: f32,
    /// The stretch allpass coefficient (0 = off, toward -1 = more).
    stretch: f32,
    key_decay: f32,
    /// What's worked out for `tuned_freq`: each string's trip length and
    /// loss per trip. Worked out again when the pitch moves.
    delays: [f32; 3],
    gains: [f32; 3],
    /// The stretch allpass coefficient for this note.
    bend: f32,
    tuned_freq: f32,
}

impl Default for Pluck {
    fn default() -> Self {
        Self {
            main: Line::new(),
            extra: [Line::new(); 2],
            rng: 0x6d2b_79f5,
            damp: 0.3,
            burst: 0.6,
            position: 0.15,
            decay: 2.0,
            hammer: 0.0,
            strings: 1,
            detune: 0.3,
            stretch: 0.0,
            key_decay: 0.0,
            delays: [0.0; 3],
            gains: [0.0; 3],
            bend: 0.0,
            tuned_freq: 0.0,
        }
    }
}

impl Module for Pluck {
    fn set_knobs(&mut self, k: &[f32], _sr: f32) {
        self.damp = 0.05 + 0.45 * (1.0 - k[TONE]);
        self.burst = 0.1 + 0.9 * k[TONE];
        self.position = k[POSITION];
        self.hammer = k[HAMMER];
        let strings = super::choice(k[STRINGS], STRING_COUNTS.len()) + 1;
        let stretch = -0.7 * k[STRETCH];
        if self.decay != k[DECAY]
            || self.strings != strings
            || self.detune != k[DETUNE]
            || self.stretch != stretch
            || self.key_decay != k[KEY_DECAY]
        {
            self.decay = k[DECAY];
            self.strings = strings;
            self.detune = k[DETUNE];
            self.stretch = stretch;
            self.key_decay = k[KEY_DECAY];
            self.tuned_freq = 0.0;
        }
    }
}

/// The phase delay, in samples, of the first-order allpass `a` at `w`
/// radians per sample.
fn allpass_delay(a: f32, w: f32) -> f32 {
    let (s, c) = w.sin_cos();
    let num = (-s).atan2(a + c);
    let den = (-a * s).atan2(1.0 + a * c);
    -(num - den) / w
}

impl Pluck {
    fn period(freq: f32, sr: f32) -> f32 {
        (sr / freq.max(1.0)).clamp(2.0, (LEN - 2) as f32)
    }

    /// How many strings sound at `freq`: the extra ones only fit notes
    /// whose period is short enough.
    fn sounding(&self, freq: f32, sr: f32) -> usize {
        if Self::period(freq, sr) < (EXTRA_LEN - 4) as f32 {
            self.strings
        } else {
            1
        }
    }

    /// Each string's pitch, as a ratio of `freq`: spread evenly across
    /// String detune.
    fn spread(&self, n: usize) -> [f32; 3] {
        let cents = self.detune * MAX_DETUNE_CENTS;
        let ratio = |c: f32| 2f32.powf(c / 1200.0);
        match n {
            1 => [1.0; 3],
            2 => [ratio(-cents / 2.0), ratio(cents / 2.0), 1.0],
            _ => [1.0, ratio(-cents), ratio(cents)],
        }
    }

    /// Works out each string's trip length and loss for `freq`.
    fn tune(&mut self, freq: f32, sr: f32) {
        let n = self.sounding(freq, sr);
        let spread = self.spread(n);
        // Higher notes die away sooner with Key decay: half as long per
        // octave above middle C at full, longer below.
        let octaves = (freq.max(1.0) / 261.6).log2();
        // Real strings are stiffest, for their length, at the top: the
        // bass barely stretches (and a strong stretch there smears each
        // strike into a zap, like a laser).
        let reach = ((octaves + 2.0) / 4.0).clamp(0.05, 1.0);
        self.bend = self.stretch * reach * reach;
        let decay = (self.decay * 2f32.powf(-octaves * self.key_decay)).clamp(0.05, 30.0);
        for (s, ratio) in spread.iter().enumerate().take(n) {
            let f = freq * ratio;
            let w = std::f32::consts::TAU * f / sr;
            // The softening filter and the stretch allpasses each delay
            // the sound a little; take that off so the string stays in
            // tune.
            let mut taken = self.damp;
            if self.bend != 0.0 {
                taken += STAGES as f32 * allpass_delay(self.bend, w.min(3.0));
            }
            self.delays[s] = (Self::period(f, sr) - taken).max(1.0);
            // Down 60 dB after `decay` seconds, `f` trips a second.
            self.gains[s] = 0.001f32.powf(1.0 / (decay * f.max(1.0)));
        }
        self.tuned_freq = freq;
    }

    /// Plucks or strikes the strings: fills one cycle's worth of each
    /// line with a burst, softened by Tone, rounded by Hammer and shaped
    /// by where it's struck.
    pub fn trigger(&mut self, freq: f32, sr: f32) {
        let period = Self::period(freq, sr) as usize;
        let mut lp = 0.0;
        let mut burst = [0.0_f32; LEN];
        // A felt hammer is a soft, rounded push rather than a scratch:
        // a smooth bump, narrower and brighter the harder the hammer.
        let width = ((0.45 - 0.3 * (1.0 - self.hammer)) * period as f32).max(2.0);
        for (i, b) in burst.iter_mut().take(period).enumerate() {
            lp += (noise(&mut self.rng) - lp) * self.burst;
            let t = i as f32 / width;
            let bump = if t < 1.0 {
                (std::f32::consts::PI * t).sin() * 1.6
            } else {
                0.0
            };
            *b = lp * (1.0 - self.hammer) + (bump + lp * 0.15) * self.hammer;
        }
        // Picking or striking away from the end cancels some harmonics,
        // like a real string: the burst minus itself a little later.
        let gap = (self.position * period as f32) as usize;
        let mut shaped = [0.0_f32; LEN];
        for i in 0..period {
            let back = if gap > 0 && i >= gap {
                burst[i - gap]
            } else {
                0.0
            };
            shaped[i] = burst[i] - back;
        }
        let n = self.sounding(freq, sr);
        let scale = 1.0 / (n as f32).sqrt();
        self.main.excite(&shaped[..period], scale);
        for line in self.extra.iter_mut().take(n - 1) {
            line.excite(&shaped[..period], scale);
        }
    }

    /// The next sample at `freq` Hz.
    pub fn next(&mut self, freq: f32, sr: f32) -> f32 {
        if (freq - self.tuned_freq).abs() > self.tuned_freq * 0.001 {
            self.tune(freq, sr);
        }
        let n = self.sounding(freq, sr);
        let (damp, a) = (self.damp, self.bend);
        let mut out = self.main.next(self.delays[0], damp, self.gains[0], a);
        for (s, line) in self.extra.iter_mut().take(n - 1).enumerate() {
            out += line.next(self.delays[s + 1], damp, self.gains[s + 1], a);
        }
        out / (n as f32).sqrt()
    }
}
