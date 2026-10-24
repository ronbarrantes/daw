//! Audio engine: runs the sequencer and mixes every track.
//!
//! The audio callback never waits on a lock. Transport state lives in
//! atomics; the project (tracks and patterns) arrives as an immutable
//! snapshot that the engine picks up with `try_lock`; one-off requests
//! like "play this note now" arrive on a channel. Outgoing MIDI is queued
//! on a bounded channel and written by the MIDI thread.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicUsize, Ordering::Relaxed};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use crate::midi::{self, Msg};
use crate::project::{Kind, Project, Sample, MAX_TRACKS, SAMPLE_ROOT, STEPS};
use crate::synth::Voice;

/// MIDI clock runs at 24 ticks per beat, so a 16th-note step is 6 ticks.
const TICKS_PER_STEP: u32 = 6;
/// Marks "no pattern" (a silent song bar).
const NONE: u8 = u8::MAX;
/// How long a clicked note sounds, in seconds.
const PREVIEW_SECONDS: f32 = 0.3;
/// How often the engine checks for a new project and events, in samples.
const UPDATE_INTERVAL: u32 = 64;

/// Requests from the UI or MIDI in.
pub enum Event {
    /// Play a step value on a track right now.
    Note { track: usize, semitone: u8 },
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

/// Playback state for one track.
struct TrackVoice {
    synth: Voice,
    /// The sample being played, if any. Holding the `Arc` keeps it alive
    /// even if the track is deleted mid-note.
    sample: Option<Arc<Sample>>,
    pos: f64,
    rate: f64,
    /// MIDI note (and channel) still on.
    held: Option<(u8, u8)>,
}

impl TrackVoice {
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

struct Engine {
    shared: Arc<Shared>,
    project: Arc<Project>,
    events: Receiver<Event>,
    midi_out: SyncSender<Msg>,
    voices: Vec<TrackVoice>,
    /// Clock tick within the bar, 0..TICKS_PER_STEP * STEPS.
    tick: u32,
    samples_to_next_tick: f32,
    samples_to_update: u32,
    /// Track whose clicked note is sounding, and samples until it stops.
    preview: Option<(usize, u32)>,
    /// Pattern playing in this bar; `None` is a silent song bar.
    pattern: Option<usize>,
    song_pos: usize,
    /// The next bar starts the song from the top instead of advancing.
    restart_song: bool,
    was_song_mode: bool,
    was_playing: bool,
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
            .map(|_| TrackVoice {
                synth: Voice::new(sr),
                sample: None,
                pos: 0.0,
                rate: 1.0,
                held: None,
            })
            .collect();
        Self {
            project: shared.project(),
            shared,
            events,
            midi_out,
            voices,
            tick: 0,
            samples_to_next_tick: 0.0,
            samples_to_update: 0,
            preview: None,
            pattern: Some(0),
            song_pos: 0,
            restart_song: true,
            was_song_mode: false,
            was_playing: false,
            sr,
        }
    }

    fn track_count(&self) -> usize {
        self.project.tracks.len().min(MAX_TRACKS)
    }

    /// Never blocks: if the MIDI thread falls behind, the message is dropped.
    fn send(&self, msg: Msg) {
        let _ = self.midi_out.try_send(msg);
    }

    fn note_on(&mut self, i: usize, semitone: u8) {
        self.note_off(i);
        let project = self.project.clone();
        let track = &project.tracks[i];
        let note = track.note(semitone);
        if let Some(ch) = track.midi_channel {
            self.send(Msg::three(0x90 | ch, note, 100));
            self.voices[i].held = Some((note, ch));
        }
        let voice = &mut self.voices[i];
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
    }

    /// Ends a note. Samples keep ringing to their end; synths release.
    fn note_off(&mut self, i: usize) {
        if let Some((note, ch)) = self.voices[i].held.take() {
            self.send(Msg::three(0x80 | ch, note, 0));
        }
        self.voices[i].synth.note_off();
    }

    fn all_notes_off(&mut self) {
        for i in 0..MAX_TRACKS {
            self.note_off(i);
        }
    }

    /// Picks up a new project snapshot, synth settings and UI events.
    fn update(&mut self) {
        if let Ok(p) = self.shared.project.try_lock() {
            if !Arc::ptr_eq(&p, &self.project) {
                self.project = p.clone();
            }
        }
        let project = self.project.clone();
        for (voice, track) in self.voices.iter_mut().zip(&project.tracks) {
            if let Kind::Synth(values) = &track.kind {
                voice.synth.set_params(values);
            }
        }
        while let Ok(Event::Note { track, semitone }) = self.events.try_recv() {
            if track < self.track_count() {
                self.note_on(track, semitone);
                self.preview = Some((track, (PREVIEW_SECONDS * self.sr) as u32));
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

    fn on_tick(&mut self) {
        let step = (self.tick / TICKS_PER_STEP) as usize;
        let sub = self.tick % TICKS_PER_STEP;
        if sub == 0 {
            self.choose_pattern(step);
            self.shared.current_step.store(step, Relaxed);
            if let Some(pattern) = self.pattern {
                for i in 0..self.track_count() {
                    if let Some(semitone) = self.project.tracks[i].patterns[pattern][step] {
                        self.note_on(i, semitone);
                    }
                }
            }
        } else if sub == TICKS_PER_STEP - 1 {
            // Release just before the next step so repeated notes retrigger.
            let previewing = self.preview.map(|(t, _)| t);
            for i in (0..MAX_TRACKS).filter(|&i| Some(i) != previewing) {
                self.note_off(i);
            }
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
            let ticks = self.shared.ext_ticks.swap(0, Relaxed);
            if playing {
                for _ in 0..ticks {
                    self.on_tick();
                }
            }
        } else if playing {
            if self.samples_to_next_tick <= 0.0 {
                self.send(Msg::one(midi::CLOCK));
                self.on_tick();
                self.samples_to_next_tick += self.sr * 60.0 / self.shared.bpm() / 24.0;
            }
            self.samples_to_next_tick -= 1.0;
        }

        if let Some((track, left)) = self.preview {
            if left <= 1 {
                self.preview = None;
                self.note_off(track);
            } else {
                self.preview = Some((track, left - 1));
            }
        }

        let mut mix = 0.0;
        for i in 0..self.track_count() {
            let track = &self.project.tracks[i];
            let voice = &mut self.voices[i];
            let out = match track.kind {
                Kind::Synth(_) => voice.synth.next_sample(),
                Kind::Sampler { .. } => voice.sample_out(),
            };
            if !track.muted {
                mix += out * track.volume;
            }
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
    use crate::project::{Track, PATTERNS, SONG_SLOTS};
    use crate::{drums, synth};
    use std::sync::mpsc::sync_channel;

    const KICK: usize = 0;
    const LEAD: usize = 4;

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
            let (midi_tx, midi) = sync_channel(4096);
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

        fn midi(&self) -> Vec<Msg> {
            self.midi.try_iter().collect()
        }
    }

    fn quiet_demo() -> Project {
        let mut p = Project::demo();
        for t in &mut p.tracks {
            t.patterns = [[None; STEPS]; PATTERNS];
        }
        p
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
        project.tracks[LEAD].midi_channel = Some(1); // channel 2
        let mut rig = Rig::new(project);
        rig.shared.playing.store(true, Relaxed);
        // One step at 120 BPM is 6000 samples.
        rig.run(6_000);
        let msgs = rig.midi();
        assert!(msgs.contains(&Msg::three(0x91, 48, 100)), "C3 on channel 2");
        assert!(msgs.contains(&Msg::three(0x81, 48, 0)), "and its note-off");
    }

    #[test]
    fn song_mode_plays_bars_in_order() {
        let mut project = Project::demo();
        project.tracks[LEAD].patterns[1][0] = Some(2); // pattern B: D on step 1
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
            rig.events
                .send(Event::Note {
                    track: LEAD,
                    semitone: 0,
                })
                .unwrap();
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
    fn mute_silences_a_track() {
        let mut project = quiet_demo();
        project.tracks[KICK].muted = true;
        let mut rig = Rig::new(project);
        rig.events
            .send(Event::Note {
                track: KICK,
                semitone: 0,
            })
            .unwrap();
        let out = rig.run(4_800);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn pitched_samples_follow_the_note() {
        let data = vec![0.5; 48_000];
        let sample = Arc::new(Sample {
            name: "tone".into(),
            data,
            rate: 48_000.0,
        });
        let mut project = quiet_demo();
        project.tracks.push(Track::sampler(sample, true));
        let i = project.tracks.len() - 1;
        let mut rig = Rig::new(project);
        rig.events
            .send(Event::Note {
                track: i,
                semitone: 0,
            })
            .unwrap();
        rig.run(1);
        assert!(
            (rig.engine.voices[i].rate - 1.0).abs() < 1e-6,
            "C4 plays as recorded"
        );
        rig.events
            .send(Event::Note {
                track: i,
                semitone: 7,
            })
            .unwrap();
        rig.run(UPDATE_INTERVAL as usize);
        let fifth = 2f64.powf(7.0 / 12.0);
        assert!(
            (rig.engine.voices[i].rate - fifth).abs() < 1e-4,
            "G4 a fifth up"
        );
    }

    #[test]
    fn deleting_tracks_mid_note_is_safe() {
        let mut rig = Rig::new(Project::demo());
        rig.shared.playing.store(true, Relaxed);
        rig.run(1_000);
        let empty = Project {
            tracks: vec![],
            song: [None; SONG_SLOTS],
        };
        rig.shared.publish(Arc::new(empty));
        let out = rig.run(10_000);
        assert!(out.iter().all(|s| s.is_finite()));
    }
}
