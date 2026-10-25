//! Loads audio files (WAV, MP3, FLAC, OGG) into memory as mono samples.

use std::io::{Cursor, ErrorKind};
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::project::Sample;

/// Longer files would eat too much memory on small laptops.
const MAX_SECONDS: f32 = 300.0;

pub const EXTENSIONS: [&str; 5] = ["wav", "mp3", "flac", "ogg", "aiff"];

/// Reads and decodes an audio file from disk.
#[cfg(not(target_arch = "wasm32"))]
pub fn load(path: &Path) -> Result<Sample, String> {
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let bytes = std::fs::read(path).map_err(|e| format!("{file_name}: {e}"))?;
    decode(&file_name, bytes)
}

/// Decodes an audio file's contents. `file_name` names the sample and
/// hints at the format.
pub fn decode(file_name: &str, bytes: Vec<u8>) -> Result<Sample, String> {
    let path = Path::new(file_name);
    let name = path
        .file_stem()
        .map_or("Sample".into(), |s| s.to_string_lossy().into_owned());
    let stream = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let err = |e: Error| format!("{name}: {e}");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(err)?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or(format!("{name}: no audio found"))?;
    let track_id = track.id;
    let rate = track
        .codec_params
        .sample_rate
        .ok_or(format!("{name}: unknown sample rate"))? as f32;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(err)?;

    let mut data = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(err(e)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A damaged packet: skip it rather than reject the whole file.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(err(e)),
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count();
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buf.copy_interleaved_ref(decoded);
        data.extend(
            buf.samples()
                .chunks(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32),
        );
        if data.len() as f32 > MAX_SECONDS * rate {
            return Err(format!(
                "{name}: longer than {} minutes",
                MAX_SECONDS / 60.0
            ));
        }
    }
    if data.is_empty() {
        return Err(format!("{name}: no audio found"));
    }
    Ok(Sample {
        name,
        data,
        rate,
        builtin: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a 16-bit stereo WAV of `frames` frames.
    fn write_wav(path: &Path, rate: u32, frames: u32) {
        let data_len = frames * 4;
        let mut b = Vec::new();
        b.extend(b"RIFF");
        b.extend((36 + data_len).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes()); // PCM
        b.extend(2u16.to_le_bytes()); // stereo
        b.extend(rate.to_le_bytes());
        b.extend((rate * 4).to_le_bytes());
        b.extend(4u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend(data_len.to_le_bytes());
        for i in 0..frames {
            let v = if i % 2 == 0 { 16_000i16 } else { -16_000 };
            b.extend(v.to_le_bytes()); // left
            b.extend(0i16.to_le_bytes()); // right
        }
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn loads_wav_as_mono() {
        let dir = std::env::temp_dir().join(format!("daw-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("guitar.wav");
        write_wav(&path, 44_100, 1_000);
        let s = load(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(s.name, "guitar");
        assert_eq!(s.rate, 44_100.0);
        assert_eq!(s.data.len(), 1_000);
        // Left at ±0.49, right silent: the mono mix is about ±0.24.
        assert!((s.data[0] - 0.244).abs() < 0.01, "{}", s.data[0]);
    }

    #[test]
    fn rejects_non_audio() {
        let path = std::env::temp_dir().join(format!("daw-test-{}.wav", std::process::id()));
        std::fs::write(&path, b"not audio").unwrap();
        let result = load(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(result.is_err());
    }
}
