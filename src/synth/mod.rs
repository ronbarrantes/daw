//! The built-in synth, made of modules: Oscillators into a Filter, shaped
//! by an Envelope (ADSR), with an LFO to wobble things, the Voice settings
//! (poly, mono, glide) and an Arpeggiator.
//!
//! Every module follows one contract (see docs/MODULES.md): a `ModuleInfo`
//! naming it and listing its knobs and their layout, and, for modules that
//! make or shape sound, a struct that reads its own knobs (`Module`). The
//! modules are listed once, in `MODULES`; from that list come the knobs
//! (`PARAMS`, which songs save by name), the instrument panel's sections,
//! and the knobs automation can move.

mod arp;
mod envelope;
mod filter;
mod fm;
#[cfg(test)]
mod legacy;
mod lfo;
mod lfo2;
mod mallet;
mod modenv;
pub mod modulation;
mod osc;
mod play;
mod pluck;

pub use lfo::Lfo;
pub use modenv::ModEnv;
pub use modulation::{Link, Source};
pub use play::Mode;

/// A user-facing synth control.
#[derive(Clone, Copy)]
pub struct Param {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// Knob moves exponentially (for frequencies and times).
    pub log: bool,
    pub unit: Unit,
}

#[derive(Clone, Copy, PartialEq)]
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
    /// A multiple, shown as x2 or x3.5.
    Ratio,
    /// One of a list, by rounded index.
    Choice(&'static [&'static str]),
}

impl Param {
    /// A value as 0-1 along the knob (log knobs evenly by ratio).
    pub fn to_unit(self, v: f32) -> f32 {
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
pub const PRESETS: [(&str, &[(usize, f32)]); 25] = [
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
    // The Pluck and Mallet engines ring and die away by themselves, so
    // their envelope just holds, and the filter stays open.
    ("Nylon guitar (pluck)", &[(ENGINE, 1.0), (STRING_TONE, 0.45), (STRING_DECAY, 2.5),
        (PICK_SPOT, 0.2), (CUTOFF, 6000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.0),
        (ATTACK, 0.001), (SUSTAIN, 1.0), (RELEASE, 0.3)]),
    ("Harp (pluck)", &[(ENGINE, 1.0), (STRING_TONE, 0.7), (STRING_DECAY, 4.0),
        (PICK_SPOT, 0.1), (CUTOFF, 8000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.0),
        (ATTACK, 0.001), (SUSTAIN, 1.0), (RELEASE, 1.0)]),
    ("Koto (pluck)", &[(ENGINE, 1.0), (STRING_TONE, 0.85), (STRING_DECAY, 1.5),
        (PICK_SPOT, 0.05), (CUTOFF, 8000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.0),
        (ATTACK, 0.001), (SUSTAIN, 1.0), (RELEASE, 0.4)]),
    ("Marimba (mallet)", &[(ENGINE, 2.0), (MATERIAL, 0.0), (HARDNESS, 0.4), (RING, 0.8),
        (CUTOFF, 8000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.0), (ATTACK, 0.001),
        (SUSTAIN, 1.0), (RELEASE, 0.5)]),
    ("Vibes (mallet)", &[(ENGINE, 2.0), (MATERIAL, 1.0), (HARDNESS, 0.5), (RING, 3.0),
        (CUTOFF, 8000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.0), (ATTACK, 0.001),
        (SUSTAIN, 1.0), (RELEASE, 1.0), (LFO_TARGET, 2.0), (LFO_AMOUNT, 0.25),
        (LFO_RATE, 5.0)]),
    ("Glass (mallet)", &[(ENGINE, 2.0), (MATERIAL, 2.0), (HARDNESS, 0.7), (RING, 2.5),
        (CUTOFF, 8000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.0), (ATTACK, 0.001),
        (SUSTAIN, 1.0), (RELEASE, 1.0)]),
    ("Church bell (mallet)", &[(ENGINE, 2.0), (MATERIAL, 3.0), (HARDNESS, 0.8), (RING, 6.0),
        (CUTOFF, 8000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.0), (ATTACK, 0.001),
        (SUSTAIN, 1.0), (RELEASE, 2.0)]),
    ("FM electric piano", &[(ENGINE, 3.0), (FM_RATIO, 1.0), (FM_DEPTH, 0.3), (FM_DECAY, 0.8),
        (CUTOFF, 6000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.0), (ATTACK, 0.002),
        (DECAY, 1.5), (SUSTAIN, 0.3), (RELEASE, 0.4), (LFO_TARGET, 2.0), (LFO_AMOUNT, 0.12),
        (LFO_RATE, 4.5)]),
    ("FM bell", &[(ENGINE, 3.0), (FM_RATIO, 3.5), (FM_DEPTH, 0.5), (FM_DECAY, 2.5),
        (CUTOFF, 8000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.0), (ATTACK, 0.001),
        (DECAY, 2.0), (SUSTAIN, 0.0), (RELEASE, 2.0)]),
    ("FM bass", &[(ENGINE, 3.0), (FM_RATIO, 1.0), (FM_DEPTH, 0.55), (FM_DECAY, 0.15),
        (FM_FEEDBACK, 0.3), (CUTOFF, 2500.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.0),
        (ATTACK, 0.002), (DECAY, 0.4), (SUSTAIN, 0.6), (RELEASE, 0.1), (VOICES, 1.0)]),
    ("Glassy keys (FM)", &[(ENGINE, 3.0), (FM_RATIO, 2.0), (FM_DEPTH, 0.35), (FM_DECAY, 1.2),
        (CUTOFF, 8000.0), (RESONANCE, 0.0), (ENV_AMOUNT, 0.0), (ATTACK, 0.002),
        (DECAY, 1.2), (SUSTAIN, 0.4), (RELEASE, 0.6)]),
];

/// The knobs for preset `i`: the defaults with its changes.
pub fn preset(i: usize) -> Values {
    let mut values = PARAMS.map(|p| p.default);
    for &(knob, value) in PRESETS[i].1 {
        values[knob] = value;
    }
    values
}

pub use arp::{
    INTERVALS as ARP_INTERVALS, MODES as ARP_MODES, NOTE_SETS as ARP_NOTE_SETS, RATES as ARP_RATES,
    RATE_STEPS as ARP_RATE_STEPS,
};
pub use lfo::PAN as LFO_PAN;
pub use osc::{SAW, WAVES};
pub use play::{ANALOG, ENGINES, FM, MALLET, PLUCK};

/// What a module is: its name (the heading of its section in the
/// instrument panel), its knobs, and how they're laid out.
pub struct ModuleInfo {
    pub name: &'static str,
    /// Knob names must be unique across all modules: songs save knobs by
    /// name.
    pub knobs: &'static [Param],
    /// The panel's rows of knobs, by their number in `knobs`. A knob in no
    /// row is hidden (automation can still move it).
    pub rows: &'static [&'static [usize]],
    /// Whether its section starts unfolded.
    pub open: bool,
    /// For a sound engine's module, which of `play::ENGINES` it belongs
    /// to: its section only shows while that engine is picked.
    pub engine: Option<usize>,
}

/// Whether module `m`'s section shows for these knobs (an engine's only
/// while it's the one picked).
pub fn shown(m: &ModuleInfo, v: &Values) -> bool {
    m.engine.is_none_or(|e| e == engine(v))
}

/// The sound engine picked, as an index into `ENGINES`.
pub fn engine(v: &Values) -> usize {
    choice(v[ENGINE], play::ENGINES.len())
}

/// A module that makes or shapes sound, one per voice.
pub trait Module {
    /// Reads its knobs (just its own, numbered as in its `ModuleInfo`).
    fn set_knobs(&mut self, knobs: &[f32], sr: f32);
}

/// Every module, in the order the panel shows them. A new module goes
/// here, and nowhere else, to get its knobs, panel section, saving and
/// automation.
pub const MODULES: [&ModuleInfo; 11] = [
    &play::INFO,
    &osc::INFO,
    &pluck::INFO,
    &mallet::INFO,
    &fm::INFO,
    &filter::INFO,
    &envelope::INFO,
    &lfo::INFO,
    &lfo2::INFO,
    &modenv::INFO,
    &arp::INFO,
];

/// Where module `m`'s knobs start in `PARAMS` and `Values`.
pub const fn first_knob(m: &ModuleInfo) -> usize {
    let mut at = 0;
    let mut i = 0;
    while i < MODULES.len() {
        if same_name(MODULES[i].name, m.name) {
            return at;
        }
        at += MODULES[i].knobs.len();
        i += 1;
    }
    panic!("module missing from MODULES");
}

const fn same_name(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// A module's own knobs out of a track's `Values`.
pub fn knobs<'a>(v: &'a Values, m: &ModuleInfo) -> &'a [f32] {
    let at = first_knob(m);
    &v[at..at + m.knobs.len()]
}

const KNOB_COUNT: usize = {
    let mut n = 0;
    let mut i = 0;
    while i < MODULES.len() {
        n += MODULES[i].knobs.len();
        i += 1;
    }
    n
};

/// Every module's knobs, one after another.
pub const PARAMS: [Param; KNOB_COUNT] = {
    let mut all = [param("", 0.0, 1.0, 0.0, false, Unit::Percent); KNOB_COUNT];
    let mut n = 0;
    let mut i = 0;
    while i < MODULES.len() {
        let mut k = 0;
        while k < MODULES[i].knobs.len() {
            all[n] = MODULES[i].knobs[k];
            n += 1;
            k += 1;
        }
        i += 1;
    }
    all
};

/// The knobs automation and modulation can move: all but the choices
/// (which would jump).
pub fn automatable() -> impl Iterator<Item = usize> {
    (0..PARAMS.len()).filter(|&k| movable(k))
}

/// Whether knob `k` can be automated or modulated.
pub fn movable(k: usize) -> bool {
    k < PARAMS.len() && !matches!(PARAMS[k].unit, Unit::Choice(_))
}

/// Track-wide modulation: what LFO 2 is doing (it runs once per track,
/// like LFO 1).
pub fn lfo2_step(v: &Values, sr: f32, beat: f32) -> f32 {
    let k = knobs(v, &lfo2::INFO);
    Lfo::step_with(k[lfo2::RATE], k[lfo2::SYNC], sr, beat)
}

pub fn lfo2_next(lfo: &mut Lfo, v: &Values, step: f32) -> f32 {
    lfo.next_with(knobs(v, &lfo2::INFO)[lfo2::SHAPE], step)
}

/// Moves a voice's mod envelope on by `n` samples.
pub fn mod_env_advance(env: &mut ModEnv, v: &Values, n: u32, sr: f32) {
    env.advance(knobs(v, &modenv::INFO), n, sr);
}

/// The knobs by name across modules, for the rest of the app.
pub const WAVE: usize = first_knob(&osc::INFO) + osc::WAVE_1;
pub const WAVE_2: usize = first_knob(&osc::INFO) + osc::WAVE_2;
pub const LEVEL_2: usize = first_knob(&osc::INFO) + osc::LEVEL_2;
pub const PITCH_2: usize = first_knob(&osc::INFO) + osc::PITCH_2;
pub const DETUNE: usize = first_knob(&osc::INFO) + osc::DETUNE;
pub const VOICES: usize = first_knob(&play::INFO) + play::VOICES;
pub const GLIDE: usize = first_knob(&play::INFO) + play::GLIDE;
pub const VOLUME: usize = first_knob(&play::INFO) + play::VOLUME;
pub const ENGINE: usize = first_knob(&play::INFO) + play::ENGINE;
pub const STRING_TONE: usize = first_knob(&pluck::INFO) + pluck::TONE;
pub const STRING_DECAY: usize = first_knob(&pluck::INFO) + pluck::DECAY;
pub const PICK_SPOT: usize = first_knob(&pluck::INFO) + pluck::POSITION;
pub const MATERIAL: usize = first_knob(&mallet::INFO) + mallet::MATERIAL;
pub const HARDNESS: usize = first_knob(&mallet::INFO) + mallet::HARDNESS;
pub const RING: usize = first_knob(&mallet::INFO) + mallet::DECAY;
pub const FM_RATIO: usize = first_knob(&fm::INFO) + fm::RATIO;
pub const FM_DEPTH: usize = first_knob(&fm::INFO) + fm::DEPTH;
pub const FM_DECAY: usize = first_knob(&fm::INFO) + fm::DECAY;
pub const FM_FEEDBACK: usize = first_knob(&fm::INFO) + fm::FEEDBACK;
pub const CUTOFF: usize = first_knob(&filter::INFO) + filter::CUTOFF;
pub const RESONANCE: usize = first_knob(&filter::INFO) + filter::RESONANCE;
pub const ENV_AMOUNT: usize = first_knob(&filter::INFO) + filter::ENV_AMOUNT;
pub const FILTER_DECAY: usize = first_knob(&filter::INFO) + filter::DECAY;
pub const ATTACK: usize = first_knob(&envelope::INFO) + envelope::ATTACK;
pub const DECAY: usize = first_knob(&envelope::INFO) + envelope::DECAY;
pub const SUSTAIN: usize = first_knob(&envelope::INFO) + envelope::SUSTAIN;
pub const RELEASE: usize = first_knob(&envelope::INFO) + envelope::RELEASE;
pub const LFO_TARGET: usize = first_knob(&lfo::INFO) + lfo::TARGET;
#[cfg(test)]
pub const LFO_SHAPE: usize = first_knob(&lfo::INFO) + lfo::SHAPE;
pub const LFO_AMOUNT: usize = first_knob(&lfo::INFO) + lfo::AMOUNT;
pub const LFO_RATE: usize = first_knob(&lfo::INFO) + lfo::RATE;
#[cfg(test)]
pub const LFO_SYNC: usize = first_knob(&lfo::INFO) + lfo::SYNC;
pub const ARP_MODE: usize = first_knob(&arp::INFO) + arp::MODE;
pub const ARP_RATE: usize = first_knob(&arp::INFO) + arp::RATE;
pub const ARP_OCTAVES: usize = first_knob(&arp::INFO) + arp::OCTAVES;
pub const ARP_GATE: usize = first_knob(&arp::INFO) + arp::GATE;
pub const ARP_NOTES: usize = first_knob(&arp::INFO) + arp::NOTES;
pub const ARP_LATCH: usize = first_knob(&arp::INFO) + arp::LATCH;

pub fn mode(v: &Values) -> Mode {
    play::mode(knobs(v, &play::INFO))
}

/// A choice knob's value as an index.
pub fn choice(v: f32, count: usize) -> usize {
    (v.round().max(0.0) as usize).min(count - 1)
}

pub(crate) const fn param(
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

/// One synth voice: its modules wired together. The pitch (with glide and
/// bends) drives the Oscillators, into the Filter, at the Envelope's
/// volume; the track's LFO moves one of them.
pub struct Voice {
    sr: f32,
    freq: f32,
    /// The pitch a glide is heading for.
    target: f32,
    osc: osc::Osc,
    pluck: pluck::Pluck,
    mallet: mallet::Mallet,
    fm: fm::Fm,
    /// Which of `ENGINES` makes the sound.
    engine: usize,
    filter: filter::Filter,
    env: envelope::Envelope,
    // Derived from the knob values by `set_params`.
    lfo_amount: f32,
    lfo_target: usize,
    /// Random LFO on pitch steps by whole semitones, for random tones.
    lfo_steps: bool,
    /// 0 = jump straight to each new pitch.
    glide_coef: f32,
    volume: f32,
    /// A smooth pitch and filter offset from `bend`, heading for its target
    /// over a few milliseconds so slider moves don't step.
    bend_semis: f32,
    bend_filter: f32,
    bend_target: (f32, f32),
    bend_coef: f32,
}

impl Voice {
    pub fn new(sr: f32) -> Self {
        let mut v = Self {
            sr,
            freq: 220.0,
            target: 220.0,
            osc: osc::Osc::default(),
            pluck: pluck::Pluck::default(),
            mallet: mallet::Mallet::default(),
            fm: fm::Fm::default(),
            engine: ANALOG,
            filter: filter::Filter::default(),
            env: envelope::Envelope::default(),
            lfo_amount: 0.0,
            lfo_target: 0,
            lfo_steps: false,
            glide_coef: 0.0,
            volume: 0.0,
            bend_semis: 0.0,
            bend_filter: 0.0,
            bend_target: (0.0, 0.0),
            bend_coef: coef(0.01, sr),
        };
        v.set_params(&PARAMS.map(|p| p.default));
        v
    }

    pub fn set_params(&mut self, v: &Values) {
        let sr = self.sr;
        self.osc.set_knobs(knobs(v, &osc::INFO), sr);
        self.pluck.set_knobs(knobs(v, &pluck::INFO), sr);
        self.mallet.set_knobs(knobs(v, &mallet::INFO), sr);
        self.fm.set_knobs(knobs(v, &fm::INFO), sr);
        self.engine = engine(v);
        self.filter.set_knobs(knobs(v, &filter::INFO), sr);
        self.env.set_knobs(knobs(v, &envelope::INFO), sr);
        let lfo = knobs(v, &lfo::INFO);
        self.lfo_amount = lfo[lfo::AMOUNT];
        self.lfo_target = choice(lfo[lfo::TARGET], lfo::TARGETS.len());
        self.lfo_steps = choice(lfo[lfo::SHAPE], lfo::SHAPES.len()) == lfo::RANDOM;
        let play = knobs(v, &play::INFO);
        // About three time constants to arrive, so the knob reads as the
        // glide's length.
        self.glide_coef = if play[play::GLIDE] < 0.005 {
            0.0
        } else {
            coef(play[play::GLIDE] / 3.0, sr)
        };
        self.volume = play[play::VOLUME];
    }

    /// Moves the note's pitch by `semis` (fractions allowed, so between the
    /// usual notes) and its filter by `filter` octaves, smoothly.
    pub fn bend(&mut self, semis: f32, filter: f32) {
        self.bend_target = (semis, filter);
    }

    /// Starts a note. With `glide`, the pitch slides from the last note's;
    /// without `retrigger` (legato), the envelopes carry on.
    pub fn note_on(&mut self, note: u8, glide: bool, retrigger: bool) {
        self.bend_semis = 0.0;
        self.bend_filter = 0.0;
        self.bend_target = (0.0, 0.0);
        self.target = midi_to_hz(note);
        // Only glide from a note that's still sounding.
        if !glide || self.glide_coef == 0.0 || self.is_idle() {
            self.freq = self.target;
        }
        if retrigger || self.env.is_releasing() {
            self.env.trigger();
            self.filter.trigger();
            match self.engine {
                PLUCK => self.pluck.trigger(self.target, self.sr),
                MALLET => self.mallet.trigger(),
                FM => self.fm.trigger(),
                _ => {}
            }
        }
    }

    /// Finished and silent, so it can be skipped to save CPU.
    pub fn is_idle(&self) -> bool {
        self.env.is_releasing() && self.env.amp < 1e-5
    }

    pub fn note_off(&mut self) {
        self.env.release();
    }

    /// The next sample; `lfo` is the track's LFO, -1 to 1.
    pub fn next_sample(&mut self, lfo: f32) -> f32 {
        if self.glide_coef > 0.0 {
            self.freq += (self.target - self.freq) * self.glide_coef;
        }
        let lfo = lfo * self.lfo_amount;
        let mut pitch = 1.0;
        let mut cutoff_octaves = self.filter.env_octaves();
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
        let amp = self.env.next();
        self.filter.decay();
        let freq = self.freq * pitch;
        let osc = match self.engine {
            PLUCK => self.pluck.next(freq, self.sr),
            MALLET => self.mallet.next(freq, self.sr),
            FM => self.fm.next(freq, self.sr),
            _ => self.osc.next(freq, self.sr),
        };
        let low = self.filter.process(osc, cutoff_octaves, self.sr);
        low * amp * self.volume * 0.5 * level
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
        v.note_on(60, false, true);
        let run = |v: &mut Voice, seconds: f32| {
            for _ in 0..(seconds * sr) as usize {
                v.next_sample(0.0);
            }
            v.env.amp
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

    /// A voice playing `values` for `samples`, with the LFO.
    fn play(values: &Values, note: u8, samples: usize) -> Vec<f32> {
        let mut v = Voice::new(48_000.0);
        v.set_params(values);
        v.note_on(note, false, true);
        let mut lfo = Lfo::default();
        let step = Lfo::step(values, 48_000.0, 24_000.0);
        (0..samples)
            .map(|_| v.next_sample(lfo.next(values, step)))
            .collect()
    }

    /// How many times the signal crosses zero going up: roughly its pitch.
    fn crossings(out: &[f32]) -> usize {
        out.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
    }

    fn open_filter() -> Values {
        let mut v = PARAMS.map(|p| p.default);
        v[CUTOFF] = 8000.0;
        v[ENV_AMOUNT] = 0.0;
        v[LEVEL_2] = 0.0;
        v
    }

    #[test]
    fn presets_stay_within_each_knob_and_make_sound() {
        for (i, (name, _)) in PRESETS.iter().enumerate() {
            let values = preset(i);
            for (p, v) in PARAMS.iter().zip(values) {
                assert!((p.min..=p.max).contains(&v), "{name}: {} = {v}", p.name);
            }
            let out = play(&values, 60, 4_800);
            let peak = out.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.01 && peak < 1.0, "{name}: peak {peak}");
        }
    }

    #[test]
    fn waves_sound_different_and_noise_has_no_pitch() {
        let mut sine = open_filter();
        sine[WAVE] = 0.0;
        let mut noisy = open_filter();
        noisy[WAVE] = 4.0;
        // A440 for a tenth of a second: 44 cycles.
        let tone = crossings(&play(&sine, 69, 4_800)[1_000..]);
        assert!((34..=40).contains(&tone), "sine crossings: {tone}");
        let hiss = crossings(&play(&noisy, 69, 4_800)[1_000..]);
        assert!(hiss > 200, "noise crosses all the time: {hiss}");
    }

    #[test]
    fn the_old_wave_knob_is_a_saw_square_blend() {
        let mut rng = 1;
        assert_eq!(osc::morph(SAW, 0.25, &mut rng), -0.5, "plain saw");
        assert_eq!(osc::morph(SAW + 0.5, 0.25, &mut rng), 0.25, "half square");
        assert_eq!(osc::morph(3.0, 0.25, &mut rng), 1.0, "square");
    }

    #[test]
    fn glide_slides_and_legato_keeps_the_envelope() {
        let mut values = PARAMS.map(|p| p.default);
        values[GLIDE] = 0.3;
        let mut v = Voice::new(48_000.0);
        v.set_params(&values);
        v.note_on(48, true, true);
        assert_eq!(v.freq, midi_to_hz(48), "nothing to glide from yet");
        for _ in 0..4_800 {
            v.next_sample(0.0);
        }
        let amp = v.env.amp;
        v.note_on(60, true, false);
        v.next_sample(0.0);
        assert!(v.freq < midi_to_hz(54), "still near C3: {}", v.freq);
        assert!(v.env.amp >= amp * 0.99, "legato: no restart");
        for _ in 0..48_000 {
            v.next_sample(0.0);
        }
        assert!((v.freq - midi_to_hz(60)).abs() < 1.0, "arrives at C4");
    }

    #[test]
    fn a_random_lfo_on_pitch_jumps_by_semitones() {
        let mut values = open_filter();
        values[WAVE] = 0.0;
        values[LFO_SHAPE] = 4.0;
        values[LFO_AMOUNT] = 1.0;
        values[LFO_RATE] = 10.0;
        let mut lfo = Lfo::default();
        let step = Lfo::step(&values, 48_000.0, 24_000.0);
        let picks: Vec<f32> = (0..48_000).map(|_| lfo.next(&values, step)).collect();
        let mut distinct: Vec<i32> = picks.iter().map(|p| (p * 1000.0) as i32).collect();
        distinct.dedup();
        assert!(
            distinct.len() >= 9,
            "a new value each cycle: {}",
            distinct.len()
        );
        // Synced to 1/4 at 120 BPM (a beat is 24_000 samples): 2 Hz.
        values[LFO_SYNC] = 3.0;
        assert!((Lfo::step(&values, 48_000.0, 24_000.0) * 48_000.0 - 2.0).abs() < 1e-3);
    }

    #[test]
    fn knob_names_are_unique_and_every_module_is_laid_out() {
        for (i, a) in PARAMS.iter().enumerate() {
            assert!(
                PARAMS[i + 1..].iter().all(|b| b.name != a.name),
                "two knobs called {}",
                a.name
            );
        }
        for m in MODULES {
            for row in m.rows {
                assert!(row.iter().all(|&k| k < m.knobs.len()), "{}", m.name);
            }
            let at = first_knob(m);
            assert_eq!(PARAMS[at].name, m.knobs[0].name);
        }
        assert_eq!(PARAMS[CUTOFF].name, "Cutoff");
        assert_eq!(PARAMS[ARP_LATCH].name, "Latch");
    }

    /// The modules play every preset, and odd settings, exactly as the
    /// synth did before it was split into modules.
    #[test]
    fn modules_sound_exactly_like_the_old_synth() {
        use legacy::{Lfo as OldLfo, Voice as OldVoice, PARAMS as OLD_PARAMS};
        let sr = 48_000.0;
        // The old synth only had the analog oscillators.
        let mut patches: Vec<Values> = (0..PRESETS.len())
            .map(preset)
            .filter(|v| engine(v) == ANALOG)
            .collect();
        for (i, p) in patches.clone().iter().enumerate() {
            let mut p = *p;
            p[LFO_AMOUNT] = 0.6;
            p[LFO_TARGET] = (i % 4) as f32;
            p[LFO_SHAPE] = (i % 5) as f32;
            p[LFO_SYNC] = (i % 3) as f32;
            p[GLIDE] = 0.1 * (i % 2) as f32;
            p[RESONANCE] = 0.9;
            patches.push(p);
        }
        for (n, values) in patches.iter().enumerate() {
            // The same knobs, found by name in the old order.
            let mut old = OLD_PARAMS.map(|p| p.default);
            for (o, p) in old.iter_mut().zip(OLD_PARAMS.iter()) {
                let k = PARAMS.iter().position(|q| q.name == p.name).unwrap();
                *o = values[k];
            }
            let (mut new_voice, mut old_voice) = (Voice::new(sr), OldVoice::new(sr));
            new_voice.set_params(values);
            old_voice.set_params(&old);
            let (mut new_lfo, mut old_lfo) = (Lfo::default(), OldLfo::default());
            let new_step = Lfo::step(values, sr, 20_000.0);
            let old_step = OldLfo::step(&old, sr, 20_000.0);
            for s in 0..30_000 {
                match s {
                    0 => {
                        new_voice.note_on(57, true, true);
                        old_voice.note_on(57, true, true);
                    }
                    6_000 => {
                        new_voice.note_on(64, true, false);
                        old_voice.note_on(64, true, false);
                    }
                    9_000 => {
                        new_voice.bend(0.7, 1.5);
                        old_voice.bend(0.7, 1.5);
                    }
                    16_000 => {
                        new_voice.note_off();
                        old_voice.note_off();
                    }
                    _ => {}
                }
                let a = new_voice.next_sample(new_lfo.next(values, new_step));
                let b = old_voice.next_sample(old_lfo.next(&old, old_step));
                assert_eq!(a.to_bits(), b.to_bits(), "patch {n}, sample {s}");
            }
            assert_eq!(new_voice.is_idle(), old_voice.is_idle());
        }
    }

    /// The pitch of a sound: the gap (40 Hz to 2 kHz) at which it best
    /// matches itself.
    fn pitch_of(out: &[f32]) -> f32 {
        let score = |lag: usize| -> f32 {
            let n = out.len() - lag;
            out[..n]
                .iter()
                .zip(&out[lag..])
                .map(|(a, b)| a * b)
                .sum::<f32>()
        };
        let best = (24..1_200)
            .max_by(|&a, &b| score(a).total_cmp(&score(b)))
            .unwrap();
        48_000.0 / best as f32
    }

    #[test]
    fn a_plucked_string_rings_in_tune_and_dies_away() {
        let mut v = preset(
            PRESETS
                .iter()
                .position(|p| p.0.starts_with("Nylon"))
                .unwrap(),
        );
        v[STRING_DECAY] = 0.5;
        for note in [45, 57, 69] {
            let out = play(&v, note, 48_000);
            let hz = pitch_of(&out[2_400..12_000]);
            let want = midi_to_hz(note);
            assert!(
                (hz / want - 1.0).abs() < 0.03,
                "note {note}: {hz} Hz, want {want}"
            );
            let loud = |o: &[f32]| o.iter().map(|s| s.abs()).sum::<f32>();
            let (start, end) = (loud(&out[..4_800]), loud(&out[43_200..]));
            assert!(end < start * 0.05, "dies away: {start} then {end}");
        }
    }

    #[test]
    fn a_mallet_rings_at_its_note_and_materials_differ() {
        let mut v = preset(
            PRESETS
                .iter()
                .position(|p| p.0.starts_with("Marimba"))
                .unwrap(),
        );
        v[HARDNESS] = 0.0; // soft: mostly the main partial
        let out = play(&v, 60, 24_000);
        let hz = pitch_of(&out[480..12_000]);
        assert!((hz / midi_to_hz(60) - 1.0).abs() < 0.03, "{hz} Hz");
        let mut bell = v;
        bell[MATERIAL] = 3.0;
        bell[HARDNESS] = 1.0;
        assert_ne!(play(&bell, 60, 4_800), play(&v, 60, 4_800));
        let quiet = play(&v, 60, 96_000)[90_000..]
            .iter()
            .fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(quiet < 0.01, "a short ring dies: {quiet}");
    }

    #[test]
    fn fm_plays_in_tune_and_mellows_as_it_rings() {
        let v = preset(
            PRESETS
                .iter()
                .position(|p| p.0 == "FM electric piano")
                .unwrap(),
        );
        let out = play(&v, 57, 48_000);
        let hz = pitch_of(&out[480..10_000]);
        assert!((hz / midi_to_hz(57) - 1.0).abs() < 0.03, "{hz} Hz");
        // Brightness: how much the wave jumps from sample to sample,
        // relative to how loud it is.
        let bright = |o: &[f32]| {
            let jumps: f32 = o.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
            jumps / o.iter().map(|s| s.abs()).sum::<f32>()
        };
        let mut deep = v;
        deep[FM_DEPTH] = 1.0;
        deep[FM_DECAY] = 0.2;
        deep[SUSTAIN] = 1.0;
        deep[LFO_AMOUNT] = 0.0;
        let out = play(&deep, 45, 48_000);
        let (early, late) = (bright(&out[200..2_600]), bright(&out[40_000..42_400]));
        assert!(early > late * 1.2, "mellows: {early} then {late}");
    }
}
