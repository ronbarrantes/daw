//! Browse sound sources without rebuilding a track or its patterns.
use std::sync::Arc;

use eframe::egui;

use crate::project::{self, Kind, Sample, SampleShape, Track};

#[derive(Default)]
pub struct Browser {
    search: String,
    builtins: Option<Vec<Arc<Sample>>>,
}

impl Browser {
    pub fn show(&mut self, ui: &mut egui::Ui, tracks: &[Track]) -> Option<Arc<Sample>> {
        let mut picked = None;
        ui.collapsing("Browse drum sounds", |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search sounds"));
            ui.weak("Choose a sound, then Play a note to hear it. Undo restores the old sound.");
            ui.weak("Use + Sample or drop files to add sounds to this song's browser.");
            let query = self.search.to_lowercase();
            let builtins = self.builtins.get_or_insert_with(project::builtin_drums);
            let mut imported: Vec<Arc<Sample>> = Vec::new();
            for track in tracks {
                if let Kind::Sampler { sample, .. } = &track.kind {
                    if sample.builtin.is_none() && !imported.iter().any(|s| Arc::ptr_eq(s, sample)) {
                        imported.push(sample.clone());
                    }
                }
            }
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for (heading, sounds) in [("Built-in drums", &*builtins), ("Song samples", &imported)] {
                    ui.strong(heading);
                    let mut count = 0;
                    for sound in sounds.iter().filter(|s| s.name.to_lowercase().contains(&query)) {
                        count += 1;
                        if ui.button(&sound.name).on_hover_text("Replace the selected track's sound; keep its notes and routing").clicked() {
                            picked = Some(sound.clone());
                        }
                    }
                    if count == 0 { ui.weak("No matching sounds"); }
                }
            });
        });
        picked
    }
}

/// Reset sample-specific trim/fades; retain the track's identity, notes,
/// pitch mode, name, mix and MIDI mapping.
pub fn replace(track: &mut Track, replacement: Arc<Sample>) -> bool {
    if let Kind::Sampler { sample, shape, .. } = &mut track.kind {
        if Arc::ptr_eq(sample, &replacement) {
            return false;
        }
        *sample = replacement;
        *shape = SampleShape::DEFAULT;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_sound_preserves_track_and_survives_save() {
        let mut p = project::Project::demo();
        let before = p.tracks[0].clone();
        let replacement = Arc::new(Sample {
            name: "Custom kick".into(),
            data: vec![0.1, 0.0],
            rate: 48000.0,
            builtin: None,
        });
        assert!(replace(&mut p.tracks[0], replacement));
        let bytes = crate::file::to_bytes(&p, 120.0).unwrap();
        let loaded = crate::file::from_bytes("test.daw", bytes).unwrap().project;
        match &loaded.tracks[0].kind {
            Kind::Sampler { sample, .. } => assert_eq!(sample.name, "Custom kick"),
            _ => unreachable!(),
        }
        assert_eq!(p.tracks[0].id, before.id);
        assert_eq!(loaded.tracks[0].patterns, before.patterns);
        assert_eq!(loaded.tracks[0].name, before.name);
        assert_eq!(loaded.tracks[0].midi_channel, before.midi_channel);
        assert_eq!(loaded.tracks[0].midi_note, before.midi_note);
        assert_eq!(loaded.tracks[0].volume, before.volume);
    }
}
