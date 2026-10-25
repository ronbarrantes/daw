//! Shared effects: one reverb and one delay that every track can send to.
//!
//! Both allocate their buffers up front, so the audio thread never does.
//! The reverb is a small Freeverb (eight combs and four all-passes per
//! side); the delay is a tempo-synced ping-pong.

/// Freeverb's comb and all-pass lengths, in samples at 44.1 kHz.
const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
/// The right side's delays are this much longer, for width.
const SPREAD: usize = 23;
/// Keeps the combs from clipping.
const REVERB_INPUT: f32 = 0.015;
const REVERB_OUTPUT: f32 = 3.0;
/// The longest delay time the buffers hold, in seconds: 8 steps at 40 BPM.
pub const MAX_DELAY_SECONDS: f32 = 3.0;

struct Comb {
    buf: Vec<f32>,
    i: usize,
    filtered: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            i: 0,
            filtered: 0.0,
        }
    }

    fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.i];
        self.filtered = out * (1.0 - damp) + self.filtered * damp;
        self.buf[self.i] = input + self.filtered * feedback;
        self.i = (self.i + 1) % self.buf.len();
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    i: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            i: 0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let delayed = self.buf[self.i];
        self.buf[self.i] = input + delayed * 0.5;
        self.i = (self.i + 1) % self.buf.len();
        delayed - input
    }
}

/// One side of the reverb.
struct Tank {
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
}

impl Tank {
    fn new(sr: f32, extra: usize) -> Self {
        let scale = |n: usize| ((n + extra) as f32 * sr / 44_100.0) as usize;
        Self {
            combs: COMBS.iter().map(|&n| Comb::new(scale(n))).collect(),
            allpasses: ALLPASSES.iter().map(|&n| Allpass::new(scale(n))).collect(),
        }
    }

    fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let mut out = 0.0;
        for comb in &mut self.combs {
            out += comb.process(input, feedback, damp);
        }
        for allpass in &mut self.allpasses {
            out = allpass.process(out);
        }
        out
    }
}

pub struct Reverb {
    left: Tank,
    right: Tank,
}

impl Reverb {
    pub fn new(sr: f32) -> Self {
        Self {
            left: Tank::new(sr, 0),
            right: Tank::new(sr, SPREAD),
        }
    }

    /// `size` and `tone` are 0-1: bigger rings longer, brighter is less
    /// muffled.
    pub fn process(&mut self, input: f32, size: f32, tone: f32) -> [f32; 2] {
        let feedback = 0.7 + size.clamp(0.0, 1.0) * 0.28;
        let damp = (1.0 - tone.clamp(0.0, 1.0)) * 0.4;
        let input = input * REVERB_INPUT;
        [
            self.left.process(input, feedback, damp) * REVERB_OUTPUT,
            self.right.process(input, feedback, damp) * REVERB_OUTPUT,
        ]
    }
}

/// Echoes that bounce between left and right.
pub struct Delay {
    left: Vec<f32>,
    right: Vec<f32>,
    i: usize,
}

impl Delay {
    pub fn new(sr: f32) -> Self {
        let len = (MAX_DELAY_SECONDS * sr) as usize + 1;
        Self {
            left: vec![0.0; len],
            right: vec![0.0; len],
            i: 0,
        }
    }

    /// `time` is in samples; `feedback` 0-1 is how much each echo repeats.
    pub fn process(&mut self, input: f32, time: usize, feedback: f32) -> [f32; 2] {
        let len = self.left.len();
        let time = time.clamp(1, len - 1);
        let read = (self.i + len - time) % len;
        let out = [self.left[read], self.right[read]];
        // The first echo is on the left, then each one crosses over.
        self.left[self.i] = input + out[1] * feedback;
        self.right[self.i] = out[0] * feedback;
        self.i = (self.i + 1) % len;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_echoes_ping_pong() {
        let mut d = Delay::new(1_000.0);
        let out: Vec<[f32; 2]> = (0..40)
            .map(|t| d.process(if t == 0 { 1.0 } else { 0.0 }, 10, 0.5))
            .collect();
        assert_eq!(out[10], [1.0, 0.0], "first echo on the left");
        assert_eq!(out[20], [0.0, 0.5], "then the right, quieter");
        assert_eq!(out[30], [0.25, 0.0], "then back");
        assert!(out[11..20].iter().all(|f| *f == [0.0, 0.0]));
    }

    #[test]
    fn reverb_rings_on_and_fades() {
        let sr = 48_000.0;
        let mut r = Reverb::new(sr);
        let out: Vec<[f32; 2]> = (0..sr as usize * 6)
            .map(|t| r.process(if t < 100 { 1.0 } else { 0.0 }, 0.5, 0.5))
            .collect();
        let energy = |range: std::ops::Range<usize>| -> f32 {
            out[range].iter().map(|[l, r]| l.abs() + r.abs()).sum()
        };
        let early = energy(2_000..12_000);
        let late = energy(250_000..260_000);
        assert!(early > 1.0, "rings: {early}");
        assert!(late < early / 100.0, "fades: {early} -> {late}");
        assert!(out.iter().all(|[l, r]| l.is_finite() && r.is_finite()));
    }
}
