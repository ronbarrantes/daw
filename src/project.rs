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
    /// The group this track belongs to, by `Group::id`.
    pub group: Option<u32>,
}

/// A folder of tracks, like a layer group in Photoshop. It can be collapsed,
/// muted, soloed and turned up or down as a whole.
#[derive(Clone)]
pub struct Group {
    pub id: u32,
    pub name: String,
    pub collapsed: bool,
    pub muted: bool,
    pub solo: bool,
    pub volume: f32,
}

/// One line in the track list.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Row {
    /// A group header, by index into `Project::groups`.
    Group(usize),
    Track(usize),
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
            group: None,
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
    pub groups: Vec<Group>,
}

impl Project {
    pub fn empty() -> Self {
        Self {
            tracks: Vec::new(),
            song: [None; SONG_SLOTS],
            swing: 0.0,
            groups: Vec::new(),
        }
    }

    pub fn group(&self, id: u32) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn group_mut(&mut self, id: u32) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.id == id)
    }

    /// How loud a track plays: its volume times its group's, or 0 when
    /// muted or when something else is soloed.
    pub fn gain(&self, i: usize) -> f32 {
        let any_solo = self.tracks.iter().any(|t| t.solo) || self.groups.iter().any(|g| g.solo);
        let t = &self.tracks[i];
        let g = t.group.and_then(|id| self.group(id));
        let muted = t.muted || g.is_some_and(|g| g.muted);
        let soloed = t.solo || g.is_some_and(|g| g.solo);
        if muted || (any_solo && !soloed) {
            0.0
        } else {
            t.volume * g.map_or(1.0, |g| g.volume)
        }
    }

    /// The track list in display order: each group's header followed by
    /// its tracks (even when collapsed), and ungrouped tracks in place.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (i, t) in self.tracks.iter().enumerate() {
            match t
                .group
                .and_then(|id| self.groups.iter().position(|g| g.id == id))
            {
                Some(g) if rows.contains(&Row::Group(g)) => {}
                Some(g) => {
                    rows.push(Row::Group(g));
                    let id = self.groups[g].id;
                    for (j, t) in self.tracks.iter().enumerate() {
                        if t.group == Some(id) {
                            rows.push(Row::Track(j));
                        }
                    }
                }
                None => rows.push(Row::Track(i)),
            }
        }
        rows
    }

    /// Puts tracks into a new group and returns its id.
    pub fn new_group(&mut self, tracks: &[usize], name: &str) -> u32 {
        let id = self.groups.iter().map(|g| g.id + 1).max().unwrap_or(0);
        self.groups.push(Group {
            id,
            name: name.to_string(),
            collapsed: false,
            muted: false,
            solo: false,
            volume: 1.0,
        });
        for &i in tracks {
            self.tracks[i].group = Some(id);
        }
        self.prune_groups();
        id
    }

    /// Takes tracks out of their group, keeping the tracks.
    pub fn ungroup(&mut self, id: u32) {
        for t in &mut self.tracks {
            if t.group == Some(id) {
                t.group = None;
            }
        }
        self.prune_groups();
    }

    /// Drops groups that no longer have any tracks.
    pub fn prune_groups(&mut self) {
        let tracks = &self.tracks;
        self.groups
            .retain(|g| tracks.iter().any(|t| t.group == Some(g.id)));
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
        let mut project = Self {
            tracks,
            song,
            swing: 0.0,
            groups: Vec::new(),
        };
        project.new_group(&[0, 1, 2, 3], "Drums");
        project
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEAD: usize = 4;

    #[test]
    fn groups_mute_solo_and_scale_their_tracks() {
        let mut p = Project::demo();
        let drums = p.groups[0].id;
        let kick = p.gain(0);
        assert!(kick > 0.0);

        p.group_mut(drums).unwrap().volume = 0.5;
        assert_eq!(p.gain(0), kick * 0.5, "group volume");

        p.group_mut(drums).unwrap().muted = true;
        assert_eq!(p.gain(0), 0.0, "group mute");
        assert!(p.gain(LEAD) > 0.0, "other tracks unaffected");

        p.group_mut(drums).unwrap().muted = false;
        p.group_mut(drums).unwrap().solo = true;
        assert!(p.gain(0) > 0.0, "soloed group plays");
        assert_eq!(p.gain(LEAD), 0.0, "everything else is quiet");
    }

    #[test]
    fn rows_put_grouped_tracks_under_their_header() {
        let mut p = Project::demo();
        p.tracks.push(Track::synth("Pad"));
        // A group whose tracks aren't next to each other.
        p.ungroup(p.groups[0].id);
        p.new_group(&[1, 5], "Mix");
        assert_eq!(
            p.rows(),
            [
                Row::Track(0),
                Row::Group(0),
                Row::Track(1),
                Row::Track(5),
                Row::Track(2),
                Row::Track(3),
                Row::Track(4),
            ]
        );
    }

    #[test]
    fn empty_groups_are_removed() {
        let mut p = Project::demo();
        let id = p.groups[0].id;
        for t in &mut p.tracks {
            t.group = None;
        }
        p.prune_groups();
        assert!(p.group(id).is_none());
    }
}
