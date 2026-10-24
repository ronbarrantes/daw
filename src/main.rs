mod audio;
mod drums;
mod help;
mod midi;
mod project;
mod sample;
mod synth;
mod widgets;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use eframe::egui::{self, Color32, RichText, Sense, Vec2};

use audio::{Event, Shared};
use midi::Midi;
use project::{
    Kind, Project, Row, Sample, Step, Steps, Track, ACCENT, GHOST, MAX_RATCHET, MAX_TRACKS, NORMAL,
    PATTERNS, SONG_SLOTS, STEPS,
};
use synth::{Param, Unit, PARAMS};
use widgets::{
    disclosure_arrow, empty_fill, knob, lit_fill, playhead, port_picker, DRUM_ON, NOTE_HELD,
    NOTE_NAMES, NOTE_ON,
};

const RECORDING: Color32 = Color32::from_rgb(220, 60, 60);
const SOLO: Color32 = Color32::from_rgb(230, 200, 60);
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

const GROUP_VOLUME: Param = Param {
    name: "Volume",
    min: 0.0,
    max: 1.0,
    default: 1.0,
    log: false,
    unit: Unit::Percent,
};

const HUMANIZE: Param = Param {
    name: "Humanize",
    min: 0.0,
    max: 1.0,
    default: 0.0,
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
        viewport: egui::ViewportBuilder::default().with_inner_size([1080.0, 700.0]),
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
        show_help: false,
        show_about: false,
        confirm: None,
        picked: Vec::new(),
        selected_group: None,
        view: View::ALL,
    };
    eframe::run_native("daw", options, Box::new(|_cc| Ok(Box::new(app))))
}

/// Which parts of the window are showing.
#[derive(Clone, Copy, PartialEq)]
struct View {
    song: bool,
    tracks: bool,
    piano: bool,
    panel: bool,
}

impl View {
    const ALL: View = View {
        song: true,
        tracks: true,
        piano: true,
        panel: true,
    };
}

/// Something to do to a track, chosen from its right-click menu.
enum TrackAction {
    Group,
    MoveTo(u32),
    Ungroup,
    Delete,
}

/// A "replace the song?" question waiting for an answer.
#[derive(Clone, Copy)]
enum StartOver {
    Empty,
    Demo,
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
    show_help: bool,
    show_about: bool,
    confirm: Option<StartOver>,
    /// Extra tracks picked with Ctrl/Cmd-click, for grouping.
    picked: Vec<usize>,
    /// A group shown in the side panel instead of a track.
    selected_group: Option<u32>,
    view: View,
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

/// Shift-click: a ghost note on an empty or normal step, normal on a ghost.
fn toggle_ghost(step: &mut Option<Step>, note: u8) {
    *step = Some(match *step {
        Some(s) if s.velocity == GHOST => Step {
            velocity: NORMAL,
            ..s
        },
        Some(s) => Step {
            velocity: GHOST,
            ..s
        },
        None => Step {
            velocity: GHOST,
            ..Step::note(note)
        },
    });
}

/// The right-click menu for one step. Returns true if anything changed.
fn step_menu(ui: &mut egui::Ui, step: &mut Step) -> bool {
    let mut changed = false;
    ui.set_min_width(200.0);
    ui.horizontal(|ui| {
        for (label, velocity) in [("Ghost", GHOST), ("Normal", NORMAL), ("Accent", ACCENT)] {
            if ui
                .selectable_label(step.velocity == velocity, label)
                .clicked()
            {
                step.velocity = velocity;
                changed = true;
            }
        }
    });
    let mut row = |ui: &mut egui::Ui, label: &str, hint: &str, widget: egui::Slider| {
        ui.label(label).on_hover_text(hint);
        changed |= ui.add(widget).changed();
    };
    egui::Grid::new("step").num_columns(2).show(ui, |ui| {
        row(
            ui,
            "Velocity",
            "How loud: low = ghost note, 127 = accent",
            egui::Slider::new(&mut step.velocity, 1..=127),
        );
        ui.end_row();
        row(
            ui,
            "Chance",
            "How often this step plays",
            egui::Slider::new(&mut step.chance, 1..=100).suffix("%"),
        );
        ui.end_row();
        row(
            ui,
            "Ratchet",
            "Repeat within the step, like a roll",
            egui::Slider::new(&mut step.ratchet, 1..=MAX_RATCHET).suffix("×"),
        );
        ui.end_row();
        row(
            ui,
            "Nudge",
            "Play a little early (−) or late (+), in % of a step",
            egui::Slider::new(&mut step.nudge, -50..=50).suffix("%"),
        );
        ui.end_row();
    });
    if ui.button("Reset").clicked() {
        *step = Step::note(step.note);
        changed = true;
    }
    changed
}

/// Small markers shown on a step: ratchet count, chance, nudge.
fn step_marks(s: &Step) -> String {
    let mut marks = String::new();
    if s.ratchet > 1 {
        marks.push_str(&format!("×{}", s.ratchet));
    }
    if s.chance < 100 {
        marks.push('?');
    }
    if s.nudge < 0 {
        marks.push('‹');
    } else if s.nudge > 0 {
        marks.push('›');
    }
    marks
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let playing = self.shared.playing.load(Relaxed);
        if !ctx.wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                self.shared.playing.store(!playing, Relaxed);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::F1)) {
                self.show_help = !self.show_help;
            }
            self.view_keys(ctx);
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::G)) {
                self.group_picked();
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

        egui::TopBottomPanel::top("menu").show(ctx, |ui| self.menu_bar(ui, ctx));

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

        if self.view.panel {
            egui::SidePanel::right("instrument")
                .resizable(false)
                .exact_width(290.0)
                .show(ctx, |ui| match self.selected_group {
                    Some(id) => self.group_panel(ui, id),
                    None => self.instrument_panel(ui),
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if self.view.song {
                    self.song_row(ui, playing);
                    ui.add_space(6.0);
                    self.pattern_row(ui);
                    ui.add_space(6.0);
                }
                let edit = self.shared.edit_pattern.load(Relaxed) as usize;
                // Only show the playhead on the pattern that's actually playing.
                let current = (playing && self.shared.play_pattern.load(Relaxed) as usize == edit)
                    .then(|| self.shared.current_step.load(Relaxed));
                if self.view.tracks {
                    self.track_list(ui, edit, current);
                    ui.add_space(4.0);
                    self.add_row(ui, ctx);
                }
                let melodic = self
                    .project
                    .tracks
                    .get(self.selected)
                    .is_some_and(Track::is_melodic);
                if self.view.piano && melodic {
                    ui.add_space(8.0);
                    self.piano_roll(ui, edit, current);
                }
                if !self.view.song && !self.view.tracks && !(self.view.piano && melodic) {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.weak(
                            "Nothing to show here. Use the View menu, or F9 to show everything.",
                        );
                    });
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

        help::guide(ctx, &mut self.show_help);
        help::about(ctx, &mut self.show_about);
        self.confirm_dialog(ctx);

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

    fn edit_pattern(&self) -> usize {
        self.shared.edit_pattern.load(Relaxed) as usize
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

    fn add_synth(&mut self) {
        let n = 1 + self
            .project
            .tracks
            .iter()
            .filter(|t| matches!(t.kind, Kind::Synth(_)))
            .count();
        self.add_track(Track::synth(&format!("Synth {n}")));
    }

    fn delete_selected(&mut self) {
        self.delete_track(self.selected);
    }

    fn delete_track(&mut self, i: usize) {
        if i < self.project.tracks.len() {
            self.project.tracks.remove(i);
            self.project.prune_groups();
            self.selected = self
                .selected
                .min(self.project.tracks.len().saturating_sub(1));
            self.picked.clear();
            self.selected_group = None;
            self.dirty = true;
        }
    }

    /// Ctrl+G: puts the picked tracks (or just the selected one) in a new group.
    fn group_picked(&mut self) {
        let mut tracks = self.picked.clone();
        if !tracks.contains(&self.selected) {
            tracks.push(self.selected);
        }
        tracks.retain(|&i| i < self.project.tracks.len());
        if tracks.is_empty() {
            return;
        }
        let name = format!("Group {}", self.project.groups.len() + 1);
        let id = self.project.new_group(&tracks, &name);
        self.selected_group = Some(id);
        self.picked.clear();
        self.dirty = true;
    }

    /// F5-F8 show or hide each part of the window; F9 shows everything.
    fn view_keys(&mut self, ctx: &egui::Context) {
        use egui::Key::*;
        let v = &mut self.view;
        ctx.input(|i| {
            for (key, shown) in [
                (F5, &mut v.song),
                (F6, &mut v.tracks),
                (F7, &mut v.piano),
                (F8, &mut v.panel),
            ] {
                if i.key_pressed(key) {
                    *shown = !*shown;
                }
            }
            if i.key_pressed(F9) {
                *v = View::ALL;
            }
        });
    }

    /// Opens the file picker and loads the chosen audio files.
    fn pick_samples(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .add_filter("Audio", &sample::EXTENSIONS)
            .pick_files();
        for path in picked.into_iter().flatten() {
            self.load_file(ctx, path);
        }
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

    fn copy_pattern(&mut self) {
        let edit = self.edit_pattern();
        self.clipboard = Some(
            self.project
                .tracks
                .iter()
                .map(|t| t.patterns[edit])
                .collect(),
        );
    }

    fn paste_pattern(&mut self) {
        let edit = self.edit_pattern();
        if let Some(copied) = &self.clipboard {
            for (track, steps) in self.project.tracks.iter_mut().zip(copied) {
                track.patterns[edit] = *steps;
            }
            self.dirty = true;
        }
    }

    fn clear_pattern(&mut self) {
        let edit = self.edit_pattern();
        for track in &mut self.project.tracks {
            track.patterns[edit] = [None; STEPS];
        }
        self.dirty = true;
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New empty song").clicked() {
                    self.confirm = Some(StartOver::Empty);
                    ui.close_menu();
                }
                if ui.button("Load demo song").clicked() {
                    self.confirm = Some(StartOver::Demo);
                    ui.close_menu();
                }
                ui.add_enabled(false, egui::Button::new("Save (coming soon)"));
                ui.separator();
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.button("Copy pattern").clicked() {
                    self.copy_pattern();
                    ui.close_menu();
                }
                let paste = egui::Button::new("Paste pattern");
                if ui.add_enabled(self.clipboard.is_some(), paste).clicked() {
                    self.paste_pattern();
                    ui.close_menu();
                }
                if ui.button("Clear pattern").clicked() {
                    self.clear_pattern();
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Add synth track").clicked() {
                    self.add_synth();
                    ui.close_menu();
                }
                if ui.button("Add sample track…").clicked() {
                    ui.close_menu();
                    self.pick_samples(ctx);
                }
                let has_track = self.selected < self.project.tracks.len();
                ui.separator();
                let group = egui::Button::new("Group selected tracks  (Ctrl+G)");
                if ui.add_enabled(has_track, group).clicked() {
                    self.group_picked();
                    ui.close_menu();
                }
                ui.weak("Ctrl/Cmd-click track names to pick several.");
                ui.separator();
                let delete = egui::Button::new("Delete selected track");
                if ui.add_enabled(has_track, delete).clicked() {
                    self.delete_selected();
                    ui.close_menu();
                }
            });
            ui.menu_button("View", |ui| {
                let v = &mut self.view;
                ui.checkbox(&mut v.song, "Song and patterns  (F5)");
                ui.checkbox(&mut v.tracks, "Tracks  (F6)");
                ui.checkbox(&mut v.piano, "Piano roll  (F7)");
                ui.checkbox(&mut v.panel, "Instrument panel  (F8)");
                ui.separator();
                ui.label(RichText::new("Focus on one thing").small().weak());
                if ui.button("Tracks only").clicked() {
                    *v = View {
                        song: false,
                        tracks: true,
                        piano: false,
                        panel: false,
                    };
                    ui.close_menu();
                }
                if ui.button("Piano roll and sound").clicked() {
                    *v = View {
                        song: false,
                        tracks: false,
                        piano: true,
                        panel: true,
                    };
                    ui.close_menu();
                }
                if ui.button("Show everything  (F9)").clicked() {
                    *v = View::ALL;
                    ui.close_menu();
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("How to use (F1)").clicked() {
                    self.show_help = true;
                    ui.close_menu();
                }
                if ui.button("About").clicked() {
                    self.show_about = true;
                    ui.close_menu();
                }
            });
        });
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.confirm else { return };
        egui::Window::new("Start over?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("This replaces the current song. Saving isn't built yet,");
                ui.label("so the current song will be lost.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Start over").clicked() {
                        self.shared.playing.store(false, Relaxed);
                        self.project = match kind {
                            StartOver::Empty => Project::empty(),
                            StartOver::Demo => Project::demo(),
                        };
                        self.selected = 0;
                        self.shared.edit_pattern.store(0, Relaxed);
                        self.dirty = true;
                        self.confirm = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm = None;
                    }
                });
            });
    }

    fn transport_row(&mut self, ui: &mut egui::Ui, playing: bool) {
        let s = self.shared.clone();
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
            let mut swing = self.project.swing * 100.0;
            ui.label("Swing");
            let drag = egui::DragValue::new(&mut swing)
                .range(0.0..=100.0)
                .speed(0.5)
                .suffix("%");
            let hint = "Delays every other 16th note for a shuffled feel.\n66% is a triplet swing.";
            if ui.add(drag).on_hover_text(hint).changed() {
                self.project.swing = swing / 100.0;
                self.dirty = true;
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
            let rec_color = if self.recording {
                RECORDING
            } else {
                Color32::GRAY
            };
            let rec =
                egui::Button::new(RichText::new("⏺ Rec").color(rec_color)).selected(self.recording);
            let rec_hint = "While playing, notes you play on the keyboard\n\
                            are written into the selected track";
            if ui.add(rec).on_hover_text(rec_hint).clicked() {
                self.recording = !self.recording;
            }
            ui.separator();
            ui.weak("Space = play · A–K = notes · Z/X = octave · F1 = help");
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
                    // Skip shortcuts like Ctrl+G.
                    egui::Event::Key {
                        key,
                        pressed,
                        repeat: false,
                        modifiers,
                        ..
                    } if !(modifiers.command || modifiers.ctrl || modifiers.alt) => {
                        Some((*key, *pressed))
                    }
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
                            track.patterns[pattern][step] = Some(Step::note(value));
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
        let edit = self.edit_pattern();
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
        let edit = self.edit_pattern();
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
            if ui.button("Copy").clicked() {
                self.copy_pattern();
            }
            let paste = ui.add_enabled(self.clipboard.is_some(), egui::Button::new("Paste"));
            if paste.clicked() {
                self.paste_pattern();
            }
            if ui.button("Clear").clicked() {
                self.clear_pattern();
            }
        });
    }

    /// The track list: group headers, and tracks unless their group is collapsed.
    fn track_list(&mut self, ui: &mut egui::Ui, pattern: usize, current: Option<usize>) {
        for row in self.project.rows() {
            match row {
                Row::Group(g) => self.group_row(ui, g, pattern),
                Row::Track(i) => {
                    let track = &self.project.tracks[i];
                    let group = track.group.and_then(|id| self.project.group(id));
                    if !group.is_some_and(|g| g.collapsed) {
                        self.track_row(ui, i, pattern, current);
                    }
                }
            }
        }
    }

    /// A group header: expand arrow, name, mute, solo, and every hit in the
    /// group combined.
    fn group_row(&mut self, ui: &mut egui::Ui, g: usize, pattern: usize) {
        let id = self.project.groups[g].id;
        let mut lit = [false; STEPS];
        for t in self.project.tracks.iter().filter(|t| t.group == Some(id)) {
            for (step, value) in t.patterns[pattern].iter().enumerate() {
                lit[step] |= value.is_some_and(|s| !s.is_tie());
            }
        }
        let mut ungroup = false;
        ui.horizontal(|ui| {
            let group = &mut self.project.groups[g];
            let hint = if group.collapsed {
                "Expand"
            } else {
                "Collapse"
            };
            let resp = disclosure_arrow(ui, group.collapsed).on_hover_text(hint);
            if resp.clicked() {
                group.collapsed = !group.collapsed;
                self.dirty = true;
            }
            let name = egui::Button::new(RichText::new(&group.name).small().strong())
                .selected(self.selected_group == Some(id));
            let resp = ui
                .add_sized([54.0, 26.0], name)
                .on_hover_text("Click for group settings · right-click to ungroup");
            if resp.clicked() {
                self.selected_group = Some(id);
            }
            resp.context_menu(|ui| {
                if ui.button("Ungroup (keep the tracks)").clicked() {
                    ungroup = true;
                    ui.close_menu();
                }
            });
            let mute = egui::Button::new(RichText::new("M").small())
                .selected(group.muted)
                .min_size(Vec2::new(22.0, 26.0));
            if ui.add(mute).on_hover_text("Mute the whole group").clicked() {
                group.muted = !group.muted;
                self.dirty = true;
            }
            let solo_color = if group.solo { SOLO } else { Color32::GRAY };
            let solo = egui::Button::new(RichText::new("S").small().color(solo_color))
                .selected(group.solo)
                .min_size(Vec2::new(22.0, 26.0));
            if ui.add(solo).on_hover_text("Solo the whole group").clicked() {
                group.solo = !group.solo;
                self.dirty = true;
            }
            // A read-only summary of the group's hits; click to expand.
            for (step, on) in lit.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(28.0, 26.0), Sense::click());
                let fill = if *on {
                    DRUM_ON.gamma_multiply(0.55)
                } else {
                    empty_fill(step, true)
                };
                ui.painter()
                    .rect_filled(rect.shrink2(Vec2::new(0.0, 6.0)), 2.0, fill);
                if resp.clicked() && group.collapsed {
                    group.collapsed = false;
                    self.dirty = true;
                }
            }
        });
        if ungroup {
            self.project.ungroup(id);
            self.selected_group = None;
            self.dirty = true;
        }
    }

    /// One channel-rack row: name, mute, solo, and 16 steps.
    fn track_row(&mut self, ui: &mut egui::Ui, i: usize, pattern: usize, current: Option<usize>) {
        let mut action = None;
        ui.horizontal(|ui| {
            let groups: Vec<(u32, String)> = self
                .project
                .groups
                .iter()
                .map(|g| (g.id, g.name.clone()))
                .collect();
            let track = &mut self.project.tracks[i];
            // Tracks in a group are indented under its header.
            let width = if track.group.is_some() {
                ui.add_space(12.0);
                68.0
            } else {
                80.0
            };
            let highlighted =
                (self.selected == i && self.selected_group.is_none()) || self.picked.contains(&i);
            let name = egui::Button::new(RichText::new(&track.name).small()).selected(highlighted);
            let hint = "Click to select and hear it · Ctrl/Cmd-click to pick several · right-click for more";
            let resp = ui.add_sized([width, 26.0], name).on_hover_text(hint);
            if resp.clicked() {
                if ui.input(|i| i.modifiers.command) {
                    if let Some(at) = self.picked.iter().position(|&p| p == i) {
                        self.picked.remove(at);
                    } else {
                        self.picked.push(i);
                    }
                } else {
                    self.selected = i;
                    self.selected_group = None;
                    self.picked.clear();
                    let _ = self.events.try_send(Event::Note {
                        track: i,
                        semitone: 0,
                    });
                }
            }
            resp.context_menu(|ui| {
                if ui.button("Group with picked tracks  (Ctrl+G)").clicked() {
                    action = Some(TrackAction::Group);
                    ui.close_menu();
                }
                if !groups.is_empty() {
                    ui.menu_button("Move to group", |ui| {
                        for (id, name) in &groups {
                            if ui.button(name).clicked() {
                                action = Some(TrackAction::MoveTo(*id));
                                ui.close_menu();
                            }
                        }
                    });
                }
                if track.group.is_some() && ui.button("Remove from group").clicked() {
                    action = Some(TrackAction::Ungroup);
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Delete track").clicked() {
                    action = Some(TrackAction::Delete);
                    ui.close_menu();
                }
            });
            let mute = egui::Button::new(RichText::new("M").small())
                .selected(track.muted)
                .min_size(Vec2::new(22.0, 26.0));
            if ui.add(mute).on_hover_text("Mute").clicked() {
                track.muted = !track.muted;
                self.dirty = true;
            }
            let solo_color = if track.solo { SOLO } else { Color32::GRAY };
            let solo = egui::Button::new(RichText::new("S").small().color(solo_color))
                .selected(track.solo)
                .min_size(Vec2::new(22.0, 26.0));
            let solo_hint = "Solo: hear only soloed tracks";
            if ui.add(solo).on_hover_text(solo_hint).clicked() {
                track.solo = !track.solo;
                self.dirty = true;
            }
            let melodic = track.is_melodic();
            let on_color = if melodic { NOTE_ON } else { DRUM_ON };
            for step in 0..STEPS {
                let value = track.patterns[pattern][step];
                // Melodic steps show their note; groove settings show as marks.
                let text = match value {
                    Some(s) if s.is_tie() => "–".to_string(),
                    Some(s) if melodic => {
                        format!("{}{}", NOTE_NAMES[s.note as usize % 12], step_marks(&s))
                    }
                    Some(s) => step_marks(&s),
                    None => String::new(),
                };
                let fill = match value {
                    Some(s) if s.is_tie() => NOTE_HELD,
                    Some(s) => lit_fill(on_color, s.velocity),
                    None => empty_fill(step, false),
                };
                let pad = egui::Button::new(RichText::new(text).small().color(Color32::BLACK))
                    .fill(fill)
                    .stroke(playhead(current == Some(step)))
                    .min_size(Vec2::new(28.0, 26.0));
                let resp = ui.add(pad);
                let cell = &mut track.patterns[pattern][step];
                if resp.clicked() {
                    if ui.input(|i| i.modifiers.shift) {
                        toggle_ghost(cell, 0);
                    } else {
                        *cell = if cell.is_some() {
                            None
                        } else {
                            Some(Step::note(0))
                        };
                    }
                    self.dirty = true;
                }
                if let Some(s) = cell.as_mut().filter(|s| !s.is_tie()) {
                    resp.context_menu(|ui| self.dirty |= step_menu(ui, s));
                }
            }
        });
        match action {
            Some(TrackAction::Group) => {
                if !self.picked.contains(&i) {
                    self.picked.push(i);
                }
                self.group_picked();
            }
            Some(TrackAction::MoveTo(id)) => {
                self.project.tracks[i].group = Some(id);
                self.project.prune_groups();
                self.dirty = true;
            }
            Some(TrackAction::Ungroup) => {
                self.project.tracks[i].group = None;
                self.project.prune_groups();
                self.dirty = true;
            }
            Some(TrackAction::Delete) => self.delete_track(i),
            None => {}
        }
    }

    fn add_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.add_space(84.0);
            let full = self.project.tracks.len() >= MAX_TRACKS;
            if ui
                .add_enabled(!full, egui::Button::new("+ Synth"))
                .clicked()
            {
                self.add_synth();
            }
            let load = ui.add_enabled(!full, egui::Button::new("+ Sample…"));
            if load.on_hover_text("WAV, MP3, FLAC or OGG").clicked() {
                self.pick_samples(ctx);
            }
            if self.loading > 0 {
                ui.spinner();
                ui.weak(format!("Loading {}…", self.loading));
            } else {
                ui.weak("or drop audio files onto the window · right-click a step for groove");
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
            ui.weak("Click: add/remove · right-click after a note: hold it · right-click a note: groove");
        });
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        let steps = &mut track.patterns[pattern];
        // The note each step sounds, following ties back to where they start.
        let mut sounding = [None; STEPS];
        let mut note = None;
        for (step, value) in steps.iter().enumerate() {
            match value {
                Some(s) if s.is_tie() => {}
                Some(s) => note = Some(s.note),
                None => note = None,
            }
            sounding[step] = note;
        }
        // Highest note on top, like a piano.
        for semitone in (0..12u8).rev() {
            let name = NOTE_NAMES[semitone as usize];
            let sharp = name.ends_with('#');
            ui.horizontal(|ui| {
                let key_fill = if sharp {
                    Color32::from_gray(25)
                } else {
                    Color32::from_gray(90)
                };
                let key = egui::Button::new(name)
                    .fill(key_fill)
                    .min_size(Vec2::new(80.0, 17.0));
                if ui.add(key).clicked() {
                    let _ = self.events.try_send(Event::Note { track: i, semitone });
                }
                ui.add_space(52.0); // line up with the steps above
                for step in 0..STEPS {
                    let value = steps[step];
                    let starts = value.is_some_and(|s| !s.is_tie() && s.note == semitone);
                    let held =
                        value.is_some_and(|s| s.is_tie()) && sounding[step] == Some(semitone);
                    let fill = match value {
                        Some(s) if starts => lit_fill(NOTE_ON, s.velocity),
                        _ if held => NOTE_HELD,
                        _ => empty_fill(step, sharp),
                    };
                    let text = match value {
                        Some(s) if starts => step_marks(&s),
                        _ => String::new(),
                    };
                    let cell = egui::Button::new(RichText::new(text).small().color(Color32::BLACK))
                        .fill(fill)
                        .stroke(playhead(current == Some(step)))
                        .min_size(Vec2::new(28.0, 17.0));
                    let resp = ui.add(cell);
                    if resp.clicked() {
                        if ui.input(|i| i.modifiers.shift) {
                            if !starts {
                                steps[step] = None;
                            }
                            toggle_ghost(&mut steps[step], semitone);
                        } else if starts || held {
                            steps[step] = None;
                            // Drop the ties that held this note on.
                            for later in steps.iter_mut().skip(step + 1) {
                                if !later.is_some_and(|s| s.is_tie()) {
                                    break;
                                }
                                *later = None;
                            }
                        } else {
                            steps[step] = Some(Step::note(semitone));
                            let _ = self.events.try_send(Event::Note { track: i, semitone });
                        }
                        self.dirty = true;
                    } else if starts {
                        if let Some(s) = steps[step].as_mut() {
                            resp.context_menu(|ui| self.dirty |= step_menu(ui, s));
                        }
                    } else if resp.secondary_clicked()
                        && step > 0
                        && sounding[step - 1] == Some(semitone)
                    {
                        // Extend the note before through this step.
                        steps[step] = Some(Step::tie());
                        self.dirty = true;
                    }
                }
            });
        }
    }

    /// Settings for the selected group.
    fn group_panel(&mut self, ui: &mut egui::Ui, id: u32) {
        ui.add_space(4.0);
        let members: Vec<(usize, String)> = self
            .project
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.group == Some(id))
            .map(|(i, t)| (i, t.name.clone()))
            .collect();
        let Some(group) = self.project.group_mut(id) else {
            self.selected_group = None;
            return;
        };
        let mut changed = false;
        let name = egui::TextEdit::singleline(&mut group.name)
            .font(egui::TextStyle::Heading)
            .desired_width(200.0);
        changed |= ui.add(name).changed();
        ui.weak("Group");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            changed |= knob(ui, &GROUP_VOLUME, &mut group.volume);
            ui.vertical(|ui| {
                changed |= ui.checkbox(&mut group.muted, "Mute").changed();
                changed |= ui.checkbox(&mut group.solo, "Solo").changed();
                changed |= ui.checkbox(&mut group.collapsed, "Collapsed").changed();
            });
        });
        ui.add_space(8.0);
        ui.label(RichText::new("Tracks").small().strong());
        for (i, name) in members {
            if ui.button(&name).clicked() {
                self.selected = i;
                self.selected_group = None;
            }
        }
        ui.add_space(12.0);
        if ui.button("Ungroup (keep the tracks)").clicked() {
            self.project.ungroup(id);
            self.selected_group = None;
            changed = true;
        }
        if changed {
            self.dirty = true;
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
                ui.label(RichText::new("Sound").small().strong());
                ui.horizontal(|ui| {
                    for i in [synth::WAVE, synth::DETUNE] {
                        changed |= knob(ui, &PARAMS[i], &mut values[i]);
                    }
                    changed |= knob(ui, &TRACK_VOLUME, &mut track.volume);
                    changed |= knob(ui, &HUMANIZE, &mut track.humanize);
                });
                let sections: [(&str, [usize; 4]); 2] = [
                    (
                        "Filter",
                        [
                            synth::CUTOFF,
                            synth::RESONANCE,
                            synth::ENV_AMOUNT,
                            synth::FILTER_DECAY,
                        ],
                    ),
                    (
                        "Envelope (ADSR)",
                        [synth::ATTACK, synth::DECAY, synth::SUSTAIN, synth::RELEASE],
                    ),
                ];
                for (title, row) in sections {
                    ui.add_space(6.0);
                    ui.label(RichText::new(title).small().strong());
                    ui.horizontal(|ui| {
                        for i in row {
                            changed |= knob(ui, &PARAMS[i], &mut values[i]);
                        }
                    });
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
                ui.horizontal(|ui| {
                    changed |= knob(ui, &TRACK_VOLUME, &mut track.volume);
                    changed |= knob(ui, &HUMANIZE, &mut track.humanize);
                });
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
            self.delete_selected();
        }
    }
}
