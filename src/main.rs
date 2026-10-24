mod audio;
mod drums;
mod midi;
mod project;
mod sample;
mod synth;

use std::collections::VecDeque;
use std::f32::consts::PI;
use std::path::PathBuf;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use eframe::egui::{self, Align, Color32, Layout, Pos2, RichText, Sense, Stroke, Vec2};

use audio::{Event, Shared};
use midi::Midi;
use project::{Kind, Project, Sample, Steps, Track, MAX_TRACKS, PATTERNS, SONG_SLOTS, STEPS, TIE};
use synth::{Param, Unit, PARAMS};

const DRUM_ON: Color32 = Color32::from_rgb(255, 140, 40);
const NOTE_ON: Color32 = Color32::from_rgb(80, 180, 255);
const NOTE_HELD: Color32 = Color32::from_rgb(45, 110, 160);
const RECORDING: Color32 = Color32::from_rgb(220, 60, 60);
/// How many past project snapshots to keep alive for the audio thread.
const HISTORY: usize = 8;

const TRACK_VOLUME: Param = Param {
    name: "Volume",
    min: 0.0,
    max: 1.0,
    default: 0.8,
    log: false,
    unit: Unit::Percent,
};

fn main() -> eframe::Result<()> {
    let project = Project::demo();
    let snapshot = Arc::new(project.clone());
    let shared = Arc::new(Shared::new(snapshot.clone()));
    let (events, events_rx) = sync_channel(256);
    let (midi_tx, midi_rx) = sync_channel(1024);
    let midi = Midi::new(shared.clone(), events.clone(), midi_rx);
    // Held until the window closes; dropping the stream stops audio.
    let stream = audio::start(shared.clone(), events_rx, midi_tx);
    let audio_error = stream.as_ref().err().cloned();
    let (load_tx, load_rx) = channel();

    // Sized to fit the 1366x768 screen of an 11" MacBook Air.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1060.0, 690.0]),
        ..Default::default()
    };
    let app = App {
        shared,
        project,
        history: VecDeque::from([snapshot]),
        dirty: false,
        events,
        midi,
        audio_error,
        clipboard: None,
        selected: 4,
        load_tx,
        load_rx,
        loading: 0,
        load_error: None,
        recording: false,
        held_key: None,
    };
    eframe::run_native("daw", options, Box::new(|_cc| Ok(Box::new(app))))
}

struct App {
    shared: Arc<Shared>,
    /// The editable project; changes are published to the engine as snapshots.
    project: Project,
    /// Recent snapshots, kept so the audio thread is never the one to free one.
    history: VecDeque<Arc<Project>>,
    /// The project changed this frame and needs publishing.
    dirty: bool,
    events: SyncSender<Event>,
    midi: Midi,
    audio_error: Option<String>,
    clipboard: Option<Vec<Steps>>,
    /// Track shown in the side panel and piano roll.
    selected: usize,
    load_tx: Sender<Result<Sample, String>>,
    load_rx: Receiver<Result<Sample, String>>,
    /// Files still being decoded.
    loading: usize,
    load_error: Option<String>,
    /// Keys played while playing are written into the pattern.
    recording: bool,
    /// The computer-keyboard key holding a note, and the track it plays.
    held_key: Option<(egui::Key, usize)>,
}

/// Computer keys laid out like a piano, as in FL Studio: the home row is
/// the white keys and the row above has the black keys.
fn key_semitone(key: egui::Key) -> Option<u8> {
    use egui::Key::*;
    let keys = [A, W, S, E, D, F, T, G, Y, H, U, J, K, O, L, P];
    keys.iter().position(|&k| k == key).map(|i| i as u8)
}

fn pattern_name(p: usize) -> String {
    char::from(b'A' + p as u8).to_string()
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let playing = self.shared.playing.load(Relaxed);
        if !ctx.wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                self.shared.playing.store(!playing, Relaxed);
            }
            self.computer_keyboard(ctx, playing);
        }

        while let Ok(result) = self.load_rx.try_recv() {
            self.loading -= 1;
            match result {
                Ok(sample) => self.add_track(Track::sampler(Arc::new(sample), false)),
                Err(e) => self.load_error = Some(e),
            }
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        for path in dropped {
            self.load_file(ctx, path);
        }

        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            ui.add_space(4.0);
            for err in [&self.audio_error, &self.midi.error, &self.load_error]
                .into_iter()
                .flatten()
            {
                ui.colored_label(Color32::RED, err);
            }
            self.transport_row(ui, playing);
            self.midi_row(ui);
            ui.add_space(4.0);
        });

        egui::SidePanel::right("instrument")
            .resizable(false)
            .exact_width(290.0)
            .show(ctx, |ui| self.instrument_panel(ui));

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.song_row(ui, playing);
                ui.add_space(6.0);
                self.pattern_row(ui);
                ui.add_space(6.0);
                let edit = self.shared.edit_pattern.load(Relaxed) as usize;
                // Only show the playhead on the pattern that's actually playing.
                let current = (playing && self.shared.play_pattern.load(Relaxed) as usize == edit)
                    .then(|| self.shared.current_step.load(Relaxed));
                for i in 0..self.project.tracks.len() {
                    self.track_row(ui, i, edit, current);
                }
                ui.add_space(4.0);
                self.add_row(ui, ctx);
                if self
                    .project
                    .tracks
                    .get(self.selected)
                    .is_some_and(Track::is_melodic)
                {
                    ui.add_space(8.0);
                    self.piano_roll(ui, edit, current);
                }
            });
            if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
                let painter = ui.painter();
                let rect = ui.max_rect();
                painter.rect_filled(rect, 0.0, Color32::from_black_alpha(180));
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Drop to add as sample tracks",
                    egui::FontId::proportional(24.0),
                    Color32::WHITE,
                );
            }
        });

        if self.dirty {
            self.publish();
        }
        // Only redraw continuously while playing, to keep CPU low when idle.
        if playing {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
}

impl App {
    fn publish(&mut self) {
        let snapshot = Arc::new(self.project.clone());
        self.shared.publish(snapshot.clone());
        self.history.push_back(snapshot);
        if self.history.len() > HISTORY {
            self.history.pop_front();
        }
        self.dirty = false;
    }

    fn play_note(&self, track: usize, semitone: u8) {
        let _ = self.events.try_send(Event::Note { track, semitone });
    }

    fn add_track(&mut self, track: Track) {
        if self.project.tracks.len() >= MAX_TRACKS {
            self.load_error = Some(format!("Can't add {}: {MAX_TRACKS} tracks max", track.name));
            return;
        }
        self.project.tracks.push(track);
        self.selected = self.project.tracks.len() - 1;
        self.load_error = None;
        self.dirty = true;
    }

    /// Decodes a file on a background thread so the UI doesn't freeze.
    fn load_file(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.loading += 1;
        let tx = self.load_tx.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _ = tx.send(sample::load(&path));
            ctx.request_repaint();
        });
    }

    fn transport_row(&mut self, ui: &mut egui::Ui, playing: bool) {
        let s = &self.shared;
        ui.horizontal(|ui| {
            let label = if playing { "■ Stop" } else { "▶ Play" };
            if ui.button(label).clicked() {
                s.playing.store(!playing, Relaxed);
            }
            let mut bpm = s.bpm();
            ui.label("BPM");
            let drag = egui::DragValue::new(&mut bpm)
                .range(40.0..=240.0)
                .speed(0.5);
            if ui.add(drag).changed() {
                s.set_bpm(bpm);
            }
            ui.separator();
            let song = s.song_mode.load(Relaxed);
            let pattern_hint = "Loop the pattern you're editing";
            if ui
                .selectable_label(!song, "Pattern")
                .on_hover_text(pattern_hint)
                .clicked()
            {
                s.song_mode.store(false, Relaxed);
            }
            let song_hint = "Play the song row, bar by bar";
            if ui
                .selectable_label(song, "Song")
                .on_hover_text(song_hint)
                .clicked()
            {
                s.song_mode.store(true, Relaxed);
            }
            ui.separator();
            let rec = egui::Button::new(RichText::new("⏺ Rec").color(if self.recording {
                RECORDING
            } else {
                Color32::GRAY
            }))
            .selected(self.recording);
            let rec_hint = "While playing, notes you play on the keyboard\n\
                            are written into the selected track";
            if ui.add(rec).on_hover_text(rec_hint).clicked() {
                self.recording = !self.recording;
            }
            ui.separator();
            ui.weak("Space = play/stop · keys A–K play the selected track · Z/X = octave");
        });
    }

    /// Plays the selected track from the computer keyboard, like a MIDI
    /// keyboard, and records into the pattern when Rec is on.
    fn computer_keyboard(&mut self, ctx: &egui::Context, playing: bool) {
        let Some(track) = self.project.tracks.get_mut(self.selected) else {
            return;
        };
        let keys: Vec<(egui::Key, bool)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed,
                        repeat: false,
                        ..
                    } => Some((*key, *pressed)),
                    _ => None,
                })
                .collect()
        });
        for (key, pressed) in keys {
            match key {
                egui::Key::Z if pressed => track.octave = track.octave.saturating_sub(1),
                egui::Key::X if pressed => track.octave = (track.octave + 1).min(7),
                _ => {
                    let Some(semitone) = key_semitone(key) else {
                        continue;
                    };
                    if pressed {
                        if let Some((_, t)) = self.held_key {
                            let _ = self.events.try_send(Event::Release { track: t });
                        }
                        self.held_key = Some((key, self.selected));
                        let _ = self.events.try_send(Event::Hold {
                            track: self.selected,
                            semitone,
                        });
                        if self.recording && playing {
                            let pattern = self.shared.edit_pattern.load(Relaxed) as usize;
                            let step = self.shared.current_step.load(Relaxed);
                            // The piano roll shows one octave, so fold into it.
                            let value = if track.is_melodic() { semitone % 12 } else { 0 };
                            track.patterns[pattern][step] = Some(value);
                        }
                    } else if self.held_key.is_some_and(|(k, _)| k == key) {
                        let (_, t) = self.held_key.take().unwrap();
                        let _ = self.events.try_send(Event::Release { track: t });
                    }
                    continue;
                }
            }
            // Octave changed.
            self.dirty = true;
        }
        if self.recording && playing {
            self.dirty = true;
        }
    }

    fn midi_row(&mut self, ui: &mut egui::Ui) {
        let s = self.shared.clone();
        ui.horizontal(|ui| {
            let m = &mut self.midi;
            if let Some(pick) = port_picker(ui, "MIDI out", &m.out_ports, &m.out_name) {
                m.select_out(pick);
            }
            if let Some(pick) = port_picker(ui, "MIDI in", &m.in_ports, &m.in_name) {
                m.select_in(pick);
            }
            if ui
                .button("⟳")
                .on_hover_text("Rescan MIDI devices")
                .clicked()
            {
                m.refresh();
            }
            let mut ext = s.external_sync.load(Relaxed);
            if ui.checkbox(&mut ext, "Follow MIDI clock").changed() {
                s.external_sync.store(ext, Relaxed);
            }
        });
    }

    /// The song arrangement: one pattern per bar.
    fn song_row(&mut self, ui: &mut egui::Ui, playing: bool) {
        let edit = self.shared.edit_pattern.load(Relaxed) as usize;
        let song_mode = self.shared.song_mode.load(Relaxed);
        let pos = self.shared.song_pos.load(Relaxed);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_sized(
                [62.0, 20.0],
                egui::Label::new(RichText::new("Song").strong()),
            )
            .on_hover_text(
                "Click a bar to place the selected pattern.\n\
                     Click it again or right-click to empty it.",
            );
            for bar in 0..SONG_SLOTS {
                let slot = self.project.song[bar].map(usize::from);
                let fill = match (slot.is_some(), bar % 4 == 0) {
                    (true, _) => Color32::from_rgb(70, 120, 70),
                    (false, true) => Color32::from_gray(60),
                    (false, false) => Color32::from_gray(45),
                };
                let cell = egui::Button::new(slot.map(pattern_name).unwrap_or_default())
                    .fill(fill)
                    .stroke(playhead(playing && song_mode && bar == pos))
                    .min_size(Vec2::new(20.0, 22.0));
                let resp = ui.add(cell);
                if resp.secondary_clicked() || (resp.clicked() && slot == Some(edit)) {
                    self.project.song[bar] = None;
                    self.dirty = true;
                } else if resp.clicked() {
                    self.project.song[bar] = Some(edit as u8);
                    self.dirty = true;
                }
            }
        });
    }

    fn pattern_row(&mut self, ui: &mut egui::Ui) {
        let edit = self.shared.edit_pattern.load(Relaxed) as usize;
        ui.horizontal(|ui| {
            ui.add_sized(
                [60.0, 20.0],
                egui::Label::new(RichText::new("Pattern").strong()),
            );
            for p in 0..PATTERNS {
                let button = egui::Button::new(pattern_name(p))
                    .selected(p == edit)
                    .min_size(Vec2::new(28.0, 22.0));
                if ui.add(button).on_hover_text("Edit this pattern").clicked() {
                    self.shared.edit_pattern.store(p as u8, Relaxed);
                }
            }
            ui.separator();
            let tracks = &mut self.project.tracks;
            if ui.button("Copy").clicked() {
                self.clipboard = Some(tracks.iter().map(|t| t.patterns[edit]).collect());
            }
            let paste = ui.add_enabled(self.clipboard.is_some(), egui::Button::new("Paste"));
            if paste.clicked() {
                if let Some(copied) = &self.clipboard {
                    for (track, steps) in tracks.iter_mut().zip(copied) {
                        track.patterns[edit] = *steps;
                    }
                    self.dirty = true;
                }
            }
            if ui.button("Clear").clicked() {
                for track in tracks.iter_mut() {
                    track.patterns[edit] = [None; STEPS];
                }
                self.dirty = true;
            }
        });
    }

    /// One channel-rack row: name, mute, and 16 pads.
    fn track_row(&mut self, ui: &mut egui::Ui, i: usize, pattern: usize, current: Option<usize>) {
        ui.horizontal(|ui| {
            let track = &mut self.project.tracks[i];
            let name = egui::Button::new(RichText::new(&track.name).small())
                .selected(self.selected == i)
                .min_size(Vec2::new(80.0, 26.0));
            let hint = "Click to select and hear it";
            if ui
                .add_sized([80.0, 26.0], name)
                .on_hover_text(hint)
                .clicked()
            {
                self.selected = i;
                let _ = self.events.try_send(Event::Note {
                    track: i,
                    semitone: 0,
                });
            }
            let mute = egui::Button::new(RichText::new("M").small())
                .selected(track.muted)
                .min_size(Vec2::new(22.0, 26.0));
            if ui.add(mute).on_hover_text("Mute").clicked() {
                track.muted = !track.muted;
                self.dirty = true;
            }
            let melodic = track.is_melodic();
            let on_color = if melodic { NOTE_ON } else { DRUM_ON };
            for step in 0..STEPS {
                let value = track.patterns[pattern][step];
                // Melodic pads show their note; click the track for the piano roll.
                let text = match value {
                    Some(TIE) if melodic => RichText::new("–").small(),
                    Some(semi) if melodic => RichText::new(NOTE_NAMES[semi as usize % 12]).small(),
                    _ => RichText::new(""),
                };
                let pad = egui::Button::new(text.color(Color32::BLACK))
                    .fill(step_fill(value.is_some(), on_color, step, false))
                    .stroke(playhead(current == Some(step)))
                    .min_size(Vec2::new(28.0, 26.0));
                if ui.add(pad).clicked() {
                    track.patterns[pattern][step] = if value.is_some() { None } else { Some(0) };
                    self.dirty = true;
                }
            }
        });
    }

    fn add_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.add_space(84.0);
            let full = self.project.tracks.len() >= MAX_TRACKS;
            if ui
                .add_enabled(!full, egui::Button::new("+ Synth"))
                .clicked()
            {
                let n = 1 + self
                    .project
                    .tracks
                    .iter()
                    .filter(|t| matches!(t.kind, Kind::Synth(_)))
                    .count();
                self.add_track(Track::synth(&format!("Synth {n}")));
            }
            let load = ui.add_enabled(!full, egui::Button::new("+ Sample…"));
            if load.on_hover_text("WAV, MP3, FLAC or OGG").clicked() {
                let picked = rfd::FileDialog::new()
                    .add_filter("Audio", &sample::EXTENSIONS)
                    .pick_files();
                for path in picked.into_iter().flatten() {
                    self.load_file(ctx, path);
                }
            }
            if self.loading > 0 {
                ui.spinner();
                ui.weak(format!("Loading {}…", self.loading));
            } else {
                ui.weak("or drop audio files onto the window");
            }
        });
    }

    fn piano_roll(&mut self, ui: &mut egui::Ui, pattern: usize, current: Option<usize>) {
        let i = self.selected;
        let track = &mut self.project.tracks[i];
        ui.horizontal(|ui| {
            ui.strong(format!("Piano roll · {}", track.name));
            ui.label("Octave");
            if ui
                .add(egui::DragValue::new(&mut track.octave).range(0..=7))
                .changed()
            {
                self.dirty = true;
            }
            ui.weak("Click: add/remove a note · right-click after a note: hold it longer");
        });
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        let steps = &mut track.patterns[pattern];
        // The note each step sounds, following ties back to where they start.
        let mut sounding = [None; STEPS];
        let mut note = None;
        for (step, value) in steps.iter().enumerate() {
            if *value != Some(TIE) {
                note = *value;
            }
            sounding[step] = note;
        }
        // Highest note on top, like a piano.
        for semitone in (0..12u8).rev() {
            let name = NOTE_NAMES[semitone as usize];
            let sharp = name.ends_with('#');
            ui.horizontal(|ui| {
                let key = egui::Button::new(name)
                    .fill(if sharp {
                        Color32::from_gray(25)
                    } else {
                        Color32::from_gray(90)
                    })
                    .min_size(Vec2::new(80.0, 17.0));
                if ui.add(key).clicked() {
                    let _ = self.events.try_send(Event::Note { track: i, semitone });
                }
                ui.add_space(26.0); // line up with the pads above
                for step in 0..STEPS {
                    let starts = steps[step] == Some(semitone);
                    let held = steps[step] == Some(TIE) && sounding[step] == Some(semitone);
                    let fill = match (starts, held) {
                        (true, _) => NOTE_ON,
                        (_, true) => NOTE_HELD,
                        _ => step_fill(false, NOTE_ON, step, sharp),
                    };
                    let cell = egui::Button::new("")
                        .fill(fill)
                        .stroke(playhead(current == Some(step)))
                        .min_size(Vec2::new(28.0, 17.0));
                    let resp = ui.add(cell);
                    if resp.clicked() {
                        if starts || held {
                            steps[step] = None;
                            // Drop the ties that held this note on.
                            for later in steps.iter_mut().skip(step + 1) {
                                if *later != Some(TIE) {
                                    break;
                                }
                                *later = None;
                            }
                        } else {
                            steps[step] = Some(semitone);
                            let _ = self.events.try_send(Event::Note { track: i, semitone });
                        }
                        self.dirty = true;
                    } else if resp.secondary_clicked()
                        && step > 0
                        && sounding[step - 1] == Some(semitone)
                    {
                        // Extend the note before through this step.
                        steps[step] = Some(TIE);
                        self.dirty = true;
                    }
                }
            });
        }
    }

    /// Settings for the selected track.
    fn instrument_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        let Some(track) = self.project.tracks.get_mut(self.selected) else {
            ui.weak("Add a track to get started.");
            return;
        };
        let mut changed = false;
        let name = egui::TextEdit::singleline(&mut track.name)
            .font(egui::TextStyle::Heading)
            .desired_width(200.0);
        changed |= ui.add(name).changed();
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            // Shown 1-16 like on hardware, stored 0-15; 0 means off.
            let mut ch = track.midi_channel.map_or(0, |c| c + 1);
            ui.label("MIDI ch");
            let drag = egui::DragValue::new(&mut ch)
                .range(0..=16)
                .custom_formatter(|v, _| {
                    if v == 0.0 {
                        "Off".into()
                    } else {
                        format!("{v}")
                    }
                });
            if ui
                .add(drag)
                .on_hover_text("Send this track's notes to MIDI gear")
                .changed()
            {
                track.midi_channel = (ch > 0).then(|| ch - 1);
                changed = true;
            }
            if !track.is_melodic() {
                ui.label("note");
                let drag = egui::DragValue::new(&mut track.midi_note).range(0..=127);
                changed |= ui
                    .add(drag)
                    .on_hover_text("MIDI note this track sends")
                    .changed();
            }
        });
        ui.add_space(8.0);

        match &mut track.kind {
            Kind::Synth(values) => {
                ui.weak("Drag knobs up/down. Double-click to reset.");
                ui.add_space(4.0);
                // `None` is the track volume, which every track has.
                let sections: [(&str, &[Option<usize>]); 3] = [
                    ("Sound", &[Some(synth::WAVE), Some(synth::DETUNE), None]),
                    (
                        "Filter",
                        &[
                            Some(synth::CUTOFF),
                            Some(synth::RESONANCE),
                            Some(synth::ENV_AMOUNT),
                            Some(synth::FILTER_DECAY),
                        ],
                    ),
                    (
                        "Envelope (ADSR)",
                        &[
                            Some(synth::ATTACK),
                            Some(synth::DECAY),
                            Some(synth::SUSTAIN),
                            Some(synth::RELEASE),
                        ],
                    ),
                ];
                for (title, row) in sections {
                    ui.label(RichText::new(title).small().strong());
                    ui.horizontal(|ui| {
                        for &slot in row {
                            changed |= match slot {
                                Some(i) => knob(ui, &PARAMS[i], &mut values[i]),
                                None => knob(ui, &TRACK_VOLUME, &mut track.volume),
                            };
                        }
                    });
                    ui.add_space(6.0);
                }
            }
            Kind::Sampler { sample, pitched } => {
                ui.label(format!("Sample: {}", sample.name));
                ui.weak(format!("{:.2} s, {} Hz", sample.seconds(), sample.rate));
                ui.add_space(6.0);
                let hint = "Off: plays as recorded, like a drum.\n\
                            On: plays at the pitch of each note, like an instrument (C4 = as recorded).";
                changed |= ui
                    .checkbox(pitched, "Pitched")
                    .on_hover_text(hint)
                    .changed();
                ui.add_space(6.0);
                changed |= knob(ui, &TRACK_VOLUME, &mut track.volume);
            }
        }
        if changed {
            self.dirty = true;
        }

        ui.add_space(12.0);
        if ui.button("Play a note").clicked() {
            self.play_note(self.selected, 0);
        }
        ui.add_space(12.0);
        if ui
            .button(RichText::new("Delete track").color(Color32::LIGHT_RED))
            .clicked()
        {
            self.project.tracks.remove(self.selected);
            self.selected = self
                .selected
                .min(self.project.tracks.len().saturating_sub(1));
            self.dirty = true;
        }
    }
}

fn playhead(on: bool) -> Stroke {
    if on {
        Stroke::new(2.0_f32, Color32::WHITE)
    } else {
        Stroke::NONE
    }
}

fn step_fill(on: bool, on_color: Color32, step: usize, dark: bool) -> Color32 {
    // Shade every other group of four so beats are easy to see.
    let light_group = (step / 4).is_multiple_of(2);
    match (on, light_group, dark) {
        (true, _, _) => on_color,
        (false, true, false) => Color32::from_gray(60),
        (false, false, false) => Color32::from_gray(45),
        (false, true, true) => Color32::from_gray(50),
        (false, false, true) => Color32::from_gray(37),
    }
}

/// A dropdown of MIDI ports. Returns `Some(choice)` when the user picks one,
/// where the choice is `None` for "Off".
fn port_picker(
    ui: &mut egui::Ui,
    label: &str,
    ports: &[String],
    current: &Option<String>,
) -> Option<Option<String>> {
    let mut picked = None;
    ui.label(label);
    egui::ComboBox::from_id_salt(label)
        .selected_text(current.as_deref().unwrap_or("Off"))
        .width(140.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), "Off").clicked() {
                picked = Some(None);
            }
            for name in ports {
                if ui
                    .selectable_label(current.as_ref() == Some(name), name)
                    .clicked()
                {
                    picked = Some(Some(name.clone()));
                }
            }
        });
    picked
}

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

fn to_unit(p: &Param, v: f32) -> f32 {
    if p.log {
        (v / p.min).ln() / (p.max / p.min).ln()
    } else {
        (v - p.min) / (p.max - p.min)
    }
}

fn from_unit(p: &Param, t: f32) -> f32 {
    if p.log {
        p.min * (p.max / p.min).powf(t)
    } else {
        p.min + (p.max - p.min) * t
    }
}

fn format_value(p: &Param, v: f32) -> String {
    match p.unit {
        Unit::Percent => format!("{:.0}%", v * 100.0),
        Unit::Hz if v >= 1000.0 => format!("{:.1} kHz", v / 1000.0),
        Unit::Hz => format!("{v:.0} Hz"),
        Unit::Seconds if v < 1.0 => format!("{:.0} ms", v * 1000.0),
        Unit::Seconds => format!("{v:.2} s"),
    }
}

fn hint(p: &Param) -> &'static str {
    match p.name {
        "Wave" => "0% = saw (buzzy), 100% = square (hollow)",
        "Detune" => "Pitch difference between the two oscillators; higher = thicker",
        "Cutoff" => "Filter brightness when the envelope is closed",
        "Reso" => "Filter peak; higher = squelchier",
        "Env amt" => "How far each note opens the filter",
        "F decay" => "How fast the filter closes after a note starts",
        "Attack" => "Fade-in time of each note",
        "Decay" => "Time to fall from full volume to the sustain level",
        "Sustain" => "Volume while a note is held",
        "Release" => "Fade-out time after a note ends",
        "Volume" => "Track level",
        _ => "",
    }
}

/// A rotary knob: drag up/down to turn, double-click to reset.
/// Returns true when the value changed.
fn knob(ui: &mut egui::Ui, p: &Param, value: &mut f32) -> bool {
    let mut changed = false;
    let size = Vec2::new(60.0, 74.0);
    ui.allocate_ui_with_layout(size, Layout::top_down(Align::Center), |ui| {
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::click_and_drag());
        let resp = resp.on_hover_text(hint(p));
        let mut t = to_unit(p, *value).clamp(0.0, 1.0);
        if resp.dragged() {
            t = (t - resp.drag_delta().y * 0.006).clamp(0.0, 1.0);
            *value = from_unit(p, t);
            changed = true;
        }
        if resp.double_clicked() {
            *value = p.default;
            t = to_unit(p, *value);
            changed = true;
        }

        // The arc sweeps 270°, from bottom-left clockwise to bottom-right.
        let painter = ui.painter();
        let center = rect.center();
        let radius = rect.width() / 2.0 - 3.0;
        let start = 0.75 * PI;
        let point = |a: f32, r: f32| Pos2::new(center.x + r * a.cos(), center.y + r * a.sin());
        let body = Color32::from_gray(if resp.hovered() { 70 } else { 55 });
        painter.circle_filled(center, radius, body);
        let arc: Vec<Pos2> = (0..=24)
            .map(|i| point(start + 1.5 * PI * t * i as f32 / 24.0, radius + 1.5))
            .collect();
        painter.add(egui::Shape::line(arc, Stroke::new(3.0_f32, DRUM_ON)));
        let angle = start + 1.5 * PI * t;
        let tip = point(angle, radius - 2.0);
        painter.line_segment([center, tip], Stroke::new(2.0_f32, Color32::WHITE));

        ui.label(RichText::new(p.name).small());
        ui.label(RichText::new(format_value(p, *value)).small().weak());
    });
    changed
}
