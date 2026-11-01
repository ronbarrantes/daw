//! The Oscillators module: two oscillators, each morphing from sine
//! through triangle, saw and square to noise, and on to a click. The second one can be mixed
//! in, tuned in semitones and detuned slightly for a thicker sound.
//! Unison stacks copies of each oscillator, each a little out of tune with
//! the others, for a big, wide "supersaw" sound; Detune sets how far apart.

use super::play::ANALOG;
use super::{noise, param, Module, ModuleInfo, Unit};

pub const WAVE_1: usize = 0;
pub const WAVE_2: usize = 1;
pub const LEVEL_2: usize = 2;
pub const PITCH_2: usize = 3;
pub const DETUNE: usize = 4;
pub const UNISON: usize = 5;

/// How many copies of each oscillator Unison stacks. Odd, so one stays
/// in tune in the middle.
pub const UNISONS: [&str; 4] = ["Off", "3", "5", "7"];
const MAX_UNISON: usize = 7;
/// How far the outermost Unison copies are from the middle one at full
/// Detune, in cents.
const UNISON_CENTS: f32 = 40.0;

/// The shapes an oscillator morphs through, in order.
pub const WAVES: [&str; 6] = ["Sine", "Tri", "Saw", "Square", "Noise", "Click"];
/// How long the Click wave's tick lasts, in seconds: a click once a cycle,
/// heard as separate clicks at a few Hz and as a buzzy tone above ~20 Hz.
const CLICK_SECONDS: f32 = 0.0005;
/// Where `Unit::Wave` puts a plain saw, the sound before shapes could be picked.
pub const SAW: f32 = 2.0;

pub const INFO: ModuleInfo = ModuleInfo {
    name: "Oscillators",
    knobs: &[
        param("Wave 1", 0.0, 5.0, SAW, false, Unit::Wave),
        param("Wave 2", 0.0, 5.0, SAW, false, Unit::Wave),
        param("Level 2", 0.0, 1.0, 1.0, false, Unit::Percent),
        param("Pitch 2", -24.0, 24.0, 0.0, false, Unit::Semitones),
        param("Detune", 0.0, 1.0, 0.2, false, Unit::Percent),
        param("Unison", 0.0, 3.0, 0.0, false, Unit::Choice(&UNISONS)),
    ],
    rows: &[&[WAVE_1, WAVE_2, LEVEL_2, PITCH_2], &[DETUNE, UNISON]],
    open: true,
    engine: Some(ANALOG),
};

/// One of `WAVES` at `phase` (0-1); `click` is how much of a cycle the
/// Click wave's tick takes.
fn shape(i: usize, phase: f32, click: f32, rng: &mut u32) -> f32 {
    match i {
        0 => (phase * std::f32::consts::TAU).sin(),
        1 => 1.0 - 4.0 * (phase - 0.5).abs(),
        2 => phase * 2.0 - 1.0,
        3 => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        4 => noise(rng),
        // Up then down, so it has no offset.
        _ if phase < click * 0.5 => 1.0,
        _ if phase < click => -1.0,
        _ => 0.0,
    }
}

/// An oscillator `wave` (0-5) of the way along `WAVES`, blending the two
/// shapes either side, at `freq` Hz.
pub(super) fn morph(wave: f32, phase: f32, freq: f32, rng: &mut u32) -> f32 {
    let click = (freq * CLICK_SECONDS).min(0.5);
    let i = (wave.max(0.0) as usize).min(WAVES.len() - 2);
    let f = (wave - i as f32).clamp(0.0, 1.0);
    let a = if f < 1.0 {
        shape(i, phase, click, rng)
    } else {
        0.0
    };
    let b = if f > 0.0 {
        shape(i + 1, phase, click, rng)
    } else {
        0.0
    };
    a + (b - a) * f
}

/// One voice's oscillators.
pub struct Osc {
    /// Each oscillator's copies' phases; only the first `unison` move.
    phase: [[f32; MAX_UNISON]; 2],
    rng: u32,
    waves: [f32; 2],
    level_2: f32,
    /// The second oscillator's pitch relative to the first: Detune and
    /// Pitch 2 together.
    ratio_2: f32,
    /// How many copies of each oscillator play (1 = Unison off).
    unison: usize,
    /// Each copy's pitch relative to the oscillator's.
    spread: [f32; MAX_UNISON],
    /// Keeps the stack about as loud as one oscillator.
    unison_gain: f32,
}

impl Default for Osc {
    fn default() -> Self {
        // Unison copies start out of step, so they don't all line up at
        // first and sweep like a flanger. The first copy starts at 0, as
        // the oscillators always have.
        let mut phase = [[0.0; MAX_UNISON]; 2];
        for row in &mut phase {
            for (i, p) in row.iter_mut().enumerate() {
                *p = (i as f32 * 0.618_034).fract();
            }
        }
        Self {
            phase,
            rng: 0x1357_9bdf,
            waves: [SAW; 2],
            level_2: 1.0,
            ratio_2: 1.0,
            unison: 1,
            spread: [1.0; MAX_UNISON],
            unison_gain: 1.0,
        }
    }
}

impl Module for Osc {
    fn set_knobs(&mut self, k: &[f32], _sr: f32) {
        self.waves = [k[WAVE_1], k[WAVE_2]];
        self.level_2 = k[LEVEL_2];
        let semis = k[PITCH_2].round();
        self.ratio_2 = (1.0 + k[DETUNE] * 0.03) * 2f32.powf(semis / 12.0);
        let unison = super::choice(k[UNISON], UNISONS.len()) * 2 + 1;
        self.unison = unison;
        self.unison_gain = 1.0 / (unison as f32).sqrt();
        if unison > 1 {
            // -1 (the lowest copy) to 1 (the highest).
            let half = (unison / 2) as f32;
            for (i, r) in self.spread.iter_mut().enumerate().take(unison) {
                let at = i as f32 / half - 1.0;
                *r = 2f32.powf(at * k[DETUNE] * UNISON_CENTS / 1200.0);
            }
        }
    }
}

impl Osc {
    /// One oscillator: `wave` at `freq` Hz, with its Unison copies.
    fn stack(&mut self, o: usize, freq: f32, sr: f32) -> f32 {
        let wave = self.waves[o];
        if self.unison == 1 {
            let p = &mut self.phase[o][0];
            *p = (*p + freq / sr).fract();
            return morph(wave, *p, freq, &mut self.rng);
        }
        let mut out = 0.0;
        for i in 0..self.unison {
            let f = freq * self.spread[i];
            let p = &mut self.phase[o][i];
            *p = (*p + f / sr).fract();
            out += morph(wave, *p, f, &mut self.rng);
        }
        out * self.unison_gain
    }

    /// The next sample at `freq` Hz.
    pub fn next(&mut self, freq: f32, sr: f32) -> f32 {
        let mut osc = self.stack(0, freq, sr);
        if self.level_2 > 0.0 {
            osc += self.stack(1, freq * self.ratio_2, sr) * self.level_2;
        }
        osc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn knobs(detune: f32, unison: f32) -> Vec<f32> {
        let mut k: Vec<f32> = INFO.knobs.iter().map(|p| p.default).collect();
        k[LEVEL_2] = 0.0;
        k[DETUNE] = detune;
        k[UNISON] = unison;
        k
    }

    fn run(k: &[f32]) -> Vec<f32> {
        let mut o = Osc::default();
        o.set_knobs(k, 48_000.0);
        (0..48_000).map(|_| o.next(220.0, 48_000.0)).collect()
    }

    #[test]
    fn unison_stacks_detuned_copies() {
        let one = run(&knobs(0.6, 0.0));
        let seven = run(&knobs(0.6, 3.0));
        assert_ne!(one, seven, "Unison changes the sound");
        // About as loud either way.
        let rms = |v: &[f32]| (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt();
        let (a, b) = (rms(&one), rms(&seven));
        assert!(b > a * 0.5 && b < a * 1.5, "one {a}, seven {b}");
        // Copies spread evenly around the note, the middle one in tune.
        let mut o = Osc::default();
        o.set_knobs(&knobs(1.0, 2.0), 48_000.0);
        assert_eq!(o.unison, 5);
        assert!((o.spread[2] - 1.0).abs() < 1e-6);
        assert!((o.spread[0] * o.spread[4] - 1.0).abs() < 1e-4);
        let cents = 1200.0 * o.spread[4].log2();
        assert!((cents - UNISON_CENTS).abs() < 0.1, "{cents}");
    }
}
