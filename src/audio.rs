//! Audio engine: runs the step sequencer and mixes drum voices.
//!
//! The UI and the audio thread share state through atomics only, so the
//! audio callback never waits on a lock.

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicUsize, Ordering::Relaxed};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use crate::drums;

pub const TRACKS: usize = 4;
pub const STEPS: usize = 16;

pub struct Shared {
    /// One bit per step, one mask per track.
    steps: [AtomicU16; TRACKS],
    /// Tracks the UI wants to hear right now (one bit per track).
    preview: AtomicU16,
    bpm_bits: AtomicU32,
    pub playing: AtomicBool,
    pub current_step: AtomicUsize,
}

impl Shared {
    pub fn new() -> Self {
        let s = Self {
            steps: [const { AtomicU16::new(0) }; TRACKS],
            preview: AtomicU16::new(0),
            bpm_bits: AtomicU32::new(120f32.to_bits()),
            playing: AtomicBool::new(false),
            current_step: AtomicUsize::new(0),
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
    kit: Vec<Vec<f32>>,
    /// Playback position per track; past the end means silent.
    pos: [usize; TRACKS],
    step: usize,
    samples_to_next_step: f32,
    was_playing: bool,
    sr: f32,
}

impl Engine {
    fn trigger(&mut self, mask: u16) {
        for t in 0..TRACKS {
            if mask & (1 << t) != 0 {
                self.pos[t] = 0;
            }
        }
    }

    fn next_sample(&mut self) -> f32 {
        let playing = self.shared.playing.load(Relaxed);
        if playing {
            if !self.was_playing {
                self.step = 0;
                self.samples_to_next_step = 0.0;
            }
            if self.samples_to_next_step <= 0.0 {
                let mask = (0..TRACKS)
                    .filter(|&t| self.shared.is_on(t, self.step))
                    .fold(0u16, |m, t| m | 1 << t);
                self.trigger(mask);
                self.shared.current_step.store(self.step, Relaxed);
                self.step = (self.step + 1) % STEPS;
                // 16th notes: four steps per beat.
                self.samples_to_next_step += self.sr * 60.0 / self.shared.bpm() / 4.0;
            }
            self.samples_to_next_step -= 1.0;
        }
        self.was_playing = playing;

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
pub fn start(shared: Arc<Shared>) -> Result<cpal::Stream, String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no audio output device found")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let sr = supported.sample_rate().0 as f32;
    let engine = Engine {
        shared,
        kit: drums::make_kit(sr),
        pos: [usize::MAX; TRACKS],
        step: 0,
        samples_to_next_step: 0.0,
        was_playing: false,
        sr,
    };
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

    #[test]
    fn plays_steps_at_tempo() {
        let shared = Arc::new(Shared::new());
        shared.playing.store(true, Relaxed);
        let sr = 48_000.0;
        let mut engine = Engine {
            shared: shared.clone(),
            kit: drums::make_kit(sr),
            pos: [usize::MAX; TRACKS],
            step: 0,
            samples_to_next_step: 0.0,
            was_playing: false,
            sr,
        };
        // One beat at 120 BPM is half a second = 4 steps.
        let out: Vec<f32> = (0..24_000).map(|_| engine.next_sample()).collect();
        assert!(out.iter().any(|s| s.abs() > 0.1), "should be audible");
        assert!(out.iter().all(|s| s.abs() <= 1.0), "should not clip");
        assert_eq!(shared.current_step.load(Relaxed), 3);
    }
}
