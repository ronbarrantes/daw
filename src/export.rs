//! Exporting a song: as a WAV file (the sound, for sharing) or a standard
//! MIDI file (the notes, for other DAWs and hardware synths).
//!
//! Both render the song offline with the real engine. They export the song
//! row once; if it's empty, the current pattern four times.

use std::io::Cursor;

use crate::audio;
use crate::project::Project;

pub const SAMPLE_RATE: u32 = 48_000;
/// Seconds of sound kept after the last bar, so notes can ring out.
const TAIL_SECONDS: f32 = 2.0;
/// How many times to play the pattern when the song row is empty.
const PATTERN_LOOPS: usize = 4;
/// MIDI ticks per beat.
const PPQ: u16 = 480;
/// General MIDI's drum channel (channel 10, counting from 1).
const DRUM_CHANNEL: u8 = 9;

/// The song as a 16-bit stereo WAV file.
pub fn wav(project: &Project, bpm: f32, pattern: usize) -> Result<Vec<u8>, String> {
    let r = audio::render(
        project.clone(),
        bpm,
        pattern,
        PATTERN_LOOPS,
        SAMPLE_RATE as f32,
        TAIL_SECONDS,
    );
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let err = |e: hound::Error| format!("Couldn't export: {e}");
    let mut bytes = Cursor::new(Vec::new());
    let mut wav = hound::WavWriter::new(&mut bytes, spec).map_err(err)?;
    for frame in r.audio {
        for s in frame {
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            wav.write_sample(v).map_err(err)?;
        }
    }
    wav.finalize().map_err(err)?;
    Ok(bytes.into_inner())
}

/// The song as a standard MIDI file, one MIDI track per channel.
pub fn midi(project: &Project, bpm: f32, pattern: usize) -> Vec<u8> {
    let (project, names) = with_channels(project);
    let r = audio::render(
        project,
        bpm,
        pattern,
        PATTERN_LOOPS,
        SAMPLE_RATE as f32,
        0.0,
    );
    smf(&r.midi, &names, bpm, SAMPLE_RATE as f32)
}

/// A copy of the project where every audible track sends MIDI, plus the
/// track names on each channel. Tracks keep their own channel if they have
/// one; drums default to channel 10 and synths take free channels. Muted
/// tracks (and those silenced by solo) are left out.
fn with_channels(project: &Project) -> (Project, [Vec<String>; 16]) {
    let mut out = project.clone();
    let mut used: Vec<u8> = project
        .tracks
        .iter()
        .filter_map(|t| t.midi_channel)
        .collect();
    used.push(DRUM_CHANNEL);
    let mut names: [Vec<String>; 16] = Default::default();
    for (i, t) in out.tracks.iter_mut().enumerate() {
        if project.gain(i) == 0.0 {
            t.midi_channel = None;
            continue;
        }
        if t.midi_channel.is_none() {
            t.midi_channel = if t.is_melodic() {
                let free = (0..16).find(|c| !used.contains(c));
                used.extend(free);
                free
            } else {
                Some(DRUM_CHANNEL)
            };
        }
        if let Some(ch) = t.midi_channel {
            names[ch as usize].push(t.name.clone());
        }
    }
    (out, names)
}

/// MIDI's variable-length numbers: 7 bits per byte, high bit = more follows.
fn vlq(mut n: u32) -> Vec<u8> {
    let mut bytes = vec![(n & 0x7F) as u8];
    n >>= 7;
    while n > 0 {
        bytes.push((n & 0x7F) as u8 | 0x80);
        n >>= 7;
    }
    bytes.reverse();
    bytes
}

fn meta(track: &mut Vec<u8>, kind: u8, data: &[u8]) {
    track.extend(vlq(0));
    track.extend([0xFF, kind]);
    track.extend(vlq(data.len() as u32));
    track.extend(data);
}

fn chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    out.extend(id);
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(data);
}

/// Builds a format-1 standard MIDI file: a tempo track, then one track per
/// channel that has notes.
fn smf(
    events: &[(usize, crate::midi::Msg)],
    names: &[Vec<String>; 16],
    bpm: f32,
    sr: f32,
) -> Vec<u8> {
    let to_ticks = |sample: usize| -> u32 {
        (sample as f64 * bpm as f64 / 60.0 * PPQ as f64 / sr as f64).round() as u32
    };
    let mut tracks = Vec::new();

    let mut tempo = Vec::new();
    meta(&mut tempo, 0x03, b"daw");
    let micros_per_beat = (60_000_000.0 / bpm).round() as u32;
    meta(&mut tempo, 0x51, &micros_per_beat.to_be_bytes()[1..]);
    meta(&mut tempo, 0x58, &[4, 2, 24, 8]); // 4/4
    meta(&mut tempo, 0x2F, &[]);
    tracks.push(tempo);

    for ch in 0..16u8 {
        let notes: Vec<(u32, &[u8])> = events
            .iter()
            .filter_map(|(t, m)| {
                let b = m.bytes();
                let is_note = b.len() == 3 && matches!(b[0] & 0xF0, 0x80 | 0x90);
                (is_note && b[0] & 0x0F == ch).then(|| (to_ticks(*t), b))
            })
            .collect();
        if notes.is_empty() {
            continue;
        }
        let mut track = Vec::new();
        meta(&mut track, 0x03, names[ch as usize].join(", ").as_bytes());
        let mut last = 0;
        for (tick, bytes) in notes {
            track.extend(vlq(tick - last));
            track.extend(bytes);
            last = tick;
        }
        meta(&mut track, 0x2F, &[]);
        tracks.push(track);
    }

    let mut out = Vec::new();
    let mut header = Vec::new();
    header.extend(1u16.to_be_bytes()); // format 1: several tracks
    header.extend((tracks.len() as u16).to_be_bytes());
    header.extend(PPQ.to_be_bytes());
    chunk(&mut out, b"MThd", &header);
    for t in &tracks {
        chunk(&mut out, b"MTrk", t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drums;
    use crate::project::Track;

    /// A note event read back: (tick, status, note, velocity).
    type Event = (u32, u8, u8, u8);

    /// Reads back a MIDI file: its ticks per beat, and each track's name and
    /// note events.
    fn read_smf(bytes: &[u8]) -> (u16, Vec<(String, Vec<Event>)>) {
        assert_eq!(&bytes[..4], b"MThd");
        let ppq = u16::from_be_bytes([bytes[12], bytes[13]]);
        let mut tracks = Vec::new();
        let mut i = 14;
        while i < bytes.len() {
            assert_eq!(&bytes[i..i + 4], b"MTrk");
            let len = u32::from_be_bytes(bytes[i + 4..i + 8].try_into().unwrap()) as usize;
            let data = &bytes[i + 8..i + 8 + len];
            i += 8 + len;
            let (mut j, mut tick, mut name, mut notes) = (0, 0, String::new(), Vec::new());
            while j < data.len() {
                let mut delta = 0u32;
                loop {
                    let b = data[j];
                    j += 1;
                    delta = (delta << 7) | (b & 0x7F) as u32;
                    if b & 0x80 == 0 {
                        break;
                    }
                }
                tick += delta;
                if data[j] == 0xFF {
                    let (kind, len) = (data[j + 1], data[j + 2] as usize);
                    if kind == 0x03 {
                        name = String::from_utf8_lossy(&data[j + 3..j + 3 + len]).into();
                    }
                    j += 3 + len;
                } else {
                    notes.push((tick, data[j], data[j + 1], data[j + 2]));
                    j += 3;
                }
            }
            tracks.push((name, notes));
        }
        (ppq, tracks)
    }

    #[test]
    fn wav_holds_the_song_plus_a_tail() {
        // The demo's song row is one bar: 2 seconds at 120 BPM.
        let bytes = wav(&Project::demo(), 120.0, 0).unwrap();
        let mut reader = hound::WavReader::new(Cursor::new(bytes)).unwrap();
        let spec = reader.spec();
        let frames = reader.duration() as f32;
        let loudest = reader
            .samples::<i16>()
            .map(|s| s.unwrap().unsigned_abs())
            .max()
            .unwrap();
        assert_eq!((spec.channels, spec.sample_rate), (2, SAMPLE_RATE));
        let seconds = frames / SAMPLE_RATE as f32;
        assert!((seconds - (2.0 + TAIL_SECONDS)).abs() < 0.01, "{seconds} s");
        assert!(loudest > 3_000, "not silent");
    }

    #[test]
    fn midi_has_every_note_on_its_channel() {
        let mut project = Project::demo();
        project.tracks[5].midi_channel = None; // the pad gets a free channel
        project.tracks[3].muted = true; // clap: left out
        let (ppq, tracks) = read_smf(&midi(&project, 120.0, 0));
        assert_eq!(ppq, PPQ);

        let on = |name: &str| -> Vec<(u32, u8, u8)> {
            let (_, notes) = tracks.iter().find(|(n, _)| n.contains(name)).unwrap();
            notes
                .iter()
                .filter(|(_, status, _, vel)| status & 0xF0 == 0x90 && *vel > 0)
                .map(|&(tick, status, note, _)| (tick, status & 0x0F, note))
                .collect()
        };
        // Kicks on beats 1-4, on channel 10, at 480 ticks per beat.
        let kick: Vec<(u32, u8, u8)> = on("Kick")
            .into_iter()
            .filter(|n| n.2 == drums::NOTES[0])
            .collect();
        assert_eq!(
            kick,
            [0, 480, 960, 1440].map(|t| (t, DRUM_CHANNEL, drums::NOTES[0]))
        );
        assert_eq!(on("Lead").len(), 7, "the riff's seven notes");
        assert_eq!(on("Pad").len(), 6, "two three-note chords");
        assert!(tracks.iter().all(|(n, _)| !n.contains("Clap")), "muted");
    }

    #[test]
    fn spare_synths_get_free_channels() {
        let mut project = Project::demo();
        project.tracks.push(Track::synth("Bass"));
        let (out, names) = with_channels(&project);
        let bass = out.tracks.last().unwrap().midi_channel.unwrap();
        assert!(![0, 1, DRUM_CHANNEL].contains(&bass), "{bass}");
        assert_eq!(names[bass as usize], ["Bass"]);
        assert_eq!(
            names[DRUM_CHANNEL as usize],
            ["Kick", "Snare", "Hat", "Clap"]
        );
    }

    #[test]
    fn variable_length_numbers() {
        assert_eq!(vlq(0), [0x00]);
        assert_eq!(vlq(0x7F), [0x7F]);
        assert_eq!(vlq(0x80), [0x81, 0x00]);
        assert_eq!(vlq(0x0FFF_FFFF), [0xFF, 0xFF, 0xFF, 0x7F]);
    }
}
