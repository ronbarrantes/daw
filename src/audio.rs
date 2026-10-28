//! Audio engine: runs the sequencer and mixes every track.
//!
//! The audio callback never waits on a lock. Transport state lives in
//! atomics; the project (tracks and patterns) arrives as an immutable
//! snapshot that the engine picks up with `try_lock`; one-off requests
//! like "play this note now" arrive on a channel. Outgoing MIDI is queued
//! on a bounded channel and written by the MIDI thread.
//!
//! Every track has a pool of voices, so it can play chords and overlapping
//! notes; a note played again takes over its own voice, and when all are
//! busy the oldest is stolen. Steps start on MIDI clock ticks, but each hit is
//! scheduled to the exact sample, so swing, nudge, humanize and ratchets
//! aren't limited to the clock's resolution.
//!
//! The mix is stereo: each track is panned with an equal-power law.
//!
//! Tracks also send to a shared reverb and delay (see `fx`), which run
//! only while something is feeding them or their tail is still ringing.
//!
//! The metronome clicks on every beat. With a count-in requested, Play
//! first clicks one bar before the sequencer and MIDI Start begin.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicUsize, Ordering::Relaxed};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};

#[cfg(not(target_arch = "wasm32"))]
use cpal::traits::StreamTrait;
use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use crate::fx::{Delay, Limiter, Reverb};
use crate::midi::{self, Msg};
use crate::project::{
    Block, Kind, MidiPart, Project, Sample, Step, Target, MAX_TRACKS, NORMAL, PPS, SAMPLE_ROOT,
    STEPS,
};
use crate::synth::{self, modulation, Lfo, ModEnv, Mode, Source, Voice};

/// MIDI clock runs at 24 ticks per beat, so a 16th-note step is 6 ticks.
const TICKS_PER_STEP: u32 = 6;
/// Marks "no pattern" (a silent part of the song).
const NONE: u8 = u8::MAX;
/// `Shared::seek` when there's nowhere to jump to.
pub const NO_SEEK: usize = usize::MAX;
/// Voices per track: how many notes it can sound at once.
const POLYPHONY: usize = 16;
/// Notes the arpeggiator can hold at once.
const ARP_MAX: usize = 16;
/// How long a clicked note sounds, in seconds.
const PREVIEW_SECONDS: f32 = 0.3;
/// How often the engine checks for a new project and events, in samples.
const UPDATE_INTERVAL: u32 = 64;
/// Notes last this fraction of their step (or ratchet slice).
const GATE: f32 = 0.8;
/// Room for scheduled hits; enough for busy patterns and long MIDI notes.
const MAX_PENDING: usize = 1024;
/// Humanize at full strength: up to this fraction of a step late...
const HUMANIZE_TIMING: f32 = 0.12;
/// ...and up to this much velocity either way.
const HUMANIZE_VELOCITY: f32 = 30.0;
/// MIDI clock ticks in a beat and in a bar of 4/4.
const TICKS_PER_BEAT: u32 = 24;
const TICKS_PER_BAR: u32 = TICKS_PER_STEP * STEPS as u32;
/// Metronome click pitches: higher on the first beat of the bar.
const CLICK_HIGH: f32 = 1_600.0;
const CLICK_LOW: f32 = 1_000.0;
const CLICK_LEVEL: f32 = 0.35;
/// How fast a click fades, in seconds to fall to about a third.
const CLICK_DECAY: f32 = 0.015;
/// The effects keep running this long after their input goes quiet, so
/// long reverb tails and echoes can finish.
const FX_TAIL_SECONDS: f32 = 10.0;
/// How often the engine reports levels to the meter, in samples.
const METER_INTERVAL: u32 = 256;

/// Requests from the UI or MIDI in.
pub enum Event {
    /// Briefly play a note on a track, e.g. when clicking a cell.
    Preview {
        track: usize,
        note: u8,
    },
    /// Start a note that lasts until `Release`, from the computer keyboard
    /// or a MIDI controller. `note` is a MIDI note number.
    Hold {
        track: usize,
        note: u8,
        velocity: u8,
    },
    Release {
        track: usize,
        note: u8,
    },
    /// Slides a held note's pitch smoothly, off the 12-note grid, and moves
    /// its filter: `semis` from the note (fractions allowed), `filter` in
    /// octaves. For the Drone's pitch slider and the theremin pad.
    Bend {
        track: usize,
        note: u8,
        semis: f32,
        filter: f32,
    },
}

pub struct Shared {
    /// The latest project snapshot from the UI.
    project: Mutex<Arc<Project>>,
    bpm_bits: AtomicU32,
    pub playing: AtomicBool,
    pub current_step: AtomicUsize,
    /// How many bars the pattern being heard has played, so far in a row:
    /// a track's part plays its bar number this modulo its length.
    pub current_bar: AtomicUsize,
    /// Play the song arrangement instead of looping the edited pattern.
    pub song_mode: AtomicBool,
    /// Outside Song mode, play the chain of the block being edited instead
    /// of looping one pattern.
    pub chain_mode: AtomicBool,
    /// The block shown in the Jam view.
    pub edit_block: AtomicUsize,
    /// The pattern shown in the editor.
    pub edit_pattern: AtomicU8,
    /// The pattern being heard, or `NONE` when nothing is.
    pub play_pattern: AtomicU8,
    /// The timeline step being heard (in Song mode), in 16ths.
    pub song_step: AtomicUsize,
    /// A timeline step to jump to, or `NO_SEEK`.
    pub seek: AtomicUsize,
    /// Song mode repeats the steps from `loop_start` up to `loop_end`.
    pub loop_on: AtomicBool,
    pub loop_start: AtomicUsize,
    pub loop_end: AtomicUsize,
    /// The track selected in the UI, which MIDI controllers play by default.
    pub selected_track: AtomicUsize,
    /// Follow incoming MIDI clock instead of the BPM setting.
    pub external_sync: AtomicBool,
    /// Clock ticks received from MIDI in, not yet handled by the engine.
    pub ext_ticks: AtomicU32,
    /// Set when MIDI in receives Start, so the engine rewinds to step 1.
    pub ext_start: AtomicBool,
    /// Click on every beat while playing.
    pub metronome: AtomicBool,
    /// Click one bar before playback starts (used while recording).
    pub count_in: AtomicBool,
    /// The count-in bar is playing; the sequencer hasn't started yet.
    pub counting_in: AtomicBool,
    /// The oscilloscope: the selected track's latest samples, before its
    /// volume and pan, as `f32` bits in a ring. Only written while on.
    pub scope_on: AtomicBool,
    scope: Box<[AtomicU32]>,
    /// Where the next scope sample goes; it only counts up.
    scope_pos: AtomicUsize,
    /// The engine's sample rate, for the scope's time scale.
    pub sample_rate: AtomicU32,
    /// The loudest left and right samples since the UI last looked, as
    /// `f32` bits (`take_meter` reads and resets them).
    meter: [AtomicU32; 2],
    /// The most the limiter turned the mix down since the UI last looked,
    /// 0-1, as `f32` bits.
    limiting: AtomicU32,
    /// Where modulation has the selected track's knobs right now (in its
    /// newest sounding voice), as `f32` bits, so knobs can show it moving.
    mod_live: Vec<AtomicU32>,
    /// `mod_live` is up to date (the track has modulation and a voice is
    /// sounding).
    mod_live_on: AtomicBool,
    /// Where the selected track's LFO 1 and LFO 2 are in their cycles,
    /// 0-1, as `f32` bits, for drawing them.
    lfo_phase: [AtomicU32; 2],
}

/// How many samples the scope keeps: about 85 ms at 48 kHz.
pub const SCOPE_LEN: usize = 4096;

impl Shared {
    pub fn new(project: Arc<Project>) -> Self {
        Self {
            project: Mutex::new(project),
            bpm_bits: AtomicU32::new(120f32.to_bits()),
            playing: AtomicBool::new(false),
            current_step: AtomicUsize::new(0),
            current_bar: AtomicUsize::new(0),
            song_mode: AtomicBool::new(false),
            chain_mode: AtomicBool::new(false),
            edit_block: AtomicUsize::new(0),
            edit_pattern: AtomicU8::new(0),
            play_pattern: AtomicU8::new(0),
            song_step: AtomicUsize::new(0),
            seek: AtomicUsize::new(NO_SEEK),
            loop_on: AtomicBool::new(false),
            loop_start: AtomicUsize::new(0),
            loop_end: AtomicUsize::new(0),
            selected_track: AtomicUsize::new(0),
            external_sync: AtomicBool::new(false),
            ext_ticks: AtomicU32::new(0),
            ext_start: AtomicBool::new(false),
            metronome: AtomicBool::new(false),
            count_in: AtomicBool::new(false),
            counting_in: AtomicBool::new(false),
            scope_on: AtomicBool::new(false),
            scope: (0..SCOPE_LEN).map(|_| AtomicU32::new(0)).collect(),
            scope_pos: AtomicUsize::new(0),
            sample_rate: AtomicU32::new(48_000),
            meter: [AtomicU32::new(0), AtomicU32::new(0)],
            limiting: AtomicU32::new(0),
            mod_live: (0..synth::PARAMS.len())
                .map(|_| AtomicU32::new(0))
                .collect(),
            mod_live_on: AtomicBool::new(false),
            lfo_phase: [AtomicU32::new(0), AtomicU32::new(0)],
        }
    }

    /// Where the selected track's LFO `n` (0 or 1) is in its cycle.
    pub fn lfo_phase(&self, n: usize) -> f32 {
        f32::from_bits(self.lfo_phase[n].load(Relaxed))
    }

    /// Where modulation has the selected track's knob `k` now, if it's
    /// moving it.
    pub fn mod_live(&self, k: usize) -> Option<f32> {
        if !self.mod_live_on.load(Relaxed) {
            return None;
        }
        self.mod_live
            .get(k)
            .map(|v| f32::from_bits(v.load(Relaxed)))
    }

    /// Copies the newest `out.len()` scope samples into `out`, oldest first.
    pub fn read_scope(&self, out: &mut [f32]) {
        let n = out.len().min(SCOPE_LEN);
        let end = self.scope_pos.load(Relaxed);
        for (i, v) in out[..n].iter_mut().enumerate() {
            let at = (end + SCOPE_LEN - n + i) % SCOPE_LEN;
            *v = f32::from_bits(self.scope[at].load(Relaxed));
        }
    }

    fn write_scope(&self, value: f32) {
        let pos = self.scope_pos.load(Relaxed);
        self.scope[pos % SCOPE_LEN].store(value.to_bits(), Relaxed);
        self.scope_pos.store(pos.wrapping_add(1), Relaxed);
    }

    /// The peak left and right levels and how much the limiter worked
    /// since the last call (all 0-1).
    pub fn take_meter(&self) -> ([f32; 2], f32) {
        let peak = self
            .meter
            .each_ref()
            .map(|m| f32::from_bits(m.swap(0, Relaxed)));
        (peak, f32::from_bits(self.limiting.swap(0, Relaxed)))
    }

    /// Positive floats keep their order as bits, so `fetch_max` works.
    fn report_meter(&self, peak: [f32; 2], limiting: f32) {
        for (m, p) in self.meter.iter().zip(peak) {
            m.fetch_max(p.max(0.0).to_bits(), Relaxed);
        }
        self.limiting
            .fetch_max(limiting.max(0.0).to_bits(), Relaxed);
    }

    pub fn project(&self) -> Arc<Project> {
        self.project.lock().unwrap().clone()
    }

    /// Hands a new snapshot to the engine. The caller should keep recent
    /// snapshots alive for a moment, so the audio thread is never the one
    /// that frees them.
    pub fn publish(&self, project: Arc<Project>) {
        *self.project.lock().unwrap() = project;
    }

    pub fn bpm(&self) -> f32 {
        f32::from_bits(self.bpm_bits.load(Relaxed))
    }

    pub fn set_bpm(&self, bpm: f32) {
        self.bpm_bits.store(bpm.to_bits(), Relaxed);
    }
}

/// Left and right gains for a pan position (-1 to 1). Equal power, scaled
/// so the centre is unity on both sides, like the old mono mix.
fn pan_gains(pan: f32) -> [f32; 2] {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    let s = std::f32::consts::SQRT_2;
    [angle.cos() * s, angle.sin() * s]
}

/// Loudness for a MIDI velocity; 100 is unity, so ghost notes (40) are
/// about a quarter as loud and accents (127) about 40% louder.
fn velocity_gain(velocity: u8) -> f32 {
    (velocity as f32 / NORMAL as f32).powf(1.5)
}

/// One of a track's voices: plays one note at a time.
struct TrackVoice {
    synth: Voice,
    /// The sample being played, if any. Holding the `Arc` keeps it alive
    /// even if the track is deleted mid-note.
    sample: Option<Arc<Sample>>,
    pos: f64,
    rate: f64,
    gain: f32,
    /// The part of the sample it plays, in samples, and its fade: `env`
    /// rises by `attack_step` each sample, and once released (unless it
    /// rings out) falls by `release_coef` of itself.
    begin: f64,
    end: f64,
    env: f32,
    attack_step: f32,
    release_coef: f32,
    ring: bool,
    releasing: bool,
    /// The note it's playing (or last played).
    note: u8,
    /// Which note this is, counting every note the engine starts, so a
    /// scheduled note-off only ends the note it belongs to.
    generation: u32,
    /// The note is still held (not released yet).
    on: bool,
    /// MIDI note (and channel) still on.
    held: Option<(u8, u8)>,
    /// A keyboard or controller is holding this note; the sequencer leaves
    /// the note alone while it's set.
    live: bool,
    /// The mod envelope, and how hard the note was played (0-1), as
    /// modulation sources.
    mod_env: ModEnv,
    velocity: f32,
}

impl TrackVoice {
    fn new(sr: f32) -> Self {
        Self {
            synth: Voice::new(sr),
            sample: None,
            pos: 0.0,
            rate: 1.0,
            gain: 1.0,
            begin: 0.0,
            end: 0.0,
            env: 1.0,
            attack_step: 1.0,
            release_coef: 0.0,
            ring: true,
            releasing: false,
            note: 0,
            generation: 0,
            on: false,
            held: None,
            live: false,
            mod_env: ModEnv::default(),
            velocity: 0.0,
        }
    }

    /// Released and silent, so free for a new note.
    fn is_free(&self) -> bool {
        !self.on && self.synth.is_idle() && self.sample.is_none()
    }

    fn sample_out(&mut self) -> f32 {
        let Some(s) = &self.sample else { return 0.0 };
        let i = self.pos as usize;
        // Stop at the trimmed end (or start, playing backwards).
        if self.pos < self.begin || self.pos + 1.0 >= self.end || i + 1 >= s.data.len() {
            self.sample = None;
            return 0.0;
        }
        // Linear interpolation, so pitched-up or -down samples stay smooth.
        let frac = (self.pos - i as f64) as f32;
        self.pos += self.rate;
        let out = s.data[i] + (s.data[i + 1] - s.data[i]) * frac;
        if self.releasing && !self.ring {
            self.env -= self.env * self.release_coef;
            if self.env < 1e-4 {
                self.sample = None;
            }
        } else if self.env < 1.0 {
            self.env = (self.env + self.attack_step).min(1.0);
        }
        out * self.env
    }
}

/// The metronome's sound: a short sine blip.
struct Click {
    phase: f32,
    step: f32,
    level: f32,
    fade: f32,
}

impl Click {
    fn new(sr: f32) -> Self {
        Self {
            phase: 0.0,
            step: 0.0,
            level: 0.0,
            fade: (-1.0 / (CLICK_DECAY * sr)).exp(),
        }
    }

    fn trigger(&mut self, freq: f32, sr: f32) {
        self.phase = 0.0;
        self.step = freq / sr * std::f32::consts::TAU;
        self.level = CLICK_LEVEL;
    }

    fn next_sample(&mut self) -> f32 {
        if self.level < 1e-4 {
            return 0.0;
        }
        let out = self.phase.sin() * self.level;
        self.phase = (self.phase + self.step) % std::f32::consts::TAU;
        self.level *= self.fade;
        out
    }
}

/// A note the arpeggiator is holding: played by the sequencer, a MIDI
/// block or live.
#[derive(Clone, Copy, Default)]
struct ArpNote {
    note: u8,
    velocity: u8,
    /// Ends it, like a voice's generation, when the sequencer's note ends.
    generation: u32,
    live: bool,
    /// Let go, but Latch keeps it playing until a new chord starts.
    released: bool,
}

/// A track's arpeggiator: the held notes, in the order they came, and
/// where it is in its pattern.
#[derive(Clone, Copy, Default)]
struct Arp {
    held: [ArpNote; ARP_MAX],
    count: usize,
    pos: usize,
    /// Samples until the next note.
    countdown: f32,
}

impl Arp {
    fn remove(&mut self, matches: impl Fn(&ArpNote) -> bool) -> bool {
        let Some(k) = self.held[..self.count].iter().position(matches) else {
            return false;
        };
        self.held.copy_within(k + 1..self.count, k);
        self.count -= 1;
        true
    }
}

/// Something the sequencer will do a few samples from now.
#[derive(Clone, Copy)]
struct Pending {
    /// Samples until it happens.
    at: u32,
    track: usize,
    note: u8,
    action: Action,
}

#[derive(Clone, Copy)]
enum Action {
    /// Start the note. `gate` is how long it lasts in samples;
    /// `None` holds it for a tied step to end.
    On { velocity: u8, gate: Option<u32> },
    /// End the note with this generation, if it's still playing.
    Off { generation: u32 },
}

struct Engine {
    shared: Arc<Shared>,
    project: Arc<Project>,
    events: Receiver<Event>,
    midi_out: SyncSender<Msg>,
    /// `voices[track]` is the track's pool of `POLYPHONY` voices.
    voices: Vec<Vec<TrackVoice>>,
    /// The generation the next note gets.
    next_generation: u32,
    /// Each synth track's LFOs, and their values now.
    lfos: Vec<Lfo>,
    lfo2s: Vec<Lfo>,
    lfo_now: [[f32; 2]; MAX_TRACKS],
    /// Each synth track's knobs as set, or where automation has them.
    /// Modulation starts from these.
    knobs: Vec<synth::Values>,
    /// `knobs` with the track-wide modulation (the LFOs) applied, for the
    /// LFOs' own knobs and pan.
    track_knobs: Vec<synth::Values>,
    /// Each synth track's arpeggiator.
    arps: [Arp; MAX_TRACKS],
    /// The generation of each note the sequencer is holding on for tied
    /// steps, by track and note, so the note ends when its ties do, or
    /// when they're erased while it plays (the arpeggiator or a voice may
    /// be holding it).
    tied: [[Option<u32>; 128]; MAX_TRACKS],
    /// Automation is moving knobs right now (the timeline is playing and
    /// has lanes), so stopping puts them back.
    automating: bool,
    /// Which track (by `Track::id`) each slot of `voices` belongs to, so a
    /// slot that gets a different track (one was deleted or added) starts
    /// silent instead of carrying on the old track's notes.
    slot_ids: [Option<u32>; MAX_TRACKS],
    /// Scheduled hits; preallocated so the audio thread never allocates.
    pending: Vec<Pending>,
    /// Clock tick within the bar, 0..TICKS_PER_STEP * STEPS.
    tick: u32,
    samples_to_next_tick: f32,
    /// Current tick length; measured when following an external clock.
    samples_per_tick: f32,
    samples_since_tick: u32,
    samples_to_update: u32,
    /// The clicked note that's sounding (track, generation), and samples
    /// until it stops.
    preview: Option<(usize, u32, u32)>,
    /// Each track's level after mute, solo and group volume; worked out
    /// once per project change rather than every sample. Left and right,
    /// so pan is included.
    gains: [[f32; 2]; MAX_TRACKS],
    /// Each track's level without pan, for when an LFO moves the pan.
    levels: [f32; MAX_TRACKS],
    /// Each track's reverb and delay send levels, after its gain.
    sends: [[f32; 2]; MAX_TRACKS],
    reverb: Reverb,
    delay: Delay,
    limiter: Limiter,
    /// Peaks and limiting gathered for the meter, sent every
    /// `METER_INTERVAL` samples.
    meter_peak: [f32; 2],
    meter_limiting: f32,
    meter_count: u32,
    /// Samples since anything was sent to the effects.
    fx_quiet: u32,
    /// The timeline step playing, in Song mode.
    song_step: usize,
    /// The next step starts the song from the top instead of advancing.
    restart_song: bool,
    /// The bar of the looping pattern, outside Song and Chain mode; `None`
    /// until the first bar starts.
    pattern_bar: Option<usize>,
    /// Song and chain mode last step, to restart when they change.
    was_mode: (bool, bool),
    was_playing: bool,
    click: Click,
    /// Clock ticks of count-in still to go; 0 when not counting in.
    count_in: u32,
    /// xorshift state for chance and humanize.
    rng: u32,
    sr: f32,
}

impl Engine {
    fn new(
        shared: Arc<Shared>,
        events: Receiver<Event>,
        midi_out: SyncSender<Msg>,
        sr: f32,
    ) -> Self {
        // Built here, before any sound, since building allocates.
        synth::warm_up();
        let voices = (0..MAX_TRACKS)
            .map(|_| (0..POLYPHONY).map(|_| TrackVoice::new(sr)).collect())
            .collect();
        let mut engine = Self {
            project: shared.project(),
            shared,
            events,
            midi_out,
            voices,
            next_generation: 1,
            lfos: (0..MAX_TRACKS).map(|_| Lfo::default()).collect(),
            lfo2s: (0..MAX_TRACKS).map(|_| Lfo::default()).collect(),
            lfo_now: [[0.0; 2]; MAX_TRACKS],
            knobs: vec![synth::PARAMS.map(|p| p.default); MAX_TRACKS],
            track_knobs: vec![synth::PARAMS.map(|p| p.default); MAX_TRACKS],
            arps: [Arp::default(); MAX_TRACKS],
            tied: [[None; 128]; MAX_TRACKS],
            slot_ids: [None; MAX_TRACKS],
            automating: false,
            pending: Vec::with_capacity(MAX_PENDING),
            tick: 0,
            samples_to_next_tick: 0.0,
            samples_per_tick: sr * 60.0 / 120.0 / 24.0,
            samples_since_tick: 0,
            samples_to_update: 0,
            preview: None,
            gains: [[0.0; 2]; MAX_TRACKS],
            levels: [0.0; MAX_TRACKS],
            sends: [[0.0; 2]; MAX_TRACKS],
            reverb: Reverb::new(sr),
            delay: Delay::new(sr),
            limiter: Limiter::new(sr),
            meter_peak: [0.0; 2],
            meter_limiting: 0.0,
            meter_count: 0,
            fx_quiet: u32::MAX,
            song_step: 0,
            restart_song: true,
            pattern_bar: None,
            was_mode: (false, false),
            was_playing: false,
            click: Click::new(sr),
            count_in: 0,
            rng: 0x2545_f491,
            sr,
        };
        engine.project_changed();
        engine.shared.sample_rate.store(sr as u32, Relaxed);
        engine
    }

    /// Refreshes everything derived from the project: track levels and
    /// synth settings.
    fn project_changed(&mut self) {
        let project = self.project.clone();
        for i in 0..MAX_TRACKS {
            match project.tracks.get(i) {
                Some(t) => self.set_mix(&project, i, [t.volume, t.pan, t.reverb, t.delay]),
                None => {
                    self.levels[i] = 0.0;
                    self.gains[i] = [0.0; 2];
                    self.sends[i] = [0.0; 2];
                }
            }
        }
        for i in 0..MAX_TRACKS {
            let id = project.tracks.get(i).map(|t| t.id);
            if id != self.slot_ids[i] {
                self.silence(i);
                self.slot_ids[i] = id;
            }
            if !self.arp_on(i) {
                self.arps[i].count = 0;
            }
            // Latch off: notes already let go stop.
            let latch = match project.tracks.get(i).map(|t| &t.kind) {
                Some(Kind::Synth(v)) => synth::choice(v[synth::ARP_LATCH], 2) == 1,
                _ => false,
            };
            if !latch {
                while self.arps[i].remove(|h| h.released) {}
            }
        }
        for (i, (lanes, track)) in self.voices.iter_mut().zip(&project.tracks).enumerate() {
            if let Kind::Synth(values) = &track.kind {
                self.knobs[i] = *values;
                self.track_knobs[i] = *values;
                for voice in lanes {
                    voice.synth.set_params(values);
                    // In case the track was a sampler a moment ago.
                    voice.sample = None;
                }
            }
        }
    }

    fn track_count(&self) -> usize {
        self.project.tracks.len().min(MAX_TRACKS)
    }

    /// A random number in 0..1.
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    /// Never blocks: if the MIDI thread falls behind, the message is dropped.
    fn send(&self, msg: Msg) {
        let _ = self.midi_out.try_send(msg);
    }

    /// Never allocates: if the queue is full, the hit is dropped.
    fn schedule(&mut self, at: f32, track: usize, note: u8, action: Action) {
        if self.pending.len() < MAX_PENDING {
            let at = at.max(0.0) as u32;
            self.pending.push(Pending {
                at,
                track,
                note,
                action,
            });
        }
    }

    /// The note a track really plays for `note`: one-shot tracks always
    /// play (and send) their own note, e.g. 36 for a kick.
    fn key(&self, i: usize, note: u8) -> u8 {
        match self.project.tracks.get(i) {
            Some(t) if !t.is_melodic() => t.midi_note,
            _ => note,
        }
    }

    /// The voice of track `i` holding `note`, if any (and if `live`, only
    /// one a keyboard or controller is holding).
    fn holding(&self, i: usize, note: u8, live: bool) -> Option<usize> {
        self.voices[i]
            .iter()
            .position(|v| v.on && v.note == note && (v.live || !live))
    }

    /// Which voice plays a new note: the one already playing that note (as
    /// on a drum machine; MIDI can't have a note on twice anyway), else a
    /// free one, else the oldest released one, else the oldest.
    fn pick_voice(&self, i: usize, note: u8) -> usize {
        let voices = &self.voices[i];
        let age = |v: &TrackVoice| self.next_generation.wrapping_sub(v.generation);
        let oldest = |released: bool| {
            (0..voices.len())
                .filter(|&v| !released || !voices[v].on)
                .max_by_key(|&v| age(&voices[v]))
        };
        voices
            .iter()
            .position(|v| v.note == note && !v.is_free())
            .or_else(|| voices.iter().position(TrackVoice::is_free))
            .or_else(|| oldest(true))
            .or_else(|| oldest(false))
            .unwrap_or(0)
    }

    /// Starts MIDI note `note` on a track and returns its generation.
    fn note_on(&mut self, i: usize, note: u8, velocity: u8) -> u32 {
        let note = self.key(i, note);
        let project = self.project.clone();
        let track = &project.tracks[i];
        let mode = match &track.kind {
            Kind::Synth(values) => synth::mode(values),
            Kind::Sampler { .. } => Mode::Poly,
        };
        // Mono and Legato play everything on the first voice.
        let v = match mode {
            Mode::Poly => self.pick_voice(i, note),
            _ => 0,
        };
        let legato = mode == Mode::Legato && self.voices[i][v].on;
        if legato {
            // The sound carries on; only the MIDI note changes.
            if let Some((held, ch)) = self.voices[i][v].held.take() {
                self.send(Msg::three(0x80 | ch, held, 0));
            }
        } else {
            self.voice_off(i, v);
        }
        if let Some(ch) = track.midi_channel {
            self.send(Msg::three(0x90 | ch, note, velocity));
            self.voices[i][v].held = Some((note, ch));
        }
        let generation = self.next_generation;
        self.next_generation = generation.wrapping_add(1);
        let voice = &mut self.voices[i][v];
        voice.gain = velocity_gain(velocity);
        voice.note = note;
        voice.generation = generation;
        voice.on = true;
        voice.live = false;
        match &track.kind {
            Kind::Synth(values) => {
                voice.synth.note_on(note, mode != Mode::Poly, !legato);
                if !legato {
                    voice.mod_env.trigger();
                    // Retrigger: the track's LFOs start their cycle again.
                    if synth::choice(values[synth::LFO_RETRIG], 2) == 1 {
                        self.lfos[i].restart();
                    }
                    if synth::choice(values[synth::LFO2_RETRIG], 2) == 1 {
                        self.lfo2s[i].restart();
                    }
                }
                voice.velocity = velocity as f32 / 127.0;
            }
            Kind::Sampler {
                sample,
                pitched,
                shape,
            } => {
                let semis = if *pitched {
                    note as f32 - SAMPLE_ROOT as f32
                } else {
                    0.0
                };
                let rate = (sample.rate / self.sr * 2f32.powf(semis / 12.0)) as f64;
                let len = sample.data.len() as f64;
                let (a, b) = (shape.start.min(shape.end), shape.start.max(shape.end));
                voice.begin = (a as f64 * len).floor();
                voice.end = (b as f64 * len).min(len);
                (voice.pos, voice.rate) = if shape.reverse {
                    (voice.end - 2.0, -rate)
                } else {
                    (voice.begin, rate)
                };
                voice.env = if shape.attack > 0.0 { 0.0 } else { 1.0 };
                voice.attack_step = 1.0 / (shape.attack * self.sr).max(1.0);
                voice.release_coef = 1.0 - (-1.0 / (shape.release.max(0.005) * self.sr)).exp();
                voice.ring = shape.ring;
                voice.releasing = false;
                voice.sample = Some(sample.clone());
            }
        }
        generation
    }

    /// Ends a voice's note. Samples keep ringing to their end; synths release.
    fn voice_off(&mut self, i: usize, v: usize) {
        let voice = &mut self.voices[i][v];
        voice.on = false;
        voice.live = false;
        voice.releasing = true;
        voice.synth.note_off();
        voice.mod_env.release();
        if let Some((note, ch)) = voice.held.take() {
            self.send(Msg::three(0x80 | ch, note, 0));
        }
    }

    /// Stops everything in voice slot `i` at once, held and live notes and
    /// release tails too, and forgets its arpeggiator's notes.
    fn silence(&mut self, i: usize) {
        for v in 0..POLYPHONY {
            if let Some((note, ch)) = self.voices[i][v].held.take() {
                self.send(Msg::three(0x80 | ch, note, 0));
            }
            let voice = &mut self.voices[i][v];
            voice.synth = Voice::new(self.sr);
            voice.sample = None;
            voice.on = false;
            voice.live = false;
        }
        self.arps[i].count = 0;
        self.tied[i] = [None; 128];
    }

    /// Whether track `i` is a synth with its arpeggiator on.
    fn arp_on(&self, i: usize) -> bool {
        match self.project.tracks.get(i).map(|t| &t.kind) {
            Some(Kind::Synth(v)) => synth::choice(v[synth::ARP_MODE], synth::ARP_MODES.len()) > 0,
            _ => false,
        }
    }

    /// Gives the arpeggiator a note to hold and returns the generation that
    /// lets it go. The first note starts the pattern right away.
    fn arp_add(&mut self, i: usize, note: u8, velocity: u8, live: bool) -> u32 {
        let generation = self.next_generation;
        self.next_generation = generation.wrapping_add(1);
        let arp = &mut self.arps[i];
        // With Latch, the first note after letting go of everything starts
        // a new chord.
        if arp.held[..arp.count].iter().all(|h| h.released) {
            arp.count = 0;
        }
        arp.remove(|h| h.note == note);
        if arp.count == ARP_MAX {
            arp.remove(|_| true);
        }
        if arp.count == 0 {
            // Plays on the next sample.
            arp.pos = 0;
            arp.countdown = 1.0;
        }
        arp.held[arp.count] = ArpNote {
            note,
            velocity,
            generation,
            live,
            released: false,
        };
        arp.count += 1;
        generation
    }

    /// Lets go of an arpeggiator note: with Latch on it keeps playing
    /// (until a new chord), otherwise it's dropped. Returns whether one
    /// matched.
    fn arp_release(&mut self, i: usize, matches: impl Fn(&ArpNote) -> bool) -> bool {
        let latch = match self.project.tracks.get(i).map(|t| &t.kind) {
            Some(Kind::Synth(v)) => synth::choice(v[synth::ARP_LATCH], 2) == 1,
            _ => false,
        };
        let arp = &mut self.arps[i];
        if latch {
            let n = arp.count;
            match arp.held[..n].iter_mut().find(|h| !h.released && matches(h)) {
                Some(h) => {
                    h.released = true;
                    true
                }
                None => false,
            }
        } else {
            arp.remove(matches)
        }
    }

    /// Plays the arpeggiator's next note on each track whose time has come.
    fn run_arps(&mut self) {
        let step_len = self.samples_per_tick * TICKS_PER_STEP as f32;
        for i in 0..self.track_count() {
            if self.arps[i].count == 0 {
                continue;
            }
            self.arps[i].countdown -= 1.0;
            if self.arps[i].countdown > 0.0 {
                continue;
            }
            let Kind::Synth(v) = &self.project.tracks[i].kind else {
                continue;
            };
            let mode = synth::choice(v[synth::ARP_MODE], synth::ARP_MODES.len());
            let rate = synth::choice(v[synth::ARP_RATE], synth::ARP_RATES.len());
            let octaves = synth::choice(v[synth::ARP_OCTAVES], 4) + 1;
            let gate = v[synth::ARP_GATE];
            let every = synth::ARP_RATE_STEPS[rate] * step_len;
            // The held notes, lowest first unless it's As played; or a
            // chord or scale on the lowest one.
            let arp = self.arps[i];
            let mut n = arp.count;
            let mut order = [(0u8, 0u8); ARP_MAX];
            for (o, h) in order.iter_mut().zip(&arp.held[..n]) {
                *o = (h.note, h.velocity);
            }
            if mode != 5 {
                order[..n].sort_unstable_by_key(|o| o.0);
            }
            let set = synth::choice(v[synth::ARP_NOTES], synth::ARP_NOTE_SETS.len());
            if set > 0 {
                let root = *order[..n].iter().min_by_key(|o| o.0).expect("a note");
                let intervals = synth::ARP_INTERVALS[set - 1];
                for (o, step) in order.iter_mut().zip(intervals) {
                    *o = (root.0.saturating_add(*step).min(127), root.1);
                }
                n = intervals.len();
            }
            let len = n * octaves;
            let at = arp.pos % len.max(1);
            let index = match mode {
                2 => len - 1 - at,
                3 if len > 1 => {
                    // Up then back down, without repeating the ends.
                    let p = arp.pos % (2 * len - 2);
                    if p < len {
                        p
                    } else {
                        2 * len - 2 - p
                    }
                }
                4 => (self.random() * len as f32) as usize % len,
                _ => at,
            };
            let (note, velocity) = order[index % n];
            let note = (note as usize + 12 * (index / n)).min(127) as u8;
            let generation = self.note_on(i, note, velocity);
            let off = Action::Off { generation };
            self.schedule((every * gate).max(1.0), i, note, off);
            let arp = &mut self.arps[i];
            arp.pos = arp.pos.wrapping_add(1);
            arp.countdown += every;
        }
    }

    /// Ends the note with this generation, if it's still held and nobody
    /// is playing it live.
    fn generation_off(&mut self, i: usize, generation: u32) {
        if i < MAX_TRACKS && self.arp_release(i, |h| h.generation == generation) {
            return;
        }
        let v = self.voices[i]
            .iter()
            .position(|v| v.on && !v.live && v.generation == generation);
        if let Some(v) = v {
            self.voice_off(i, v);
        }
    }

    fn all_notes_off(&mut self) {
        self.pending.clear();
        self.tied = [[None; 128]; MAX_TRACKS];
        for arp in &mut self.arps {
            while arp.remove(|h| !h.live) {}
        }
        for i in 0..MAX_TRACKS {
            for v in 0..POLYPHONY {
                if !self.voices[i][v].live {
                    self.voice_off(i, v);
                }
            }
        }
    }

    /// Picks up a new project snapshot and UI or MIDI events.
    /// Works out track `i`'s levels from its volume, pan and sends (the
    /// knobs, or where automation has them).
    fn set_mix(&mut self, project: &Project, i: usize, [volume, pan, reverb, delay]: [f32; 4]) {
        let gain = project.gain_with(i, volume);
        self.levels[i] = gain;
        self.gains[i] = pan_gains(pan).map(|g| g * gain);
        self.sends[i] = [reverb * gain, delay * gain];
    }

    /// While the timeline plays, sets the knobs automation lanes move to
    /// where their lines are now. When it stops, puts them back.
    fn apply_automation(&mut self) {
        let project = self.project.clone();
        let running = self.shared.song_mode.load(Relaxed)
            && self.shared.playing.load(Relaxed)
            && !project.automation.is_empty();
        if !running {
            if self.automating {
                self.automating = false;
                self.project_changed();
            }
            return;
        }
        self.automating = true;
        // Where the song is, in PPSths of a step, between clock ticks too.
        let tick = (self.tick + TICKS_PER_BAR - 1) % TICKS_PER_STEP;
        let between = (1.0 - self.samples_to_next_tick / self.samples_per_tick).clamp(0.0, 1.0);
        let at = self.song_step as f32 * PPS as f32
            + (tick as f32 + between) * PPS as f32 / TICKS_PER_STEP as f32;
        for i in 0..self.track_count() {
            let track = &project.tracks[i];
            let mut mix = [track.volume, track.pan, track.reverb, track.delay];
            let mut values = match &track.kind {
                Kind::Synth(v) => Some(*v),
                Kind::Sampler { .. } => None,
            };
            let (mut moved_mix, mut moved_synth) = (false, false);
            for lane in project.automation.iter().filter(|l| l.track == i) {
                let Some(u) = lane.value_at(at) else {
                    continue;
                };
                let v = lane.target.param().at_unit(u.clamp(0.0, 1.0));
                match lane.target {
                    Target::Volume => mix[0] = v,
                    Target::Pan => mix[1] = v,
                    Target::Reverb => mix[2] = v,
                    Target::Delay => mix[3] = v,
                    Target::Synth(k) => {
                        if let Some(values) = values.as_mut().filter(|_| k < synth::PARAMS.len()) {
                            values[k] = v;
                            moved_synth = true;
                        }
                    }
                }
                moved_mix |= !matches!(lane.target, Target::Synth(_));
            }
            if moved_mix {
                self.set_mix(&project, i, mix);
            }
            if let (true, Some(values)) = (moved_synth, values) {
                self.knobs[i] = values;
                self.track_knobs[i] = values;
                for voice in &mut self.voices[i] {
                    voice.synth.set_params(&values);
                }
            }
        }
    }

    /// Moves the knobs that modulation sources are connected to, in each
    /// sounding voice, and the track-wide ones (for the LFOs and pan).
    /// Runs every `UPDATE_INTERVAL` samples.
    fn apply_modulation(&mut self) {
        let project = self.project.clone();
        let scoped = self.shared.selected_track.load(Relaxed);
        let mut live = false;
        for i in 0..self.track_count() {
            let track = &project.tracks[i];
            if track.mods.is_empty() || !matches!(track.kind, Kind::Synth(_)) {
                continue;
            }
            let [lfo1, lfo2] = self.lfo_now[i];
            self.track_knobs[i] =
                modulation::apply(&self.knobs[i], &track.mods, [lfo1, lfo2, 0.0, 0.0]);
            let mut newest: Option<(u32, synth::Values)> = None;
            for voice in &mut self.voices[i] {
                if voice.synth.is_idle() {
                    continue;
                }
                synth::mod_env_advance(
                    &mut voice.mod_env,
                    &self.knobs[i],
                    UPDATE_INTERVAL,
                    self.sr,
                );
                let sources = [lfo1, lfo2, voice.mod_env.value, voice.velocity];
                let values = modulation::apply(&self.knobs[i], &track.mods, sources);
                voice.synth.set_params(&values);
                if newest.is_none_or(|(g, _)| voice.generation.wrapping_sub(g) < u32::MAX / 2) {
                    newest = Some((voice.generation, values));
                }
            }
            if i == scoped {
                let values = newest.map_or(self.track_knobs[i], |(_, v)| v);
                for (k, v) in values.iter().enumerate() {
                    let unit = synth::PARAMS[k].to_unit(*v);
                    self.shared.mod_live[k].store(unit.to_bits(), Relaxed);
                }
                live = true;
            }
        }
        self.shared.mod_live_on.store(live, Relaxed);
    }

    fn update(&mut self) {
        let latest = match self.shared.project.try_lock() {
            Ok(p) if !Arc::ptr_eq(&p, &self.project) => Some(p.clone()),
            _ => None,
        };
        if let Some(p) = latest {
            self.project = p;
            self.project_changed();
        }
        self.apply_automation();
        self.apply_modulation();
        let scoped = self.shared.selected_track.load(Relaxed).min(MAX_TRACKS - 1);
        self.shared.lfo_phase[0].store(self.lfos[scoped].phase().to_bits(), Relaxed);
        self.shared.lfo_phase[1].store(self.lfo2s[scoped].phase().to_bits(), Relaxed);
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Preview { track, note } if track < self.track_count() => {
                    if let Some((t, generation, _)) = self.preview.take() {
                        self.generation_off(t, generation);
                    }
                    let generation = self.note_on(track, note, NORMAL);
                    self.preview = Some((track, generation, (PREVIEW_SECONDS * self.sr) as u32));
                }
                Event::Hold {
                    track,
                    note,
                    velocity,
                } if track < self.track_count() => {
                    if self.arp_on(track) {
                        self.arp_add(track, note, velocity, true);
                        continue;
                    }
                    let generation = self.note_on(track, note, velocity);
                    if let Some(v) = self.voices[track]
                        .iter_mut()
                        .find(|v| v.generation == generation)
                    {
                        v.live = true;
                    }
                }
                Event::Bend {
                    track,
                    note,
                    semis,
                    filter,
                } if track < self.track_count() => {
                    if let Some(v) = self.holding(track, self.key(track, note), true) {
                        self.voices[track][v].synth.bend(semis, filter);
                    }
                }
                Event::Release { track, note } if track < MAX_TRACKS => {
                    self.arp_release(track, |h| h.live && h.note == note);
                    // The track may have been deleted; then any voice
                    // holding the note will do.
                    let held = self.holding(track, self.key(track, note), true);
                    if let Some(v) = held.or_else(|| self.holding(track, note, true)) {
                        self.voice_off(track, v);
                    }
                }
                _ => {}
            }
        }
    }

    /// Works out what plays at this step and schedules it. Pattern mode
    /// loops the pattern being edited; Song mode walks the timeline.
    fn next_step(&mut self) {
        let song_mode = self.shared.song_mode.load(Relaxed);
        let chain_mode = self.shared.chain_mode.load(Relaxed);
        if (song_mode, chain_mode) != self.was_mode {
            self.restart_song = true;
            self.was_mode = (song_mode, chain_mode);
        }
        let project = self.project.clone();
        let song_len = project.song_len();
        if song_mode && song_len > 0 {
            self.timeline_step(&project, song_len);
            return;
        }
        let block = self.shared.edit_block.load(Relaxed);
        if !song_mode && chain_mode {
            if let Some(block) = project.blocks.get(block) {
                self.chain_step(block);
                return;
            }
        }
        // Pattern mode follows the editor right away, like FL Studio.
        let pattern = self.shared.edit_pattern.load(Relaxed) as usize;
        let step = (self.tick / TICKS_PER_STEP) as usize;
        if step == 0 || self.pattern_bar.is_none() {
            let next = self.pattern_bar.map_or(0, |b| b + 1);
            self.pattern_bar = Some(next % project.pattern_bars(pattern));
        }
        let bar = self.pattern_bar.unwrap_or(0);
        self.shared.play_pattern.store(pattern as u8, Relaxed);
        self.shared.current_step.store(step, Relaxed);
        self.shared.current_bar.store(bar, Relaxed);
        self.schedule_hits(pattern, bar, step, step, STEPS - 1 - step, step == 0);
    }

    /// The next song position: one step on, or where a seek says, kept
    /// inside `len` and (when `looping`) the loop region.
    fn advance(&mut self, len: usize, looping: Option<(usize, usize)>) -> usize {
        let seek = self.shared.seek.swap(NO_SEEK, Relaxed);
        let mut pos = if seek != NO_SEEK {
            self.all_notes_off();
            seek
        } else if self.restart_song {
            0
        } else {
            self.song_step + 1
        };
        self.restart_song = false;
        if let Some((start, end)) = looping {
            // Jump back at the loop's end (but not when seeking past it).
            if seek == NO_SEEK && self.song_step < end && pos >= end {
                pos = start;
            }
        }
        if pos >= len {
            pos = looping.map_or(0, |(start, _)| start.min(len - 1));
        }
        if pos != self.song_step + 1 {
            // Keep the clock's place in the bar in step with the song, so
            // the metronome still accents beat 1.
            self.tick = (pos % STEPS) as u32 * TICKS_PER_STEP;
        }
        self.song_step = pos;
        self.shared.song_step.store(pos, Relaxed);
        pos
    }

    /// Jam view's Chain mode: plays the block being edited, bar by bar.
    fn chain_step(&mut self, block: &Block) {
        let pos = self.advance(block.len(), None);
        let (pattern, bar, step) = block.at(pos);
        let shown = pattern.map_or(NONE, |p| p as u8);
        self.shared.play_pattern.store(shown, Relaxed);
        self.shared.current_step.store(step, Relaxed);
        self.shared.current_bar.store(bar, Relaxed);
        if let Some(p) = pattern {
            self.schedule_hits(p, bar, step, pos, STEPS - 1 - step, step == 0);
        }
    }

    /// Song mode: plays every block on the timeline that covers this step.
    fn timeline_step(&mut self, project: &Project, song_len: usize) {
        let loop_start = self.shared.loop_start.load(Relaxed);
        let loop_end = self.shared.loop_end.load(Relaxed);
        let looping = (self.shared.loop_on.load(Relaxed) && loop_start < loop_end)
            .then_some((loop_start, loop_end));
        let pos = self.advance(song_len, looping);

        // The editor follows the pattern being edited if it's playing.
        let edit = self.shared.edit_pattern.load(Relaxed) as usize;
        let mut shown = None;
        for c in &project.clips {
            let Some(block) = project.blocks.get(c.block) else {
                continue;
            };
            let Some(at) = c.step_at(pos, block.len()) else {
                continue;
            };
            if let Some(part) = &block.midi {
                self.schedule_notes(part, at, c.end() - pos);
                continue;
            }
            let (Some(pattern), bar, step) = block.at(at) else {
                continue;
            };
            // Steps left in this bar of the block, within the clip.
            let room = (c.end() - pos - 1).min(STEPS - 1 - step);
            // Nothing looked ahead to a clip's first step.
            let fresh = pos == c.start || step == 0;
            self.schedule_hits(pattern, bar, step, pos, room, fresh);
            if shown.is_none() || pattern == edit {
                shown = Some((pattern, bar, step));
            }
        }
        let (pattern, bar, step) = shown.map_or((NONE, 0, 0), |(p, b, s)| (p as u8, b, s));
        self.shared.play_pattern.store(pattern, Relaxed);
        self.shared.current_step.store(step, Relaxed);
        self.shared.current_bar.store(bar, Relaxed);
    }

    /// Schedules the notes of a MIDI block that start during its step
    /// `at`, at their exact times. `left` is how many steps of the clip
    /// remain, counting this one; notes are cut off where it ends.
    fn schedule_notes(&mut self, part: &MidiPart, at: usize, left: usize) {
        let Some(i) = part.track.filter(|&i| i < self.track_count()) else {
            return;
        };
        let step_len = self.samples_per_tick * TICKS_PER_STEP as f32;
        let from = at as u32 * PPS;
        let room = left as u32 * PPS;
        let samples = |parts: u32| parts as f32 / PPS as f32 * step_len;
        for n in &part.notes {
            if n.start < from || n.start >= from + PPS {
                continue;
            }
            let into = n.start - from;
            let len = n.len.min(room - into).max(1);
            let action = Action::On {
                velocity: n.velocity.clamp(1, 127),
                gate: Some((samples(len) as u32).max(1)),
            };
            self.schedule(samples(into), i, n.pitch, action);
        }
    }

    /// Where a step's hit lands, in steps from the step's start: swing
    /// delays every other 16th, and nudge moves it either way.
    fn offset(&self, step: usize, s: &Step) -> f32 {
        let swing = if step % 2 == 1 {
            self.project.swing * 0.5
        } else {
            0.0
        };
        swing + s.nudge as f32 / 100.0
    }

    /// Schedules one lane's step to play `start` samples from now. `room`
    /// is how many steps of the pattern follow before it stops or wraps,
    /// so a tie only carries on if the next step will actually play.
    #[allow(clippy::too_many_arguments)]
    fn schedule_step(
        &mut self,
        i: usize,
        lane: usize,
        pattern: usize,
        bar: usize,
        step: usize,
        start: f32,
        room: usize,
    ) {
        let project = self.project.clone();
        let track = &project.tracks[i];
        let Some(grid) = track.grid(pattern, bar) else {
            return;
        };
        let steps = &grid[lane];
        let Some(s) = steps[step] else {
            return;
        };
        let note = track.note(lane);
        let step_len = self.samples_per_tick * TICKS_PER_STEP as f32;
        let next_tied = room > 0 && steps[step + 1].is_some_and(|n| n.tie);

        if s.tie {
            // The note was started by an earlier step; end it here unless
            // the tie carries on.
            if !next_tied {
                if let Some(generation) = self.tied[i][note as usize].take() {
                    let at = start + step_len * GATE;
                    self.schedule(at, i, note, Action::Off { generation });
                }
            }
            return;
        }
        if self.random() * 100.0 >= s.chance as f32 {
            return;
        }
        let ratchet = s.ratchet.clamp(1, 4);
        let slice = step_len / ratchet as f32;
        for r in 0..ratchet {
            let late = self.random() * track.humanize * HUMANIZE_TIMING * step_len;
            let spread = (self.random() * 2.0 - 1.0) * track.humanize * HUMANIZE_VELOCITY;
            let velocity = (s.velocity as f32 + spread).clamp(1.0, 127.0) as u8;
            let last = r + 1 == ratchet;
            let gate = (!(last && next_tied)).then_some((slice * GATE) as u32);
            let action = Action::On { velocity, gate };
            self.schedule(start + r as f32 * slice + late, i, note, action);
        }
    }

    /// Schedules the hits that belong to step `step` of a pattern's `bar`
    /// (counted from where it started playing), plus
    /// any on the next step nudged early into this one. `at` is the step's
    /// place in the song (it decides swing); `room` is how many steps
    /// follow before the pattern stops or wraps. When `fresh`, nothing
    /// looked ahead to this step, so its early hits just play on time.
    fn schedule_hits(
        &mut self,
        pattern: usize,
        bar: usize,
        step: usize,
        at: usize,
        room: usize,
        fresh: bool,
    ) {
        let step_len = self.samples_per_tick * TICKS_PER_STEP as f32;
        let project = self.project.clone();
        for (i, track) in project.tracks.iter().enumerate().take(MAX_TRACKS) {
            let Some(grid) = track.grid(pattern, bar) else {
                continue;
            };
            for (lane, steps) in grid.iter().enumerate().take(track.lanes()) {
                // A note held on for ties whose ties are gone (erased
                // while it played) ends now.
                if !steps[step].is_some_and(|s| s.tie) {
                    let note = track.note(lane) as usize;
                    if let Some(generation) = self.tied[i][note].take() {
                        self.schedule(0.0, i, note as u8, Action::Off { generation });
                    }
                }
                if let Some(s) = steps[step] {
                    let offset = self.offset(at, &s);
                    if offset >= 0.0 || fresh {
                        let start = offset.max(0.0) * step_len;
                        self.schedule_step(i, lane, pattern, bar, step, start, room);
                    }
                }
                if room > 0 {
                    if let Some(s) = steps[step + 1] {
                        let offset = self.offset(at + 1, &s);
                        if offset < 0.0 {
                            let start = (1.0 + offset) * step_len;
                            let next = step + 1;
                            self.schedule_step(i, lane, pattern, bar, next, start, room - 1);
                        }
                    }
                }
            }
        }
    }

    fn fire(&mut self, p: Pending) {
        if p.track >= self.track_count() {
            return;
        }
        match p.action {
            Action::On { velocity, gate } => {
                if self
                    .holding(p.track, self.key(p.track, p.note), true)
                    .is_some()
                {
                    return; // you're playing this note live
                }
                let generation = if self.arp_on(p.track) {
                    self.arp_add(p.track, p.note, velocity, false)
                } else {
                    self.note_on(p.track, p.note, velocity)
                };
                if let Some(gate) = gate {
                    let off = Action::Off { generation };
                    self.schedule(gate as f32, p.track, p.note, off);
                } else if p.track < MAX_TRACKS {
                    // Held for the tied steps after it, which end it.
                    self.tied[p.track][p.note as usize] = Some(generation);
                }
            }
            Action::Off { generation } => self.generation_off(p.track, generation),
        }
    }

    /// Fires everything due now and counts the rest down.
    fn run_pending(&mut self) {
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].at == 0 {
                let p = self.pending.swap_remove(i);
                self.fire(p);
            } else {
                self.pending[i].at -= 1;
                i += 1;
            }
        }
    }

    /// Clicks if `tick` (within the bar) starts a beat.
    fn click_on(&mut self, tick: u32) {
        if tick.is_multiple_of(TICKS_PER_BEAT) {
            let freq = if tick == 0 { CLICK_HIGH } else { CLICK_LOW };
            self.click.trigger(freq, self.sr);
        }
    }

    fn on_tick(&mut self) {
        if self.tick.is_multiple_of(TICKS_PER_STEP) {
            self.next_step();
        }
        // After the step: a jump in the song moves the place in the bar.
        if self.shared.metronome.load(Relaxed) {
            self.click_on(self.tick);
        }
        self.tick = (self.tick + 1) % TICKS_PER_BAR;
    }

    /// The next stereo frame, left and right.
    fn next_sample(&mut self) -> [f32; 2] {
        if self.samples_to_update == 0 {
            self.update();
            self.samples_to_update = UPDATE_INTERVAL;
        }
        self.samples_to_update -= 1;

        let playing = self.shared.playing.load(Relaxed);
        let external = self.shared.external_sync.load(Relaxed);

        if playing != self.was_playing {
            if playing {
                self.tick = 0;
                self.samples_to_next_tick = 0.0;
                self.restart_song = true;
                self.pattern_bar = None;
                // Another device's clock can't wait for a count-in.
                if !external && self.shared.count_in.load(Relaxed) {
                    self.count_in = TICKS_PER_BAR;
                    self.shared.counting_in.store(true, Relaxed);
                }
            } else {
                self.all_notes_off();
                self.count_in = 0;
                self.shared.counting_in.store(false, Relaxed);
            }
            // When following another device, it owns Start/Stop. After a
            // count-in, Start goes out when the count-in ends.
            if !external && self.count_in == 0 {
                self.send(Msg::one(if playing { midi::START } else { midi::STOP }));
            }
            self.was_playing = playing;
        }

        if external {
            if self.shared.ext_start.swap(false, Relaxed) {
                self.tick = 0;
                self.restart_song = true;
                self.pattern_bar = None;
            }
            self.samples_since_tick += 1;
            let ticks = self.shared.ext_ticks.swap(0, Relaxed);
            if ticks > 0 {
                // Smooth the measured tick length; MIDI clock is jittery.
                let measured = self.samples_since_tick as f32 / ticks as f32;
                self.samples_per_tick += (measured - self.samples_per_tick) * 0.2;
                self.samples_since_tick = 0;
                if playing {
                    for _ in 0..ticks {
                        self.on_tick();
                    }
                }
            }
        } else if playing {
            self.samples_per_tick = self.sr * 60.0 / self.shared.bpm() / 24.0;
            if self.samples_to_next_tick <= 0.0 {
                if self.count_in > 0 {
                    self.click_on(TICKS_PER_BAR - self.count_in);
                    self.count_in -= 1;
                    if self.count_in == 0 {
                        // Like MIDI itself: the clock after Start is beat 1.
                        self.shared.counting_in.store(false, Relaxed);
                        self.send(Msg::one(midi::START));
                    }
                } else {
                    self.send(Msg::one(midi::CLOCK));
                    self.on_tick();
                }
                self.samples_to_next_tick += self.samples_per_tick;
            }
            self.samples_to_next_tick -= 1.0;
        }
        self.run_pending();
        self.run_arps();

        if let Some((track, generation, left)) = self.preview {
            if left <= 1 {
                self.preview = None;
                self.generation_off(track, generation);
            } else {
                self.preview = Some((track, generation, left - 1));
            }
        }

        let mut mix = [0.0; 2];
        let mut send = [0.0; 2];
        let scoped = self.shared.selected_track.load(Relaxed);
        let mut scope = 0.0;
        let beat = self.samples_per_tick * TICKS_PER_BEAT as f32;
        for i in 0..self.track_count() {
            let values = match &self.project.tracks[i].kind {
                Kind::Synth(values) => Some(values),
                Kind::Sampler { .. } => None,
            };
            // The LFO runs whether or not notes are playing, so it's
            // somewhere in its cycle rather than restarting each note.
            // They follow the knobs as automation and modulation have them.
            let mods = &self.project.tracks[i].mods;
            let knobs = &self.track_knobs[i];
            let lfo = match values {
                Some(_)
                    if knobs[synth::LFO_AMOUNT] > 0.0 || modulation::uses(mods, Source::Lfo1) =>
                {
                    let step = Lfo::step(knobs, self.sr, beat);
                    self.lfos[i].next(knobs, step)
                }
                _ => 0.0,
            };
            self.lfo_now[i][0] = lfo;
            if values.is_some() && modulation::uses(mods, Source::Lfo2) {
                let step = synth::lfo2_step(knobs, self.sr, beat);
                self.lfo_now[i][1] = synth::lfo2_next(&mut self.lfo2s[i], knobs, step);
            }
            let mut out = 0.0;
            for voice in &mut self.voices[i] {
                let sample = if values.is_some() {
                    if voice.synth.is_idle() {
                        continue;
                    }
                    voice.synth.next_sample(lfo)
                } else {
                    voice.sample_out()
                };
                out += sample * voice.gain;
            }
            if i == scoped {
                scope = out;
            }
            // An LFO on pan sweeps the track left and right (auto-pan).
            let knobs = &self.track_knobs[i];
            let gains = match values {
                Some(_)
                    if lfo != 0.0
                        && knobs[synth::LFO_AMOUNT] > 0.0
                        && synth::choice(knobs[synth::LFO_TARGET], 4) == synth::LFO_PAN =>
                {
                    let pan = self.project.tracks[i].pan + lfo * knobs[synth::LFO_AMOUNT];
                    pan_gains(pan).map(|g| g * self.levels[i])
                }
                _ => self.gains[i],
            };
            mix[0] += out * gains[0];
            mix[1] += out * gains[1];
            send[0] += out * self.sends[i][0];
            send[1] += out * self.sends[i][1];
        }
        if send != [0.0; 2] {
            self.fx_quiet = 0;
        } else {
            self.fx_quiet = self.fx_quiet.saturating_add(1);
        }
        if (self.fx_quiet as f32) < FX_TAIL_SECONDS * self.sr {
            let fx = self.project.effects;
            let reverb = self.reverb.process(send[0], fx.reverb_size, fx.reverb_tone);
            let step = self.samples_per_tick * TICKS_PER_STEP as f32;
            let time = (fx.delay_steps.round() * step) as usize;
            let delay = self.delay.process(send[1], time, fx.delay_feedback);
            for side in 0..2 {
                mix[side] += reverb[side] + delay[side];
            }
        }
        if self.shared.scope_on.load(Relaxed) {
            self.shared.write_scope(scope);
        }
        let click = self.click.next_sample();
        // Master volume, then the limiter so it never clips. The click
        // isn't part of the song, so it goes on after.
        let master = self.project.master;
        let out = self.limiter.process(mix.map(|m| m * master));
        for (p, o) in self.meter_peak.iter_mut().zip(out) {
            *p = p.max(o.abs());
        }
        self.meter_limiting = self.meter_limiting.max(1.0 - self.limiter.gain);
        self.meter_count += 1;
        if self.meter_count >= METER_INTERVAL {
            self.shared
                .report_meter(self.meter_peak, self.meter_limiting);
            self.meter_peak = [0.0; 2];
            self.meter_limiting = 0.0;
            self.meter_count = 0;
        }
        out.map(|o| (o + click).clamp(-1.0, 1.0))
    }
}

/// Opens the default output device and starts the engine.
/// Keep the returned stream alive; dropping it stops the sound.
pub fn start(
    shared: Arc<Shared>,
    events: Receiver<Event>,
    midi_out: SyncSender<Msg>,
) -> Result<cpal::Stream, String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no audio output device found")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let sr = supported.sample_rate().0 as f32;
    let engine = Engine::new(shared, events, midi_out, sr);
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, engine),
        SampleFormat::I16 => build::<i16>(&device, &config, engine),
        SampleFormat::U16 => build::<u16>(&device, &config, engine),
        other => Err(format!("unsupported sample format: {other}")),
    }?;
    // Browsers only allow sound after a click or key press, so the web
    // version calls `play` then (and calling it twice would double up).
    #[cfg(not(target_arch = "wasm32"))]
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

/// What's open in the Jam view, which decides what an export plays when
/// the timeline is empty.
#[derive(Clone, Copy)]
pub struct Edit {
    pub pattern: usize,
    pub block: usize,
}

/// What an offline render made: the audio (left and right), and the MIDI
/// the engine sent, each message with the sample it went out on.
pub struct Rendered {
    pub audio: Vec<[f32; 2]>,
    pub midi: Vec<(usize, Msg)>,
}

/// Plays a project offline, faster than real time, for exporting. Plays the
/// timeline once if it has blocks, else the edited block's chain once,
/// else the edited pattern `loops` times, then
/// lets notes ring out for `tail` seconds. Uses the same engine as live
/// playback, so swing, ratchets, ties and the rest sound the same.
pub fn render(
    project: Project,
    bpm: f32,
    edit: Edit,
    loops: usize,
    sr: f32,
    tail: f32,
) -> Rendered {
    let song_steps = project.song_len();
    let chain = project
        .blocks
        .get(edit.block)
        .filter(|b| b.chain.iter().any(Option::is_some))
        .map(Block::len);
    let steps = if song_steps > 0 {
        song_steps
    } else {
        chain.unwrap_or(loops * project.pattern_bars(edit.pattern) * STEPS)
    };
    let shared = Arc::new(Shared::new(Arc::new(project)));
    shared.set_bpm(bpm);
    shared.song_mode.store(song_steps > 0, Relaxed);
    shared.chain_mode.store(chain.is_some(), Relaxed);
    shared.edit_pattern.store(edit.pattern as u8, Relaxed);
    shared.edit_block.store(edit.block, Relaxed);
    let (_events, events_rx) = sync_channel(1);
    let (midi_tx, midi_rx) = sync_channel(1024);
    let mut engine = Engine::new(shared.clone(), events_rx, midi_tx, sr);
    shared.playing.store(true, Relaxed);

    let step = sr * 60.0 / bpm / 4.0;
    // Stop just before the next step would start.
    let play = (steps as f32 * step) as usize - 1;
    let total = play + (tail * sr) as usize;
    let mut audio = Vec::with_capacity(total);
    let mut midi = Vec::new();
    for t in 0..total {
        if t == play {
            shared.playing.store(false, Relaxed);
        }
        audio.push(engine.next_sample());
        midi.extend(midi_rx.try_iter().map(|m| (t, m)));
    }
    Rendered { audio, midi }
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut engine: Engine,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                for frame in data.chunks_mut(channels) {
                    let [l, r] = engine.next_sample();
                    match frame {
                        [mono] => *mono = T::from_sample((l + r) * 0.5),
                        [left, right, rest @ ..] => {
                            *left = T::from_sample(l);
                            *right = T::from_sample(r);
                            rest.fill(T::EQUILIBRIUM);
                        }
                        [] => {}
                    }
                }
            },
            |e| eprintln!("audio error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Clip, Track, EMPTY_GRID, GHOST};
    use crate::{drums, synth};
    use std::sync::mpsc::sync_channel;

    const KICK: usize = 0;
    const HAT: usize = 2;
    const LEAD: usize = 4;
    const PAD: usize = 5;
    /// One 16th-note step at 120 BPM and 48 kHz.
    const STEP: usize = 6_000;

    struct Rig {
        engine: Engine,
        shared: Arc<Shared>,
        events: SyncSender<Event>,
        midi: Receiver<Msg>,
    }

    impl Rig {
        fn new(project: Project) -> Self {
            let shared = Arc::new(Shared::new(Arc::new(project)));
            let (events, events_rx) = sync_channel(64);
            let (midi_tx, midi) = sync_channel(8192);
            let engine = Engine::new(shared.clone(), events_rx, midi_tx, 48_000.0);
            Rig {
                engine,
                shared,
                events,
                midi,
            }
        }

        /// Plays and returns the mix folded to mono.
        fn run(&mut self, samples: usize) -> Vec<f32> {
            self.run_stereo(samples)
                .iter()
                .map(|[l, r]| (l + r) * 0.5)
                .collect()
        }

        fn run_stereo(&mut self, samples: usize) -> Vec<[f32; 2]> {
            (0..samples).map(|_| self.engine.next_sample()).collect()
        }

        fn send(&self, event: Event) {
            self.events.send(event).unwrap();
        }

        fn midi(&self) -> Vec<Msg> {
            self.midi.try_iter().collect()
        }

        /// The voice of a track that last played `note`.
        fn voice(&self, track: usize, note: u8) -> &TrackVoice {
            let voices = &self.engine.voices[track];
            voices.iter().find(|v| v.note == note).expect("a voice")
        }

        /// How many of a track's voices are making sound.
        fn sounding(&self, track: usize) -> usize {
            let voices = &self.engine.voices[track];
            voices.iter().filter(|v| !v.is_free()).count()
        }

        /// Plays and returns the sample index of each MIDI note-on for `note`.
        fn note_times(&mut self, samples: usize, note: u8) -> Vec<usize> {
            self.shared.playing.store(true, Relaxed);
            let mut times = Vec::new();
            for t in 0..samples {
                self.engine.next_sample();
                for m in self.midi.try_iter() {
                    let b = m.bytes();
                    if b.len() == 3 && b[0] & 0xF0 == 0x90 && b[1] == note {
                        times.push(t);
                    }
                }
            }
            times
        }
    }

    fn quiet_demo() -> Project {
        let mut p = Project::demo();
        for t in &mut p.tracks {
            t.patterns.fill(vec![EMPTY_GRID]);
        }
        p
    }

    fn hit() -> Option<Step> {
        Some(Step::HIT)
    }

    /// Clicks a track's name: plays its first lane.
    fn preview(track: usize) -> Event {
        let note = Project::demo().tracks[track].note(0);
        Event::Preview { track, note }
    }

    fn hold(track: usize, note: u8) -> Event {
        Event::Hold {
            track,
            note,
            velocity: 90,
        }
    }

    #[test]
    fn long_parts_play_their_bars_in_turn_and_short_ones_repeat() {
        let mut project = quiet_demo();
        project.tracks[KICK].set_bars(0, 2);
        project.tracks[KICK].patterns[0][1][0][4] = hit(); // bar 2 only
        project.tracks[HAT].patterns[0][0][0][0] = hit(); // one bar, looping
        let (kick, hat) = (project.tracks[KICK].note(0), project.tracks[HAT].note(0));
        let mut rig = Rig::new(project.clone());
        let bar = STEPS * STEP;
        let kicks = rig.note_times(4 * bar, kick);
        assert_eq!(kicks, [bar + 4 * STEP, 3 * bar + 4 * STEP]);
        let mut rig = Rig::new(project);
        let hats = rig.note_times(2 * bar, hat);
        assert_eq!(hats, [0, bar]);
    }

    #[test]
    fn plays_steps_at_tempo() {
        let mut rig = Rig::new(Project::demo());
        rig.shared.playing.store(true, Relaxed);
        // One beat at 120 BPM is half a second = 4 steps.
        let out = rig.run(24_000);
        assert!(out.iter().any(|s| s.abs() > 0.1), "should be audible");
        assert!(out.iter().all(|s| s.abs() <= 1.0), "should not clip");
        assert_eq!(rig.shared.current_step.load(Relaxed), 3);
    }

    #[test]
    fn sends_start_clock_and_drum_notes() {
        let mut rig = Rig::new(Project::demo());
        rig.shared.playing.store(true, Relaxed);
        rig.run(24_000);
        let msgs = rig.midi();
        assert_eq!(msgs[0].bytes(), [midi::START]);
        let clocks = msgs.iter().filter(|m| m.bytes() == [midi::CLOCK]).count();
        assert_eq!(clocks, 24, "one beat of clock");
        // The demo has a kick on step 1, sent on channel 10.
        assert!(msgs.contains(&Msg::three(0x99, drums::NOTES[0], 100)));
        assert!(msgs.contains(&Msg::three(0x89, drums::NOTES[0], 0)));
    }

    #[test]
    fn follows_external_clock() {
        let mut rig = Rig::new(Project::demo());
        rig.shared.external_sync.store(true, Relaxed);
        rig.shared.playing.store(true, Relaxed);
        rig.shared.ext_ticks.store(7, Relaxed); // step 1 at tick 0, step 2 at tick 6
        rig.run(1);
        assert_eq!(rig.shared.current_step.load(Relaxed), 1);
        let msgs = rig.midi();
        assert!(
            msgs.iter().all(|m| m.bytes() != [midi::CLOCK]),
            "no clock echo"
        );
    }

    #[test]
    fn plays_melody_on_its_channel() {
        let mut project = Project::demo();
        project.tracks[LEAD].midi_channel = Some(2); // channel 3
        let mut rig = Rig::new(project);
        rig.shared.playing.store(true, Relaxed);
        rig.run(STEP);
        let msgs = rig.midi();
        assert!(msgs.contains(&Msg::three(0x92, 48, 100)), "C3 on channel 3");
        assert!(msgs.contains(&Msg::three(0x82, 48, 0)), "and its note-off");
    }

    #[test]
    fn plays_chords() {
        let mut rig = Rig::new(Project::demo());
        rig.shared.playing.store(true, Relaxed);
        rig.run(10);
        let msgs = rig.midi();
        // The demo pad starts with C minor (C4, E♭4, G4) on channel 2.
        for note in [60, 63, 67] {
            assert!(msgs.contains(&Msg::three(0x91, note, 100)), "note {note}");
        }
        assert_eq!(rig.sounding(PAD), 3, "three voices at once");
    }

    #[test]
    fn an_lfo_on_pan_sweeps_left_and_right() {
        let mut project = quiet_demo();
        if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
            v[synth::LFO_TARGET] = synth::LFO_PAN as f32;
            v[synth::LFO_AMOUNT] = 1.0;
            v[synth::LFO_SHAPE] = 2.0; // square: hard right, then hard left
            v[synth::LFO_RATE] = 2.0;
            v[synth::SUSTAIN] = 1.0;
        }
        let mut rig = Rig::new(project);
        rig.send(hold(LEAD, 48));
        // Half a second per side at 2 Hz is a quarter second; look at
        // the middle of each half-cycle.
        let out = rig.run_stereo(24_000);
        let side = |range: std::ops::Range<usize>| {
            out[range]
                .iter()
                .fold([0.0; 2], |[l, r], [a, b]| [l + a.abs(), r + b.abs()])
        };
        let [l, r] = side(3_000..9_000);
        assert!(l < r * 0.01, "first half right: {l} {r}");
        let [l, r] = side(15_000..21_000);
        assert!(r < l * 0.01, "then left: {l} {r}");
    }

    #[test]
    fn held_notes_bend_smoothly_off_the_grid() {
        let mut project = quiet_demo();
        if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
            v[synth::WAVE] = 0.0; // sine
            v[synth::LEVEL_2] = 0.0;
            v[synth::CUTOFF] = 8000.0;
            v[synth::SUSTAIN] = 1.0;
        }
        let mut rig = Rig::new(project);
        // A3 (220 Hz), bent up half a semitone: about 226.5 Hz.
        rig.send(hold(LEAD, 57));
        rig.send(Event::Bend {
            track: LEAD,
            note: 57,
            semis: 0.5,
            filter: 0.0,
        });
        rig.run(4_800); // let the bend settle
        let out = rig.run(48_000);
        let ups = out.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((225..=228).contains(&ups), "about 226.5 Hz: {ups}");
    }

    #[test]
    fn click_wave_clicks_once_a_cycle_far_below_the_lowest_note() {
        let mut project = quiet_demo();
        if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
            v[synth::WAVE] = 5.0; // click
            v[synth::LEVEL_2] = 0.0;
            v[synth::CUTOFF] = 8000.0;
            v[synth::SUSTAIN] = 1.0;
        }
        let mut rig = Rig::new(project);
        // MIDI note 0 is about 8.18 Hz; bent down two octaves, about 2 Hz.
        rig.send(hold(LEAD, 0));
        rig.send(Event::Bend {
            track: LEAD,
            note: 0,
            semis: -24.0,
            filter: 0.0,
        });
        rig.run(48_000); // let the bend settle
        let out = rig.run(96_000);
        // Count the ticks: loud samples after quiet ones.
        let ticks = out
            .windows(2)
            .filter(|w| w[0].abs() < 0.01 && w[1].abs() > 0.05)
            .count();
        assert!((3..=5).contains(&ticks), "about 2 a second: {ticks}");
    }

    #[test]
    fn every_engine_survives_the_drones_whole_range() {
        for engine in 0..synth::ENGINES.len() {
            for (note, semis) in [(0, -76.0), (127, 5.5)] {
                let mut project = quiet_demo();
                if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
                    v[synth::ENGINE] = engine as f32;
                    v[synth::WAVE] = 5.0;
                }
                let mut rig = Rig::new(project);
                rig.send(hold(LEAD, note));
                rig.send(Event::Bend {
                    track: LEAD,
                    note,
                    semis,
                    filter: 0.0,
                });
                let out = rig.run(24_000);
                assert!(
                    out.iter().all(|x| x.is_finite() && x.abs() < 4.0),
                    "engine {engine}, note {note} bent {semis}"
                );
            }
        }
    }

    /// The lead with its arpeggiator on, sending MIDI on channel 1.
    fn arp_lead(mode: f32, octaves: f32) -> Project {
        let mut project = quiet_demo();
        project.tracks[LEAD].midi_channel = Some(0);
        if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
            v[synth::ARP_MODE] = mode;
            v[synth::ARP_OCTAVES] = octaves;
        }
        project
    }

    /// Holds a chord and returns the notes played, in order, for `steps`
    /// 16th notes at 120 BPM.
    fn arp_notes(project: Project, chord: &[u8], steps: usize) -> Vec<u8> {
        let mut rig = Rig::new(project);
        for &note in chord {
            rig.send(hold(LEAD, note));
        }
        let mut notes = Vec::new();
        for _ in 0..STEP * steps {
            rig.engine.next_sample();
            for m in rig.midi.try_iter() {
                let b = m.bytes();
                if b.len() == 3 && b[0] == 0x90 {
                    notes.push(b[1]);
                }
            }
        }
        notes
    }

    #[test]
    fn the_arpeggiator_walks_through_a_held_chord() {
        // Held out of order; Up plays them low to high, a 16th apart.
        let up = arp_notes(arp_lead(1.0, 0.0), &[55, 48, 52], 6);
        assert_eq!(up, [48, 52, 55, 48, 52, 55]);
        let down = arp_notes(arp_lead(2.0, 0.0), &[48, 52, 55], 4);
        assert_eq!(down, [55, 52, 48, 55]);
        let up_down = arp_notes(arp_lead(3.0, 0.0), &[48, 52, 55], 6);
        assert_eq!(up_down, [48, 52, 55, 52, 48, 52]);
        // Two octaves: up through the chord, then again an octave higher.
        let wide = arp_notes(arp_lead(1.0, 1.0), &[48, 55], 5);
        assert_eq!(wide, [48, 55, 60, 67, 48]);
        let played = arp_notes(arp_lead(5.0, 0.0), &[55, 48], 3);
        assert_eq!(played, [55, 48, 55], "as played");
        // Off: the chord plays together, once.
        let chord = arp_notes(arp_lead(0.0, 0.0), &[48, 52, 55], 4);
        assert_eq!(chord, [48, 52, 55]);
    }

    #[test]
    fn one_note_can_play_a_chord_or_scale() {
        let mut project = arp_lead(1.0, 0.0);
        if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
            v[synth::ARP_NOTES] = 2.0; // minor
        }
        assert_eq!(arp_notes(project, &[48], 4), [48, 51, 55, 48]);
        let mut project = arp_lead(1.0, 0.0);
        if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
            v[synth::ARP_NOTES] = 9.0; // pentatonic
        }
        assert_eq!(arp_notes(project, &[60], 6), [60, 62, 64, 67, 69, 60]);
    }

    #[test]
    fn latch_keeps_playing_after_letting_go_until_a_new_chord() {
        let mut project = arp_lead(1.0, 0.0);
        if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
            v[synth::ARP_LATCH] = 1.0;
        }
        let mut rig = Rig::new(project);
        let ons = |rig: &mut Rig, steps: usize| {
            rig.run(STEP * steps);
            rig.midi()
                .iter()
                .filter(|m| m.bytes()[0] == 0x90)
                .map(|m| m.bytes()[1])
                .collect::<Vec<u8>>()
        };
        for n in [48, 52] {
            rig.send(hold(LEAD, n));
        }
        ons(&mut rig, 1);
        for note in [48, 52] {
            rig.send(Event::Release { track: LEAD, note });
        }
        let after = ons(&mut rig, 4);
        assert_eq!(after.len(), 4, "still playing after letting go: {after:?}");
        rig.send(hold(LEAD, 60));
        ons(&mut rig, 1);
        let next = ons(&mut rig, 3);
        assert_eq!(next, [60, 60, 60], "a new note replaces the chord");
    }

    #[test]
    fn the_arpeggiator_stops_when_notes_are_let_go() {
        let mut rig = Rig::new(arp_lead(1.0, 0.0));
        rig.send(hold(LEAD, 48));
        rig.run(STEP * 2);
        rig.send(Event::Release {
            track: LEAD,
            note: 48,
        });
        rig.run(UPDATE_INTERVAL as usize);
        rig.midi();
        rig.run(STEP * 4);
        let ons = rig.midi().iter().filter(|m| m.bytes()[0] == 0x90).count();
        assert_eq!(ons, 0, "nothing after letting go");
    }

    #[test]
    fn mono_tracks_play_one_note_at_a_time() {
        let mut project = quiet_demo();
        if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
            v[synth::VOICES] = 1.0;
        }
        project.tracks[LEAD].midi_channel = Some(0);
        let mut rig = Rig::new(project);
        rig.send(hold(LEAD, 48));
        rig.send(hold(LEAD, 55));
        rig.run(UPDATE_INTERVAL as usize);
        assert_eq!(rig.sounding(LEAD), 1);
        assert!(rig.midi().contains(&Msg::three(0x80, 48, 0)), "C3 gave way");
        rig.send(Event::Release {
            track: LEAD,
            note: 55,
        });
        rig.run(UPDATE_INTERVAL as usize);
        assert!(rig.midi().contains(&Msg::three(0x80, 55, 0)), "G3 ends");
    }

    #[test]
    fn notes_an_octave_apart_get_their_own_voices() {
        // One voice per piano-roll row used to put C3 and C4 on one voice.
        let mut rig = Rig::new(quiet_demo());
        for note in [48, 60, 72, 52, 64] {
            rig.send(hold(LEAD, note));
        }
        rig.run(UPDATE_INTERVAL as usize);
        assert_eq!(rig.sounding(LEAD), 5);
        let msgs = rig.midi();
        assert!(!msgs.iter().any(|m| m.bytes()[0] & 0xF0 == 0x80), "no cuts");
    }

    #[test]
    fn the_oldest_voice_is_stolen_when_all_are_busy() {
        let mut project = quiet_demo();
        project.tracks[LEAD].midi_channel = Some(0);
        let mut rig = Rig::new(project);
        for note in 40..40 + POLYPHONY as u8 + 2 {
            rig.send(hold(LEAD, note));
        }
        rig.run(UPDATE_INTERVAL as usize);
        assert_eq!(rig.sounding(LEAD), POLYPHONY);
        let offs: Vec<u8> = rig
            .midi()
            .iter()
            .filter(|m| m.bytes()[0] & 0xF0 == 0x80)
            .map(|m| m.bytes()[1])
            .collect();
        assert_eq!(offs, [40, 41], "the first two notes gave way");
    }

    #[test]
    fn a_note_played_again_takes_over_its_voice() {
        let mut rig = Rig::new(quiet_demo());
        rig.send(preview(LEAD));
        rig.run(2_000);
        rig.send(preview(LEAD));
        rig.run(UPDATE_INTERVAL as usize);
        assert_eq!(rig.sounding(LEAD), 1, "one C3, played twice");
    }

    #[test]
    fn song_mode_plays_bars_in_order() {
        let mut project = Project::demo();
        project.tracks[LEAD].patterns[1][0][2][0] = hit(); // pattern B: D on step 1
        project.blocks.push(chain_block(&[Some(1)]));
        project.clips.push(Clip {
            block: 1,
            row: 0,
            start: STEPS,
            len: STEPS,
            offset: 0,
        });
        let mut rig = Rig::new(project);
        rig.shared.song_mode.store(true, Relaxed);
        rig.shared.playing.store(true, Relaxed);
        // One bar at 120 BPM is 2 seconds = 96_000 samples; play into bar 2.
        rig.run(100_000);
        assert_eq!(rig.shared.song_step.load(Relaxed), STEPS);
        assert_eq!(rig.shared.play_pattern.load(Relaxed), 1);
        let msgs = rig.midi();
        assert!(
            msgs.contains(&Msg::three(0x90, 48, 100)),
            "bar 1 plays A (C3)"
        );
        assert!(
            msgs.contains(&Msg::three(0x90, 50, 100)),
            "bar 2 plays B (D3)"
        );
    }

    fn chain_block(chain: &[Option<usize>]) -> Block {
        let mut block = Block::new("Test");
        block.chain[..chain.len()].copy_from_slice(chain);
        block
    }

    /// A one-kick pattern B in a one-bar block, placed as `clips`
    /// (start, len, offset).
    fn kick_clips(clips: &[(usize, usize, usize)]) -> Project {
        let mut project = quiet_demo();
        project.tracks[KICK].patterns[1][0][0][0] = hit();
        project.blocks = vec![chain_block(&[Some(1)])];
        project.clips = clips
            .iter()
            .map(|&(start, len, offset)| Clip {
                block: 0,
                row: 0,
                start,
                len,
                offset,
            })
            .collect();
        project
    }

    fn song_kicks(project: Project, samples: usize, setup: impl Fn(&Shared)) -> Vec<usize> {
        let mut rig = Rig::new(project);
        rig.shared.song_mode.store(true, Relaxed);
        setup(&rig.shared);
        rig.note_times(samples, drums::NOTES[0])
    }

    #[test]
    fn clips_repeat_trim_and_overlap() {
        // A 2-bar clip repeats its 1-bar pattern: kicks on both downbeats.
        let kicks = song_kicks(kick_clips(&[(0, 32, 0)]), STEP * 32, |_| {});
        assert_eq!(kicks, [0, STEP * 16]);
        // Trimmed to start 4 steps into the pattern: the next kick is 12 later.
        let kicks = song_kicks(kick_clips(&[(0, 20, 4)]), STEP * 20, |_| {});
        assert_eq!(kicks, [STEP * 12]);
        // A gap in the timeline is silent, then the song loops to the top.
        let kicks = song_kicks(kick_clips(&[(8, 8, 0)]), STEP * 24 + 1, |_| {});
        assert_eq!(kicks, [STEP * 8, STEP * 24]);
    }

    #[test]
    fn chain_mode_plays_the_block_bar_by_bar() {
        // Kick (pattern B), a silent bar, the kick again, then round again.
        let mut project = kick_clips(&[]);
        project.blocks[0] = chain_block(&[Some(1), None, Some(1)]);
        let mut rig = Rig::new(project);
        rig.shared.chain_mode.store(true, Relaxed);
        let kicks = rig.note_times(STEP * 48 + 1, drums::NOTES[0]);
        assert_eq!(kicks, [0, STEP * 32, STEP * 48]);
    }

    #[test]
    fn blocks_on_the_timeline_play_their_chain() {
        // A 2-bar block (kick, then empty pattern C) stretched to 4 bars
        // repeats.
        let mut project = kick_clips(&[(0, 64, 0)]);
        project.blocks[0] = chain_block(&[Some(1), Some(2)]);
        let kicks = song_kicks(project, STEP * 64, |_| {});
        assert_eq!(kicks, [0, STEP * 32]);
    }

    #[test]
    fn midi_blocks_play_exact_notes_on_their_track() {
        let mut project = quiet_demo();
        project.tracks[LEAD].midi_channel = Some(0);
        let b = project.new_midi_block(Some(LEAD)).unwrap();
        let notes = &mut project.blocks[b].midi.as_mut().unwrap().notes;
        // A triplet-ish note a third of a step in, two octaves up, and a
        // long note that the clip's end cuts short.
        notes.push(crate::project::Note {
            start: PPS / 3,
            len: PPS / 2,
            pitch: 84,
            velocity: 70,
        });
        notes.push(crate::project::Note {
            start: 4 * PPS,
            len: 100 * PPS,
            pitch: 36,
            velocity: 100,
        });
        project.clips = vec![Clip {
            block: b,
            row: 0,
            start: 0,
            len: 8,
            offset: 0,
        }];
        let mut rig = Rig::new(project);
        rig.shared.song_mode.store(true, Relaxed);
        rig.shared.playing.store(true, Relaxed);
        let mut events = Vec::new();
        for t in 0..STEP * 9 {
            rig.engine.next_sample();
            events.extend(
                rig.midi
                    .try_iter()
                    .filter(|m| m.bytes().len() == 3)
                    .map(|m| (t, m)),
            );
        }
        let at = |msg: Msg| events.iter().find(|(_, m)| *m == msg).map(|(t, _)| *t);
        assert_eq!(at(Msg::three(0x90, 84, 70)), Some(STEP / 3));
        assert_eq!(at(Msg::three(0x80, 84, 0)), Some(STEP / 3 + STEP / 2));
        assert_eq!(at(Msg::three(0x90, 36, 100)), Some(4 * STEP));
        assert_eq!(
            at(Msg::three(0x80, 36, 0)),
            Some(8 * STEP),
            "cut where the clip ends"
        );
    }

    #[test]
    fn loop_region_and_seeking() {
        // Loop steps 16-32 of a 4-bar song: bar 2 plays, then bar 2 again.
        let project = kick_clips(&[(0, 64, 0)]);
        let kicks = song_kicks(project.clone(), STEP * 40, |s| {
            s.loop_on.store(true, Relaxed);
            s.loop_start.store(16, Relaxed);
            s.loop_end.store(32, Relaxed);
        });
        assert_eq!(kicks, [0, STEP * 16, STEP * 32]);
        // Seeking to bar 3 jumps straight there.
        let mut rig = Rig::new(project);
        rig.shared.song_mode.store(true, Relaxed);
        rig.shared.playing.store(true, Relaxed);
        rig.run(STEP * 2);
        rig.shared.seek.store(32, Relaxed);
        rig.run(STEP);
        assert_eq!(rig.shared.song_step.load(Relaxed), 32);
    }

    #[test]
    fn synth_knobs_change_the_sound() {
        let mut rig = Rig::new(quiet_demo());
        let play = |rig: &mut Rig| {
            rig.send(preview(LEAD));
            rig.run(4_800).iter().map(|s| s.abs()).sum::<f32>()
        };
        let before = play(&mut rig);
        let mut project = quiet_demo();
        if let Kind::Synth(values) = &mut project.tracks[LEAD].kind {
            values[synth::VOLUME] = 0.0;
        }
        rig.shared.publish(Arc::new(project));
        rig.run(48_000); // let the old note fade
        let after = play(&mut rig);
        assert!(
            before > 1.0 && after < 0.01,
            "volume knob: {before} -> {after}"
        );
    }

    #[test]
    fn modulation_moves_knobs_while_notes_play() {
        // The mod envelope pulls the synth's volume down to nothing right
        // after each note starts and holds it there, so the held note goes
        // quiet.
        let mut project = quiet_demo();
        let mod_sustain = synth::PARAMS
            .iter()
            .position(|p| p.name == "Mod sustain")
            .unwrap();
        if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
            v[synth::SUSTAIN] = 1.0;
            v[mod_sustain] = 1.0;
        }
        project.tracks[LEAD].mods.push(synth::Link {
            source: Source::ModEnv,
            knob: synth::VOLUME,
            amount: -1.0,
        });
        let mut rig = Rig::new(project);
        rig.shared.selected_track.store(LEAD, Relaxed);
        rig.send(hold(LEAD, 60));
        let loudness = |out: &[f32]| out.iter().map(|s| s.abs()).sum::<f32>();
        // Mod attack is 10 ms by default: louder at first, then silent.
        let early = loudness(&rig.run(240));
        rig.run(4_800);
        let late = loudness(&rig.run(4_800));
        assert!(early > 0.01 && late < 1e-3, "{early} then {late}");
        let unit = rig
            .shared
            .mod_live(synth::VOLUME)
            .expect("knob shown moving");
        assert!(unit < 0.01, "volume knob shown at the bottom: {unit}");

        // LFO 2 on the cutoff: the same held note keeps changing.
        let mut project = quiet_demo();
        if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
            v[synth::SUSTAIN] = 1.0;
        }
        project.tracks[LEAD].mods.push(synth::Link {
            source: Source::Lfo2,
            knob: synth::CUTOFF,
            amount: 0.8,
        });
        let mut rig = Rig::new(project);
        rig.send(hold(LEAD, 48));
        rig.run(4_800);
        let windows: Vec<f32> = (0..8).map(|_| loudness(&rig.run(6_000))).collect();
        let (lo, hi) = windows
            .iter()
            .fold((f32::MAX, 0.0_f32), |(lo, hi), w| (lo.min(*w), hi.max(*w)));
        assert!(hi > lo * 1.3, "the cutoff sweeps: {windows:?}");
    }

    #[test]
    fn retrigger_restarts_the_lfo_at_each_note() {
        let mut project = quiet_demo();
        if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
            v[synth::LFO_AMOUNT] = 0.5;
            v[synth::LFO_RATE] = 1.0;
        }
        let phase_after_a_note = |project: Project| {
            let mut rig = Rig::new(project);
            rig.run(30_000); // the LFO runs on its own meanwhile
            rig.send(hold(LEAD, 60));
            rig.run(UPDATE_INTERVAL as usize * 2);
            rig.engine.lfos[LEAD].phase()
        };
        let free = phase_after_a_note(project.clone());
        assert!(free > 0.5, "free-running: well into its cycle ({free})");
        if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
            v[synth::LFO_RETRIG] = 1.0;
        }
        let retrig = phase_after_a_note(project);
        assert!(retrig < 0.01, "retrigger: back at the top ({retrig})");
    }

    #[test]
    fn mute_and_solo_pick_what_is_heard() {
        let mut project = quiet_demo();
        project.tracks[KICK].muted = true;
        let mut rig = Rig::new(project);
        rig.send(preview(KICK));
        assert!(rig.run(4_800).iter().all(|s| *s == 0.0), "muted");

        let mut project = quiet_demo();
        project.tracks[LEAD].solo = true;
        rig.shared.publish(Arc::new(project));
        rig.run(48_000);
        rig.send(preview(KICK));
        assert!(
            rig.run(4_800).iter().all(|s| *s == 0.0),
            "another track is soloed"
        );
    }

    #[test]
    fn metronome_clicks_on_every_beat() {
        let mut rig = Rig::new(quiet_demo());
        rig.shared.metronome.store(true, Relaxed);
        rig.shared.playing.store(true, Relaxed);
        // Two beats at 120 BPM; a click starts each one.
        let out = rig.run(STEP * 8);
        let loud = |range: std::ops::Range<usize>| out[range].iter().any(|s| s.abs() > 0.1);
        assert!(loud(0..200), "beat 1");
        assert!(!loud(STEP * 2..STEP * 3), "quiet between beats");
        assert!(loud(STEP * 4..STEP * 4 + 200), "beat 2");

        let mut rig = Rig::new(quiet_demo());
        rig.shared.playing.store(true, Relaxed);
        assert!(rig.run(STEP * 8).iter().all(|s| *s == 0.0), "off");
    }

    #[test]
    fn count_in_waits_a_bar_before_playing() {
        let mut project = quiet_demo();
        project.tracks[KICK].patterns[0][0][0][0] = hit();
        let mut rig = Rig::new(project);
        rig.shared.count_in.store(true, Relaxed);
        let times = rig.note_times(STEP * STEPS + 10, drums::NOTES[0]);
        assert_eq!(times, [STEP * STEPS], "the kick waits one bar");
        assert!(!rig.shared.counting_in.load(Relaxed));

        let mut rig = Rig::new(quiet_demo());
        rig.shared.count_in.store(true, Relaxed);
        rig.shared.playing.store(true, Relaxed);
        let out = rig.run(STEP * 4 + 200);
        assert!(rig.shared.counting_in.load(Relaxed));
        assert!(out[..200].iter().any(|s| s.abs() > 0.1), "clicks");
        assert!(
            !rig.midi().iter().any(|m| m.bytes() == [midi::START]),
            "no Start until the count-in ends"
        );
    }

    #[test]
    fn sends_feed_the_reverb_and_delay() {
        // How loud a kick's tail is, well after the dry hit has ended.
        let tail = |reverb: f32, delay: f32| {
            let mut project = quiet_demo();
            project.tracks[KICK].reverb = reverb;
            project.tracks[KICK].delay = delay;
            let mut rig = Rig::new(project);
            rig.send(preview(KICK));
            let out = rig.run(48_000);
            out[30_000..].iter().map(|s| s.abs()).sum::<f32>()
        };
        let dry = tail(0.0, 0.0);
        assert!(dry < 1e-3, "no sends, no tail: {dry}");
        assert!(tail(1.0, 0.0) > 1.0, "reverb rings on");
        // 3 steps at 120 BPM = 18_000 samples, so echoes at 36k, 54k...
        assert!(tail(0.0, 1.0) > 1.0, "delay echoes");
    }

    #[test]
    fn scope_shows_the_selected_track() {
        let mut rig = Rig::new(quiet_demo());
        rig.shared.selected_track.store(LEAD, Relaxed);
        rig.send(preview(LEAD));
        rig.run(2_000);
        let mut seen = [0.0; 512];
        rig.shared.read_scope(&mut seen);
        assert!(seen.iter().all(|s| *s == 0.0), "off until turned on");

        rig.shared.scope_on.store(true, Relaxed);
        rig.run(2_000);
        rig.shared.read_scope(&mut seen);
        assert!(seen.iter().any(|s| s.abs() > 0.05), "the lead's wave");
    }

    #[test]
    fn pan_moves_a_track_left_or_right() {
        let loudness = |pan: f32| {
            let mut project = quiet_demo();
            project.tracks[KICK].pan = pan;
            let mut rig = Rig::new(project);
            rig.send(preview(KICK));
            rig.run_stereo(4_800)
                .iter()
                .fold([0.0; 2], |[l, r], [a, b]| [l + a.abs(), r + b.abs()])
        };
        let [l, r] = loudness(0.0);
        assert!(l > 1.0 && (l - r).abs() < 1e-3, "centre: {l} {r}");
        let [l, r] = loudness(-1.0);
        assert!(l > 1.0 && r < 1e-3, "hard left: {l} {r}");
        let [l, r] = loudness(0.5);
        assert!(r > l * 2.0, "right of centre: {l} {r}");
    }

    #[test]
    fn pitched_samples_follow_the_note() {
        let data = vec![0.5; 48_000];
        let sample = Arc::new(Sample {
            name: "tone".into(),
            data,
            rate: 48_000.0,
            builtin: None,
        });
        let mut project = quiet_demo();
        project.tracks.push(Track::sampler(sample, true));
        let i = project.tracks.len() - 1;
        let mut rig = Rig::new(project);
        rig.send(Event::Preview {
            track: i,
            note: SAMPLE_ROOT,
        });
        rig.run(1);
        let rate = rig.voice(i, SAMPLE_ROOT).rate;
        assert!((rate - 1.0).abs() < 1e-6, "C4 plays as recorded");
        rig.send(Event::Preview {
            track: i,
            note: SAMPLE_ROOT + 7,
        });
        rig.run(UPDATE_INTERVAL as usize);
        let fifth = 2f64.powf(7.0 / 12.0);
        let rate = rig.voice(i, SAMPLE_ROOT + 7).rate;
        assert!((rate - fifth).abs() < 1e-4, "G4 a fifth up");
    }

    #[test]
    fn a_new_track_never_inherits_a_deleted_one_s_notes() {
        // A held note (say a drone) on the last track, then that track is
        // deleted and a new one added in its place.
        let mut project = quiet_demo();
        let last = project.tracks.len() - 1;
        project.tracks[last].reverb = 0.0; // no tail to wait out
        let mut rig = Rig::new(project.clone());
        rig.send(hold(last, 60));
        rig.run(4_800);
        assert!(rig.sounding(last) > 0);
        project.remove_track(last);
        rig.shared.publish(Arc::new(project.clone()));
        rig.run(UPDATE_INTERVAL as usize * 2);
        project.add_track(Track::synth("New"));
        rig.shared.publish(Arc::new(project));
        rig.run(UPDATE_INTERVAL as usize * 2);
        assert_eq!(rig.sounding(last), 0, "the new track starts silent");
        let out = rig.run(4_800);
        assert!(out.iter().all(|s| *s == 0.0), "and stays quiet");
    }

    #[test]
    fn master_volume_scales_the_mix_and_the_limiter_stops_clipping() {
        let peak = |master: f32, notes: &[u8]| {
            let mut project = quiet_demo();
            project.master = master;
            if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
                v[synth::SUSTAIN] = 1.0;
                v[synth::VOLUME] = 1.0;
                v[synth::CUTOFF] = 8000.0;
            }
            project.tracks[LEAD].volume = 1.0;
            let mut rig = Rig::new(project);
            for &n in notes {
                rig.send(hold(LEAD, n));
            }
            rig.run_stereo(24_000)
                .iter()
                .fold(0.0_f32, |m, [l, r]| m.max(l.abs()).max(r.abs()))
        };
        let full = peak(0.8, &[48]);
        let half = peak(0.4, &[48]);
        assert!(
            (half / full - 0.5).abs() < 0.05,
            "half volume: {full} {half}"
        );
        // Twelve loud notes at once would clip; the limiter holds them.
        let loud = peak(1.0, &[36, 40, 43, 48, 52, 55, 60, 64, 67, 72, 76, 79]);
        assert!(loud <= crate::fx::LIMIT + 1e-4, "limited: {loud}");
    }

    /// A track playing a steady 1-second sample, with this shape.
    fn shaped_sample(shape: crate::project::SampleShape) -> (Project, usize) {
        let sample = Arc::new(Sample {
            name: "ramp".into(),
            // A ramp, so where it's playing shows in the value.
            data: (0..48_000)
                .map(|i| 0.1 + 0.8 * i as f32 / 48_000.0)
                .collect(),
            rate: 48_000.0,
            builtin: None,
        });
        let mut project = quiet_demo();
        let mut track = Track::sampler(sample, false);
        if let Kind::Sampler { shape: s, .. } = &mut track.kind {
            *s = shape;
        }
        track.volume = 1.0;
        project.tracks.push(track);
        let i = project.tracks.len() - 1;
        (project, i)
    }

    #[test]
    fn samples_play_their_trimmed_part_forwards_or_backwards() {
        use crate::project::SampleShape;
        let half = SampleShape {
            start: 0.25,
            end: 0.75,
            ..SampleShape::DEFAULT
        };
        let (project, i) = shaped_sample(half);
        let mut rig = Rig::new(project);
        rig.send(Event::Preview { track: i, note: 60 });
        let out = rig.run(48_000);
        let sound: Vec<f32> = out.iter().copied().filter(|s| *s != 0.0).collect();
        assert!(
            (23_000..=24_100).contains(&sound.len()),
            "half a second: {}",
            sound.len()
        );
        assert!(sound[0] < sound[sound.len() - 1], "rising");
        let (project, i) = shaped_sample(SampleShape {
            reverse: true,
            ..half
        });
        let mut rig = Rig::new(project);
        rig.send(Event::Preview { track: i, note: 60 });
        let sound: Vec<f32> = rig.run(48_000).into_iter().filter(|s| *s != 0.0).collect();
        assert!(sound[0] > sound[sound.len() - 1], "falling: backwards");
    }

    #[test]
    fn samples_fade_out_after_the_note_unless_they_ring() {
        use crate::project::SampleShape;
        let fade = SampleShape {
            ring: false,
            release: 0.01,
            ..SampleShape::DEFAULT
        };
        let (project, i) = shaped_sample(fade);
        let mut rig = Rig::new(project);
        rig.send(hold(i, 60));
        rig.run(4_800);
        rig.send(Event::Release { track: i, note: 60 });
        rig.run(4_800);
        assert!(rig.run(100).iter().all(|s| s.abs() < 1e-3), "faded out");
        let (project, i) = shaped_sample(SampleShape::DEFAULT);
        let mut rig = Rig::new(project);
        rig.send(hold(i, 60));
        rig.run(4_800);
        rig.send(Event::Release { track: i, note: 60 });
        rig.run(4_800);
        assert!(rig.run(100).iter().any(|s| s.abs() > 0.1), "rings on");
    }

    #[test]
    fn automation_moves_a_track_s_volume_over_the_song() {
        use crate::project::{AutoLane, AutoPoint};
        // A steady held note on the lead, its volume automated from
        // silent to full over the first bar.
        let mut project = kick_clips(&[(0, 64, 0)]);
        project.tracks[KICK].muted = true;
        if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
            v[synth::SUSTAIN] = 1.0;
        }
        project.automation.push(AutoLane {
            track: LEAD,
            target: Target::Volume,
            points: vec![
                AutoPoint { at: 0, value: 0.0 },
                AutoPoint {
                    at: 16 * PPS,
                    value: 1.0,
                },
            ],
        });
        let mut rig = Rig::new(project);
        rig.shared.song_mode.store(true, Relaxed);
        rig.shared.playing.store(true, Relaxed);
        rig.send(hold(LEAD, 60));
        let loudness = |out: &[f32]| out.iter().map(|s| s.abs()).sum::<f32>();
        let early = loudness(&rig.run(STEP * 2));
        rig.run(STEP * 12);
        let late = loudness(&rig.run(STEP * 2));
        assert!(late > early * 3.0, "fades in: {early} then {late}");
        // Stopped, the knob is back as set.
        rig.shared.playing.store(false, Relaxed);
        rig.run(UPDATE_INTERVAL as usize * 2);
        assert_eq!(rig.engine.levels[LEAD], rig.engine.project.gain(LEAD));
    }

    #[test]
    fn deleting_tracks_mid_note_is_safe() {
        let mut rig = Rig::new(Project::demo());
        rig.shared.playing.store(true, Relaxed);
        rig.run(1_000);
        rig.shared.publish(Arc::new(Project::empty()));
        let out = rig.run(10_000);
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn tied_steps_hold_the_note() {
        let mut project = quiet_demo();
        let lane = &mut project.tracks[LEAD].patterns[0][0][0];
        lane[0] = hit();
        lane[1] = Some(Step::TIE);
        lane[2] = Some(Step::TIE);
        let mut rig = Rig::new(project);
        rig.shared.playing.store(true, Relaxed);
        rig.run(STEP * 2); // two steps: still held
        assert!(
            !rig.midi().contains(&Msg::three(0x80, 48, 0)),
            "held across the tie"
        );
        rig.run(STEP); // end of the third step
        assert!(
            rig.midi().contains(&Msg::three(0x80, 48, 0)),
            "released after the ties"
        );
    }

    /// The arpeggiating lead with a C major chord from step 0, tied
    /// through step `last`.
    fn tied_chord(last: usize) -> Project {
        let mut project = arp_lead(1.0, 0.0);
        for lane in [0, 4, 7] {
            let steps = &mut project.tracks[LEAD].patterns[0][0][lane];
            steps[0] = hit();
            for s in &mut steps[1..=last] {
                *s = Some(Step::TIE);
            }
        }
        project
    }

    /// Plays `steps` 16ths and returns the note-ons sent.
    fn note_ons(rig: &mut Rig, steps: usize) -> Vec<u8> {
        rig.run(STEP * steps);
        let msgs = rig.midi();
        msgs.iter()
            .map(|m| m.bytes())
            .filter(|b| b.len() == 3 && b[0] == 0x90)
            .map(|b| b[1])
            .collect()
    }

    #[test]
    fn the_arpeggiator_lets_go_when_a_tied_chord_ends() {
        let mut rig = Rig::new(tied_chord(3));
        rig.shared.playing.store(true, Relaxed);
        assert!(!note_ons(&mut rig, 4).is_empty(), "arpeggiates the chord");
        rig.run(STEP); // the last note's tail
        assert_eq!(
            note_ons(&mut rig, 6),
            Vec::<u8>::new(),
            "stops after the ties"
        );
    }

    #[test]
    fn erasing_a_held_chord_while_it_plays_lets_go() {
        let mut rig = Rig::new(tied_chord(STEPS - 1));
        rig.shared.playing.store(true, Relaxed);
        assert!(!note_ons(&mut rig, 4).is_empty(), "arpeggiates the chord");
        rig.shared.publish(Arc::new(arp_lead(1.0, 0.0)));
        rig.run(STEP * 2);
        assert_eq!(
            note_ons(&mut rig, 6),
            Vec::<u8>::new(),
            "the erased chord stops"
        );
        // The same without the arpeggiator: the held notes end.
        let mut project = tied_chord(STEPS - 1);
        if let Kind::Synth(v) = &mut project.tracks[LEAD].kind {
            v[synth::ARP_MODE] = 0.0;
        }
        let mut rig = Rig::new(project.clone());
        rig.shared.playing.store(true, Relaxed);
        rig.run(STEP * 4);
        project.tracks[LEAD].patterns[0] = vec![EMPTY_GRID];
        rig.shared.publish(Arc::new(project));
        rig.run(STEP * 2);
        let offs = rig.midi().iter().filter(|m| m.bytes()[0] == 0x80).count();
        assert_eq!(offs, 3, "all three notes end");
    }

    #[test]
    fn live_notes_last_until_released() {
        let mut rig = Rig::new(quiet_demo());
        rig.shared.playing.store(true, Relaxed);
        rig.send(Event::Hold {
            track: LEAD,
            note: 52,
            velocity: 90,
        });
        rig.send(Event::Hold {
            track: LEAD,
            note: 55,
            velocity: 90,
        });
        rig.run(48_000); // a second, through many sequencer steps
        let msgs = rig.midi();
        assert!(msgs.contains(&Msg::three(0x90, 52, 90)), "E3 on");
        assert!(
            msgs.contains(&Msg::three(0x90, 55, 90)),
            "G3 on too: a chord"
        );
        assert!(!msgs.contains(&Msg::three(0x80, 52, 0)), "still held");
        rig.send(Event::Release {
            track: LEAD,
            note: 52,
        });
        rig.run(UPDATE_INTERVAL as usize);
        let msgs = rig.midi();
        assert!(msgs.contains(&Msg::three(0x80, 52, 0)), "E3 released");
        assert!(!msgs.contains(&Msg::three(0x80, 55, 0)), "G3 still held");
    }

    #[test]
    fn swing_delays_every_other_step() {
        let mut project = quiet_demo();
        project.swing = 0.5;
        project.tracks[HAT].patterns[0][0][0] = [hit(); STEPS];
        let mut rig = Rig::new(project);
        let times = rig.note_times(STEP * 4, drums::NOTES[2]);
        // Even steps on the grid; odd ones a quarter step (swing/2) late.
        assert_eq!(times, [0, STEP + STEP / 4, 2 * STEP, 3 * STEP + STEP / 4]);
    }

    #[test]
    fn nudge_moves_a_hit_early_or_late() {
        let mut project = quiet_demo();
        let lane = &mut project.tracks[KICK].patterns[0][0][0];
        lane[2] = Some(Step {
            nudge: -50,
            ..Step::HIT
        });
        lane[4] = Some(Step {
            nudge: 25,
            ..Step::HIT
        });
        let mut rig = Rig::new(project);
        let times = rig.note_times(STEP * 6, drums::NOTES[0]);
        assert_eq!(times, [2 * STEP - STEP / 2, 4 * STEP + STEP / 4]);
    }

    #[test]
    fn ratchets_repeat_within_a_step() {
        let mut project = quiet_demo();
        project.tracks[HAT].patterns[0][0][0][0] = Some(Step {
            ratchet: 4,
            ..Step::HIT
        });
        let mut rig = Rig::new(project);
        let times = rig.note_times(STEP, drums::NOTES[2]);
        assert_eq!(times, [0, STEP / 4, STEP / 2, 3 * STEP / 4]);
    }

    #[test]
    fn velocity_sets_loudness_and_midi_velocity() {
        let mut project = quiet_demo();
        project.tracks[KICK].patterns[0][0][0][0] = Some(Step {
            velocity: GHOST,
            ..Step::HIT
        });
        let mut rig = Rig::new(project);
        rig.shared.playing.store(true, Relaxed);
        let ghost: f32 = rig.run(STEP).iter().map(|s| s.abs()).sum();
        assert!(rig
            .midi()
            .contains(&Msg::three(0x99, drums::NOTES[0], GHOST)));

        let mut project = quiet_demo();
        project.tracks[KICK].patterns[0][0][0][0] = hit();
        let mut rig = Rig::new(project);
        rig.shared.playing.store(true, Relaxed);
        let normal: f32 = rig.run(STEP).iter().map(|s| s.abs()).sum();
        let ratio = ghost / normal;
        assert!(
            (ratio - velocity_gain(GHOST)).abs() < 0.01,
            "ghost is quieter: {ratio}"
        );
    }

    #[test]
    fn chance_skips_some_hits() {
        let mut project = quiet_demo();
        project.tracks[HAT].patterns[0][0][0] = [Some(Step {
            chance: 50,
            ..Step::HIT
        }); STEPS];
        let mut rig = Rig::new(project);
        let bars = 8;
        let hits = rig.note_times(STEP * STEPS * bars, drums::NOTES[2]).len();
        let total = STEPS * bars;
        assert!(
            hits > total / 4 && hits < total * 3 / 4,
            "about half: {hits}/{total}"
        );
    }

    #[test]
    fn humanize_varies_timing_slightly() {
        let mut project = quiet_demo();
        project.tracks[HAT].patterns[0][0][0] = [hit(); STEPS];
        project.tracks[HAT].humanize = 1.0;
        let mut rig = Rig::new(project);
        let times = rig.note_times(STEP * STEPS, drums::NOTES[2]);
        assert_eq!(times.len(), STEPS);
        let max_late = (STEP as f32 * HUMANIZE_TIMING) as usize;
        let offsets: Vec<usize> = times
            .iter()
            .enumerate()
            .map(|(i, t)| t - i * STEP)
            .collect();
        assert!(offsets.iter().all(|&o| o <= max_late), "{offsets:?}");
        assert!(offsets.iter().any(|&o| o > 0), "some hits move");
    }
}
