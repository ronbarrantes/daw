#![allow(dead_code)]
//! The synth as it was before it was split into modules (v0.32), kept
//! only so tests can check the modules sound exactly the same. Delete it
//! once that's no longer useful.
//!
//! A small synth voice: two oscillators (each morphing from sine through
//! triangle, saw and square to noise) into a resonant low-pass filter. An
//! ADSR envelope shapes the volume, and a separate decay envelope sweeps
//! the filter for plucky, analog-ish sounds. An LFO (one per track, see
//! `Lfo`) can wobble the pitch, filter or volume, and notes can glide.

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
    /// A count of 16th notes, shown as a note length like 3/16 or 1/4.
    Steps,
    /// An oscillator's shape, 0-4 along `WAVES`, blending in between.
    Wave,
    /// Semitones, shown as +7 or -12.
    Semitones,
    /// One of a list, by rounded index.
    Choice(&'static [&'static str]),
}

/// The shapes an oscillator morphs through, in order.
pub const WAVES: [&str; 5] = ["Sine", "Tri", "Saw", "Square", "Noise"];
impl Param {
    /// A value as 0-1 along the knob (log knobs evenly by ratio).
    pub fn to_unit(&self, v: f32) -> f32 {
        if self.log {
            (v / self.min).ln() / (self.max / self.min).ln()
        } else {
            (v - self.min) / (self.max - self.min)
        }
    }

    /// The value at 0-1 along the knob.
    pub fn at_unit(&self, t: f32) -> f32 {
        if self.log {
            self.min * (self.max / self.min).powf(t)
        } else {
            self.min + (self.max - self.min) * t
        }
    }
}

/// Ready-made sounds: each changes some knobs from their defaults. They're
/// synth versions of the instruments, a starting point to adjust.
#[rustfmt::skip]
pub const PRESETS: [(&str, &[(usize, f32)]); 14] = [
    ("Piano (synth)", &[(WAVE, 2.6), (WAVE_2, 1.0), (LEVEL_2, 0.5), (PITCH_2, 12.0), (DETUNE, 0.1),
        (CUTOFF, 1800.0), (RESONANCE, 0.1), (ENV_AMOUNT, 0.5), (FILTER_DECAY, 0.4),
        (ATTACK, 0.002), (DECAY, 0.9), (SUSTAIN, 0.0), (RELEASE, 0.4)]),
    ("Electric piano", &[(WAVE, 0.0), (WAVE_2, 0.0), (LEVEL_2, 0.35), (PITCH_2, 12.0),
        (CUTOFF, 3000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.3), (FILTER_DECAY, 0.3),
        (ATTACK, 0.002), (DECAY, 1.2), (SUSTAIN, 0.2), (RELEASE, 0.5),
        (LFO_TARGET, 2.0), (LFO_AMOUNT, 0.15), (LFO_RATE, 4.0)]),
    ("Guitar pluck", &[(WAVE, 2.0), (WAVE_2, 3.0), (LEVEL_2, 0.5), (DETUNE, 0.15),
        (CUTOFF, 600.0), (RESONANCE, 0.25), (ENV_AMOUNT, 0.7), (FILTER_DECAY, 0.2),
        (ATTACK, 0.001), (DECAY, 0.6), (SUSTAIN, 0.0), (RELEASE, 0.25)]),
    ("Bass", &[(WAVE, 2.0), (WAVE_2, 3.0), (LEVEL_2, 0.7), (PITCH_2, -12.0),
        (CUTOFF, 300.0), (RESONANCE, 0.3), (ENV_AMOUNT, 0.5), (FILTER_DECAY, 0.15),
        (ATTACK, 0.002), (DECAY, 0.3), (SUSTAIN, 0.6), (RELEASE, 0.1), (VOICES, 1.0)]),
    ("Sub bass", &[(WAVE, 0.0), (LEVEL_2, 0.0), (CUTOFF, 400.0), (RESONANCE, 0.0),
        (ENV_AMOUNT, 0.0), (ATTACK, 0.005), (DECAY, 0.3), (SUSTAIN, 1.0), (RELEASE, 0.1),
        (VOICES, 1.0)]),
    ("Acid bass", &[(WAVE, 2.0), (LEVEL_2, 0.0), (CUTOFF, 180.0), (RESONANCE, 0.85),
        (ENV_AMOUNT, 0.8), (FILTER_DECAY, 0.18), (ATTACK, 0.002), (DECAY, 0.2),
        (SUSTAIN, 0.5), (RELEASE, 0.08), (VOICES, 2.0), (GLIDE, 0.08)]),
    ("Strings", &[(WAVE, 2.0), (WAVE_2, 2.0), (DETUNE, 0.8), (CUTOFF, 1500.0),
        (RESONANCE, 0.1), (ENV_AMOUNT, 0.1), (ATTACK, 0.35), (DECAY, 0.5), (SUSTAIN, 0.85),
        (RELEASE, 0.7), (LFO_AMOUNT, 0.02), (LFO_RATE, 5.0)]),
    ("Violin lead", &[(WAVE, 2.0), (WAVE_2, 2.0), (DETUNE, 0.3), (CUTOFF, 2200.0),
        (RESONANCE, 0.15), (ENV_AMOUNT, 0.2), (ATTACK, 0.12), (SUSTAIN, 0.9),
        (RELEASE, 0.3), (VOICES, 2.0), (GLIDE, 0.05), (LFO_AMOUNT, 0.03), (LFO_RATE, 5.5)]),
    ("Brass", &[(WAVE, 2.0), (WAVE_2, 2.0), (DETUNE, 0.4), (CUTOFF, 500.0),
        (RESONANCE, 0.2), (ENV_AMOUNT, 0.6), (FILTER_DECAY, 0.35), (ATTACK, 0.05),
        (DECAY, 0.3), (SUSTAIN, 0.8), (RELEASE, 0.2)]),
    ("Organ", &[(WAVE, 0.0), (WAVE_2, 0.0), (LEVEL_2, 0.7), (PITCH_2, 12.0),
        (CUTOFF, 5000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.0), (ATTACK, 0.005),
        (DECAY, 0.1), (SUSTAIN, 1.0), (RELEASE, 0.05),
        (LFO_TARGET, 2.0), (LFO_AMOUNT, 0.1), (LFO_RATE, 6.0)]),
    ("Flute", &[(WAVE, 0.3), (WAVE_2, 4.0), (LEVEL_2, 0.08), (CUTOFF, 2500.0),
        (RESONANCE, 0.0), (ENV_AMOUNT, 0.1), (ATTACK, 0.08), (SUSTAIN, 0.9),
        (RELEASE, 0.2), (LFO_AMOUNT, 0.02), (LFO_RATE, 5.0)]),
    ("Bell", &[(WAVE, 0.0), (WAVE_2, 0.0), (LEVEL_2, 0.6), (PITCH_2, 19.0), (DETUNE, 0.6),
        (CUTOFF, 6000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.2), (ATTACK, 0.001),
        (DECAY, 1.8), (SUSTAIN, 0.0), (RELEASE, 1.5)]),
    ("Warm pad", &[(WAVE, 2.5), (WAVE_2, 2.5), (DETUNE, 0.7), (CUTOFF, 700.0),
        (RESONANCE, 0.2), (ENV_AMOUNT, 0.15), (FILTER_DECAY, 1.5), (ATTACK, 0.8),
        (DECAY, 1.0), (SUSTAIN, 0.8), (RELEASE, 1.5),
        (LFO_TARGET, 1.0), (LFO_AMOUNT, 0.15), (LFO_RATE, 0.3)]),
    ("Shaker (noise)", &[(WAVE, 4.0), (LEVEL_2, 0.0), (CUTOFF, 7000.0), (RESONANCE, 0.2),
        (ENV_AMOUNT, 0.0), (ATTACK, 0.005), (DECAY, 0.06), (SUSTAIN, 0.0), (RELEASE, 0.05)]),
];

/// The knobs for preset `i`: the defaults with its changes.
pub fn preset(i: usize) -> Values {
    let mut values = PARAMS.map(|p| p.default);
    for &(knob, value) in PRESETS[i].1 {
        values[knob] = value;
    }
    values
}

/// Where `Unit::Wave` puts a plain saw, the sound before shapes could be picked.
pub const SAW: f32 = 2.0;
pub const LFO_SHAPES: [&str; 5] = ["Sine", "Tri", "Square", "Saw", "Random"];
pub const LFO_TARGETS: [&str; 4] = ["Pitch", "Filter", "Volume", "Pan"];
/// `LFO_TARGETS` index of Pan, which the engine applies to the whole track.
pub const LFO_PAN: usize = 3;
/// LFO speeds tied to the tempo, and how many beats each cycle lasts.
pub const LFO_SYNCS: [&str; 8] = [
    "Off", "1 bar", "1/2", "1/4", "1/8", "1/16", "1/8 T", "1/16 T",
];
const SYNC_BEATS: [f32; 8] = [0.0, 4.0, 2.0, 1.0, 0.5, 0.25, 1.0 / 3.0, 1.0 / 6.0];
pub const VOICE_MODES: [&str; 3] = ["Poly", "Mono", "Legato"];
/// How the arpeggiator walks through the held notes; Off plays them as
/// chords like normal.
pub const ARP_MODES: [&str; 6] = ["Off", "Up", "Down", "Up/Down", "Random", "As played"];
/// How often the arpeggiator plays a note, and how many 16th-note steps
/// that is.
pub const ARP_RATES: [&str; 6] = ["1/4", "1/8", "1/16", "1/32", "1/8 T", "1/16 T"];
pub const ARP_RATE_STEPS: [f32; 6] = [4.0, 2.0, 1.0, 0.5, 4.0 / 3.0, 2.0 / 3.0];
pub const ARP_OCTAVE_CHOICES: [&str; 4] = ["1", "2", "3", "4"];
pub const OFF_ON: [&str; 2] = ["Off", "On"];
/// What the arpeggiator plays: the notes held, or a chord or scale built
/// on the lowest held note, so one finger makes a whole pattern.
pub const ARP_NOTE_SETS: [&str; 10] = [
    "Held",
    "Major",
    "Minor",
    "7th",
    "Maj7",
    "Min7",
    "Sus4",
    "Major scale",
    "Minor scale",
    "Pentatonic",
];
/// The semitones above the root for each of `ARP_NOTE_SETS` after Held.
pub const ARP_INTERVALS: [&[u8]; 9] = [
    &[0, 4, 7],
    &[0, 3, 7],
    &[0, 4, 7, 10],
    &[0, 4, 7, 11],
    &[0, 3, 7, 10],
    &[0, 5, 7],
    &[0, 2, 4, 5, 7, 9, 11],
    &[0, 2, 3, 5, 7, 8, 10],
    &[0, 2, 4, 7, 9],
];
const RANDOM: usize = 4;

/// How a synth track plays overlapping notes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    /// Chords: each note gets its own voice.
    Poly,
    /// One note at a time; each new note restarts the envelope.
    Mono,
    /// One note at a time; a note played while another is held just
    /// changes pitch (gliding if Glide is up), without restarting.
    Legato,
}

pub fn mode(v: &Values) -> Mode {
    match choice(v[VOICES], VOICE_MODES.len()) {
        1 => Mode::Mono,
        2 => Mode::Legato,
        _ => Mode::Poly,
    }
}

/// A choice knob's value as an index.
pub fn choice(v: f32, count: usize) -> usize {
    (v.round().max(0.0) as usize).min(count - 1)
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
pub const WAVE_2: usize = 11;
pub const LEVEL_2: usize = 12;
pub const PITCH_2: usize = 13;
pub const LFO_RATE: usize = 14;
pub const LFO_AMOUNT: usize = 15;
pub const LFO_SHAPE: usize = 16;
pub const LFO_TARGET: usize = 17;
pub const LFO_SYNC: usize = 18;
pub const GLIDE: usize = 19;
pub const VOICES: usize = 20;
pub const ARP_MODE: usize = 21;
pub const ARP_RATE: usize = 22;
pub const ARP_OCTAVES: usize = 23;
pub const ARP_GATE: usize = 24;
pub const ARP_LATCH: usize = 25;
pub const ARP_NOTES: usize = 26;

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

/// The knobs. Song files store them by name, so a renamed knob needs
/// converting when old songs open (see `file`); the defaults sound like
/// the synth did before each knob existed.
pub const PARAMS: [Param; 27] = [
    param("Wave 1", 0.0, 4.0, SAW, false, Unit::Wave),
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
    param("Wave 2", 0.0, 4.0, SAW, false, Unit::Wave),
    param("Level 2", 0.0, 1.0, 1.0, false, Unit::Percent),
    param("Pitch 2", -24.0, 24.0, 0.0, false, Unit::Semitones),
    param("LFO rate", 0.05, 20.0, 4.0, true, Unit::Hz),
    param("LFO amt", 0.0, 1.0, 0.0, false, Unit::Percent),
    param("LFO wave", 0.0, 4.0, 0.0, false, Unit::Choice(&LFO_SHAPES)),
    param("LFO to", 0.0, 3.0, 0.0, false, Unit::Choice(&LFO_TARGETS)),
    param("LFO sync", 0.0, 7.0, 0.0, false, Unit::Choice(&LFO_SYNCS)),
    param("Glide", 0.0, 1.0, 0.0, false, Unit::Seconds),
    param("Voices", 0.0, 2.0, 0.0, false, Unit::Choice(&VOICE_MODES)),
    param("Arp", 0.0, 5.0, 0.0, false, Unit::Choice(&ARP_MODES)),
    param("Arp rate", 0.0, 5.0, 2.0, false, Unit::Choice(&ARP_RATES)),
    param(
        "Octaves",
        0.0,
        3.0,
        0.0,
        false,
        Unit::Choice(&ARP_OCTAVE_CHOICES),
    ),
    param("Arp gate", 0.05, 1.0, 0.5, false, Unit::Percent),
    param("Latch", 0.0, 1.0, 0.0, false, Unit::Choice(&OFF_ON)),
    param(
        "Arp notes",
        0.0,
        9.0,
        0.0,
        false,
        Unit::Choice(&ARP_NOTE_SETS),
    ),
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

/// Tiny xorshift noise, -1 to 1.
fn noise(state: &mut u32) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state as f32 / u32::MAX as f32 * 2.0 - 1.0
}

/// One of `WAVES` at `phase` (0-1).
fn shape(i: usize, phase: f32, rng: &mut u32) -> f32 {
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
        _ => noise(rng),
    }
}

/// An oscillator `wave` (0-4) of the way along `WAVES`, blending the two
/// shapes either side.
fn morph(wave: f32, phase: f32, rng: &mut u32) -> f32 {
    let i = (wave.max(0.0) as usize).min(WAVES.len() - 2);
    let f = (wave - i as f32).clamp(0.0, 1.0);
    let a = if f < 1.0 { shape(i, phase, rng) } else { 0.0 };
    let b = if f > 0.0 {
        shape(i + 1, phase, rng)
    } else {
        0.0
    };
    a + (b - a) * f
}

/// A track's LFO, shared by its voices so they wobble together.
pub struct Lfo {
    phase: f32,
    /// The Random shape's current value, picked each cycle.
    held: f32,
    rng: u32,
}

impl Default for Lfo {
    fn default() -> Self {
        Self {
            phase: 0.0,
            held: 0.0,
            rng: 0x2f6b_1d93,
        }
    }
}

impl Lfo {
    /// How far through a cycle one sample goes, from the rate knob or,
    /// when synced, the tempo (`beat` is a beat's length in samples).
    pub fn step(v: &Values, sr: f32, beat: f32) -> f32 {
        match choice(v[LFO_SYNC], LFO_SYNCS.len()) {
            0 => v[LFO_RATE] / sr,
            sync => 1.0 / (SYNC_BEATS[sync] * beat).max(1.0),
        }
    }

    /// The next value, -1 to 1.
    pub fn next(&mut self, v: &Values, step: f32) -> f32 {
        self.phase += step;
        if self.phase >= 1.0 {
            self.phase = self.phase.fract();
            self.held = noise(&mut self.rng);
        }
        match choice(v[LFO_SHAPE], LFO_SHAPES.len()) {
            0 => (self.phase * std::f32::consts::TAU).sin(),
            1 => 1.0 - 4.0 * (self.phase - 0.5).abs(),
            2 => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            3 => 1.0 - 2.0 * self.phase,
            _ => self.held,
        }
    }
}

pub struct Voice {
    sr: f32,
    freq: f32,
    /// The pitch a glide is heading for.
    target: f32,
    phase: [f32; 2],
    stage: Stage,
    amp: f32,
    filter_env: f32,
    // State-variable filter memory.
    low: f32,
    band: f32,
    rng: u32,
    // Derived from the knob values by `set_params`.
    waves: [f32; 2],
    level_2: f32,
    /// The second oscillator's pitch relative to the first: Detune and
    /// Pitch 2 together.
    ratio_2: f32,
    lfo_amount: f32,
    lfo_target: usize,
    /// Random LFO on pitch steps by whole semitones, for random tones.
    lfo_steps: bool,
    /// 0 = jump straight to each new pitch.
    glide_coef: f32,
    /// A smooth pitch and filter offset from `bend`, heading for its target
    /// over a few milliseconds so slider moves don't step.
    bend_semis: f32,
    bend_filter: f32,
    bend_target: (f32, f32),
    bend_coef: f32,
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
            target: 220.0,
            phase: [0.0; 2],
            stage: Stage::Release,
            amp: 0.0,
            filter_env: 0.0,
            low: 0.0,
            band: 0.0,
            rng: 0x1357_9bdf,
            waves: [SAW; 2],
            level_2: 1.0,
            ratio_2: 1.0,
            lfo_amount: 0.0,
            lfo_target: 0,
            lfo_steps: false,
            glide_coef: 0.0,
            bend_semis: 0.0,
            bend_filter: 0.0,
            bend_target: (0.0, 0.0),
            bend_coef: coef(0.01, sr),
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
        self.waves = [v[WAVE], v[WAVE_2]];
        self.level_2 = v[LEVEL_2];
        let semis = v[PITCH_2].round();
        self.ratio_2 = (1.0 + v[DETUNE] * 0.03) * 2f32.powf(semis / 12.0);
        self.lfo_amount = v[LFO_AMOUNT];
        self.lfo_target = choice(v[LFO_TARGET], LFO_TARGETS.len());
        self.lfo_steps = choice(v[LFO_SHAPE], LFO_SHAPES.len()) == RANDOM;
        // About three time constants to arrive, so the knob reads as the
        // glide's length.
        self.glide_coef = if v[GLIDE] < 0.005 {
            0.0
        } else {
            coef(v[GLIDE] / 3.0, self.sr)
        };
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

    /// Starts a note. With `glide`, the pitch slides from the last note's;
    /// without `retrigger` (legato), the envelopes carry on.
    /// Moves the note's pitch by `semis` (fractions allowed, so between the
    /// usual notes) and its filter by `filter` octaves, smoothly.
    pub fn bend(&mut self, semis: f32, filter: f32) {
        self.bend_target = (semis, filter);
    }

    pub fn note_on(&mut self, note: u8, glide: bool, retrigger: bool) {
        self.bend_semis = 0.0;
        self.bend_filter = 0.0;
        self.bend_target = (0.0, 0.0);
        self.target = midi_to_hz(note);
        // Only glide from a note that's still sounding.
        if !glide || self.glide_coef == 0.0 || self.is_idle() {
            self.freq = self.target;
        }
        if retrigger || self.stage == Stage::Release {
            self.stage = Stage::Attack;
            self.filter_env = 1.0;
        }
    }

    /// Finished and silent, so it can be skipped to save CPU.
    pub fn is_idle(&self) -> bool {
        self.stage == Stage::Release && self.amp < 1e-5
    }

    pub fn note_off(&mut self) {
        self.stage = Stage::Release;
    }

    /// The next sample; `lfo` is the track's LFO, -1 to 1.
    pub fn next_sample(&mut self, lfo: f32) -> f32 {
        if self.glide_coef > 0.0 {
            self.freq += (self.target - self.freq) * self.glide_coef;
        }
        let lfo = lfo * self.lfo_amount;
        let mut pitch = 1.0;
        let mut cutoff_octaves = self.env_octaves * self.filter_env;
        if self.bend_target != (0.0, 0.0) || self.bend_semis != 0.0 || self.bend_filter != 0.0 {
            self.bend_semis += (self.bend_target.0 - self.bend_semis) * self.bend_coef;
            self.bend_filter += (self.bend_target.1 - self.bend_filter) * self.bend_coef;
            pitch *= 2f32.powf(self.bend_semis / 12.0);
            cutoff_octaves += self.bend_filter;
        }
        let mut level = 1.0;
        if self.lfo_amount > 0.0 {
            match self.lfo_target {
                0 => {
                    let semis = lfo * 12.0;
                    let semis = if self.lfo_steps { semis.round() } else { semis };
                    pitch *= 2f32.powf(semis / 12.0);
                }
                1 => cutoff_octaves += lfo * 4.0,
                2 => level = 1.0 - (self.lfo_amount - lfo) * 0.5,
                // Pan moves the whole track; the engine does it.
                _ => {}
            }
        }
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

        let freq = self.freq * pitch;
        self.phase[0] = (self.phase[0] + freq / self.sr).fract();
        let mut osc = morph(self.waves[0], self.phase[0], &mut self.rng);
        if self.level_2 > 0.0 {
            self.phase[1] = (self.phase[1] + freq * self.ratio_2 / self.sr).fract();
            osc += morph(self.waves[1], self.phase[1], &mut self.rng) * self.level_2;
        }

        // Chamberlin state-variable filter. Keeping the cutoff below sr/6.5
        // keeps it stable.
        let cutoff = (self.cutoff * 2f32.powf(cutoff_octaves)).min(self.sr / 6.5);
        let f = 2.0 * (PI * cutoff / self.sr).sin();
        let high = osc * 0.5 - self.low - self.damping * self.band;
        self.band += f * high;
        self.low += f * self.band;

        self.low * self.amp * self.volume * 0.5 * level
    }
}
