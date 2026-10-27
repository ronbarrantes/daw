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
        shape: SampleShape,
    },
}

/// How a sample track plays its sample: which part, and how it fades.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SampleShape {
    /// Where it starts and ends, 0-1 of the sample's length.
    pub start: f32,
    pub end: f32,
    /// Fade-in time, in seconds.
    pub attack: f32,
    /// Fade-out time after the note ends, in seconds, unless it rings out.
    pub release: f32,
    /// Plays to the end however short the note (drums usually do).
    pub ring: bool,
    /// Plays backwards.
    pub reverse: bool,
}

impl SampleShape {
    /// The whole sample, ringing out: how sample tracks always played.
    pub const DEFAULT: SampleShape = SampleShape {
        start: 0.0,
        end: 1.0,
        attack: 0.0,
        release: 0.1,
        ring: true,
        reverse: false,
    };
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
/// Makes the note starting at `head` last until step `end` (or as close
/// as it can get without running into the next note), by adding or
/// removing the ties after it. Returns whether anything changed.
pub fn set_note_length(lane: &mut Lane, head: usize, end: usize) -> bool {
    let before = *lane;
    // How far it can go: up to `end`, or just before the next note.
    let room = lane[head + 1..=end.max(head).min(STEPS - 1)]
        .iter()
        .take_while(|s| !Step::is_hit(s))
        .count();
    let last = head + room;
    for (step, cell) in lane.iter_mut().enumerate().skip(head + 1) {
        if step <= last {
            *cell = Some(Step::TIE);
        } else if cell.is_some_and(|s| s.tie) {
            *cell = None;
        } else {
            break;
        }
    }
    *lane != before
}

/// The step where the note covering `step` starts, if there is one.
pub fn note_head(lane: &Lane, step: usize) -> Option<usize> {
    (0..=step)
        .rev()
        .find(|&s| !lane[s].is_some_and(|x| x.tie))
        .filter(|&s| Step::is_hit(&lane[s]))
}

/// One pattern of a track: a lane per piano-roll row. One-shot tracks only
/// use lane 0.
pub type Grid = [Lane; LANES];
pub const EMPTY_GRID: Grid = [[None; STEPS]; LANES];

#[derive(Clone)]
pub struct Track {
    /// Tells tracks apart (not saved), so the engine knows when a slot
    /// holds a different track and should start it silent.
    pub id: u32,
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
        let shape = SampleShape::DEFAULT;
        Self::new(
            &name,
            Kind::Sampler {
                sample,
                pitched,
                shape,
            },
            4,
        )
    }

    fn new(name: &str, kind: Kind, octave: u8) -> Self {
        Self {
            id: 0,
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

/// Note positions in MIDI blocks are in 240ths of a step: 960 per beat,
/// as in most DAWs, with room for triplets.
pub const PPS: u32 = 240;
/// Most notes a MIDI block holds.
pub const MAX_NOTES: usize = 20_000;

/// One note in a MIDI block: any pitch, any length.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Note {
    /// Where it starts, in `PPS`ths of a step from the block's start.
    pub start: u32,
    /// How long it lasts, in `PPS`ths of a step.
    pub len: u32,
    /// MIDI note number, 60 = C4.
    pub pitch: u8,
    /// 1-127.
    pub velocity: u8,
}

impl Note {
    pub fn end(&self) -> u32 {
        self.start + self.len
    }
}

/// What's in a MIDI block: free notes, played by one track.
#[derive(Clone, PartialEq, Debug)]
pub struct MidiPart {
    /// The track that plays it; `None` if that track was deleted.
    pub track: Option<usize>,
    /// In no particular order.
    pub notes: Vec<Note>,
    /// Length in steps, at least one.
    pub len: usize,
}

impl MidiPart {
    /// Makes the part long enough for its notes, in whole bars. Returns
    /// whether it grew.
    pub fn fit_notes(&mut self) -> bool {
        let end = self.notes.iter().map(Note::end).max().unwrap_or(0);
        let bars = (end as usize).div_ceil(PPS as usize * STEPS);
        let len = (bars * STEPS).min(MAX_SONG_STEPS);
        if len > self.len {
            self.len = len;
            true
        } else {
            false
        }
    }
}

/// A block: the unit songs are built from on the timeline. A jam block is
/// a chain of patterns, one per bar, made in the Jam view. A MIDI block is
/// a list of free notes for one track, made in Arrange (audio blocks come
/// later).
#[derive(Clone, PartialEq, Debug)]
pub struct Block {
    pub name: String,
    /// The pattern for each bar; `None` is a silent bar. `CHAIN_SLOTS` long
    /// (and all `None` in a MIDI block).
    pub chain: Vec<Option<usize>>,
    /// The notes, if it's a MIDI block.
    pub midi: Option<MidiPart>,
}

impl Block {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            chain: vec![None; CHAIN_SLOTS],
            midi: None,
        }
    }

    /// An empty MIDI block for a track.
    pub fn new_midi(name: &str, track: Option<usize>, bars: usize) -> Self {
        Self {
            midi: Some(MidiPart {
                track,
                notes: Vec::new(),
                len: bars.max(1) * STEPS,
            }),
            ..Self::new(name)
        }
    }

    pub fn is_midi(&self) -> bool {
        self.midi.is_some()
    }

    /// Bars up to and including the last filled one, at least one. A MIDI
    /// block counts a part-filled last bar too.
    pub fn bars(&self) -> usize {
        if let Some(m) = &self.midi {
            return m.len.div_ceil(STEPS).max(1);
        }
        self.chain
            .iter()
            .rposition(Option::is_some)
            .map_or(1, |b| b + 1)
    }

    /// Length in steps.
    pub fn len(&self) -> usize {
        match &self.midi {
            Some(m) => m.len.max(1),
            None => self.bars() * STEPS,
        }
    }

    /// The pattern playing `step` steps into the block, and its step.
    pub fn at(&self, step: usize) -> (Option<usize>, usize) {
        let pattern = self.chain.get(step / STEPS).copied().flatten();
        (pattern, step % STEPS)
    }
}

/// A block placed on the timeline. It points to its block, so editing the
/// block changes every copy of it ("Make unique" gives one its own).
/// Positions and lengths are in steps (16th notes).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Clip {
    pub block: usize,
    /// Which timeline row it sits on; rows are only for layout.
    pub row: usize,
    pub start: usize,
    pub len: usize,
    /// Where in the block it begins, when its start is trimmed.
    pub offset: usize,
}

impl Clip {
    pub fn end(&self) -> usize {
        self.start + self.len
    }

    /// The step into its block playing at song step `at`, if the clip
    /// covers it. Stretched past the block's length, it repeats.
    pub fn step_at(&self, at: usize, block_len: usize) -> Option<usize> {
        (self.start..self.end())
            .contains(&at)
            .then(|| (self.offset + at - self.start) % block_len.max(1))
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

/// `base`, or `base 2`, `base 3`... whichever isn't `taken` first.
fn free_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|n| !taken(n))
        .expect("some number is free")
}

/// What an automation lane moves: one of a track's mix knobs, or one of
/// its synth knobs (by index into `synth::PARAMS`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Target {
    Volume,
    Pan,
    Reverb,
    Delay,
    Synth(usize),
}

/// The mix knobs automation can move, as knobs for the range and name.
pub const MIX_TARGETS: [(Target, synth::Param); 4] = [
    (
        Target::Volume,
        mix_param("Volume", 0.0, 1.0, synth::Unit::Percent),
    ),
    (Target::Pan, mix_param("Pan", -1.0, 1.0, synth::Unit::Pan)),
    (
        Target::Reverb,
        mix_param("Reverb", 0.0, 1.0, synth::Unit::Percent),
    ),
    (
        Target::Delay,
        mix_param("Delay", 0.0, 1.0, synth::Unit::Percent),
    ),
];

const fn mix_param(name: &'static str, min: f32, max: f32, unit: synth::Unit) -> synth::Param {
    synth::Param {
        name,
        min,
        max,
        default: 0.0,
        log: false,
        unit,
    }
}

impl Target {
    /// Its name in menus and on lanes; the synth's own Volume is told
    /// apart from the track's.
    pub fn label(&self) -> &'static str {
        match self {
            Target::Synth(k) if *k == synth::VOLUME => "Synth volume",
            t => t.param().name,
        }
    }

    /// The knob it moves, for its range and how to show its values.
    pub fn param(&self) -> &'static synth::Param {
        match self {
            Target::Synth(k) => &synth::PARAMS[(*k).min(synth::PARAMS.len() - 1)],
            t => {
                let i = MIX_TARGETS.iter().position(|(m, _)| m == t).unwrap_or(0);
                &MIX_TARGETS[i].1
            }
        }
    }
}

/// One point on an automation lane.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AutoPoint {
    /// When, in `PPS`ths of a step from the song's start.
    pub at: u32,
    /// The knob's position there, 0-1 along it.
    pub value: f32,
}

/// Changes to one knob over the song: a line through its points, drawn
/// in Arrange. Before the first point and after the last, it holds.
#[derive(Clone, PartialEq, Debug)]
pub struct AutoLane {
    pub track: usize,
    pub target: Target,
    /// Kept in time order.
    pub points: Vec<AutoPoint>,
}

impl AutoLane {
    /// The knob's position (0-1) at song position `at` (in `PPS`ths of a
    /// step), if the lane has any points.
    pub fn value_at(&self, at: f32) -> Option<f32> {
        let first = self.points.first()?;
        if at <= first.at as f32 {
            return Some(first.value);
        }
        let next = self.points.iter().position(|p| p.at as f32 > at);
        Some(match next {
            None => self.points.last()?.value,
            Some(k) => {
                let (a, b) = (self.points[k - 1], self.points[k]);
                let t = (at - a.at as f32) / (b.at - a.at).max(1) as f32;
                a.value + (b.value - a.value) * t
            }
        })
    }

    /// Puts the points back in time order after an edit.
    pub fn sort(&mut self) {
        self.points.sort_by_key(|p| p.at);
    }
}

/// The master volume a new song starts with: the level songs always had.
pub const MASTER_DEFAULT: f32 = 0.8;

/// Bars in a chain (a jam block).
pub const CHAIN_SLOTS: usize = 32;
pub const MAX_BLOCKS: usize = 64;

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
    /// The block library; there's always at least one.
    pub blocks: Vec<Block>,
    /// The timeline: blocks placed in time.
    pub clips: Vec<Clip>,
    /// 0-1: how late every other 16th note plays. 0.66 is a triplet shuffle.
    pub swing: f32,
    pub groups: Vec<Group>,
    pub effects: Effects,
    /// Master volume, 0-1, before the limiter.
    pub master: f32,
    /// Knob changes over the song, drawn in Arrange.
    pub automation: Vec<AutoLane>,
}

impl Project {
    pub fn empty() -> Self {
        Self {
            tracks: Vec::new(),
            patterns: default_patterns(),
            blocks: vec![Block::new("Block 1")],
            clips: Vec::new(),
            swing: 0.0,
            groups: Vec::new(),
            effects: Effects::DEFAULT,
            master: MASTER_DEFAULT,
            automation: Vec::new(),
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
        self.gain_with(i, self.tracks[i].volume)
    }

    /// Like `gain`, with the track's volume set to `volume` (for
    /// automation).
    pub fn gain_with(&self, i: usize, volume: f32) -> f32 {
        let any_solo = self.tracks.iter().any(|t| t.solo) || self.groups.iter().any(|g| g.solo);
        let t = &self.tracks[i];
        let g = t.group.and_then(|id| self.group(id));
        let muted = t.muted || g.is_some_and(|g| g.muted);
        let soloed = t.solo || g.is_some_and(|g| g.solo);
        if muted || (any_solo && !soloed) {
            0.0
        } else {
            volume * g.map_or(1.0, |g| g.volume)
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
        track.id = self.tracks.iter().map(|t| t.id + 1).max().unwrap_or(1);
        self.tracks.push(track);
    }

    /// Copies a track (sound, notes and all) to just below it and returns
    /// the copy's index, unless the song is full.
    pub fn duplicate_track(&mut self, i: usize) -> Option<usize> {
        if i >= self.tracks.len() || self.tracks.len() >= MAX_TRACKS {
            return None;
        }
        let mut copy = self.tracks[i].clone();
        copy.name = free_name(&copy.name, |n| self.tracks.iter().any(|t| t.name == n));
        copy.id = self.tracks.iter().map(|t| t.id + 1).max().unwrap_or(1);
        self.tracks.insert(i + 1, copy);
        for m in self.blocks.iter_mut().filter_map(|b| b.midi.as_mut()) {
            if let Some(t) = m.track.as_mut().filter(|t| **t > i) {
                *t += 1;
            }
        }
        for l in &mut self.automation {
            if l.track > i {
                l.track += 1;
            }
        }
        Some(i + 1)
    }

    /// Swaps two tracks' places in the list; MIDI blocks follow them.
    pub fn swap_tracks(&mut self, a: usize, b: usize) {
        if a.max(b) >= self.tracks.len() {
            return;
        }
        self.tracks.swap(a, b);
        for m in self.blocks.iter_mut().filter_map(|b| b.midi.as_mut()) {
            m.track = match m.track {
                Some(t) if t == a => Some(b),
                Some(t) if t == b => Some(a),
                other => other,
            };
        }
        for l in &mut self.automation {
            if l.track == a {
                l.track = b;
            } else if l.track == b {
                l.track = a;
            }
        }
    }

    /// Numbers the tracks 1, 2, 3... (for songs just built or opened).
    pub fn number_tracks(&mut self) {
        for (i, t) in self.tracks.iter_mut().enumerate() {
            t.id = i as u32 + 1;
        }
    }

    /// Removes a track. MIDI blocks it played keep their notes but go
    /// silent until another track is picked.
    pub fn remove_track(&mut self, i: usize) {
        if i >= self.tracks.len() {
            return;
        }
        self.tracks.remove(i);
        self.prune_groups();
        for m in self.blocks.iter_mut().filter_map(|b| b.midi.as_mut()) {
            m.track = match m.track {
                Some(t) if t == i => None,
                Some(t) if t > i => Some(t - 1),
                other => other,
            };
        }
        self.automation.retain(|l| l.track != i);
        for l in &mut self.automation {
            if l.track > i {
                l.track -= 1;
            }
        }
    }

    /// Adds an empty one-bar MIDI block for a track and returns it, unless
    /// the library is full.
    pub fn new_midi_block(&mut self, track: Option<usize>) -> Option<usize> {
        if self.blocks.len() >= MAX_BLOCKS {
            return None;
        }
        let base = match track.and_then(|t| self.tracks.get(t)) {
            Some(t) => format!("{} notes", t.name),
            None => "Notes".to_string(),
        };
        let name = self.unused_block_name(&base);
        self.blocks.push(Block::new_midi(&name, track, 1));
        Some(self.blocks.len() - 1)
    }

    /// The first chain block, which the Jam view falls back to.
    pub fn first_chain_block(&self) -> usize {
        self.blocks.iter().position(|b| !b.is_midi()).unwrap_or(0)
    }

    /// A name not used by any pattern yet: `base`, or `base 2`, `base 3`...
    fn unused_name(&self, base: &str) -> String {
        free_name(base, |n| self.patterns.iter().any(|p| p.name == n))
    }

    fn unused_block_name(&self, base: &str) -> String {
        free_name(base, |n| self.blocks.iter().any(|b| b.name == n))
    }

    /// A block's length in steps (one bar if it doesn't exist).
    pub fn block_len(&self, b: usize) -> usize {
        self.blocks.get(b).map_or(STEPS, Block::len)
    }

    /// Adds an empty block and returns it, unless the library is full.
    pub fn new_block(&mut self) -> Option<usize> {
        if self.blocks.len() >= MAX_BLOCKS {
            return None;
        }
        let name = self.unused_block_name(&format!("Block {}", self.blocks.len() + 1));
        self.blocks.push(Block::new(&name));
        Some(self.blocks.len() - 1)
    }

    /// Copies a block and returns the copy.
    pub fn duplicate_block(&mut self, b: usize) -> Option<usize> {
        if b >= self.blocks.len() || self.blocks.len() >= MAX_BLOCKS {
            return None;
        }
        let mut copy = self.blocks[b].clone();
        copy.name = self.unused_block_name(&copy.name);
        self.blocks.push(copy);
        Some(self.blocks.len() - 1)
    }

    /// Whether a block can be deleted: there must always be a chain block
    /// left for the Jam view.
    pub fn can_delete_block(&self, b: usize) -> bool {
        let chains = self.blocks.iter().filter(|b| !b.is_midi()).count();
        self.blocks
            .get(b)
            .is_some_and(|block| block.is_midi() || chains > 1)
    }

    /// Removes a block and its copies on the timeline. The last chain
    /// block can't be removed.
    pub fn delete_block(&mut self, b: usize) {
        if !self.can_delete_block(b) {
            return;
        }
        self.blocks.remove(b);
        self.clips.retain(|c| c.block != b);
        for c in &mut self.clips {
            if c.block > b {
                c.block -= 1;
            }
        }
    }

    /// After block `b` changed length from `old_len`, makes the copies
    /// that played it exactly once follow the new length. Copies that were
    /// stretched or trimmed keep theirs.
    pub fn fit_clips(&mut self, b: usize, old_len: usize) {
        let new_len = self.block_len(b);
        for c in &mut self.clips {
            if c.block == b && c.offset == 0 && c.len == old_len {
                c.len = new_len.min(MAX_SONG_STEPS - c.start);
            }
        }
    }

    /// How many times a block is on the timeline.
    pub fn block_uses(&self, b: usize) -> usize {
        self.clips.iter().filter(|c| c.block == b).count()
    }

    /// How many blocks use a pattern.
    pub fn pattern_uses(&self, p: usize) -> usize {
        self.blocks
            .iter()
            .filter(|b| b.chain.contains(&Some(p)))
            .count()
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

    /// Removes a pattern, emptying the bars that played it. The last
    /// pattern can't be removed.
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
        for slot in self.blocks.iter_mut().flat_map(|b| b.chain.iter_mut()) {
            *slot = match *slot {
                Some(q) if q == p => None,
                Some(q) if q > p => Some(q - 1),
                other => other,
            };
        }
    }

    /// Gives a placed block its own copy, so editing it no longer changes
    /// the other copies. Returns the new block.
    pub fn make_unique(&mut self, clip: usize) -> Option<usize> {
        let b = self.clips.get(clip)?.block;
        let copy = self.duplicate_block(b)?;
        self.clips[clip].block = copy;
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
        let block_len = self.block_len(c.block);
        self.clips.push(Clip {
            start: at,
            len: c.end() - at,
            offset: (c.offset + at - c.start) % block_len,
            ..c
        });
        Some(self.clips.len() - 1)
    }

    /// Built-in drums and a synth lead, with a beat and a riff in pattern A.
    pub fn demo() -> Self {
        let mut tracks: Vec<Track> = builtin_drums()
            .into_iter()
            .take(drums::STARTER_KIT)
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
            v[synth::WAVE] = synth::SAW + 0.3;
            v[synth::WAVE_2] = synth::SAW + 0.3;
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
            blocks: vec![Block {
                chain: {
                    let mut chain = vec![None; CHAIN_SLOTS];
                    chain[0] = Some(0);
                    chain
                },
                ..Block::new("Intro")
            }],
            clips: vec![Clip {
                block: 0,
                row: 0,
                start: 0,
                len: STEPS,
                offset: 0,
            }],
            swing: 0.0,
            groups: Vec::new(),
            effects: Effects::DEFAULT,
            master: MASTER_DEFAULT,
            automation: Vec::new(),
        };
        project.new_group(&[0, 1, 2, 3], "Drums");
        project.number_tracks();
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

    fn clip(block: usize, start: usize, len: usize) -> Clip {
        Clip {
            block,
            row: 0,
            start,
            len,
            offset: 0,
        }
    }

    /// A block chaining these patterns, one per bar.
    fn block(name: &str, chain: &[Option<usize>]) -> Block {
        let mut b = Block::new(name);
        b.chain[..chain.len()].copy_from_slice(chain);
        b
    }

    #[test]
    fn blocks_chain_patterns_bar_by_bar() {
        let b = block("Verse", &[Some(0), None, Some(2)]);
        assert_eq!(b.bars(), 3);
        assert_eq!(b.len(), 48);
        assert_eq!(b.at(5), (Some(0), 5));
        assert_eq!(b.at(20), (None, 4), "a silent bar");
        assert_eq!(b.at(35), (Some(2), 3));
        assert_eq!(Block::new("Empty").bars(), 1, "at least one bar");
    }

    #[test]
    fn clips_repeat_their_block_and_can_be_split() {
        let mut p = Project::demo();
        p.blocks = vec![block("Two bars", &[Some(0), Some(1)])];
        p.clips = vec![clip(0, 16, 80)];
        let c = p.clips[0];
        assert_eq!(c.step_at(15, 32), None);
        assert_eq!(c.step_at(16, 32), Some(0));
        assert_eq!(
            c.step_at(49, 32),
            Some(1),
            "repeats after the block's 32 steps"
        );
        assert_eq!(c.step_at(96, 32), None, "ends after 80 steps");
        assert_eq!(p.song_len(), 96);

        let right = p.split_clip(0, 22).unwrap();
        assert_eq!((p.clips[0].start, p.clips[0].len), (16, 6));
        let r = p.clips[right];
        assert_eq!((r.start, r.len, r.offset), (22, 74, 6));
        assert_eq!(r.step_at(22, 32), Some(6), "carries on where it was split");
        assert_eq!(p.split_clip(0, 16), None, "not at the very edge");
    }

    #[test]
    fn whole_copies_follow_their_block_s_length() {
        let mut p = Project::demo();
        p.clips = vec![clip(0, 0, 16), clip(0, 32, 40)];
        p.blocks[0].chain[2] = Some(1);
        p.fit_clips(0, 16);
        assert_eq!(p.clips[0].len, 48, "a whole copy grows with the block");
        assert_eq!(p.clips[1].len, 40, "a stretched one stays as it was");
    }

    #[test]
    fn make_unique_copies_the_block_for_one_copy() {
        let mut p = Project::demo();
        p.clips = vec![clip(0, 0, 16), clip(0, 16, 16)];
        let copy = p.make_unique(1).unwrap();
        assert_eq!(p.blocks[copy].name, "Intro 2");
        assert_eq!(p.blocks[copy].chain, p.blocks[0].chain);
        assert_eq!(p.clips[0].block, 0, "the other copy stays linked");
        assert_eq!(p.clips[1].block, copy);
        p.blocks[copy].chain[1] = Some(3);
        assert_ne!(p.blocks[copy].chain, p.blocks[0].chain);
    }

    #[test]
    fn deleting_blocks_and_patterns_tidies_up() {
        let mut p = Project::demo();
        p.blocks = vec![
            block("A", &[Some(0)]),
            block("B", &[Some(2), Some(5)]),
            block("C", &[Some(5)]),
        ];
        p.clips = vec![clip(0, 0, 16), clip(1, 16, 16), clip(2, 32, 16)];
        p.delete_block(1);
        let blocks: Vec<usize> = p.clips.iter().map(|c| c.block).collect();
        assert_eq!(blocks, [0, 1], "its copies go, the rest renumber");

        p.delete_pattern(2);
        assert_eq!(p.patterns.len(), DEFAULT_PATTERNS - 1);
        assert_eq!(p.patterns[2].name, "D");
        assert_eq!(p.blocks[1].chain[0], Some(4), "pattern 5 is now 4");
        assert!(p
            .tracks
            .iter()
            .all(|t| t.patterns.len() == p.patterns.len()));
        assert_eq!(p.pattern_uses(4), 1);
        assert_eq!(p.block_uses(0), 1);
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
    fn notes_stretch_and_shrink_but_stop_at_the_next_note() {
        let mut lane: Lane = [None; STEPS];
        lane[2] = Some(Step::HIT);
        lane[9] = Some(Step::HIT);
        assert!(set_note_length(&mut lane, 2, 5));
        let held: Vec<usize> = (0..STEPS).filter(|&s| lane[s] == Some(Step::TIE)).collect();
        assert_eq!(held, [3, 4, 5]);
        assert_eq!(note_head(&lane, 4), Some(2));
        // Shorter again.
        set_note_length(&mut lane, 2, 3);
        assert_eq!(lane[4], None);
        assert_eq!(lane[3], Some(Step::TIE));
        // Too far: it stops just before the next note, which stays.
        set_note_length(&mut lane, 2, 14);
        assert_eq!(lane[8], Some(Step::TIE));
        assert_eq!(lane[9], Some(Step::HIT));
        assert_eq!(lane[10], None);
        assert!(!set_note_length(&mut lane, 2, 8), "no change");
        assert_eq!(note_head(&lane, 0), None);
    }

    #[test]
    fn midi_blocks_have_free_lengths_and_follow_track_deletes() {
        let mut p = Project::demo();
        let b = p.new_midi_block(Some(LEAD)).unwrap();
        assert_eq!(p.blocks[b].name, "Lead notes");
        assert_eq!(p.block_len(b), STEPS);
        let m = p.blocks[b].midi.as_mut().unwrap();
        m.notes.push(Note {
            start: 20 * PPS,
            len: PPS * 3 / 2,
            pitch: 64,
            velocity: 100,
        });
        assert!(m.fit_notes(), "grows to two bars for a note in bar 2");
        assert_eq!(p.blocks[b].len(), 2 * STEPS);
        assert_eq!(p.blocks[b].bars(), 2);

        p.remove_track(0);
        assert_eq!(p.blocks[b].midi.as_ref().unwrap().track, Some(LEAD - 1));
        p.remove_track(LEAD - 1);
        assert_eq!(p.blocks[b].midi.as_ref().unwrap().track, None, "silent");
        assert_eq!(p.blocks[b].midi.as_ref().unwrap().notes.len(), 1);
    }

    #[test]
    fn tracks_duplicate_and_move_with_their_midi_blocks() {
        let mut p = Project::demo();
        let b = p.new_midi_block(Some(LEAD)).unwrap();
        let copy = p.duplicate_track(1).unwrap();
        assert_eq!(copy, 2);
        assert_eq!(p.tracks[2].name, "Snare 2");
        assert_eq!(p.tracks[2].patterns, p.tracks[1].patterns, "notes and all");
        assert_ne!(p.tracks[2].id, p.tracks[1].id);
        let lead = p.blocks[b].midi.as_ref().unwrap().track;
        assert_eq!(lead, Some(LEAD + 1), "the lead moved down one");
        p.swap_tracks(LEAD + 1, LEAD);
        assert_eq!(p.tracks[LEAD].name, "Lead");
        assert_eq!(p.blocks[b].midi.as_ref().unwrap().track, Some(LEAD));
    }

    #[test]
    fn automation_lanes_draw_a_line_through_their_points() {
        let lane = AutoLane {
            track: 0,
            target: Target::Volume,
            points: vec![
                AutoPoint { at: 0, value: 0.0 },
                AutoPoint {
                    at: 16 * PPS,
                    value: 1.0,
                },
                AutoPoint {
                    at: 32 * PPS,
                    value: 0.5,
                },
            ],
        };
        assert_eq!(lane.value_at(0.0), Some(0.0));
        assert_eq!(lane.value_at((8 * PPS) as f32), Some(0.5), "halfway up");
        assert_eq!(lane.value_at((24 * PPS) as f32), Some(0.75));
        assert_eq!(lane.value_at((100 * PPS) as f32), Some(0.5), "holds after");
        let empty = AutoLane {
            points: Vec::new(),
            ..lane
        };
        assert_eq!(empty.value_at(0.0), None);
        assert_eq!(Target::Synth(synth::CUTOFF).param().name, "Cutoff");
        assert_eq!(Target::Pan.param().min, -1.0);
    }

    #[test]
    fn the_last_chain_block_stays() {
        let mut p = Project::demo();
        let b = p.new_midi_block(None).unwrap();
        assert!(!p.can_delete_block(0), "Jam needs a chain block");
        assert!(p.can_delete_block(b));
        p.delete_block(0);
        assert_eq!(p.blocks.len(), 2);
        assert_eq!(p.first_chain_block(), 0);
        p.delete_block(b);
        assert_eq!(p.blocks.len(), 1);
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
