//! Ready-made sounds, in groups. Each changes some knobs from their
//! defaults and can bring modulation with it. Most respond to how hard
//! notes are played (Velocity), for keyboards that send it.

use super::*;

/// A ready-made sound.
pub struct Preset {
    pub group: &'static str,
    pub name: &'static str,
    /// Knobs changed from their defaults.
    pub knobs: &'static [(usize, f32)],
    /// Modulation that comes with it: source, knob, amount.
    pub mods: &'static [(Source, usize, f32)],
}

/// The groups, in the order the preset list shows them.
pub const PRESET_GROUPS: [&str; 7] = [
    "Keys",
    "Plucks",
    "Mallets & bells",
    "Bass",
    "Strings & pads",
    "Leads",
    "Effects",
];

const fn p(
    group: &'static str,
    name: &'static str,
    knobs: &'static [(usize, f32)],
    mods: &'static [(Source, usize, f32)],
) -> Preset {
    Preset {
        group,
        name,
        knobs,
        mods,
    }
}

// The Pluck, Mallet and FM engines shape each note themselves, so their
// presets open the filter up and let the envelope just hold.
const OPEN: [(usize, f32); 3] = [(CUTOFF, 8000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.0)];

/// Joins knob lists at compile time (for sharing `OPEN`).
macro_rules! knobs {
    ($($k:expr),* $(,)?) => {{
        const PARTS: &[&[(usize, f32)]] = &[$(&$k),*];
        const N: usize = {
            let mut n = 0;
            let mut i = 0;
            while i < PARTS.len() {
                n += PARTS[i].len();
                i += 1;
            }
            n
        };
        const ALL: [(usize, f32); N] = {
            let mut all = [(0, 0.0); N];
            let (mut n, mut i) = (0, 0);
            while i < PARTS.len() {
                let mut j = 0;
                while j < PARTS[i].len() {
                    all[n] = PARTS[i][j];
                    n += 1;
                    j += 1;
                }
                i += 1;
            }
            all
        };
        &ALL
    }};
}

use Source::{Lfo2, ModEnv, Velocity};

#[rustfmt::skip]
pub const PRESETS: [Preset; 34] = [
    // Keys
    // A real-ish grand: three strings a few cents apart struck by a felt
    // hammer, overtones stretched a little sharp like stiff steel, high
    // notes dying away sooner than low ones, and a quick drop then a long
    // quiet ring (the envelope's Decay to Sustain). Harder notes hit with
    // a harder, brighter hammer.
    p("Keys", "Grand piano", knobs!(OPEN, [(ENGINE, 1.0), (STRING_TONE, 0.55),
        (STRING_DECAY, 9.0), (PICK_SPOT, 0.12), (HAMMER, 0.9), (STRING_COUNT, 2.0),
        (STRING_DETUNE, 0.12), (STRETCH, 0.3), (KEY_DECAY, 0.7), (CUTOFF, 3500.0),
        (ENV_AMOUNT, 0.35), (FILTER_DECAY, 1.2), (ATTACK, 0.001), (DECAY, 1.0),
        (SUSTAIN, 0.45), (RELEASE, 0.3)]),
        &[(Velocity, HAMMER, -0.25), (Velocity, STRING_TONE, 0.25), (Velocity, CUTOFF, 0.2)]),
    // Quills pluck the strings: bright and wiry, two strings together,
    // the same however hard it's played, and cut short when let go.
    p("Keys", "Harpsichord", knobs!(OPEN, [(ENGINE, 1.0), (STRING_TONE, 0.9),
        (STRING_DECAY, 4.0), (PICK_SPOT, 0.08), (STRING_COUNT, 1.0), (STRING_DETUNE, 0.15),
        (KEY_DECAY, 0.5), (ATTACK, 0.001), (SUSTAIN, 1.0), (RELEASE, 0.12)]), &[]),
    p("Keys", "Piano (FM)", knobs!(OPEN, [(ENGINE, 3.0), (FM_RATIO, 1.0), (FM_DEPTH, 0.28),
        (FM_DECAY, 0.5), (FM_FEEDBACK, 0.08), (CUTOFF, 5000.0), (ATTACK, 0.001), (DECAY, 2.0),
        (SUSTAIN, 0.0), (RELEASE, 0.35)]),
        &[(Velocity, FM_DEPTH, 0.3), (Velocity, CUTOFF, 0.1)]),
    p("Keys", "Electric piano", knobs!(OPEN, [(ENGINE, 3.0), (FM_RATIO, 1.0), (FM_DEPTH, 0.3),
        (FM_DECAY, 0.8), (CUTOFF, 6000.0), (ATTACK, 0.002), (DECAY, 1.5), (SUSTAIN, 0.3),
        (RELEASE, 0.4), (LFO_TARGET, 2.0), (LFO_AMOUNT, 0.12), (LFO_RATE, 4.5)]),
        &[(Velocity, FM_DEPTH, 0.35)]),
    p("Keys", "Glassy keys", knobs!(OPEN, [(ENGINE, 3.0), (FM_RATIO, 2.0), (FM_DEPTH, 0.35),
        (FM_DECAY, 1.2), (ATTACK, 0.002), (DECAY, 1.2), (SUSTAIN, 0.4), (RELEASE, 0.6)]),
        &[(Velocity, FM_DEPTH, 0.25)]),
    p("Keys", "Organ", &[(WAVE, 0.0), (WAVE_2, 0.0), (LEVEL_2, 0.7), (PITCH_2, 12.0),
        (CUTOFF, 5000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.0), (ATTACK, 0.005),
        (DECAY, 0.1), (SUSTAIN, 1.0), (RELEASE, 0.05),
        (LFO_TARGET, 2.0), (LFO_AMOUNT, 0.1), (LFO_RATE, 6.0)], &[]),
    p("Keys", "Analog keys", &[(WAVE, 2.6), (WAVE_2, 1.0), (LEVEL_2, 0.5), (PITCH_2, 12.0),
        (DETUNE, 0.1), (CUTOFF, 1800.0), (RESONANCE, 0.1), (ENV_AMOUNT, 0.5),
        (FILTER_DECAY, 0.4), (ATTACK, 0.002), (DECAY, 0.9), (SUSTAIN, 0.0), (RELEASE, 0.4)],
        &[(Velocity, CUTOFF, 0.2)]),
    // Plucks
    p("Plucks", "Nylon guitar", knobs!(OPEN, [(ENGINE, 1.0), (STRING_TONE, 0.45),
        (STRING_DECAY, 2.5), (PICK_SPOT, 0.2), (CUTOFF, 6000.0), (ATTACK, 0.001),
        (SUSTAIN, 1.0), (RELEASE, 0.3)]),
        &[(Velocity, STRING_TONE, 0.3)]),
    p("Plucks", "Steel guitar", knobs!(OPEN, [(ENGINE, 1.0), (STRING_TONE, 0.8),
        (STRING_DECAY, 3.5), (PICK_SPOT, 0.08), (ATTACK, 0.001), (SUSTAIN, 1.0),
        (RELEASE, 0.4)]),
        &[(Velocity, STRING_TONE, 0.2)]),
    p("Plucks", "Harp", knobs!(OPEN, [(ENGINE, 1.0), (STRING_TONE, 0.7), (STRING_DECAY, 4.0),
        (PICK_SPOT, 0.1), (ATTACK, 0.001), (SUSTAIN, 1.0), (RELEASE, 1.0)]),
        &[(Velocity, STRING_TONE, 0.25)]),
    p("Plucks", "Koto", knobs!(OPEN, [(ENGINE, 1.0), (STRING_TONE, 0.85), (STRING_DECAY, 1.5),
        (PICK_SPOT, 0.05), (ATTACK, 0.001), (SUSTAIN, 1.0), (RELEASE, 0.4)]),
        &[(Velocity, STRING_TONE, 0.15)]),
    // Mallets & bells
    p("Mallets & bells", "Marimba", knobs!(OPEN, [(ENGINE, 2.0), (MATERIAL, 0.0),
        (HARDNESS, 0.4), (RING, 0.8), (ATTACK, 0.001), (SUSTAIN, 1.0), (RELEASE, 0.5)]),
        &[(Velocity, HARDNESS, 0.4)]),
    p("Mallets & bells", "Vibes", knobs!(OPEN, [(ENGINE, 2.0), (MATERIAL, 1.0),
        (HARDNESS, 0.5), (RING, 3.0), (ATTACK, 0.001), (SUSTAIN, 1.0), (RELEASE, 1.0),
        (LFO_TARGET, 2.0), (LFO_AMOUNT, 0.25), (LFO_RATE, 5.0)]),
        &[(Velocity, HARDNESS, 0.35)]),
    p("Mallets & bells", "Glass", knobs!(OPEN, [(ENGINE, 2.0), (MATERIAL, 2.0),
        (HARDNESS, 0.7), (RING, 2.5), (ATTACK, 0.001), (SUSTAIN, 1.0), (RELEASE, 1.0)]),
        &[(Velocity, HARDNESS, 0.3)]),
    p("Mallets & bells", "Church bell", knobs!(OPEN, [(ENGINE, 2.0), (MATERIAL, 3.0),
        (HARDNESS, 0.8), (RING, 6.0), (ATTACK, 0.001), (SUSTAIN, 1.0), (RELEASE, 2.0)]),
        &[(Velocity, HARDNESS, 0.2)]),
    p("Mallets & bells", "FM bell", knobs!(OPEN, [(ENGINE, 3.0), (FM_RATIO, 3.5),
        (FM_DEPTH, 0.5), (FM_DECAY, 2.5), (ATTACK, 0.001), (DECAY, 2.0), (SUSTAIN, 0.0),
        (RELEASE, 2.0)]),
        &[(Velocity, FM_DEPTH, 0.3)]),
    // Bass
    p("Bass", "Bass", &[(WAVE, 2.0), (WAVE_2, 3.0), (LEVEL_2, 0.7), (PITCH_2, -12.0),
        (CUTOFF, 300.0), (RESONANCE, 0.3), (ENV_AMOUNT, 0.5), (FILTER_DECAY, 0.15),
        (ATTACK, 0.002), (DECAY, 0.3), (SUSTAIN, 0.6), (RELEASE, 0.1), (VOICES, 1.0)],
        &[(Velocity, CUTOFF, 0.15)]),
    p("Bass", "Sub bass", &[(WAVE, 0.0), (LEVEL_2, 0.0), (CUTOFF, 400.0), (RESONANCE, 0.0),
        (ENV_AMOUNT, 0.0), (ATTACK, 0.005), (DECAY, 0.3), (SUSTAIN, 1.0), (RELEASE, 0.1),
        (VOICES, 1.0)], &[]),
    p("Bass", "Acid bass", &[(WAVE, 2.0), (LEVEL_2, 0.0), (CUTOFF, 180.0), (RESONANCE, 0.85),
        (ENV_AMOUNT, 0.8), (FILTER_DECAY, 0.18), (ATTACK, 0.002), (DECAY, 0.2),
        (SUSTAIN, 0.5), (RELEASE, 0.08), (VOICES, 2.0), (GLIDE, 0.08)],
        &[(Velocity, ENV_AMOUNT, 0.2)]),
    p("Bass", "FM bass", knobs!(OPEN, [(ENGINE, 3.0), (FM_RATIO, 1.0), (FM_DEPTH, 0.55),
        (FM_DECAY, 0.15), (FM_FEEDBACK, 0.3), (CUTOFF, 2500.0), (ATTACK, 0.002),
        (DECAY, 0.4), (SUSTAIN, 0.6), (RELEASE, 0.1), (VOICES, 1.0)]),
        &[(Velocity, FM_DEPTH, 0.2)]),
    p("Bass", "Pluck bass", knobs!(OPEN, [(ENGINE, 1.0), (STRING_TONE, 0.3),
        (STRING_DECAY, 1.2), (PICK_SPOT, 0.25), (CUTOFF, 2000.0), (ATTACK, 0.001),
        (SUSTAIN, 1.0), (RELEASE, 0.15), (VOICES, 1.0)]),
        &[(Velocity, STRING_TONE, 0.25)]),
    p("Bass", "Wavetable bass", &[(ENGINE, 4.0), (WT_TABLE, 2.0), (WT_POSITION, 0.6),
        (WT_SUB, 0.6), (CUTOFF, 1200.0), (RESONANCE, 0.2), (ENV_AMOUNT, 0.4),
        (FILTER_DECAY, 0.2), (ATTACK, 0.002), (DECAY, 0.3), (SUSTAIN, 0.7), (RELEASE, 0.1),
        (VOICES, 1.0)],
        &[(ModEnv, WT_POSITION, -0.4)]),
    // Strings & pads: slow movement from LFO 2 keeps them alive.
    p("Strings & pads", "Strings", &[(WAVE, 2.0), (WAVE_2, 2.0), (DETUNE, 0.8),
        (CUTOFF, 1500.0), (RESONANCE, 0.1), (ENV_AMOUNT, 0.1), (ATTACK, 0.35), (DECAY, 0.5),
        (SUSTAIN, 0.85), (RELEASE, 0.7), (LFO_AMOUNT, 0.02), (LFO_RATE, 5.0),
        (LFO2_RATE, 0.2)],
        &[(Lfo2, CUTOFF, 0.06), (Velocity, CUTOFF, 0.1)]),
    p("Strings & pads", "Warm pad", &[(WAVE, 2.5), (WAVE_2, 2.5), (DETUNE, 0.7),
        (CUTOFF, 700.0), (RESONANCE, 0.2), (ENV_AMOUNT, 0.15), (FILTER_DECAY, 1.5),
        (ATTACK, 0.8), (DECAY, 1.0), (SUSTAIN, 0.8), (RELEASE, 1.5), (LFO2_RATE, 0.15)],
        &[(Lfo2, CUTOFF, 0.12), (Lfo2, DETUNE, 0.1)]),
    p("Strings & pads", "Morphing pad", &[(ENGINE, 4.0), (WT_TABLE, 0.0), (WT_POSITION, 0.4),
        (CUTOFF, 3000.0), (RESONANCE, 0.1), (ENV_AMOUNT, 0.0), (ATTACK, 0.6), (DECAY, 1.0),
        (SUSTAIN, 0.85), (RELEASE, 1.2), (LFO2_RATE, 0.25)],
        &[(Lfo2, WT_POSITION, 0.4)]),
    // Leads
    p("Leads", "Brass", &[(WAVE, 2.0), (WAVE_2, 2.0), (DETUNE, 0.4), (CUTOFF, 400.0),
        (RESONANCE, 0.2), (ENV_AMOUNT, 0.0), (ATTACK, 0.05), (DECAY, 0.3), (SUSTAIN, 0.8),
        (RELEASE, 0.2), (MOD_ATTACK, 0.08), (MOD_DECAY, 0.4), (MOD_SUSTAIN, 0.5)],
        // The swell: the mod envelope opens the filter at each note.
        &[(ModEnv, CUTOFF, 0.35), (Velocity, CUTOFF, 0.15)]),
    p("Leads", "Violin lead", &[(WAVE, 2.0), (WAVE_2, 2.0), (DETUNE, 0.3), (CUTOFF, 2200.0),
        (RESONANCE, 0.15), (ENV_AMOUNT, 0.2), (ATTACK, 0.12), (SUSTAIN, 0.9),
        (RELEASE, 0.3), (VOICES, 2.0), (GLIDE, 0.05), (LFO_AMOUNT, 0.03), (LFO_RATE, 5.5)],
        &[(Velocity, CUTOFF, 0.15)]),
    p("Leads", "Flute", &[(WAVE, 0.3), (WAVE_2, 4.0), (LEVEL_2, 0.08), (CUTOFF, 2500.0),
        (RESONANCE, 0.0), (ENV_AMOUNT, 0.1), (ATTACK, 0.08), (SUSTAIN, 0.9),
        (RELEASE, 0.2), (LFO_AMOUNT, 0.02), (LFO_RATE, 5.0), (MOD_ATTACK, 0.05),
        (MOD_DECAY, 0.15)],
        // A puff of breath at the start of each note.
        &[(ModEnv, LEVEL_2, 0.2)]),
    p("Leads", "Talking lead", &[(ENGINE, 4.0), (WT_TABLE, 1.0), (WT_POSITION, 0.2),
        (CUTOFF, 5000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.0), (ATTACK, 0.01), (SUSTAIN, 0.9),
        (RELEASE, 0.2), (VOICES, 2.0), (GLIDE, 0.06)],
        &[(ModEnv, WT_POSITION, 0.6)]),
    p("Leads", "Sync-style lead", &[(WAVE, 2.0), (WAVE_2, 2.0), (PITCH_2, 7.0), (DETUNE, 0.1),
        (CUTOFF, 2500.0), (RESONANCE, 0.4), (ENV_AMOUNT, 0.3), (FILTER_DECAY, 0.3),
        (ATTACK, 0.005), (SUSTAIN, 0.8), (RELEASE, 0.2), (VOICES, 1.0), (GLIDE, 0.04)],
        &[(ModEnv, PITCH_2, 0.15), (Velocity, CUTOFF, 0.15)]),
    // Effects
    // Bend dives every note an octave: a sci-fi zap.
    p("Effects", "Laser", &[(LEVEL_2, 0.0), (CUTOFF, 5000.0), (RESONANCE, 0.2),
        (ENV_AMOUNT, 0.0), (ATTACK, 0.001), (DECAY, 0.3), (SUSTAIN, 0.0), (RELEASE, 0.1),
        (BEND, -12.0), (BEND_TIME, 0.2)], &[]),
    p("Effects", "Shaker (noise)", &[(WAVE, 4.0), (LEVEL_2, 0.0), (CUTOFF, 7000.0),
        (RESONANCE, 0.2), (ENV_AMOUNT, 0.0), (ATTACK, 0.005), (DECAY, 0.06), (SUSTAIN, 0.0),
        (RELEASE, 0.05)], &[]),
    p("Effects", "Wind", &[(WAVE, 4.0), (LEVEL_2, 0.0), (CUTOFF, 900.0), (RESONANCE, 0.7),
        (ENV_AMOUNT, 0.0), (ATTACK, 0.8), (SUSTAIN, 1.0), (RELEASE, 1.5), (LFO2_RATE, 0.2)],
        &[(Lfo2, CUTOFF, 0.25)]),
    p("Effects", "Random bleeps", &[(WAVE, 3.0), (LEVEL_2, 0.0), (CUTOFF, 3000.0),
        (RESONANCE, 0.3), (ENV_AMOUNT, 0.2), (SUSTAIN, 0.8), (LFO_TARGET, 0.0),
        (LFO_SHAPE, 4.0), (LFO_AMOUNT, 0.5), (LFO_SYNC, 5.0)], &[]),
];

/// The knobs for preset `i`: the defaults with its changes.
pub fn preset(i: usize) -> Values {
    let mut values = PARAMS.map(|p| p.default);
    for &(knob, value) in PRESETS[i].knobs {
        values[knob] = value;
    }
    values
}

impl Preset {
    /// The modulation that comes with it.
    pub fn links(&self) -> Vec<Link> {
        self.mods
            .iter()
            .map(|&(source, knob, amount)| Link {
                source,
                knob,
                amount,
            })
            .collect()
    }
}
