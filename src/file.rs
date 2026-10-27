//! Saving and opening songs.
//!
//! A song file (`.daw`) is a zip archive, like `.docx` or Bitwig's
//! `.dawproject`:
//!
//! - `song.json`: everything about the song, readable by people and tools.
//! - `samples/N.wav`: each loaded sample, so the song works on any computer.
//!
//! Built-in drums aren't stored; they're regenerated when the song opens.
//! Notes are a sparse list and synth settings are stored by name, and
//! unknown fields are ignored while missing ones get defaults, so files
//! keep opening as the app grows. `format` only goes up if a change would
//! make older versions of the app misread the file.

use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read, Write};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::drums;
use crate::project::{
    builtin_drums, default_patterns, Block, Clip, Effects, Group, Kind, MidiPart, Pattern, Project,
    Sample, SampleShape, Step, Track, EMPTY_GRID, LANES, MAX_BLOCKS, MAX_NOTES, MAX_PATTERNS,
    MAX_SONG_STEPS, MAX_TRACKS, NORMAL, PPS, STEPS,
};
use crate::synth;

/// File extension for song files.
pub const EXTENSION: &str = "daw";
/// The newest file layout this version can read.
/// 2: the timeline (`clips`) and a pattern library replaced the Song row.
/// 3: the timeline holds blocks (chains of patterns), not patterns.
/// 4: MIDI blocks. Only written when a song has one, so other songs still
/// open in older versions.
const FORMAT: u32 = 4;
const SONG_JSON: &str = "song.json";

/// What a song file holds besides the project itself.
pub struct Song {
    pub project: Project,
    pub bpm: f32,
}

#[derive(Serialize, Deserialize)]
struct SongFile {
    format: u32,
    /// The app version that saved the file, for troubleshooting.
    #[serde(default)]
    app_version: String,
    #[serde(default = "default_bpm")]
    bpm: f32,
    #[serde(default)]
    swing: f32,
    /// Format 1's Song row: a pattern per bar, `null` for a silent bar.
    /// Read from old files and turned into clips; no longer written.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    song: Vec<Option<u8>>,
    /// The pattern library, by name; notes refer to patterns by index.
    #[serde(default)]
    patterns: Vec<PatternFile>,
    /// The block library.
    #[serde(default)]
    blocks: Vec<BlockFile>,
    /// The timeline.
    #[serde(default)]
    clips: Vec<ClipFile>,
    #[serde(default)]
    groups: Vec<GroupFile>,
    #[serde(default)]
    effects: EffectsFile,
    #[serde(default = "default_master")]
    master_volume: f32,
    #[serde(default)]
    tracks: Vec<TrackFile>,
}

#[derive(Serialize, Deserialize)]
struct PatternFile {
    name: String,
}

/// A jam block: a pattern (by index) or `null` for each bar. Or a MIDI
/// block, with `midi` set and no chain.
#[derive(Serialize, Deserialize)]
struct BlockFile {
    name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    chain: Vec<Option<usize>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    midi: Option<MidiFile>,
}

/// A MIDI block's notes and the track that plays them (by index).
#[derive(Serialize, Deserialize)]
struct MidiFile {
    #[serde(default)]
    track: Option<usize>,
    /// Length in 16th notes.
    len: usize,
    #[serde(default)]
    notes: Vec<NoteFile>,
}

/// A MIDI block's note. Times are in 960ths of a beat (240 per 16th).
#[derive(Serialize, Deserialize)]
struct NoteFile {
    start: u32,
    len: u32,
    pitch: u8,
    #[serde(default = "normal", skip_serializing_if = "is_normal")]
    velocity: u8,
}

/// A block on the timeline; positions and lengths are in 16th notes.
#[derive(Serialize, Deserialize)]
struct ClipFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    block: Option<usize>,
    /// Format 2 placed patterns directly; they're turned into blocks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pattern: Option<usize>,
    #[serde(default)]
    row: usize,
    start: usize,
    len: usize,
    #[serde(default)]
    offset: usize,
}

/// The shared reverb and delay. Missing values get the defaults.
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct EffectsFile {
    reverb_size: f32,
    reverb_tone: f32,
    /// In 16th-note steps.
    delay_steps: f32,
    delay_feedback: f32,
}

impl Default for EffectsFile {
    fn default() -> Self {
        let e = Effects::DEFAULT;
        Self {
            reverb_size: e.reverb_size,
            reverb_tone: e.reverb_tone,
            delay_steps: e.delay_steps,
            delay_feedback: e.delay_feedback,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct GroupFile {
    id: u32,
    name: String,
    #[serde(default)]
    collapsed: bool,
    #[serde(default)]
    muted: bool,
    #[serde(default)]
    solo: bool,
    #[serde(default = "one")]
    volume: f32,
}

#[derive(Serialize, Deserialize)]
struct TrackFile {
    name: String,
    instrument: Instrument,
    #[serde(default)]
    notes: Vec<Note>,
    #[serde(default = "default_volume")]
    volume: f32,
    /// -1 (left) to 1 (right).
    #[serde(default)]
    pan: f32,
    /// Reverb and delay send levels, 0-1.
    #[serde(default)]
    reverb: f32,
    #[serde(default)]
    delay: f32,
    #[serde(default)]
    muted: bool,
    #[serde(default)]
    solo: bool,
    #[serde(default)]
    humanize: f32,
    #[serde(default = "default_octave")]
    octave: u8,
    /// 1-16 as shown in the app; `null` when off.
    #[serde(default)]
    midi_channel: Option<u8>,
    #[serde(default = "default_midi_note")]
    midi_note: u8,
    #[serde(default)]
    group: Option<u32>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Instrument {
    /// Knob values by name, e.g. `"Cutoff": 250.0`.
    Synth { settings: BTreeMap<String, f32> },
    Sampler {
        sample: SampleRef,
        #[serde(default)]
        pitched: bool,
        /// The part played (0-1), fades in seconds, ringing out and
        /// playing backwards. Missing ones are the whole sample, ringing.
        #[serde(default)]
        start: f32,
        #[serde(default = "one")]
        end: f32,
        #[serde(default)]
        attack: f32,
        #[serde(default = "default_release")]
        release: f32,
        #[serde(default = "yes")]
        ring: bool,
        #[serde(default)]
        reverse: bool,
    },
}

fn default_release() -> f32 {
    SampleShape::DEFAULT.release
}
fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
struct SampleRef {
    name: String,
    /// A built-in drum, by name (e.g. "Kick").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    builtin: Option<String>,
    /// A sample stored in the file, e.g. "samples/0.wav".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<String>,
}

/// One note: where it is, and any groove settings that aren't the default.
#[derive(Serialize, Deserialize)]
struct Note {
    pattern: u8,
    lane: u8,
    step: u8,
    #[serde(default, skip_serializing_if = "is_false")]
    tie: bool,
    #[serde(default = "normal", skip_serializing_if = "is_normal")]
    velocity: u8,
    #[serde(default = "hundred", skip_serializing_if = "is_hundred")]
    chance: u8,
    #[serde(default = "one_u8", skip_serializing_if = "is_one_u8")]
    ratchet: u8,
    #[serde(default, skip_serializing_if = "is_zero_i8")]
    nudge: i8,
}

fn default_master() -> f32 {
    crate::project::MASTER_DEFAULT
}
fn default_bpm() -> f32 {
    120.0
}
fn default_volume() -> f32 {
    0.8
}
fn default_octave() -> u8 {
    3
}
fn default_midi_note() -> u8 {
    60
}
fn one() -> f32 {
    1.0
}
fn normal() -> u8 {
    NORMAL
}
fn hundred() -> u8 {
    100
}
fn one_u8() -> u8 {
    1
}
fn is_false(b: &bool) -> bool {
    !b
}
fn is_normal(v: &u8) -> bool {
    *v == NORMAL
}
fn is_hundred(v: &u8) -> bool {
    *v == 100
}
fn is_one_u8(v: &u8) -> bool {
    *v == 1
}
fn is_zero_i8(v: &i8) -> bool {
    *v == 0
}

/// Writes a song. Saves to a temporary file first and renames it into place,
/// so a crash mid-save can't leave a half-written song behind.
#[cfg(not(target_arch = "wasm32"))]
pub fn save(path: &Path, project: &Project, bpm: f32) -> Result<(), String> {
    let bytes = to_bytes(project, bpm).map_err(|e| format!("Couldn't save: {e}"))?;
    let tmp = path.with_extension("daw-saving");
    std::fs::write(&tmp, bytes)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("Couldn't save: {e}")
        })
}

/// A song file's contents.
pub fn to_bytes(project: &Project, bpm: f32) -> Result<Vec<u8>, String> {
    let err = |e: &dyn std::fmt::Display| e.to_string();
    // Each loaded sample is stored once, even if several tracks use it.
    let mut stored: Vec<Arc<Sample>> = Vec::new();
    let mut tracks = Vec::new();
    for t in &project.tracks {
        let instrument = match &t.kind {
            Kind::Synth(values) => Instrument::Synth {
                settings: synth::PARAMS
                    .iter()
                    .zip(values)
                    .map(|(p, v)| (p.name.to_string(), *v))
                    .collect(),
            },
            Kind::Sampler {
                sample,
                pitched,
                shape,
            } => {
                let sample_ref = match sample.builtin {
                    Some(i) => SampleRef {
                        name: sample.name.clone(),
                        builtin: Some(drums::NAMES[i].to_string()),
                        file: None,
                    },
                    None => {
                        let n = match stored.iter().position(|s| Arc::ptr_eq(s, sample)) {
                            Some(n) => n,
                            None => {
                                stored.push(sample.clone());
                                stored.len() - 1
                            }
                        };
                        SampleRef {
                            name: sample.name.clone(),
                            builtin: None,
                            file: Some(format!("samples/{n}.wav")),
                        }
                    }
                };
                Instrument::Sampler {
                    sample: sample_ref,
                    pitched: *pitched,
                    start: shape.start,
                    end: shape.end,
                    attack: shape.attack,
                    release: shape.release,
                    ring: shape.ring,
                    reverse: shape.reverse,
                }
            }
        };
        tracks.push(TrackFile {
            name: t.name.clone(),
            instrument,
            notes: notes_of(t),
            volume: t.volume,
            pan: t.pan,
            reverb: t.reverb,
            delay: t.delay,
            muted: t.muted,
            solo: t.solo,
            humanize: t.humanize,
            octave: t.octave,
            midi_channel: t.midi_channel.map(|c| c + 1),
            midi_note: t.midi_note,
            group: t.group,
        });
    }
    let has_midi = project.blocks.iter().any(Block::is_midi);
    let song = SongFile {
        format: if has_midi { FORMAT } else { 3 },
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        bpm,
        swing: project.swing,
        song: Vec::new(),
        patterns: project
            .patterns
            .iter()
            .map(|p| PatternFile {
                name: p.name.clone(),
            })
            .collect(),
        blocks: project
            .blocks
            .iter()
            .map(|b| match &b.midi {
                Some(m) => BlockFile {
                    name: b.name.clone(),
                    chain: Vec::new(),
                    midi: Some(MidiFile {
                        track: m.track,
                        len: m.len,
                        notes: m
                            .notes
                            .iter()
                            .map(|n| NoteFile {
                                start: n.start,
                                len: n.len,
                                pitch: n.pitch,
                                velocity: n.velocity,
                            })
                            .collect(),
                    }),
                },
                None => BlockFile {
                    name: b.name.clone(),
                    chain: b.chain[..b.bars()].to_vec(),
                    midi: None,
                },
            })
            .collect(),
        clips: project
            .clips
            .iter()
            .map(|c| ClipFile {
                block: Some(c.block),
                pattern: None,
                row: c.row,
                start: c.start,
                len: c.len,
                offset: c.offset,
            })
            .collect(),
        groups: project
            .groups
            .iter()
            .map(|g| GroupFile {
                id: g.id,
                name: g.name.clone(),
                collapsed: g.collapsed,
                muted: g.muted,
                solo: g.solo,
                volume: g.volume,
            })
            .collect(),
        master_volume: project.master,
        effects: EffectsFile {
            reverb_size: project.effects.reverb_size,
            reverb_tone: project.effects.reverb_tone,
            delay_steps: project.effects.delay_steps,
            delay_feedback: project.effects.delay_feedback,
        },
        tracks,
    };

    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let deflate = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file(SONG_JSON, deflate).map_err(|e| err(&e))?;
    let json = serde_json::to_vec_pretty(&song).map_err(|e| err(&e))?;
    zip.write_all(&json).map_err(|e| err(&e))?;
    for (n, sample) in stored.iter().enumerate() {
        zip.start_file(format!("samples/{n}.wav"), deflate)
            .map_err(|e| err(&e))?;
        zip.write_all(&wav_bytes(sample)?).map_err(|e| err(&e))?;
    }
    Ok(zip.finish().map_err(|e| err(&e))?.into_inner())
}

/// Every step of every pattern that isn't a rest.
fn notes_of(t: &Track) -> Vec<Note> {
    let mut notes = Vec::new();
    for (pattern, grid) in t.patterns.iter().enumerate() {
        for (lane, steps) in grid.iter().enumerate() {
            for (step, value) in steps.iter().enumerate() {
                if let Some(s) = value {
                    notes.push(Note {
                        pattern: pattern as u8,
                        lane: lane as u8,
                        step: step as u8,
                        tie: s.tie,
                        velocity: s.velocity,
                        chance: s.chance,
                        ratchet: s.ratchet,
                        nudge: s.nudge,
                    });
                }
            }
        }
    }
    notes
}

/// A sample as a 16-bit mono WAV file.
fn wav_bytes(sample: &Sample) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: sample.rate as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut bytes = Cursor::new(Vec::new());
    let mut wav = hound::WavWriter::new(&mut bytes, spec).map_err(|e| e.to_string())?;
    for s in &sample.data {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        wav.write_sample(v).map_err(|e| e.to_string())?;
    }
    wav.finalize().map_err(|e| e.to_string())?;
    Ok(bytes.into_inner())
}

/// Reads a song.
#[cfg(not(target_arch = "wasm32"))]
pub fn open(path: &Path) -> Result<Song, String> {
    let name = path
        .file_name()
        .map_or_else(|| "the song".into(), |n| n.to_string_lossy().into_owned());
    let bytes = std::fs::read(path).map_err(|e| format!("Couldn't open {name}: {e}"))?;
    from_bytes(&name, bytes)
}

/// Reads a song file's contents; `name` is only for error messages.
pub fn from_bytes(name: &str, bytes: Vec<u8>) -> Result<Song, String> {
    read_zip(bytes).map_err(|e| format!("Couldn't open {name}: {e}"))
}

fn read_zip(bytes: Vec<u8>) -> Result<Song, String> {
    let err = |e: &dyn std::fmt::Display| e.to_string();
    let mut zip =
        ZipArchive::new(Cursor::new(bytes)).map_err(|_| "not a daw song file".to_string())?;
    let mut json = String::new();
    zip.by_name(SONG_JSON)
        .map_err(|_| "not a daw song file".to_string())?
        .read_to_string(&mut json)
        .map_err(|e| err(&e))?;
    let song: SongFile = serde_json::from_str(&json).map_err(|e| err(&e))?;
    if song.format > FORMAT {
        return Err("it was saved by a newer version of daw. Please update.".into());
    }

    // Format 1 files have no library: they had patterns A to H.
    let patterns: Vec<Pattern> = if song.patterns.is_empty() {
        default_patterns()
    } else {
        song.patterns
            .iter()
            .take(MAX_PATTERNS)
            .map(|p| Pattern {
                name: p.name.clone(),
            })
            .collect()
    };

    let (blocks, clips) = read_timeline(&song, &patterns);

    let mut drums: Option<Vec<Arc<Sample>>> = None;
    let mut loaded: HashMap<String, Arc<Sample>> = HashMap::new();
    let mut tracks = Vec::new();
    for t in song.tracks.into_iter().take(MAX_TRACKS) {
        let kind = match t.instrument {
            Instrument::Synth { settings } => {
                let mut values = synth::PARAMS.map(|p| p.default);
                for (i, p) in synth::PARAMS.iter().enumerate() {
                    if let Some(v) = settings.get(p.name) {
                        values[i] = v.clamp(p.min, p.max);
                    }
                }
                // Before v0.19 one Wave knob (0-1, saw to square) set both
                // oscillators.
                if let (Some(old), None) = (settings.get("Wave"), settings.get("Wave 1")) {
                    let wave = synth::SAW + old.clamp(0.0, 1.0);
                    values[synth::WAVE] = wave;
                    values[synth::WAVE_2] = wave;
                }
                Kind::Synth(values)
            }
            Instrument::Sampler {
                sample,
                pitched,
                start,
                end,
                attack,
                release,
                ring,
                reverse,
            } => {
                let shape = SampleShape {
                    start: start.clamp(0.0, 1.0),
                    end: end.clamp(0.0, 1.0),
                    attack: attack.clamp(0.0, 1.0),
                    release: release.clamp(0.01, 3.0),
                    ring,
                    reverse,
                };
                let sample = if let Some(name) = &sample.builtin {
                    let i = drums::NAMES
                        .iter()
                        .position(|n| n == name)
                        .ok_or(format!("unknown built-in sound \"{name}\""))?;
                    drums.get_or_insert_with(builtin_drums)[i].clone()
                } else if let Some(entry) = &sample.file {
                    match loaded.get(entry) {
                        Some(s) => s.clone(),
                        None => {
                            let s = Arc::new(read_wav(&mut zip, entry, &sample.name)?);
                            loaded.insert(entry.clone(), s.clone());
                            s
                        }
                    }
                } else {
                    return Err(format!("track \"{}\" has no sound", t.name));
                };
                Kind::Sampler {
                    sample,
                    pitched,
                    shape,
                }
            }
        };
        let mut track = Track::synth(&t.name);
        track.kind = kind;
        track.patterns = vec![EMPTY_GRID; patterns.len()];
        for n in t.notes {
            let (p, l, s) = (n.pattern as usize, n.lane as usize, n.step as usize);
            if p < patterns.len() && l < LANES && s < STEPS {
                track.patterns[p][l][s] = Some(Step {
                    tie: n.tie,
                    velocity: n.velocity.clamp(1, 127),
                    chance: n.chance.clamp(1, 100),
                    ratchet: n.ratchet.clamp(1, 4),
                    nudge: n.nudge.clamp(-50, 50),
                });
            }
        }
        track.volume = t.volume.clamp(0.0, 1.0);
        track.pan = t.pan.clamp(-1.0, 1.0);
        track.reverb = t.reverb.clamp(0.0, 1.0);
        track.delay = t.delay.clamp(0.0, 1.0);
        track.muted = t.muted;
        track.solo = t.solo;
        track.humanize = t.humanize.clamp(0.0, 1.0);
        track.octave = t.octave.min(7);
        track.midi_channel = t
            .midi_channel
            .filter(|c| (1..=16).contains(c))
            .map(|c| c - 1);
        track.midi_note = t.midi_note.min(127);
        track.group = t.group;
        tracks.push(track);
    }

    // MIDI blocks can only point to tracks that made it in.
    let track_count = tracks.len();
    let mut blocks = blocks;
    for m in blocks.iter_mut().filter_map(|b| b.midi.as_mut()) {
        m.track = m.track.filter(|&t| t < track_count);
    }

    let mut project = Project::empty();
    project.tracks = tracks;
    project.number_tracks();
    project.swing = song.swing.clamp(0.0, 1.0);
    let fx = &song.effects;
    project.master = song.master_volume.clamp(0.0, 1.0);
    project.effects = Effects {
        reverb_size: fx.reverb_size.clamp(0.0, 1.0),
        reverb_tone: fx.reverb_tone.clamp(0.0, 1.0),
        delay_steps: fx.delay_steps.round().clamp(1.0, 8.0),
        delay_feedback: fx.delay_feedback.clamp(0.0, 0.9),
    };
    project.blocks = blocks;
    project.clips = clips;
    project.patterns = patterns;
    project.groups = song
        .groups
        .into_iter()
        .map(|g| Group {
            id: g.id,
            name: g.name,
            collapsed: g.collapsed,
            muted: g.muted,
            solo: g.solo,
            volume: g.volume.clamp(0.0, 1.0),
        })
        .collect();
    project.prune_groups();
    Ok(Song {
        project,
        bpm: song.bpm.clamp(40.0, 240.0),
    })
}

/// Highest timeline row a file may use; more would only be empty space.
const MAX_ROWS: usize = 64;

/// The block library and timeline, including from older files: format 2
/// placed patterns on the timeline (each becomes a one-bar block), and
/// format 1 had a single Song row (it becomes a block called Song).
fn read_timeline(song: &SongFile, patterns: &[Pattern]) -> (Vec<Block>, Vec<Clip>) {
    let pattern = |p: &Option<usize>| p.filter(|&p| p < patterns.len());
    let mut blocks: Vec<Block> = song
        .blocks
        .iter()
        .take(MAX_BLOCKS)
        .map(|b| {
            let mut block = Block::new(&b.name);
            if let Some(m) = &b.midi {
                block.midi = Some(read_midi(m));
                return block;
            }
            for (slot, p) in block.chain.iter_mut().zip(&b.chain) {
                *slot = pattern(p);
            }
            block
        })
        .collect();
    // Format 2: one block per pattern that was on the timeline.
    let mut for_pattern: HashMap<usize, usize> = HashMap::new();
    let mut clips = Vec::new();
    for c in &song.clips {
        if c.len == 0 || c.start + c.len > MAX_SONG_STEPS {
            continue;
        }
        let block = match (c.block, pattern(&c.pattern)) {
            (Some(b), _) => b,
            (None, Some(p)) if blocks.len() < MAX_BLOCKS => {
                *for_pattern.entry(p).or_insert_with(|| {
                    let mut block = Block::new(&patterns[p].name);
                    block.chain[0] = Some(p);
                    blocks.push(block);
                    blocks.len() - 1
                })
            }
            _ => continue,
        };
        if block < blocks.len() {
            clips.push(Clip {
                block,
                row: c.row.min(MAX_ROWS),
                start: c.start,
                len: c.len,
                offset: c.offset % blocks[block].len(),
            });
        }
    }
    // Format 1: the Song row becomes a block, placed at the start.
    if song.clips.is_empty() && song.song.iter().any(Option::is_some) {
        let mut block = Block::new("Song");
        for (slot, p) in block.chain.iter_mut().zip(&song.song) {
            *slot = pattern(&p.map(usize::from));
        }
        clips.push(Clip {
            block: blocks.len(),
            row: 0,
            start: 0,
            len: block.len(),
            offset: 0,
        });
        blocks.push(block);
    }
    // There's always a chain block for the Jam view.
    if blocks.iter().all(Block::is_midi) && blocks.len() < MAX_BLOCKS {
        blocks.push(Block::new("Block 1"));
    } else if blocks.iter().all(Block::is_midi) {
        blocks[0] = Block::new("Block 1");
    }
    (blocks, clips)
}

/// A MIDI block's notes, kept within sensible limits.
fn read_midi(m: &MidiFile) -> MidiPart {
    let len = m.len.clamp(1, MAX_SONG_STEPS);
    let notes = m
        .notes
        .iter()
        .take(MAX_NOTES)
        .filter(|n| n.start < (MAX_SONG_STEPS as u32) * PPS)
        .map(|n| crate::project::Note {
            start: n.start,
            len: n.len.clamp(1, MAX_SONG_STEPS as u32 * PPS),
            pitch: n.pitch.min(127),
            velocity: n.velocity.clamp(1, 127),
        })
        .collect();
    MidiPart {
        track: m.track,
        notes,
        len,
    }
}

fn read_wav(
    zip: &mut ZipArchive<Cursor<Vec<u8>>>,
    entry: &str,
    name: &str,
) -> Result<Sample, String> {
    let mut bytes = Vec::new();
    zip.by_name(entry)
        .map_err(|_| format!("missing sample {entry}"))?
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let mut wav = hound::WavReader::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let rate = wav.spec().sample_rate as f32;
    let data = wav
        .samples::<i16>()
        .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
        .collect::<Result<Vec<f32>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(Sample {
        name: name.to_string(),
        data,
        rate,
        builtin: None,
    })
}

/// Where the app remembers which instrument panel sections are closed.
#[cfg(not(target_arch = "wasm32"))]
fn closed_sections_record() -> Option<std::path::PathBuf> {
    Some(dirs::config_dir()?.join("daw").join("closed-sections"))
}

/// The instrument panel sections closed last time, one title per line.
pub fn closed_sections() -> Vec<String> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(text) = closed_sections_record().and_then(|p| std::fs::read_to_string(p).ok()) {
        return text.lines().map(str::to_string).collect();
    }
    Vec::new()
}

/// Remembers which sections are closed. Failing to is harmless, and the
/// web version only remembers until the page closes.
pub fn remember_closed_sections(closed: &[String]) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(record) = closed_sections_record() {
        if let Some(dir) = record.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(record, closed.join("\n"));
    }
    #[cfg(target_arch = "wasm32")]
    let _ = closed;
}

/// Where the app remembers the last song, so it reopens at startup.
#[cfg(not(target_arch = "wasm32"))]
fn last_song_record() -> Option<std::path::PathBuf> {
    Some(dirs::config_dir()?.join("daw").join("last-song"))
}

/// The song that was open last time, if it still exists.
#[cfg(not(target_arch = "wasm32"))]
pub fn last_song() -> Option<std::path::PathBuf> {
    let path = std::fs::read_to_string(last_song_record()?).ok()?;
    let path = std::path::PathBuf::from(path.trim());
    path.exists().then_some(path)
}

/// Remembers a song to reopen next time. Failing to is harmless.
#[cfg(not(target_arch = "wasm32"))]
pub fn remember(song: &Path) {
    if let Some(record) = last_song_record() {
        if let Some(dir) = record.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(record, song.to_string_lossy().as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("daw-file-test-{}-{name}.daw", std::process::id()))
    }

    #[test]
    fn songs_survive_a_save_and_open() {
        let mut project = Project::demo();
        project.swing = 0.4;
        project.duplicate_pattern(0);
        project.patterns[8].name = "Chorus".into();
        let mut chorus = Block::new("Chorus");
        chorus.chain[0] = Some(2);
        chorus.chain[2] = Some(8);
        project.blocks.push(chorus);
        let riff = project.new_midi_block(Some(4)).unwrap();
        project.blocks[riff].midi.as_mut().unwrap().notes = vec![
            crate::project::Note {
                start: 80,
                len: 1000,
                pitch: 91,
                velocity: 64,
            },
            crate::project::Note {
                start: 0,
                len: 240,
                pitch: 12,
                velocity: NORMAL,
            },
        ];
        project.clips.push(Clip {
            block: 1,
            row: 1,
            start: 48,
            len: 100,
            offset: 3,
        });
        project.tracks[1].patterns[2][0][5] = Some(Step {
            velocity: 30,
            chance: 50,
            ratchet: 3,
            nudge: -20,
            tie: false,
        });
        if let Kind::Synth(v) = &mut project.tracks[4].kind {
            v[synth::CUTOFF] = 1234.0;
        }
        project.groups[0].collapsed = true;
        project.groups[0].volume = 0.5;
        project.tracks[5].pan = -0.3;
        project.tracks[4].delay = 0.6;
        project.effects.delay_steps = 6.0;
        project.effects.reverb_size = 0.8;
        project.master = 0.55;
        // A loaded sample, used by two tracks.
        let tone = Arc::new(Sample {
            name: "tone".into(),
            data: (0..100).map(|i| (i as f32 / 100.0) - 0.5).collect(),
            rate: 44_100.0,
            builtin: None,
        });
        project.add_track(Track::sampler(tone.clone(), true));
        project.add_track(Track::sampler(tone, false));
        let shaped = SampleShape {
            start: 0.2,
            end: 0.9,
            attack: 0.05,
            release: 0.4,
            ring: false,
            reverse: true,
        };
        if let Some(Kind::Sampler { shape, .. }) = project.tracks.last_mut().map(|t| &mut t.kind) {
            *shape = shaped;
        }

        let path = temp("roundtrip");
        save(&path, &project, 97.0).unwrap();
        let opened = open(&path).unwrap();
        let mut zip = ZipArchive::new(File::open(&path).unwrap()).unwrap();
        let stored_samples = zip
            .file_names()
            .filter(|n| n.starts_with("samples/"))
            .count();
        let _ = zip.by_name(SONG_JSON).unwrap();
        fs::remove_file(&path).unwrap();

        let p = &opened.project;
        assert_eq!(opened.bpm, 97.0);
        assert!(song_format(&project) == FORMAT, "MIDI blocks need format 4");
        assert_eq!(p.swing, 0.4);
        assert_eq!(p.clips, project.clips);
        assert_eq!(p.patterns, project.patterns);
        assert_eq!(p.blocks, project.blocks);
        assert_eq!(p.tracks.len(), project.tracks.len());
        for (a, b) in p.tracks.iter().zip(&project.tracks) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.patterns, b.patterns, "{}", a.name);
            assert_eq!(a.midi_channel, b.midi_channel);
            assert_eq!(a.group, b.group);
            assert_eq!(a.pan, b.pan);
            assert_eq!((a.reverb, a.delay), (b.reverb, b.delay));
        }
        let Kind::Synth(v) = &p.tracks[4].kind else {
            panic!("lead is a synth")
        };
        assert_eq!(v[synth::CUTOFF], 1234.0);
        assert!(p.groups[0].collapsed);
        assert_eq!(p.groups[0].volume, 0.5);
        assert_eq!(p.effects, project.effects);
        assert_eq!(p.master, 0.55);
        let Some(Kind::Sampler { shape, .. }) = p.tracks.last().map(|t| &t.kind) else {
            panic!("a sample track")
        };
        assert_eq!(*shape, shaped);

        // Built-in drums aren't stored; the shared sample is stored once.
        assert_eq!(stored_samples, 1);
        let samples: Vec<&Arc<Sample>> = p.tracks[6..]
            .iter()
            .map(|t| match &t.kind {
                Kind::Sampler { sample, .. } => sample,
                Kind::Synth(_) => panic!("sample track"),
            })
            .collect();
        assert!(Arc::ptr_eq(samples[0], samples[1]), "still shared");
        assert_eq!(samples[0].rate, 44_100.0);
        assert_eq!(samples[0].data.len(), 100);
        assert!(
            (samples[0].data[10] - (-0.4)).abs() < 0.001,
            "16-bit is close"
        );
        let Kind::Sampler { sample: kick, .. } = &p.tracks[0].kind else {
            panic!("kick is a sample")
        };
        assert_eq!(kick.builtin, Some(0));
    }

    /// The format a song would be saved with.
    fn song_format(project: &Project) -> u32 {
        let bytes = to_bytes(project, 120.0).unwrap();
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut json = String::new();
        zip.by_name(SONG_JSON)
            .unwrap()
            .read_to_string(&mut json)
            .unwrap();
        serde_json::from_str::<SongFile>(&json).unwrap().format
    }

    #[test]
    fn songs_without_midi_blocks_stay_format_3() {
        assert_eq!(song_format(&Project::demo()), 3, "older versions open it");
    }

    #[test]
    fn midi_blocks_with_missing_tracks_go_silent() {
        let json = r#"{
            "format": 4,
            "blocks": [ { "name": "Riff", "midi": { "track": 5, "len": 0,
                "notes": [ { "start": 0, "len": 0, "pitch": 200 } ] } } ]
        }"#;
        let song = read_json(json);
        let p = &song.project;
        assert_eq!(p.blocks.len(), 2, "a chain block is added for Jam");
        let m = p.blocks[0].midi.as_ref().unwrap();
        assert_eq!(m.track, None);
        assert_eq!(m.len, 1);
        let n = m.notes[0];
        assert_eq!((n.len, n.pitch, n.velocity), (1, 127, NORMAL));
    }

    fn read_json(json: &str) -> Song {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(SONG_JSON, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(json.as_bytes()).unwrap();
        from_bytes("test", zip.finish().unwrap().into_inner()).unwrap()
    }

    #[test]
    fn old_and_hand_written_files_fill_in_defaults() {
        // Only the bare minimum; everything else should get a default.
        let json = r#"{
            "format": 1,
            "song": [0, 0, null, 3],
            "tracks": [
                { "name": "Bass", "instrument": { "type": "synth", "settings": { "Cutoff": 500, "Wave": 0.25 } },
                  "notes": [ { "pattern": 0, "lane": 2, "step": 4 } ],
                  "some_future_field": true }
            ]
        }"#;
        let path = temp("minimal");
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        zip.start_file(SONG_JSON, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(json.as_bytes()).unwrap();
        zip.finish().unwrap();
        let song = open(&path).unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(song.bpm, 120.0);
        assert_eq!(song.project.effects, Effects::DEFAULT);
        // Format 1's Song row becomes a block, placed at the start.
        let p = &song.project;
        assert_eq!(p.blocks.len(), 1);
        assert_eq!(p.blocks[0].name, "Song");
        assert_eq!(
            p.blocks[0].chain[..5],
            [Some(0), Some(0), None, Some(3), None]
        );
        let clips: Vec<(usize, usize, usize)> =
            p.clips.iter().map(|c| (c.block, c.start, c.len)).collect();
        assert_eq!(clips, [(0, 0, 64)]);
        assert_eq!(song.project.patterns, default_patterns());
        let t = &song.project.tracks[0];
        assert_eq!(t.patterns[0][2][4], Some(Step::HIT));
        let Kind::Synth(v) = &t.kind else {
            panic!("synth")
        };
        assert_eq!(v[synth::CUTOFF], 500.0);
        assert_eq!(v[synth::WAVE], synth::SAW + 0.25, "old Wave knob");
        assert_eq!(v[synth::WAVE_2], synth::SAW + 0.25);
        assert_eq!(v[synth::LFO_AMOUNT], 0.0, "new knobs start off");
        assert_eq!(v[synth::RESONANCE], synth::PARAMS[synth::RESONANCE].default);
    }

    #[test]
    fn format_2_pattern_clips_become_blocks() {
        let json = r#"{
            "format": 2,
            "patterns": [ { "name": "A" }, { "name": "Verse" } ],
            "clips": [
                { "pattern": 1, "start": 0, "len": 32, "offset": 4 },
                { "pattern": 0, "row": 1, "start": 16, "len": 16 },
                { "pattern": 1, "start": 64, "len": 16 }
            ]
        }"#;
        let path = temp("format2");
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        zip.start_file(SONG_JSON, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(json.as_bytes()).unwrap();
        zip.finish().unwrap();
        let song = open(&path).unwrap();
        fs::remove_file(&path).unwrap();

        let p = &song.project;
        let names: Vec<&str> = p.blocks.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["Verse", "A"], "one block per pattern");
        assert_eq!(p.blocks[0].chain[0], Some(1));
        let clips: Vec<(usize, usize, usize, usize)> = p
            .clips
            .iter()
            .map(|c| (c.block, c.start, c.len, c.offset))
            .collect();
        assert_eq!(clips, [(0, 0, 32, 4), (1, 16, 16, 0), (0, 64, 16, 0)]);
    }

    #[test]
    fn newer_formats_and_other_files_are_refused_politely() {
        let path = temp("future");
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        zip.start_file(SONG_JSON, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{ "format": 99 }"#).unwrap();
        zip.finish().unwrap();
        let future = open(&path);
        fs::write(&path, b"just some text").unwrap();
        let text = open(&path);
        fs::remove_file(&path).unwrap();

        assert!(future.err().unwrap().contains("newer version"));
        assert!(text.err().unwrap().contains("not a daw song file"));
    }
}
