//! Drum sounds, synthesized at startup so the app needs no sample files.

use std::f32::consts::TAU;

pub const NAMES: [&str; 4] = ["Kick", "Snare", "Hat", "Clap"];
/// General MIDI drum notes for each track, as used on channel 10.
pub const NOTES: [u8; 4] = [36, 38, 42, 39];

/// Which track an incoming MIDI note plays. Accepts the common variants
/// (e.g. 40 for snare, open/pedal hats) so most drum gear just works.
pub fn track_for_note(note: u8) -> Option<usize> {
    match note {
        35 | 36 => Some(0),
        38 | 40 => Some(1),
        42 | 44 | 46 => Some(2),
        39 => Some(3),
        _ => None,
    }
}

/// One sample buffer per track, in the same order as `NAMES`.
pub fn make_kit(sr: f32) -> Vec<Vec<f32>> {
    vec![kick(sr), snare(sr), hat(sr), clap(sr)]
}

/// Tiny xorshift noise source.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn len(sr: f32, seconds: f32) -> usize {
    (sr * seconds) as usize
}

/// Sine with a fast downward pitch sweep.
fn kick(sr: f32) -> Vec<f32> {
    let mut phase = 0.0;
    (0..len(sr, 0.5))
        .map(|i| {
            let t = i as f32 / sr;
            let freq = 50.0 + 110.0 * (-t * 30.0).exp();
            phase += TAU * freq / sr;
            phase.sin() * (-t * 6.0).exp()
        })
        .collect()
}

/// Short tone plus a noise burst.
fn snare(sr: f32) -> Vec<f32> {
    let mut noise = Noise(0x1234_5678);
    (0..len(sr, 0.25))
        .map(|i| {
            let t = i as f32 / sr;
            let tone = (TAU * 185.0 * t).sin() * (-t * 20.0).exp() * 0.5;
            tone + noise.next() * (-t * 15.0).exp() * 0.5
        })
        .collect()
}

/// Very short high-passed noise.
fn hat(sr: f32) -> Vec<f32> {
    let mut noise = Noise(0x9e37_79b9);
    let mut prev = 0.0;
    (0..len(sr, 0.08))
        .map(|i| {
            let t = i as f32 / sr;
            let x = noise.next();
            let y = x - prev; // crude high-pass
            prev = x;
            y * (-t * 60.0).exp() * 0.3
        })
        .collect()
}

/// Three quick noise bursts and a short tail.
fn clap(sr: f32) -> Vec<f32> {
    let mut noise = Noise(0x0bad_cafe);
    let mut prev = 0.0;
    (0..len(sr, 0.3))
        .map(|i| {
            let t = i as f32 / sr;
            let env = if t < 0.03 {
                (-(t % 0.01) * 300.0).exp()
            } else {
                (-(t - 0.03) * 18.0).exp()
            };
            let x = noise.next();
            let y = x - prev * 0.5;
            prev = x;
            y * env * 0.45
        })
        .collect()
}
