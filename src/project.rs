//! The song being edited: its tracks, their patterns, and the arrangement.
//!
//! The UI owns the editable copy. After each change it publishes an
//! immutable snapshot (`Arc<Project>`) that the audio thread picks up, so
//! tracks can be added and removed without the audio thread ever locking.

use std::sync::Arc;

use crate::{drums, synth};

pub const STEPS: usize = 16;
/// Patterns a new song starts with (A-H); more can be added.
pub const DEFAULT_PATTERNS: usize = 8;
pub const MAX_PATTERNS: usize = 64;
/// Timeline positions are in steps (16th notes); this caps a song at
/// 512 bars, over 15 minutes at 120 BPM.
pub const MAX_SONG_STEPS: usize = 512 * STEPS;
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
    /// Which built-in drum this is (index into `drums::NAMES`), if any.
    /// Built-ins are regenerated rather than stored in song files.
    pub builtin: Option<usize>,
}

impl Sample {
    pub fn seconds(&self) -> f32 {
        self.data.len() as f32 / self.rate
    }
}

/// The built-in drum sounds, in the order of `drums::NAMES`.
pub fn builtin_drums() -> Vec<Arc<Sample>> {
    drums::make_kit(DRUM_RATE)
        .into_iter()
        .enumerate()
        .map(|(i, data)| {
            Arc::new(Sample {
                name: drums::NAMES[i].to_string(),
                data,
                rate: DRUM_RATE,
                builtin: Some(i),
            })
        })
        .collect()
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

/// Rows in the piano roll: one octave, C to B. Each row is its own lane,
/// so a melodic step can hold a chord.
pub const LANES: usize = 12;
pub const GHOST: u8 = 40;
pub const NORMAL: u8 = 100;
pub const ACCENT: u8 = 127;
pub const MAX_RATCHET: u8 = 4;

/// One note in one lane at one step.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Step {
    /// Holds the lane's previous note instead of playing a new one.
    pub tie: bool,
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
    /// A plain note at normal velocity.
    pub const HIT: Step = Step {
        tie: false,
        velocity: NORMAL,
        chance: 100,
        ratchet: 1,
        nudge: 0,
    };
    pub const TIE: Step = Step {
        tie: true,
        ..Step::HIT
    };

    pub fn is_hit(step: &Option<Step>) -> bool {
        step.is_some_and(|s| !s.tie)
    }
}

/// One lane's steps in one pattern; `None` is a rest.
pub type Lane = [Option<Step>; STEPS];
/// One pattern of a track: a lane per piano-roll row. One-shot tracks only
/// use lane 0.
pub type Grid = [Lane; LANES];
pub const EMPTY_GRID: Grid = [[None; STEPS]; LANES];

#[derive(Clone)]
pub struct Track {
    pub name: String,
    pub kind: Kind,
    /// One grid per pattern, in the order of `Project::patterns`.
    pub patterns: Vec<Grid>,
    pub volume: f32,
    /// Stereo position, -1 (left) to 1 (right).
    pub pan: f32,
    /// How much goes to the shared reverb and delay, 0-1.
    pub reverb: f32,
    pub delay: f32,
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
            patterns: vec![EMPTY_GRID; DEFAULT_PATTERNS],
            volume: 0.8,
            pan: 0.0,
            reverb: 0.0,
            delay: 0.0,
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

    /// How many lanes the track uses: a piano roll's worth, or one.
    pub fn lanes(&self) -> usize {
        if self.is_melodic() {
            LANES
        } else {
            1
        }
    }

    /// MIDI note number of C in the track's octave (octave 3 = C3 = 48).
    pub fn base_note(&self) -> u8 {
        12 * (self.octave + 1)
    }

    /// The MIDI note a lane plays.
    pub fn note(&self, lane: usize) -> u8 {
        if self.is_melodic() {
            self.base_note() + lane as u8
        } else {
            self.midi_note
        }
    }

    /// The lane an incoming MIDI note belongs to (wrapping into the octave).
    pub fn lane_for(&self, note: u8) -> usize {
        if self.is_melodic() {
            (note as i32 - self.base_note() as i32).rem_euclid(LANES as i32) as usize
        } else {
            0
        }
    }

    /// Whether any lane starts a note at this step.
    pub fn hit_at(&self, pattern: usize, step: usize) -> bool {
        self.patterns
            .get(pattern)
            .is_some_and(|grid| grid.iter().any(|lane| Step::is_hit(&lane[step])))
    }
}

/// A pattern's name; its notes live in each track's `patterns`.
#[derive(Clone, PartialEq, Debug)]
pub struct Pattern {
    pub name: String,
}

/// A pattern placed on the timeline. Clips point to their pattern, so
/// editing the pattern changes every clip of it ("Make unique" gives a
/// clip its own copy). Positions and lengths are in steps (16th notes).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Clip {
    pub pattern: usize,
    /// Which timeline row it sits on; rows are only for layout.
    pub row: usize,
    pub start: usize,
    pub len: usize,
    /// Where in the pattern the clip begins, when its start is trimmed.
    pub offset: usize,
}

impl Clip {
    pub fn end(&self) -> usize {
        self.start + self.len
    }

    /// The pattern step playing at song step `at`, if the clip covers it.
    /// A clip longer than its pattern repeats it.
    pub fn step_at(&self, at: usize) -> Option<usize> {
        (self.start..self.end())
            .contains(&at)
            .then(|| (self.offset + at - self.start) % STEPS)
    }
}

/// Settings for the shared reverb and delay.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Effects {
    /// 0-1: how long the reverb rings.
    pub reverb_size: f32,
    /// 0-1: muffled to bright.
    pub reverb_tone: f32,
    /// Delay time in 16th-note steps, 1-8.
    pub delay_steps: f32,
    /// 0-0.9: how much each echo repeats.
    pub delay_feedback: f32,
}

impl Effects {
    pub const DEFAULT: Effects = Effects {
        reverb_size: 0.5,
        reverb_tone: 0.5,
        delay_steps: 3.0,
        delay_feedback: 0.35,
    };
}

/// Patterns A to H, as every new song starts with.
pub fn default_patterns() -> Vec<Pattern> {
    (0..DEFAULT_PATTERNS)
        .map(|i| Pattern {
            name: char::from(b'A' + i as u8).to_string(),
        })
        .collect()
}

#[derive(Clone)]
pub struct Project {
    pub tracks: Vec<Track>,
    /// The pattern library; every track has a grid for each.
    pub patterns: Vec<Pattern>,
    /// The timeline.
    pub clips: Vec<Clip>,
    /// 0-1: how late every other 16th note plays. 0.66 is a triplet shuffle.
    pub swing: f32,
    pub groups: Vec<Group>,
    pub effects: Effects,
}

impl Project {
    pub fn empty() -> Self {
        Self {
            tracks: Vec::new(),
            patterns: default_patterns(),
            clips: Vec::new(),
            swing: 0.0,
            groups: Vec::new(),
            effects: Effects::DEFAULT,
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

    /// Steps up to the end of the last clip.
    pub fn song_len(&self) -> usize {
        self.clips.iter().map(Clip::end).max().unwrap_or(0)
    }

    /// Adds a track, giving it a grid for every pattern.
    pub fn add_track(&mut self, mut track: Track) {
        track.patterns.resize(self.patterns.len(), EMPTY_GRID);
        self.tracks.push(track);
    }

    /// A name not used by any pattern yet: `base`, or `base 2`, `base 3`...
    fn unused_name(&self, base: &str) -> String {
        let taken = |n: &str| self.patterns.iter().any(|p| p.name == n);
        if !taken(base) {
            return base.to_string();
        }
        (2..)
            .map(|i| format!("{base} {i}"))
            .find(|n| !taken(n))
            .expect("some number is free")
    }

    /// Adds an empty pattern and returns its index, unless the library is full.
    pub fn new_pattern(&mut self) -> Option<usize> {
        if self.patterns.len() >= MAX_PATTERNS {
            return None;
        }
        // The next letter, or "P9" style names past Z.
        let n = self.patterns.len();
        let base = if n < 26 {
            char::from(b'A' + n as u8).to_string()
        } else {
            format!("P{}", n + 1)
        };
        let name = self.unused_name(&base);
        self.patterns.push(Pattern { name });
        for t in &mut self.tracks {
            t.patterns.push(EMPTY_GRID);
        }
        Some(self.patterns.len() - 1)
    }

    /// Copies a pattern (notes and all) and returns the copy's index.
    pub fn duplicate_pattern(&mut self, p: usize) -> Option<usize> {
        if p >= self.patterns.len() || self.patterns.len() >= MAX_PATTERNS {
            return None;
        }
        let name = self.unused_name(&self.patterns[p].name);
        self.patterns.push(Pattern { name });
        for t in &mut self.tracks {
            let grid = t.patterns.get(p).copied().unwrap_or(EMPTY_GRID);
            t.patterns.push(grid);
        }
        Some(self.patterns.len() - 1)
    }

    /// Removes a pattern and its clips. The last pattern can't be removed.
    pub fn delete_pattern(&mut self, p: usize) {
        if p >= self.patterns.len() || self.patterns.len() == 1 {
            return;
        }
        self.patterns.remove(p);
        for t in &mut self.tracks {
            if p < t.patterns.len() {
                t.patterns.remove(p);
            }
        }
        self.clips.retain(|c| c.pattern != p);
        for c in &mut self.clips {
            if c.pattern > p {
                c.pattern -= 1;
            }
        }
    }

    /// Gives a clip its own copy of its pattern, so editing it no longer
    /// changes the other clips. Returns the new pattern.
    pub fn make_unique(&mut self, clip: usize) -> Option<usize> {
        let p = self.clips.get(clip)?.pattern;
        let copy = self.duplicate_pattern(p)?;
        self.clips[clip].pattern = copy;
        Some(copy)
    }

    /// Splits a clip in two at song step `at`. Returns the right half's
    /// index, or `None` if `at` isn't inside the clip.
    pub fn split_clip(&mut self, clip: usize, at: usize) -> Option<usize> {
        let c = *self.clips.get(clip)?;
        if at <= c.start || at >= c.end() {
            return None;
        }
        self.clips[clip].len = at - c.start;
        self.clips.push(Clip {
            start: at,
            len: c.end() - at,
            offset: (c.offset + at - c.start) % STEPS,
            ..c
        });
        Some(self.clips.len() - 1)
    }

    /// How many clips use a pattern.
    pub fn uses(&self, p: usize) -> usize {
        self.clips.iter().filter(|c| c.pattern == p).count()
    }

    /// Built-in drums and a synth lead, with a beat and a riff in pattern A.
    pub fn demo() -> Self {
        let mut tracks: Vec<Track> = builtin_drums()
            .into_iter()
            .enumerate()
            .map(|(i, sample)| {
                let mut t = Track::sampler(sample, false);
                t.midi_channel = Some(9); // channel 10, the drum channel
                t.midi_note = drums::NOTES[i];
                t
            })
            .collect();
        let beat: [&[usize]; 3] = [&[0, 4, 8, 12], &[4, 12], &[2, 6, 10, 14]];
        for (track, steps) in tracks.iter_mut().zip(beat) {
            for &s in steps {
                track.patterns[0][0][s] = Some(Step::HIT);
            }
        }
        // A ghost snare and a hi-hat roll, to show off the groove features.
        tracks[1].patterns[0][0][7] = Some(Step {
            velocity: GHOST,
            ..Step::HIT
        });
        tracks[2].patterns[0][0][15] = Some(Step {
            ratchet: 2,
            velocity: 70,
            ..Step::HIT
        });

        let mut lead = Track::synth("Lead");
        lead.midi_channel = Some(0);
        for (step, lane) in [(0, 0), (3, 3), (6, 7), (8, 10), (10, 7), (12, 3), (14, 5)] {
            lead.patterns[0][lane][step] = Some(Step::HIT);
        }
        tracks.push(lead);

        // Soft held chords, to show off polyphony: C minor, then A♭ major.
        let mut pad = Track::synth("Pad");
        pad.midi_channel = Some(1);
        pad.volume = 0.5;
        pad.reverb = 0.35;
        pad.octave = 4;
        if let Kind::Synth(v) = &mut pad.kind {
            v[synth::WAVE] = 0.3;
            v[synth::DETUNE] = 0.6;
            v[synth::CUTOFF] = 900.0;
            v[synth::ENV_AMOUNT] = 0.2;
            v[synth::ATTACK] = 0.25;
            v[synth::SUSTAIN] = 0.9;
            v[synth::RELEASE] = 0.6;
        }
        for (start, chord) in [(0, [0, 3, 7]), (8, [8, 0, 3])] {
            for lane in chord {
                pad.patterns[0][lane][start] = Some(Step::HIT);
                for step in start + 1..start + 7 {
                    pad.patterns[0][lane][step] = Some(Step::TIE);
                }
            }
        }
        tracks.push(pad);

        let mut project = Self {
            tracks,
            patterns: default_patterns(),
            clips: vec![Clip {
                pattern: 0,
                row: 0,
                start: 0,
                len: STEPS,
                offset: 0,
            }],
            swing: 0.0,
            groups: Vec::new(),
            effects: Effects::DEFAULT,
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

    fn clip(pattern: usize, start: usize, len: usize) -> Clip {
        Clip {
            pattern,
            row: 0,
            start,
            len,
            offset: 0,
        }
    }

    #[test]
    fn clips_repeat_their_pattern_and_can_be_split() {
        let mut p = Project::demo();
        p.clips = vec![clip(1, 16, 40)];
        let c = p.clips[0];
        assert_eq!(c.step_at(15), None);
        assert_eq!(c.step_at(16), Some(0));
        assert_eq!(c.step_at(33), Some(1), "repeats after 16 steps");
        assert_eq!(c.step_at(56), None, "ends after 40 steps");
        assert_eq!(p.song_len(), 56);

        let right = p.split_clip(0, 22).unwrap();
        assert_eq!((p.clips[0].start, p.clips[0].len), (16, 6));
        let r = p.clips[right];
        assert_eq!((r.start, r.len, r.offset), (22, 34, 6));
        assert_eq!(r.step_at(22), Some(6), "carries on where it was split");
        assert_eq!(p.split_clip(0, 16), None, "not at the very edge");
    }

    #[test]
    fn make_unique_copies_the_pattern_for_one_clip() {
        let mut p = Project::demo();
        p.clips = vec![clip(0, 0, 16), clip(0, 16, 16)];
        let copy = p.make_unique(1).unwrap();
        assert_eq!(p.patterns[copy].name, "A 2");
        assert_eq!(p.clips[0].pattern, 0, "the other clip stays linked");
        assert_eq!(p.clips[1].pattern, copy);
        for t in &p.tracks {
            assert_eq!(t.patterns.len(), p.patterns.len());
            assert_eq!(t.patterns[copy], t.patterns[0], "same notes to start");
        }
        p.tracks[0].patterns[copy][0][1] = Some(Step::HIT);
        assert_ne!(p.tracks[0].patterns[copy], p.tracks[0].patterns[0]);
    }

    #[test]
    fn deleting_a_pattern_removes_its_clips_and_renumbers() {
        let mut p = Project::demo();
        p.clips = vec![clip(0, 0, 16), clip(2, 16, 16), clip(5, 32, 16)];
        p.delete_pattern(2);
        assert_eq!(p.patterns.len(), DEFAULT_PATTERNS - 1);
        assert_eq!(p.patterns[2].name, "D");
        let patterns: Vec<usize> = p.clips.iter().map(|c| c.pattern).collect();
        assert_eq!(patterns, [0, 4]);
        assert!(p
            .tracks
            .iter()
            .all(|t| t.patterns.len() == p.patterns.len()));
    }

    #[test]
    fn new_patterns_get_free_names_up_to_the_limit() {
        let mut p = Project::empty();
        assert_eq!(p.new_pattern(), Some(8));
        assert_eq!(p.patterns[8].name, "I");
        while p.new_pattern().is_some() {}
        assert_eq!(p.patterns.len(), MAX_PATTERNS);
        assert_eq!(p.patterns[30].name, "P31");
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
