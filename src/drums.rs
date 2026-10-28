//! Drum sounds, synthesized at startup so the app needs no sample files.

use std::f32::consts::TAU;

pub const NAMES: [&str; 18] = [
    "Kick",
    "Snare",
    "Hat",
    "Clap",
    "Clave",
    "Bongo hi",
    "Bongo lo",
    "Conga hi",
    "Conga lo",
    "Tom hi",
    "Tom mid",
    "Tom lo",
    "Shaker",
    "Cowbell",
    "Timbale hi",
    "Timbale lo",
    "Cascara",
    "Guiro",
];
/// General MIDI drum notes for each track, as used on channel 10. The
/// shaker uses Maracas and the cascara (a stick on the timbale's shell)
/// Side Stick.
pub const NOTES: [u8; 18] = [
    36, 38, 42, 39, 75, 60, 61, 63, 64, 50, 47, 45, 70, 56, 65, 66, 37, 73,
];
/// How many of the sounds a new song's kit starts with.
pub const STARTER_KIT: usize = 4;

/// Folds common drum-note variants (e.g. 40 for snare, open and pedal hats)
/// onto the notes in `NOTES`, so most drum gear just works.
pub fn canonical_note(note: u8) -> u8 {
    match note {
        35 => 36,
        40 => 38,
        44 | 46 => 42,
        n => n,
    }
}

/// One sample buffer per track, in the same order as `NAMES`.
pub fn make_kit(sr: f32) -> Vec<Vec<f32>> {
    vec![
        kick(sr),
        snare(sr),
        hat(sr),
        clap(sr),
        clave(sr),
        drum(sr, Drum::skin(420.0, 0.06, 26.0), 0x1111_2222),
        drum(sr, Drum::skin(300.0, 0.06, 22.0), 0x3333_4444),
        drum(sr, Drum::skin(250.0, 0.04, 11.0), 0x5555_6666),
        drum(sr, Drum::skin(185.0, 0.04, 9.0), 0x7777_8888),
        drum(sr, Drum::tom(200.0), 0x0102_0304),
        drum(sr, Drum::tom(145.0), 0x0506_0708),
        drum(sr, Drum::tom(105.0), 0x090a_0b0c),
        shaker(sr),
        cowbell(sr),
        timbale(sr, 560.0, 0x2468_ace0),
        timbale(sr, 400.0, 0x1357_9bdf),
        cascara(sr),
        guiro(sr),
    ]
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

/// A tuned drum: a sine that bends down to its pitch, plus a little noise
/// for the stick or hand hitting the skin.
struct Drum {
    freq: f32,
    /// How far above `freq` it starts, as a fraction of it.
    bend: f32,
    /// How fast it fades; higher is shorter.
    decay: f32,
    noise: f32,
    seconds: f32,
}

impl Drum {
    /// Bongos and congas: little bend, a slap of noise.
    fn skin(freq: f32, bend: f32, decay: f32) -> Self {
        Self {
            freq,
            bend,
            decay,
            noise: 0.25,
            seconds: 6.0 / decay,
        }
    }

    /// Toms: a deeper bend and a longer ring.
    fn tom(freq: f32) -> Self {
        Self {
            freq,
            bend: 0.5,
            decay: 7.0,
            noise: 0.15,
            seconds: 0.7,
        }
    }
}

fn drum(sr: f32, d: Drum, seed: u32) -> Vec<f32> {
    let mut noise = Noise(seed);
    let mut phase = 0.0;
    (0..len(sr, d.seconds))
        .map(|i| {
            let t = i as f32 / sr;
            let freq = d.freq * (1.0 + d.bend * (-t * 40.0).exp());
            phase += TAU * freq / sr;
            let tone = phase.sin() * (-t * d.decay).exp();
            tone * 0.7 + noise.next() * d.noise * (-t * 120.0).exp()
        })
        .collect()
}

/// Two hardwood sticks: a bright, very short tone.
fn clave(sr: f32) -> Vec<f32> {
    (0..len(sr, 0.15))
        .map(|i| {
            let t = i as f32 / sr;
            let env = (-t * 45.0).exp();
            ((TAU * 2500.0 * t).sin() + 0.3 * (TAU * 5100.0 * t).sin()) * env * 0.5
        })
        .collect()
}

/// Band-passed noise that swells in quickly, like beads hitting a shell.
fn shaker(sr: f32) -> Vec<f32> {
    let mut noise = Noise(0x5eed_5eed);
    // A state-variable band-pass around 6 kHz.
    let f = 2.0 * (std::f32::consts::PI * 6_000.0 / sr).sin();
    let (mut low, mut band) = (0.0, 0.0);
    (0..len(sr, 0.16))
        .map(|i| {
            let t = i as f32 / sr;
            let env = (t / 0.015).min(1.0) * (-t * 28.0).exp();
            let high = noise.next() - low - 0.6 * band;
            band += f * high;
            low += f * band;
            band * env * 0.6
        })
        .collect()
}

/// Two square waves a dissonant interval apart, like the 808's cowbell.
fn cowbell(sr: f32) -> Vec<f32> {
    let square = |freq: f32, t: f32| if (freq * t).fract() < 0.5 { 1.0 } else { -1.0 };
    let mut prev = 0.0;
    (0..len(sr, 0.4))
        .map(|i| {
            let t = i as f32 / sr;
            let x = (square(540.0, t) + square(800.0, t)) * 0.5;
            // Take the edge off the squares.
            prev += (x - prev) * 0.3;
            prev * (-t * 9.0).exp() * 0.35
        })
        .collect()
}

/// A timbale: a thin metal-shelled drum with a steel-bright ring. A
/// bending tone, two partials above it that don't line up (the shell),
/// and a crack of noise from the stick.
fn timbale(sr: f32, freq: f32, seed: u32) -> Vec<f32> {
    let mut noise = Noise(seed);
    let mut prev = 0.0;
    let mut phases = [0.0_f32; 3];
    let partials = [(1.0, 0.6, 9.0), (1.73, 0.3, 12.0), (2.41, 0.18, 16.0)];
    (0..len(sr, 0.6))
        .map(|i| {
            let t = i as f32 / sr;
            let bend = 1.0 + 0.12 * (-t * 60.0).exp();
            let mut tone = 0.0;
            for (phase, (ratio, gain, decay)) in phases.iter_mut().zip(partials) {
                *phase += TAU * freq * ratio * bend / sr;
                tone += phase.sin() * gain * (-t * decay).exp();
            }
            let x = noise.next();
            let crack = (x - prev) * (-t * 90.0).exp() * 0.15;
            prev = x;
            tone * 0.6 + crack
        })
        .collect()
}

/// A stick on the side of the timbale: a dry, woody click with a short
/// metallic ring.
fn cascara(sr: f32) -> Vec<f32> {
    let mut noise = Noise(0x0c0f_fee5);
    (0..len(sr, 0.12))
        .map(|i| {
            let t = i as f32 / sr;
            let ring =
                ((TAU * 1_850.0 * t).sin() + 0.5 * (TAU * 3_300.0 * t).sin()) * (-t * 55.0).exp();
            let click = noise.next() * (-t * 400.0).exp();
            (ring * 0.45 + click * 0.4) * 0.9
        })
        .collect()
}

/// A stick scraped down a gourd's ridges: quick ticks of band-passed
/// noise, fading in and out.
fn guiro(sr: f32) -> Vec<f32> {
    let mut noise = Noise(0x6017_0bad);
    let f = 2.0 * (std::f32::consts::PI * 3_200.0 / sr).sin();
    let (mut low, mut band) = (0.0, 0.0);
    let seconds = 0.22;
    let ridges = 26.0;
    (0..len(sr, seconds))
        .map(|i| {
            let t = i as f32 / sr;
            // Each ridge is a sharp tick that dies away before the next.
            let tick = (-(t * ridges / seconds).fract() * 9.0).exp();
            let swell = (t / 0.03).min(1.0) * (1.0 - t / seconds).max(0.0).powf(0.7);
            let high = noise.next() - low - 0.5 * band;
            band += f * high;
            low += f * band;
            band * tick * swell * 0.9
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_drum_makes_a_sound_that_fades_without_clipping() {
        let kit = make_kit(48_000.0);
        assert_eq!(kit.len(), NAMES.len());
        assert_eq!(NOTES.len(), NAMES.len());
        for (sound, name) in kit.iter().zip(NAMES) {
            let peak = sound.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.1 && peak <= 1.0, "{name}: peak {peak}");
            let tail = sound[sound.len() - 100..]
                .iter()
                .fold(0.0_f32, |m, s| m.max(s.abs()));
            assert!(tail < 0.05, "{name} fades out: {tail}");
        }
    }
}
