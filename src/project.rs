//! The song being edited: its tracks, their patterns, and the arrangement.
//!
//! The UI owns the editable copy. After each change it publishes an
//! immutable snapshot (`Arc<Project>`) that the audio thread picks up, so
//! tracks can be added and removed without the audio thread ever locking.

use std::sync::Arc;

use crate::{drums, synth};

pub const STEPS: usize = 16;
pub const PATTERNS: usize = 8;
/// Song length limit, in bars (one pattern per bar).
pub const SONG_SLOTS: usize = 32;
pub const MAX_TRACKS: usize = 16;
/// Pitched samples play at their original pitch on this note (C4).
pub const SAMPLE_ROOT: u8 = 60;
/// The built-in drums are generated at this rate.
const DRUM_RATE: f32 = 48_000.0;

/// Decoded audio, mixed down to mono.
pub struct Sample {
    pub name: String,
    pub data: Vec<f32>,
    pub rate: f32,
}

impl Sample {
    pub fn seconds(&self) -> f32 {
        self.data.len() as f32 / self.rate
    }
}

#[derive(Clone)]
pub enum Kind {
    Synth(synth::Values),
    /// Plays a sample. One-shot samples play as recorded; pitched ones play
    /// higher or lower depending on the note.
    Sampler {
        sample: Arc<Sample>,
        pitched: bool,
    },
}

/// A step's note value that holds the previous note instead of playing a
/// new one.
pub const TIE: u8 = 0x80;
pub const GHOST: u8 = 40;
pub const NORMAL: u8 = 100;
pub const ACCENT: u8 = 127;
pub const MAX_RATCHET: u8 = 4;

/// One step that plays.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Step {
    /// For melodic tracks, the semitone above the track's octave C, or
    /// `TIE`. One-shot tracks ignore it.
    pub note: u8,
    /// Loudness, 1-127 like MIDI. Low values make ghost notes.
    pub velocity: u8,
    /// Percent chance the step plays each time around, 1-100.
    pub chance: u8,
    /// How many times the step repeats within itself, 1-4 (a roll).
    pub ratchet: u8,
    /// Timing shift, -50 to 50 percent of a step (negative = early).
    pub nudge: i8,
}

impl Step {
    pub const fn note(note: u8) -> Self {
        Self {
            note,
            velocity: NORMAL,
            chance: 100,
            ratchet: 1,
            nudge: 0,
        }
    }

    pub const fn tie() -> Self {
        Self::note(TIE)
    }

    pub fn is_tie(&self) -> bool {
        self.note == TIE
    }
}

/// One pattern's steps; `None` is a rest.
pub type Steps = [Option<Step>; STEPS];

#[derive(Clone)]
pub struct Track {
    pub name: String,
    pub kind: Kind,
    pub patterns: [Steps; PATTERNS],
    pub volume: f32,
    pub muted: bool,
    /// When any track is soloed, only soloed tracks are heard.
    pub solo: bool,
    /// 0-1: random variation in each hit's timing and loudness.
    pub humanize: f32,
    pub octave: u8,
    /// MIDI channel (0-based) to send this track's notes on, if any.
    pub midi_channel: Option<u8>,
    /// Note sent over MIDI by one-shot tracks, e.g. 36 for a kick.
    pub midi_note: u8,
}

impl Track {
    pub fn synth(name: &str) -> Self {
        Self::new(name, Kind::Synth(synth::PARAMS.map(|p| p.default)), 3)
    }

    pub fn sampler(sample: Arc<Sample>, pitched: bool) -> Self {
        // Octave 4 puts C on SAMPLE_ROOT, so C plays the sample as recorded.
        let name = sample.name.clone();
        Self::new(&name, Kind::Sampler { sample, pitched }, 4)
    }

    fn new(name: &str, kind: Kind, octave: u8) -> Self {
        Self {
            name: name.to_string(),
            kind,
            patterns: [[None; STEPS]; PATTERNS],
            volume: 0.8,
            muted: false,
            solo: false,
            humanize: 0.0,
            octave,
            midi_channel: None,
            midi_note: SAMPLE_ROOT,
        }
    }

    /// Melodic tracks get a piano roll; one-shot tracks get a row of pads.
    pub fn is_melodic(&self) -> bool {
        match self.kind {
            Kind::Synth(_) => true,
            Kind::Sampler { pitched, .. } => pitched,
        }
    }

    /// The MIDI note a step value plays.
    pub fn note(&self, semitone: u8) -> u8 {
        if self.is_melodic() {
            12 * (self.octave + 1) + semitone
        } else {
            self.midi_note
        }
    }
}

#[derive(Clone)]
pub struct Project {
    pub tracks: Vec<Track>,
    /// Pattern index per bar; `None` is a silent bar.
    pub song: [Option<u8>; SONG_SLOTS],
    /// 0-1: how late every other 16th note plays. 0.66 is a triplet shuffle.
    pub swing: f32,
}

impl Project {
    pub fn empty() -> Self {
        Self {
            tracks: Vec::new(),
            song: [None; SONG_SLOTS],
            swing: 0.0,
        }
    }

    /// Whether a track should be heard, given mute and solo.
    pub fn audible(&self, i: usize) -> bool {
        let any_solo = self.tracks.iter().any(|t| t.solo);
        let t = &self.tracks[i];
        !t.muted && (t.solo || !any_solo)
    }

    /// Bars up to and including the last filled one.
    pub fn song_len(&self) -> usize {
        self.song
            .iter()
            .rposition(Option::is_some)
            .map_or(0, |b| b + 1)
    }

    /// Built-in drums and a synth lead, with a beat and a riff in pattern A.
    pub fn demo() -> Self {
        let mut tracks: Vec<Track> = drums::make_kit(DRUM_RATE)
            .into_iter()
            .enumerate()
            .map(|(i, data)| {
                let sample = Sample {
                    name: drums::NAMES[i].to_string(),
                    data,
                    rate: DRUM_RATE,
                };
                let mut t = Track::sampler(Arc::new(sample), false);
                t.midi_channel = Some(9); // channel 10, the drum channel
                t.midi_note = drums::NOTES[i];
                t
            })
            .collect();
        let beat: [&[usize]; 3] = [&[0, 4, 8, 12], &[4, 12], &[2, 6, 10, 14]];
        for (track, steps) in tracks.iter_mut().zip(beat) {
            for &s in steps {
                track.patterns[0][s] = Some(Step::note(0));
            }
        }
        // A ghost snare and a hi-hat roll, to show off the groove features.
        tracks[1].patterns[0][7] = Some(Step {
            velocity: GHOST,
            ..Step::note(0)
        });
        tracks[2].patterns[0][15] = Some(Step {
            ratchet: 2,
            velocity: 70,
            ..Step::note(0)
        });

        let mut lead = Track::synth("Lead");
        lead.midi_channel = Some(0);
        for (step, note) in [(0, 0), (3, 3), (6, 7), (8, 10), (10, 7), (12, 3), (14, 5)] {
            lead.patterns[0][step] = Some(Step::note(note));
        }
        tracks.push(lead);

        let mut song = [None; SONG_SLOTS];
        song[0] = Some(0);
        Self {
            tracks,
            song,
            swing: 0.0,
        }
    }
}
