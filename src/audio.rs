//! Audio engine: runs the step sequencer and mixes drums and the synth.
//!
//! The UI and the audio thread share state through atomics only, so the
//! audio callback never waits on a lock. Outgoing MIDI is queued on a
//! bounded channel and written by the MIDI thread.

use std::sync::atomic::{
    AtomicBool, AtomicU16, AtomicU32, AtomicU8, AtomicUsize, Ordering::Relaxed,
};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use crate::drums;
use crate::midi::{self, Msg};
use crate::synth::{self, Voice};

pub const TRACKS: usize = 4;
pub const STEPS: usize = 16;
pub const PATTERNS: usize = 8;
/// Song length limit, in bars (one pattern per bar).
pub const SONG_SLOTS: usize = 32;
/// MIDI clock runs at 24 ticks per beat, so a 16th-note step is 6 ticks.
const TICKS_PER_STEP: u32 = 6;
/// Marks a melody step with no note, or an empty song slot.
const NONE: u8 = u8::MAX;
/// How long a clicked melody note sounds, in seconds.
const PREVIEW_SECONDS: f32 = 0.3;
/// How often the engine re-reads the synth knobs, in samples.
const PARAM_INTERVAL: u32 = 64;

/// A copy of one pattern, for copy and paste.
#[derive(Clone, Copy)]
pub struct PatternData {
    drums: [u16; TRACKS],
    melody: [u8; STEPS],
}

pub struct Shared {
    /// One bit per step, one mask per track, `TRACKS` masks per pattern.
    drums: [AtomicU16; PATTERNS * TRACKS],
    /// Melody note per step, in semitones above the octave's C, or `NONE`.
    melody: [AtomicU8; PATTERNS * STEPS],
    /// Pattern index per bar, or `NONE` for a silent bar.
    song: [AtomicU8; SONG_SLOTS],
    /// Play the song arrangement instead of looping the edited pattern.
    pub song_mode: AtomicBool,
    /// The pattern shown in the editor.
    pub edit_pattern: AtomicU8,
    /// The pattern being heard, or `NONE` during a silent bar.
    pub play_pattern: AtomicU8,
    /// The song bar being heard.
    pub song_pos: AtomicUsize,
    /// Tracks the UI wants to hear right now (one bit per track).
    preview: AtomicU16,
    bpm_bits: AtomicU32,
    pub playing: AtomicBool,
    pub current_step: AtomicUsize,
    /// Send drum hits as MIDI notes on channel 10.
    pub send_notes: AtomicBool,
    /// Follow incoming MIDI clock instead of the BPM setting.
    pub external_sync: AtomicBool,
    /// Clock ticks received from MIDI in, not yet handled by the engine.
    pub ext_ticks: AtomicU32,
    /// Set when MIDI in receives Start, so the engine rewinds to step 1.
    pub ext_start: AtomicBool,
    pub octave: AtomicU8,
    /// MIDI channel for the melody, 0-based (0 = channel 1).
    pub melody_channel: AtomicU8,
    /// Play the melody on the built-in synth (turn off to hear only MIDI gear).
    pub synth_on: AtomicBool,
    /// Synth knob values (f32 bits), indexed like `synth::PARAMS`.
    synth_params: [AtomicU32; synth::PARAMS.len()],
    /// Melody note the UI wants to hear right now, or `NONE`.
    preview_note: AtomicU8,
}

impl Shared {
    pub fn new() -> Self {
        let s = Self {
            drums: [const { AtomicU16::new(0) }; PATTERNS * TRACKS],
            melody: [const { AtomicU8::new(NONE) }; PATTERNS * STEPS],
            song: [const { AtomicU8::new(NONE) }; SONG_SLOTS],
            song_mode: AtomicBool::new(false),
            edit_pattern: AtomicU8::new(0),
            play_pattern: AtomicU8::new(0),
            song_pos: AtomicUsize::new(0),
            preview: AtomicU16::new(0),
            bpm_bits: AtomicU32::new(120f32.to_bits()),
            playing: AtomicBool::new(false),
            current_step: AtomicUsize::new(0),
            send_notes: AtomicBool::new(true),
            external_sync: AtomicBool::new(false),
            ext_ticks: AtomicU32::new(0),
            ext_start: AtomicBool::new(false),
            octave: AtomicU8::new(3),
            melody_channel: AtomicU8::new(0),
            synth_on: AtomicBool::new(true),
            synth_params: [const { AtomicU32::new(0) }; synth::PARAMS.len()],
            preview_note: AtomicU8::new(NONE),
        };
        for (i, p) in synth::PARAMS.iter().enumerate() {
            s.set_synth_param(i, p.default);
        }
        // A basic beat and a little C minor riff in pattern A, so there's
        // something to hear on first run.
        s.drums[0].store(0b0001_0001_0001_0001, Relaxed);
        s.drums[1].store(0b0001_0000_0001_0000, Relaxed);
        s.drums[2].store(0b0100_0100_0100_0100, Relaxed);
        for (step, note) in [(0, 0), (3, 3), (6, 7), (8, 10), (10, 7), (12, 3), (14, 5)] {
            s.melody[step].store(note, Relaxed);
        }
        s.song[0].store(0, Relaxed);
        s
    }

    pub fn is_on(&self, pattern: usize, track: usize, step: usize) -> bool {
        self.drums[pattern * TRACKS + track].load(Relaxed) & (1 << step) != 0
    }

    pub fn toggle(&self, pattern: usize, track: usize, step: usize) {
        self.drums[pattern * TRACKS + track].fetch_xor(1 << step, Relaxed);
    }

    pub fn melody_note(&self, pattern: usize, step: usize) -> Option<u8> {
        Some(self.melody[pattern * STEPS + step].load(Relaxed)).filter(|&n| n != NONE)
    }

    /// Sets the step to `semitone`, or clears it if it already has that note.
    pub fn toggle_melody(&self, pattern: usize, step: usize, semitone: u8) {
        let new = if self.melody_note(pattern, step) == Some(semitone) {
            NONE
        } else {
            semitone
        };
        self.melody[pattern * STEPS + step].store(new, Relaxed);
    }

    pub fn get_pattern(&self, pattern: usize) -> PatternData {
        PatternData {
            drums: std::array::from_fn(|t| self.drums[pattern * TRACKS + t].load(Relaxed)),
            melody: std::array::from_fn(|s| self.melody[pattern * STEPS + s].load(Relaxed)),
        }
    }

    pub fn set_pattern(&self, pattern: usize, data: &PatternData) {
        for (t, mask) in data.drums.iter().enumerate() {
            self.drums[pattern * TRACKS + t].store(*mask, Relaxed);
        }
        for (s, note) in data.melody.iter().enumerate() {
            self.melody[pattern * STEPS + s].store(*note, Relaxed);
        }
    }

    pub fn clear_pattern(&self, pattern: usize) {
        self.set_pattern(
            pattern,
            &PatternData {
                drums: [0; TRACKS],
                melody: [NONE; STEPS],
            },
        );
    }

    pub fn song_slot(&self, bar: usize) -> Option<usize> {
        Some(self.song[bar].load(Relaxed))
            .filter(|&p| p != NONE)
            .map(usize::from)
    }

    pub fn set_song_slot(&self, bar: usize, pattern: Option<usize>) {
        self.song[bar].store(pattern.map_or(NONE, |p| p as u8), Relaxed);
    }

    /// Bars up to and including the last filled one.
    pub fn song_len(&self) -> usize {
        (0..SONG_SLOTS)
            .rev()
            .find(|&b| self.song_slot(b).is_some())
            .map_or(0, |b| b + 1)
    }

    /// MIDI note number of C in the current octave (octave 3 = C3 = 48).
    pub fn base_note(&self) -> u8 {
        12 * (self.octave.load(Relaxed) + 1)
    }

    pub fn preview(&self, track: usize) {
        self.preview.fetch_or(1 << track, Relaxed);
    }

    pub fn preview_note(&self, semitone: u8) {
        self.preview_note.store(semitone, Relaxed);
    }

    pub fn bpm(&self) -> f32 {
        f32::from_bits(self.bpm_bits.load(Relaxed))
    }

    pub fn set_bpm(&self, bpm: f32) {
        self.bpm_bits.store(bpm.to_bits(), Relaxed);
    }

    pub fn synth_param(&self, i: usize) -> f32 {
        f32::from_bits(self.synth_params[i].load(Relaxed))
    }

    pub fn set_synth_param(&self, i: usize, value: f32) {
        self.synth_params[i].store(value.to_bits(), Relaxed);
    }
}

struct Engine {
    shared: Arc<Shared>,
    midi_out: SyncSender<Msg>,
    kit: Vec<Vec<f32>>,
    /// Playback position per track; past the end means silent.
    pos: [usize; TRACKS],
    /// Clock tick within the pattern, 0..TICKS_PER_STEP * STEPS.
    tick: u32,
    samples_to_next_tick: f32,
    /// Tracks whose MIDI note is still on.
    held: u16,
    synth: Voice,
    /// Melody note (and its channel) that is still on.
    melody_held: Option<(u8, u8)>,
    /// Samples left before a clicked melody note is released.
    preview_left: u32,
    /// Pattern playing in this bar; `None` is a silent song bar.
    pattern: Option<usize>,
    song_pos: usize,
    /// The next bar starts the song from the top instead of advancing.
    restart_song: bool,
    was_song_mode: bool,
    samples_to_params: u32,
    was_playing: bool,
    sr: f32,
}

impl Engine {
    fn new(shared: Arc<Shared>, midi_out: SyncSender<Msg>, sr: f32) -> Self {
        Self {
            shared,
            midi_out,
            kit: drums::make_kit(sr),
            pos: [usize::MAX; TRACKS],
            tick: 0,
            samples_to_next_tick: 0.0,
            held: 0,
            synth: Voice::new(sr),
            melody_held: None,
            preview_left: 0,
            pattern: Some(0),
            song_pos: 0,
            restart_song: true,
            was_song_mode: false,
            samples_to_params: 0,
            was_playing: false,
            sr,
        }
    }

    /// Never blocks: if the MIDI thread falls behind, the message is dropped.
    fn send(&self, msg: Msg) {
        let _ = self.midi_out.try_send(msg);
    }

    fn trigger(&mut self, mask: u16) {
        for t in 0..TRACKS {
            if mask & (1 << t) != 0 {
                self.pos[t] = 0;
            }
        }
    }

    fn release_notes(&mut self) {
        for t in 0..TRACKS {
            if self.held & (1 << t) != 0 {
                self.send(Msg::three(midi::NOTE_OFF_CH10, drums::NOTES[t], 0));
            }
        }
        self.held = 0;
    }

    fn melody_on(&mut self, note: u8) {
        self.melody_off();
        let ch = self.shared.melody_channel.load(Relaxed) & 0x0F;
        self.send(Msg::three(0x90 | ch, note, 100));
        self.melody_held = Some((note, ch));
        if self.shared.synth_on.load(Relaxed) {
            self.synth.note_on(note);
        }
    }

    fn melody_off(&mut self) {
        if let Some((note, ch)) = self.melody_held.take() {
            self.send(Msg::three(0x80 | ch, note, 0));
        }
        self.synth.note_off();
    }

    /// Picks which pattern plays for the current step.
    fn choose_pattern(&mut self, step: usize) {
        let song_mode = self.shared.song_mode.load(Relaxed);
        if song_mode != self.was_song_mode {
            self.restart_song = true;
            self.was_song_mode = song_mode;
        }
        let song_len = self.shared.song_len();
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
            self.pattern = self.shared.song_slot(self.song_pos);
            self.shared.song_pos.store(self.song_pos, Relaxed);
        }
        self.shared
            .play_pattern
            .store(self.pattern.map_or(NONE, |p| p as u8), Relaxed);
    }

    fn on_tick(&mut self) {
        // Drum notes last one tick, then get their note-off.
        self.release_notes();
        if self.tick.is_multiple_of(TICKS_PER_STEP) {
            let step = (self.tick / TICKS_PER_STEP) as usize;
            self.choose_pattern(step);
            let Some(pattern) = self.pattern else {
                self.shared.current_step.store(step, Relaxed);
                self.melody_off();
                self.tick = (self.tick + 1) % (TICKS_PER_STEP * STEPS as u32);
                return;
            };
            let mask = (0..TRACKS)
                .filter(|&t| self.shared.is_on(pattern, t, step))
                .fold(0u16, |m, t| m | 1 << t);
            self.trigger(mask);
            self.shared.current_step.store(step, Relaxed);
            if self.shared.send_notes.load(Relaxed) {
                for t in (0..TRACKS).filter(|t| mask & (1 << t) != 0) {
                    self.send(Msg::three(midi::NOTE_ON_CH10, drums::NOTES[t], 100));
                }
                self.held = mask;
            }
            if let Some(semitone) = self.shared.melody_note(pattern, step) {
                self.melody_on(self.shared.base_note() + semitone);
            }
        } else if self.tick % TICKS_PER_STEP == TICKS_PER_STEP - 1 && self.preview_left == 0 {
            // Release just before the next step so repeated notes retrigger.
            self.melody_off();
        }
        self.tick = (self.tick + 1) % (TICKS_PER_STEP * STEPS as u32);
    }

    fn next_sample(&mut self) -> f32 {
        let playing = self.shared.playing.load(Relaxed);
        let external = self.shared.external_sync.load(Relaxed);

        if playing != self.was_playing {
            if playing {
                self.tick = 0;
                self.samples_to_next_tick = 0.0;
                self.restart_song = true;
            } else {
                self.release_notes();
                self.melody_off();
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

        let preview = self.shared.preview.swap(0, Relaxed);
        if preview != 0 {
            self.trigger(preview);
        }
        let note = self.shared.preview_note.swap(NONE, Relaxed);
        if note != NONE {
            self.melody_on(self.shared.base_note() + note);
            self.preview_left = (PREVIEW_SECONDS * self.sr) as u32;
        }
        if self.preview_left > 0 {
            self.preview_left -= 1;
            if self.preview_left == 0 {
                self.melody_off();
            }
        }

        if self.samples_to_params == 0 {
            let values = std::array::from_fn(|i| self.shared.synth_param(i));
            self.synth.set_params(&values);
            self.samples_to_params = PARAM_INTERVAL;
        }
        self.samples_to_params -= 1;

        let mut mix = 0.0;
        for (t, sample) in self.kit.iter().enumerate() {
            if let Some(s) = sample.get(self.pos[t]) {
                mix += s;
                self.pos[t] += 1;
            }
        }
        mix += self.synth.next_sample();
        (mix * 0.7).clamp(-1.0, 1.0)
    }
}

/// Opens the default output device and starts the engine.
/// Keep the returned stream alive; dropping it stops the sound.
pub fn start(shared: Arc<Shared>, midi_out: SyncSender<Msg>) -> Result<cpal::Stream, String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no audio output device found")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let engine = Engine::new(shared, midi_out, supported.sample_rate().0 as f32);
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
    use std::sync::mpsc::{sync_channel, Receiver};

    fn engine() -> (Engine, Arc<Shared>, Receiver<Msg>) {
        let shared = Arc::new(Shared::new());
        let (tx, rx) = sync_channel(1024);
        (Engine::new(shared.clone(), tx, 48_000.0), shared, rx)
    }

    #[test]
    fn plays_steps_at_tempo() {
        let (mut engine, shared, _rx) = engine();
        shared.playing.store(true, Relaxed);
        // One beat at 120 BPM is half a second = 4 steps.
        let out: Vec<f32> = (0..24_000).map(|_| engine.next_sample()).collect();
        assert!(out.iter().any(|s| s.abs() > 0.1), "should be audible");
        assert!(out.iter().all(|s| s.abs() <= 1.0), "should not clip");
        assert_eq!(shared.current_step.load(Relaxed), 3);
    }

    #[test]
    fn sends_start_clock_and_notes() {
        let (mut engine, shared, rx) = engine();
        shared.playing.store(true, Relaxed);
        for _ in 0..24_000 {
            engine.next_sample();
        }
        let msgs: Vec<Msg> = rx.try_iter().collect();
        assert_eq!(msgs[0].bytes(), [midi::START]);
        let clocks = msgs.iter().filter(|m| m.bytes() == [midi::CLOCK]).count();
        assert_eq!(clocks, 24, "one beat of clock");
        // The default pattern has a kick on step 1.
        assert!(msgs.contains(&Msg::three(midi::NOTE_ON_CH10, drums::NOTES[0], 100)));
        assert!(msgs.contains(&Msg::three(midi::NOTE_OFF_CH10, drums::NOTES[0], 0)));
    }

    #[test]
    fn follows_external_clock() {
        let (mut engine, shared, rx) = engine();
        shared.external_sync.store(true, Relaxed);
        shared.playing.store(true, Relaxed);
        shared.ext_ticks.store(7, Relaxed); // step 1 at tick 0, step 2 at tick 6
        engine.next_sample();
        assert_eq!(shared.current_step.load(Relaxed), 1);
        assert!(
            rx.try_iter().all(|m| m.bytes() != [midi::CLOCK]),
            "no clock echo"
        );
    }

    #[test]
    fn plays_melody_on_its_channel() {
        let (mut engine, shared, rx) = engine();
        shared.melody_channel.store(1, Relaxed); // channel 2
        shared.playing.store(true, Relaxed);
        // One step at 120 BPM is 6000 samples.
        let out: Vec<f32> = (0..6_000).map(|_| engine.next_sample()).collect();
        let msgs: Vec<Msg> = rx.try_iter().collect();
        assert!(msgs.contains(&Msg::three(0x91, 48, 100)), "C3 on channel 2");
        assert!(msgs.contains(&Msg::three(0x81, 48, 0)), "and its note-off");
        assert!(out.iter().all(|s| s.abs() <= 1.0));

        // With drums cleared, what remains is the synth.
        shared.clear_pattern(0);
        shared.preview_note(7);
        let out: Vec<f32> = (0..4_800).map(|_| engine.next_sample()).collect();
        assert!(
            out.iter().any(|s| s.abs() > 0.05),
            "synth should be audible"
        );
    }

    #[test]
    fn song_mode_plays_bars_in_order() {
        let (mut engine, shared, rx) = engine();
        // Pattern B: one melody note, D, on step 1.
        shared.toggle_melody(1, 0, 2);
        shared.set_song_slot(1, Some(1));
        shared.song_mode.store(true, Relaxed);
        shared.playing.store(true, Relaxed);
        // One bar at 120 BPM is 2 seconds = 96_000 samples; play into bar 2.
        for _ in 0..100_000 {
            engine.next_sample();
        }
        assert_eq!(shared.song_pos.load(Relaxed), 1);
        assert_eq!(shared.play_pattern.load(Relaxed), 1);
        let msgs: Vec<Msg> = rx.try_iter().collect();
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
        let (mut engine, shared, _rx) = engine();
        shared.clear_pattern(0);
        let render = |engine: &mut Engine| {
            shared.preview_note(0);
            (0..4_800).map(|_| engine.next_sample().abs()).sum::<f32>()
        };
        let before = render(&mut engine);
        shared.set_synth_param(synth::VOLUME, 0.0);
        for _ in 0..48_000 {
            engine.next_sample(); // let the old note fade
        }
        let after = render(&mut engine);
        assert!(
            before > 1.0 && after < 0.01,
            "volume knob: {before} -> {after}"
        );
    }
}
