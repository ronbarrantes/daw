//! Audio engine: runs the step sequencer and mixes drum voices.
//!
//! The UI and the audio thread share state through atomics only, so the
//! audio callback never waits on a lock. Outgoing MIDI is queued on a
//! bounded channel and written by the MIDI thread.

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicUsize, Ordering::Relaxed};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use crate::drums;
use crate::midi::{self, Msg};

pub const TRACKS: usize = 4;
pub const STEPS: usize = 16;
/// MIDI clock runs at 24 ticks per beat, so a 16th-note step is 6 ticks.
const TICKS_PER_STEP: u32 = 6;

pub struct Shared {
    /// One bit per step, one mask per track.
    steps: [AtomicU16; TRACKS],
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
}

impl Shared {
    pub fn new() -> Self {
        let s = Self {
            steps: [const { AtomicU16::new(0) }; TRACKS],
            preview: AtomicU16::new(0),
            bpm_bits: AtomicU32::new(120f32.to_bits()),
            playing: AtomicBool::new(false),
            current_step: AtomicUsize::new(0),
            send_notes: AtomicBool::new(true),
            external_sync: AtomicBool::new(false),
            ext_ticks: AtomicU32::new(0),
            ext_start: AtomicBool::new(false),
        };
        // A basic beat so there's something to hear on first run.
        s.steps[0].store(0b0001_0001_0001_0001, Relaxed);
        s.steps[1].store(0b0001_0000_0001_0000, Relaxed);
        s.steps[2].store(0b0100_0100_0100_0100, Relaxed);
        s
    }

    pub fn is_on(&self, track: usize, step: usize) -> bool {
        self.steps[track].load(Relaxed) & (1 << step) != 0
    }

    pub fn toggle(&self, track: usize, step: usize) {
        self.steps[track].fetch_xor(1 << step, Relaxed);
    }

    pub fn clear(&self) {
        for t in &self.steps {
            t.store(0, Relaxed);
        }
    }

    pub fn preview(&self, track: usize) {
        self.preview.fetch_or(1 << track, Relaxed);
    }

    pub fn bpm(&self) -> f32 {
        f32::from_bits(self.bpm_bits.load(Relaxed))
    }

    pub fn set_bpm(&self, bpm: f32) {
        self.bpm_bits.store(bpm.to_bits(), Relaxed);
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

    fn on_tick(&mut self) {
        // Drum notes last one tick, then get their note-off.
        self.release_notes();
        if self.tick.is_multiple_of(TICKS_PER_STEP) {
            let step = (self.tick / TICKS_PER_STEP) as usize;
            let mask = (0..TRACKS)
                .filter(|&t| self.shared.is_on(t, step))
                .fold(0u16, |m, t| m | 1 << t);
            self.trigger(mask);
            self.shared.current_step.store(step, Relaxed);
            if self.shared.send_notes.load(Relaxed) {
                for t in (0..TRACKS).filter(|t| mask & (1 << t) != 0) {
                    self.send(Msg::three(midi::NOTE_ON_CH10, drums::NOTES[t], 100));
                }
                self.held = mask;
            }
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
            } else {
                self.release_notes();
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

        let mut mix = 0.0;
        for (t, sample) in self.kit.iter().enumerate() {
            if let Some(s) = sample.get(self.pos[t]) {
                mix += s;
                self.pos[t] += 1;
            }
        }
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
        assert!(rx.try_iter().all(|m| m.bytes() != [midi::CLOCK]), "no clock echo");
    }
}
