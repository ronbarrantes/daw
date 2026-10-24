//! Audio engine: runs the sequencer and mixes every track.
//!
//! The audio callback never waits on a lock. Transport state lives in
//! atomics; the project (tracks and patterns) arrives as an immutable
//! snapshot that the engine picks up with `try_lock`; one-off requests
//! like "play this note now" arrive on a channel. Outgoing MIDI is queued
//! on a bounded channel and written by the MIDI thread.
//!
//! Every track has one voice per lane (piano-roll row), so melodic tracks
//! can play chords. Steps start on MIDI clock ticks, but each hit is
//! scheduled to the exact sample, so swing, nudge, humanize and ratchets
//! aren't limited to the clock's resolution.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicUsize, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use crate::midi::{self, Msg};
use crate::project::{Kind, Project, Sample, Step, LANES, MAX_TRACKS, NORMAL, SAMPLE_ROOT, STEPS};
use crate::synth::Voice;

/// MIDI clock runs at 24 ticks per beat, so a 16th-note step is 6 ticks.
const TICKS_PER_STEP: u32 = 6;
/// Marks "no pattern" (a silent song bar).
const NONE: u8 = u8::MAX;
/// How long a clicked note sounds, in seconds.
const PREVIEW_SECONDS: f32 = 0.3;
/// How often the engine checks for a new project and events, in samples.
const UPDATE_INTERVAL: u32 = 64;
/// Notes last this fraction of their step (or ratchet slice).
const GATE: f32 = 0.8;
/// Room for scheduled hits; enough for a busy pattern with chords.
const MAX_PENDING: usize = 512;
/// Humanize at full strength: up to this fraction of a step late...
const HUMANIZE_TIMING: f32 = 0.12;
/// ...and up to this much velocity either way.
const HUMANIZE_VELOCITY: f32 = 30.0;

/// Requests from the UI or MIDI in.
pub enum Event {
    /// Briefly play one lane of a track, e.g. when clicking a cell.
    Preview {
        track: usize,
        lane: usize,
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
}

pub struct Shared {
    /// The latest project snapshot from the UI.
    project: Mutex<Arc<Project>>,
    bpm_bits: AtomicU32,
    pub playing: AtomicBool,
    pub current_step: AtomicUsize,
    /// Play the song arrangement instead of looping the edited pattern.
    pub song_mode: AtomicBool,
    /// The pattern shown in the editor.
    pub edit_pattern: AtomicU8,
    /// The pattern being heard, or `NONE` during a silent bar.
    pub play_pattern: AtomicU8,
    /// The song bar being heard.
    pub song_pos: AtomicUsize,
    /// The track selected in the UI, which MIDI controllers play by default.
    pub selected_track: AtomicUsize,
    /// Follow incoming MIDI clock instead of the BPM setting.
    pub external_sync: AtomicBool,
    /// Clock ticks received from MIDI in, not yet handled by the engine.
    pub ext_ticks: AtomicU32,
    /// Set when MIDI in receives Start, so the engine rewinds to step 1.
    pub ext_start: AtomicBool,
}

impl Shared {
    pub fn new(project: Arc<Project>) -> Self {
        Self {
            project: Mutex::new(project),
            bpm_bits: AtomicU32::new(120f32.to_bits()),
            playing: AtomicBool::new(false),
            current_step: AtomicUsize::new(0),
            song_mode: AtomicBool::new(false),
            edit_pattern: AtomicU8::new(0),
            play_pattern: AtomicU8::new(0),
            song_pos: AtomicUsize::new(0),
            selected_track: AtomicUsize::new(0),
            external_sync: AtomicBool::new(false),
            ext_ticks: AtomicU32::new(0),
            ext_start: AtomicBool::new(false),
        }
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

/// Loudness for a MIDI velocity; 100 is unity, so ghost notes (40) are
/// about a quarter as loud and accents (127) about 40% louder.
fn velocity_gain(velocity: u8) -> f32 {
    (velocity as f32 / NORMAL as f32).powf(1.5)
}

/// Playback state for one lane of one track.
struct LaneVoice {
    synth: Voice,
    /// The sample being played, if any. Holding the `Arc` keeps it alive
    /// even if the track is deleted mid-note.
    sample: Option<Arc<Sample>>,
    pos: f64,
    rate: f64,
    gain: f32,
    /// Counts notes, so a scheduled note-off only ends the note it belongs to.
    generation: u32,
    /// MIDI note (and channel) still on.
    held: Option<(u8, u8)>,
    /// The note a keyboard or controller is holding here, if any; the
    /// sequencer leaves the lane alone while it's set.
    live: Option<u8>,
}

impl LaneVoice {
    fn new(sr: f32) -> Self {
        Self {
            synth: Voice::new(sr),
            sample: None,
            pos: 0.0,
            rate: 1.0,
            gain: 1.0,
            generation: 0,
            held: None,
            live: None,
        }
    }

    fn sample_out(&mut self) -> f32 {
        let Some(s) = &self.sample else { return 0.0 };
        let i = self.pos as usize;
        if i + 1 >= s.data.len() {
            self.sample = None;
            return 0.0;
        }
        // Linear interpolation, so pitched-up or -down samples stay smooth.
        let frac = (self.pos - i as f64) as f32;
        self.pos += self.rate;
        s.data[i] + (s.data[i + 1] - s.data[i]) * frac
    }
}

/// Something the sequencer will do a few samples from now.
#[derive(Clone, Copy)]
struct Pending {
    /// Samples until it happens.
    at: u32,
    track: usize,
    lane: usize,
    action: Action,
}

#[derive(Clone, Copy)]
enum Action {
    /// Start the lane's note. `gate` is how long it lasts in samples;
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
    /// `voices[track][lane]`.
    voices: Vec<Vec<LaneVoice>>,
    /// Scheduled hits; preallocated so the audio thread never allocates.
    pending: Vec<Pending>,
    /// Clock tick within the bar, 0..TICKS_PER_STEP * STEPS.
    tick: u32,
    samples_to_next_tick: f32,
    /// Current tick length; measured when following an external clock.
    samples_per_tick: f32,
    samples_since_tick: u32,
    samples_to_update: u32,
    /// The clicked note that's sounding (track, lane), and samples until it stops.
    preview: Option<(usize, usize, u32)>,
    /// Each track's level after mute, solo and group volume; worked out
    /// once per project change rather than every sample.
    gains: [f32; MAX_TRACKS],
    /// Pattern playing in this bar; `None` is a silent song bar.
    pattern: Option<usize>,
    song_pos: usize,
    /// The next bar starts the song from the top instead of advancing.
    restart_song: bool,
    was_song_mode: bool,
    was_playing: bool,
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
        let voices = (0..MAX_TRACKS)
            .map(|_| (0..LANES).map(|_| LaneVoice::new(sr)).collect())
            .collect();
        let mut engine = Self {
            project: shared.project(),
            shared,
            events,
            midi_out,
            voices,
            pending: Vec::with_capacity(MAX_PENDING),
            tick: 0,
            samples_to_next_tick: 0.0,
            samples_per_tick: sr * 60.0 / 120.0 / 24.0,
            samples_since_tick: 0,
            samples_to_update: 0,
            preview: None,
            gains: [0.0; MAX_TRACKS],
            pattern: Some(0),
            song_pos: 0,
            restart_song: true,
            was_song_mode: false,
            was_playing: false,
            rng: 0x2545_f491,
            sr,
        };
        engine.project_changed();
        engine
    }

    /// Refreshes everything derived from the project: track levels and
    /// synth settings.
    fn project_changed(&mut self) {
        let project = self.project.clone();
        for i in 0..MAX_TRACKS {
            self.gains[i] = if i < project.tracks.len() {
                project.gain(i)
            } else {
                0.0
            };
        }
        for (lanes, track) in self.voices.iter_mut().zip(&project.tracks) {
            if let Kind::Synth(values) = &track.kind {
                for voice in lanes {
                    voice.synth.set_params(values);
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
    fn schedule(&mut self, at: f32, track: usize, lane: usize, action: Action) {
        if self.pending.len() < MAX_PENDING {
            let at = at.max(0.0) as u32;
            self.pending.push(Pending {
                at,
                track,
                lane,
                action,
            });
        }
    }

    /// Starts MIDI note `note` in a track's lane and returns its generation.
    fn note_on(&mut self, i: usize, lane: usize, note: u8, velocity: u8) -> u32 {
        self.note_off(i, lane);
        let project = self.project.clone();
        let track = &project.tracks[i];
        // One-shot tracks always send their own note (e.g. 36 for a kick).
        let midi_note = if track.is_melodic() {
            note
        } else {
            track.midi_note
        };
        if let Some(ch) = track.midi_channel {
            self.send(Msg::three(0x90 | ch, midi_note, velocity));
            self.voices[i][lane].held = Some((midi_note, ch));
        }
        let voice = &mut self.voices[i][lane];
        voice.gain = velocity_gain(velocity);
        voice.generation = voice.generation.wrapping_add(1);
        match &track.kind {
            Kind::Synth(_) => voice.synth.note_on(note),
            Kind::Sampler { sample, pitched } => {
                let semis = if *pitched {
                    note as f32 - SAMPLE_ROOT as f32
                } else {
                    0.0
                };
                voice.rate = (sample.rate / self.sr * 2f32.powf(semis / 12.0)) as f64;
                voice.pos = 0.0;
                voice.sample = Some(sample.clone());
            }
        }
        voice.generation
    }

    /// Ends a lane's note. Samples keep ringing to their end; synths release.
    fn note_off(&mut self, i: usize, lane: usize) {
        if let Some((note, ch)) = self.voices[i][lane].held.take() {
            self.send(Msg::three(0x80 | ch, note, 0));
        }
        self.voices[i][lane].synth.note_off();
    }

    fn all_notes_off(&mut self) {
        self.pending.clear();
        for i in 0..MAX_TRACKS {
            for lane in 0..LANES {
                if self.voices[i][lane].live.is_none() {
                    self.note_off(i, lane);
                }
            }
        }
    }

    /// Picks up a new project snapshot and UI or MIDI events.
    fn update(&mut self) {
        let latest = match self.shared.project.try_lock() {
            Ok(p) if !Arc::ptr_eq(&p, &self.project) => Some(p.clone()),
            _ => None,
        };
        if let Some(p) = latest {
            self.project = p;
            self.project_changed();
        }
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Preview { track, lane } if track < self.track_count() => {
                    let lane = lane.min(LANES - 1);
                    let note = self.project.tracks[track].note(lane);
                    self.note_on(track, lane, note, NORMAL);
                    self.preview = Some((track, lane, (PREVIEW_SECONDS * self.sr) as u32));
                }
                Event::Hold {
                    track,
                    note,
                    velocity,
                } if track < self.track_count() => {
                    let lane = self.project.tracks[track].lane_for(note);
                    self.note_on(track, lane, note, velocity);
                    self.voices[track][lane].live = Some(note);
                }
                Event::Release { track, note } if track < MAX_TRACKS => {
                    // The track may have been deleted; then lane 0 is as good as any.
                    let lane = self
                        .project
                        .tracks
                        .get(track)
                        .map_or(0, |t| t.lane_for(note));
                    if self.voices[track][lane].live == Some(note) {
                        self.voices[track][lane].live = None;
                        self.note_off(track, lane);
                    }
                }
                _ => {}
            }
        }
    }

    /// Picks which pattern plays for the current step.
    fn choose_pattern(&mut self, step: usize) {
        let song_mode = self.shared.song_mode.load(Relaxed);
        if song_mode != self.was_song_mode {
            self.restart_song = true;
            self.was_song_mode = song_mode;
        }
        let song_len = self.project.song_len();
        if !song_mode || song_len == 0 {
            // Pattern mode follows the editor right away, like FL Studio.
            self.pattern = Some(self.shared.edit_pattern.load(Relaxed) as usize);
        } else if step == 0 {
            self.song_pos = if self.restart_song {
                0
            } else {
                (self.song_pos + 1) % song_len
            };
            self.restart_song = false;
            self.pattern = self.project.song[self.song_pos].map(usize::from);
            self.shared.song_pos.store(self.song_pos, Relaxed);
        }
        let shown = self.pattern.map_or(NONE, |p| p as u8);
        self.shared.play_pattern.store(shown, Relaxed);
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

    /// Schedules one lane's step to play `start` samples from now.
    fn schedule_step(&mut self, i: usize, lane: usize, pattern: usize, step: usize, start: f32) {
        let project = self.project.clone();
        let track = &project.tracks[i];
        let steps = &track.patterns[pattern][lane];
        let Some(s) = steps[step] else {
            return;
        };
        let step_len = self.samples_per_tick * TICKS_PER_STEP as f32;
        let next_tied = steps
            .get(step + 1)
            .is_some_and(|n| n.is_some_and(|n| n.tie));

        if s.tie {
            // The note was started by an earlier step; end it here unless
            // the tie carries on.
            if !next_tied {
                let generation = self.voices[i][lane].generation;
                let at = start + step_len * GATE;
                self.schedule(at, i, lane, Action::Off { generation });
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
            self.schedule(start + r as f32 * slice + late, i, lane, action);
        }
    }

    /// Schedules the hits that belong to this step. Hits nudged early into
    /// the previous step were already scheduled one step ago.
    fn schedule_hits(&mut self, step: usize) {
        let Some(pattern) = self.pattern else { return };
        let step_len = self.samples_per_tick * TICKS_PER_STEP as f32;
        let project = self.project.clone();
        for (i, track) in project.tracks.iter().enumerate().take(MAX_TRACKS) {
            for lane in 0..track.lanes() {
                let steps = &track.patterns[pattern][lane];
                if let Some(s) = steps[step] {
                    let offset = self.offset(step, &s);
                    // Step 1 can't look back into the previous bar, so early
                    // hits there just play on time.
                    if offset >= 0.0 || step == 0 {
                        let start = offset.max(0.0) * step_len;
                        self.schedule_step(i, lane, pattern, step, start);
                    }
                }
                if let Some(s) = steps.get(step + 1).copied().flatten() {
                    let offset = self.offset(step + 1, &s);
                    if offset < 0.0 {
                        let start = (1.0 + offset) * step_len;
                        self.schedule_step(i, lane, pattern, step + 1, start);
                    }
                }
            }
        }
    }

    fn fire(&mut self, p: Pending) {
        if p.track >= self.track_count() {
            return;
        }
        let voice = &self.voices[p.track][p.lane];
        match p.action {
            Action::On { velocity, gate } => {
                if voice.live.is_some() {
                    return; // you're playing this lane live
                }
                let note = self.project.tracks[p.track].note(p.lane);
                let generation = self.note_on(p.track, p.lane, note, velocity);
                if let Some(gate) = gate {
                    let off = Action::Off { generation };
                    self.schedule(gate as f32, p.track, p.lane, off);
                }
            }
            Action::Off { generation } => {
                let previewing = self
                    .preview
                    .is_some_and(|(t, l, _)| t == p.track && l == p.lane);
                if voice.generation == generation && voice.live.is_none() && !previewing {
                    self.note_off(p.track, p.lane);
                }
            }
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

    fn on_tick(&mut self) {
        if self.tick.is_multiple_of(TICKS_PER_STEP) {
            let step = (self.tick / TICKS_PER_STEP) as usize;
            self.choose_pattern(step);
            self.shared.current_step.store(step, Relaxed);
            self.schedule_hits(step);
        }
        self.tick = (self.tick + 1) % (TICKS_PER_STEP * STEPS as u32);
    }

    fn next_sample(&mut self) -> f32 {
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
            } else {
                self.all_notes_off();
            }
            // When following another device, it owns Start/Stop.
            if !external {
                self.send(Msg::one(if playing { midi::START } else { midi::STOP }));
            }
            self.was_playing = playing;
        }

        if external {
            if self.shared.ext_start.swap(false, Relaxed) {
                self.tick = 0;
                self.restart_song = true;
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
                self.send(Msg::one(midi::CLOCK));
                self.on_tick();
                self.samples_to_next_tick += self.samples_per_tick;
            }
            self.samples_to_next_tick -= 1.0;
        }
        self.run_pending();

        if let Some((track, lane, left)) = self.preview {
            if left <= 1 {
                self.preview = None;
                if self.voices[track][lane].live.is_none() {
                    self.note_off(track, lane);
                }
            } else {
                self.preview = Some((track, lane, left - 1));
            }
        }

        let mut mix = 0.0;
        for i in 0..self.track_count() {
            let synth = matches!(self.project.tracks[i].kind, Kind::Synth(_));
            let mut out = 0.0;
            for voice in &mut self.voices[i] {
                let sample = if synth {
                    if voice.synth.is_idle() {
                        continue;
                    }
                    voice.synth.next_sample()
                } else {
                    voice.sample_out()
                };
                out += sample * voice.gain;
            }
            mix += out * self.gains[i];
        }
        (mix * 0.8).clamp(-1.0, 1.0)
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
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
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
                    let s = T::from_sample(engine.next_sample());
                    frame.fill(s);
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
    use crate::project::{Track, EMPTY_GRID, GHOST, PATTERNS};
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

        fn run(&mut self, samples: usize) -> Vec<f32> {
            (0..samples).map(|_| self.engine.next_sample()).collect()
        }

        fn send(&self, event: Event) {
            self.events.send(event).unwrap();
        }

        fn midi(&self) -> Vec<Msg> {
            self.midi.try_iter().collect()
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
            t.patterns = [EMPTY_GRID; PATTERNS];
        }
        p
    }

    fn hit() -> Option<Step> {
        Some(Step::HIT)
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
        let voices = &rig.engine.voices[PAD];
        let sounding = voices.iter().filter(|v| !v.synth.is_idle()).count();
        assert_eq!(sounding, 3, "three voices at once");
    }

    #[test]
    fn song_mode_plays_bars_in_order() {
        let mut project = Project::demo();
        project.tracks[LEAD].patterns[1][2][0] = hit(); // pattern B: D on step 1
        project.song[1] = Some(1);
        let mut rig = Rig::new(project);
        rig.shared.song_mode.store(true, Relaxed);
        rig.shared.playing.store(true, Relaxed);
        // One bar at 120 BPM is 2 seconds = 96_000 samples; play into bar 2.
        rig.run(100_000);
        assert_eq!(rig.shared.song_pos.load(Relaxed), 1);
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

    #[test]
    fn synth_knobs_change_the_sound() {
        let mut rig = Rig::new(quiet_demo());
        let play = |rig: &mut Rig| {
            rig.send(Event::Preview {
                track: LEAD,
                lane: 0,
            });
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
    fn mute_and_solo_pick_what_is_heard() {
        let mut project = quiet_demo();
        project.tracks[KICK].muted = true;
        let mut rig = Rig::new(project);
        rig.send(Event::Preview {
            track: KICK,
            lane: 0,
        });
        assert!(rig.run(4_800).iter().all(|s| *s == 0.0), "muted");

        let mut project = quiet_demo();
        project.tracks[LEAD].solo = true;
        rig.shared.publish(Arc::new(project));
        rig.run(48_000);
        rig.send(Event::Preview {
            track: KICK,
            lane: 0,
        });
        assert!(
            rig.run(4_800).iter().all(|s| *s == 0.0),
            "another track is soloed"
        );
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
        rig.send(Event::Preview { track: i, lane: 0 });
        rig.run(1);
        let rate = rig.engine.voices[i][0].rate;
        assert!((rate - 1.0).abs() < 1e-6, "C4 plays as recorded");
        rig.send(Event::Preview { track: i, lane: 7 });
        rig.run(UPDATE_INTERVAL as usize);
        let fifth = 2f64.powf(7.0 / 12.0);
        let rate = rig.engine.voices[i][7].rate;
        assert!((rate - fifth).abs() < 1e-4, "G4 a fifth up");
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
        let lane = &mut project.tracks[LEAD].patterns[0][0];
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
        project.tracks[HAT].patterns[0][0] = [hit(); STEPS];
        let mut rig = Rig::new(project);
        let times = rig.note_times(STEP * 4, drums::NOTES[2]);
        // Even steps on the grid; odd ones a quarter step (swing/2) late.
        assert_eq!(times, [0, STEP + STEP / 4, 2 * STEP, 3 * STEP + STEP / 4]);
    }

    #[test]
    fn nudge_moves_a_hit_early_or_late() {
        let mut project = quiet_demo();
        let lane = &mut project.tracks[KICK].patterns[0][0];
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
        project.tracks[HAT].patterns[0][0][0] = Some(Step {
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
        project.tracks[KICK].patterns[0][0][0] = Some(Step {
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
        project.tracks[KICK].patterns[0][0][0] = hit();
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
        project.tracks[HAT].patterns[0][0] = [Some(Step {
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
        project.tracks[HAT].patterns[0][0] = [hit(); STEPS];
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
