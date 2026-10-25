mod audio;
mod drums;
mod export;
mod file;
mod fx;
mod help;
mod midi;
mod project;
mod sample;
mod synth;
mod undo;
mod web;
mod widgets;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender};
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
use std::time::Duration;

use eframe::egui::{self, Color32, RichText, Sense, Vec2};

use audio::{Event, Shared};
use midi::{Midi, Played, Route};
use project::{
    Effects, Grid, Kind, Project, Row, Sample, Step, Track, ACCENT, EMPTY_GRID, GHOST, LANES,
    MAX_RATCHET, MAX_TRACKS, NORMAL, PATTERNS, SONG_SLOTS, STEPS,
};
use synth::{Param, Unit, PARAMS};
use undo::Undo;
use widgets::{
    disclosure_arrow, empty_fill, knob, lit_fill, playhead, port_picker, DRUM_ON, NOTE_HELD,
    NOTE_NAMES, NOTE_ON,
};

const RECORDING: Color32 = Color32::from_rgb(220, 60, 60);
const SOLO: Color32 = Color32::from_rgb(230, 200, 60);
/// Notes from other tracks, shown faintly in the piano roll.
const OTHER_NOTE: Color32 = Color32::from_rgb(55, 75, 105);
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

const PAN: Param = Param {
    name: "Pan",
    min: -1.0,
    max: 1.0,
    default: 0.0,
    log: false,
    unit: Unit::Pan,
};

const REVERB_SEND: Param = Param {
    name: "Reverb",
    min: 0.0,
    max: 1.0,
    default: 0.0,
    log: false,
    unit: Unit::Percent,
};

const DELAY_SEND: Param = Param {
    name: "Delay",
    ..REVERB_SEND
};

const REVERB_SIZE: Param = Param {
    name: "Size",
    min: 0.0,
    max: 1.0,
    default: Effects::DEFAULT.reverb_size,
    log: false,
    unit: Unit::Percent,
};

const REVERB_TONE: Param = Param {
    name: "Tone",
    default: Effects::DEFAULT.reverb_tone,
    ..REVERB_SIZE
};

const DELAY_TIME: Param = Param {
    name: "Time",
    min: 1.0,
    max: 8.0,
    default: Effects::DEFAULT.delay_steps,
    log: false,
    unit: Unit::Steps,
};

const DELAY_FEEDBACK: Param = Param {
    name: "Feedback",
    min: 0.0,
    max: 0.9,
    default: Effects::DEFAULT.delay_feedback,
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

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    // Reopen the song from last time, or start with the demo.
    let last = file::last_song().and_then(|path| Some((file::open(&path).ok()?, path)));
    let (project, path, bpm) = match last {
        Some((song, path)) => (song.project, Some(path), song.bpm),
        None => (Project::demo(), None, 120.0),
    };

    // Sized to fit the 1366x768 screen of an 11" MacBook Air.
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1080.0, 700.0])
        // Matches daw.desktop, so Linux desktops (e.g. Hyprland) show the
        // right name and icon for the window.
        .with_app_id("daw");
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "daw",
        options,
        Box::new(move |_cc| Ok(Box::new(App::new(project, path, bpm)))),
    )
}

/// The web version: draws on the page's canvas (see `web/index.html`).
#[cfg(target_arch = "wasm32")]
fn main() {
    web::show_crashes();
    // Reopen the song kept from last visit, or start with the demo.
    let kept = web::stored_song()
        .and_then(|(name, bytes)| Some((file::from_bytes(&name, bytes).ok()?, name)));
    let (project, path, bpm) = match kept {
        Some((song, name)) => {
            let path = (!name.is_empty()).then(|| PathBuf::from(name));
            (song.project, path, song.bpm)
        }
        None => (Project::demo(), None, 120.0),
    };
    wasm_bindgen_futures::spawn_local(async move {
        eframe::WebRunner::new()
            .start(
                web::canvas(),
                eframe::WebOptions::default(),
                Box::new(move |_cc| Ok(Box::new(App::new(project, path, bpm)))),
            )
            .await
            .expect("couldn't start daw");
    });
}

/// Which parts of the window are showing.
#[derive(Clone, Copy, PartialEq)]
struct View {
    song: bool,
    tracks: bool,
    piano: bool,
    panel: bool,
    /// Show other tracks' notes faintly in the piano roll.
    others: bool,
}

impl View {
    const ALL: View = View {
        song: true,
        tracks: true,
        piano: true,
        panel: true,
        others: true,
    };
}

/// Something to do to a track, chosen from its right-click menu.
enum TrackAction {
    Group,
    MoveTo(u32),
    Ungroup,
    Delete,
}

/// What to do once any unsaved changes are saved or discarded.
#[derive(Clone)]
enum Then {
    NewSong,
    Demo,
    Open(Opened),
    Quit,
}

/// A song file that's been read in, ready to open.
#[derive(Clone)]
struct Opened {
    /// Where it came from; on the web, just its name.
    path: PathBuf,
    bytes: Vec<u8>,
}

/// The track to select in a newly opened song: the first melodic one, so
/// the piano roll shows something.
fn first_melodic(project: &Project) -> usize {
    project
        .tracks
        .iter()
        .position(Track::is_melodic)
        .unwrap_or(0)
}

struct App {
    shared: Arc<Shared>,
    /// The editable project; changes are published to the engine as snapshots.
    project: Project,
    /// Recent snapshots, kept so the audio thread is never the one to free one.
    history: VecDeque<Arc<Project>>,
    /// Earlier versions of the song, for Ctrl+Z.
    undo: Undo<Project>,
    /// The project changed this frame and needs publishing.
    dirty: bool,
    events: SyncSender<Event>,
    midi: Midi,
    audio_error: Option<String>,
    clipboard: Option<Vec<Grid>>,
    /// Track shown in the side panel and piano roll.
    selected: usize,
    /// Samples decoded on background threads (the desktop app only).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    load_tx: Sender<Result<Sample, String>>,
    load_rx: Receiver<Result<Sample, String>>,
    /// Files still being decoded.
    loading: usize,
    load_error: Option<String>,
    /// Keys played while playing are written into the pattern.
    recording: bool,
    /// The computer-keyboard key holding a note, its track and MIDI note.
    held_key: Option<(egui::Key, usize, u8)>,
    show_help: bool,
    show_about: bool,
    show_fx: bool,
    /// Waiting for "save changes first?" to be answered.
    confirm: Option<Then>,
    /// Extra tracks picked with Ctrl/Cmd-click, for grouping.
    picked: Vec<usize>,
    /// A group shown in the side panel instead of a track.
    selected_group: Option<u32>,
    view: View,
    /// Notes played on MIDI controllers, for Rec.
    played: Receiver<Played>,
    show_midi_inputs: bool,
    /// The song file being edited, once saved or opened.
    file: Option<PathBuf>,
    /// Changed since the last save or open.
    modified: bool,
    /// Tempo lives in the engine, so it's compared to what was saved.
    saved_bpm: f32,
    /// Closing has been confirmed; don't ask again.
    may_close: bool,
    /// The window title last set.
    title: String,
    /// Finished exports: a message to show, or an error.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    export_tx: Sender<Result<String, String>>,
    export_rx: Receiver<Result<String, String>>,
    exporting: bool,
    /// A short success message, e.g. "Exported song.wav".
    notice: Option<String>,
    /// Kept so sound keeps playing; dropping it stops audio. (The web
    /// version keeps it with the handler that starts it on first click.)
    #[cfg(not(target_arch = "wasm32"))]
    _stream: Option<cpal::Stream>,
    /// Songs and samples picked in the browser's file dialog.
    #[cfg(target_arch = "wasm32")]
    inbox_tx: Sender<web::Picked>,
    #[cfg(target_arch = "wasm32")]
    inbox_rx: Receiver<web::Picked>,
    /// Counts published changes, so autosave knows when there's news.
    #[cfg(target_arch = "wasm32")]
    edits: u64,
    /// What autosave last kept: the edit count and tempo.
    #[cfg(target_arch = "wasm32")]
    autosaved: (u64, f32),
}

impl App {
    fn new(project: Project, path: Option<PathBuf>, bpm: f32) -> Self {
        let snapshot = Arc::new(project.clone());
        let shared = Arc::new(Shared::new(snapshot.clone()));
        let (events, events_rx) = sync_channel(256);
        let (midi_tx, midi_rx) = sync_channel(1024);
        let (played_tx, played) = channel();
        let midi = Midi::new(shared.clone(), events.clone(), played_tx, midi_rx);
        let stream = audio::start(shared.clone(), events_rx, midi_tx);
        let audio_error = stream.as_ref().err().cloned();
        #[cfg(target_arch = "wasm32")]
        if let Ok(stream) = stream {
            web::play_on_first_input(stream);
        }
        let (load_tx, load_rx) = channel();
        let (export_tx, export_rx) = channel();
        #[cfg(target_arch = "wasm32")]
        let (inbox_tx, inbox_rx) = channel();
        shared.set_bpm(bpm);
        let selected = first_melodic(&project);
        let undo = Undo::new(project.clone());
        App {
            shared,
            project,
            history: VecDeque::from([snapshot]),
            undo,
            dirty: false,
            events,
            midi,
            audio_error,
            clipboard: None,
            selected,
            load_tx,
            load_rx,
            loading: 0,
            load_error: None,
            recording: false,
            held_key: None,
            show_help: false,
            show_about: false,
            show_fx: false,
            confirm: None,
            picked: Vec::new(),
            selected_group: None,
            view: View::ALL,
            played,
            show_midi_inputs: false,
            file: path,
            modified: false,
            saved_bpm: bpm,
            may_close: false,
            title: String::new(),
            export_tx,
            export_rx,
            exporting: false,
            notice: None,
            #[cfg(not(target_arch = "wasm32"))]
            _stream: stream.ok(),
            #[cfg(target_arch = "wasm32")]
            inbox_tx,
            #[cfg(target_arch = "wasm32")]
            inbox_rx,
            #[cfg(target_arch = "wasm32")]
            edits: 0,
            #[cfg(target_arch = "wasm32")]
            autosaved: (0, bpm),
        }
    }
}

/// What File > Export writes.
#[derive(Clone, Copy)]
enum ExportKind {
    Wav,
    Midi,
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
fn toggle_ghost(step: &mut Option<Step>) {
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
            ..Step::HIT
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
        *step = Step::HIT;
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
        marks.push('<');
    } else if s.nudge > 0 {
        marks.push('>');
    }
    marks
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let playing = self.shared.playing.load(Relaxed);
        // Nothing is recorded during the count-in bar.
        let rolling = playing && !self.shared.counting_in.load(Relaxed);
        self.shared.count_in.store(self.recording, Relaxed);
        if ctx.input(|i| i.viewport().close_requested()) && !self.may_close && self.unsaved() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Then::Quit);
        }
        self.file_shortcuts(ctx);
        if !ctx.wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                self.shared.playing.store(!playing, Relaxed);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::F1)) {
                self.show_help = !self.show_help;
            }
            self.view_keys(ctx);
            self.undo_keys(ctx);
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::G)) {
                self.group_picked();
            }
            self.computer_keyboard(ctx, rolling);
        }
        self.shared.selected_track.store(self.selected, Relaxed);
        while let Ok(result) = self.export_rx.try_recv() {
            self.exporting = false;
            match result {
                Ok(message) => self.notice = Some(message),
                Err(e) => self.load_error = Some(e),
            }
        }
        while let Ok(p) = self.played.try_recv() {
            if self.recording && rolling {
                self.record(p.track, p.note, p.velocity);
            }
        }

        while let Ok(result) = self.load_rx.try_recv() {
            self.loading -= 1;
            match result {
                Ok(sample) => self.add_track(Track::sampler(Arc::new(sample), false)),
                Err(e) => self.load_error = Some(e),
            }
        }
        // Desktop drops come as paths; browser drops as the file's bytes.
        let dropped: Vec<egui::DroppedFile> = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(path) = f.path {
                self.load_file(ctx, path);
                continue;
            }
            if let Some(bytes) = f.bytes {
                self.load_bytes(&f.name, bytes.to_vec());
            }
        }
        #[cfg(target_arch = "wasm32")]
        while let Ok(picked) = self.inbox_rx.try_recv() {
            match picked {
                web::Picked::Song(name, bytes) => {
                    let path = PathBuf::from(name);
                    self.request(ctx, Then::Open(Opened { path, bytes }));
                }
                web::Picked::Sample(name, bytes) => self.load_bytes(&name, bytes),
            }
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
            if self.exporting {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Exporting…");
                });
            } else if let Some(notice) = &self.notice {
                ui.colored_label(Color32::LIGHT_GREEN, notice);
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
        self.fx_window(ctx);
        self.confirm_dialog(ctx);
        self.midi_inputs_window(ctx);

        if self.dirty {
            self.publish();
            self.undo.changed();
        }
        // A knob drag or a burst of typing is one undo step, settled once
        // it's over.
        if !ctx.input(|i| i.pointer.any_down()) && !ctx.wants_keyboard_input() {
            self.undo.settle(&self.project);
        }
        self.update_title(ctx);
        #[cfg(target_arch = "wasm32")]
        {
            self.midi.send_queued();
            // Autosave once a change is finished (not mid-drag).
            let news = self.autosaved != (self.edits, self.shared.bpm());
            if news && !ctx.input(|i| i.pointer.any_down()) {
                self.autosave();
            }
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
        self.modified = true;
        #[cfg(target_arch = "wasm32")]
        {
            self.edits += 1;
        }
    }

    /// Ctrl+Z undoes; Ctrl+Shift+Z or Ctrl+Y redoes (Cmd on Mac).
    fn undo_keys(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let pressed = |modifiers, key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(modifiers, key)))
        };
        // Check Shift+Z first: plain Ctrl+Z would also match it.
        if pressed(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
            || pressed(Modifiers::COMMAND, Key::Y)
        {
            self.redo();
        } else if pressed(Modifiers::COMMAND, Key::Z) {
            self.undo();
        }
    }

    fn undo(&mut self) {
        if let Some(project) = self.undo.undo(&self.project) {
            self.restore(project);
        }
    }

    fn redo(&mut self) {
        if let Some(project) = self.undo.redo(&self.project) {
            self.restore(project);
        }
    }

    /// Switches to an earlier or later version of the song.
    fn restore(&mut self, project: Project) {
        self.project = project;
        self.selected = self
            .selected
            .min(self.project.tracks.len().saturating_sub(1));
        if self
            .selected_group
            .is_some_and(|id| self.project.group(id).is_none())
        {
            self.selected_group = None;
        }
        self.picked.clear();
        self.publish();
    }

    fn unsaved(&self) -> bool {
        self.modified || self.shared.bpm() != self.saved_bpm
    }

    fn song_name(&self) -> String {
        self.file
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or_else(|| "Untitled".into(), |n| n.to_string_lossy().into_owned())
    }

    /// Shows the song's name, with a dot while there are unsaved changes.
    fn update_title(&mut self, ctx: &egui::Context) {
        let dot = if self.unsaved() { "• " } else { "" };
        let title = format!("{dot}{} - daw", self.song_name());
        if title != self.title {
            #[cfg(target_arch = "wasm32")]
            web::set_title(&title);
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    /// Ctrl+N, Ctrl+O, Ctrl+S and Ctrl+Shift+S (Cmd on Mac).
    fn file_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let pressed = |modifiers, key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(modifiers, key)))
        };
        // Check Shift+S first: plain Ctrl+S would also match it.
        if pressed(Modifiers::COMMAND | Modifiers::SHIFT, Key::S) {
            self.save_as();
        } else if pressed(Modifiers::COMMAND, Key::S) {
            self.save();
        } else if pressed(Modifiers::COMMAND, Key::O) {
            self.open_dialog(ctx);
        } else if pressed(Modifiers::COMMAND, Key::N) {
            self.request(ctx, Then::NewSong);
        }
    }

    /// Does `then`, first asking to save if there are unsaved changes.
    fn request(&mut self, ctx: &egui::Context, then: Then) {
        if self.unsaved() {
            self.confirm = Some(then);
        } else {
            self.carry_out(ctx, then);
        }
    }

    fn carry_out(&mut self, ctx: &egui::Context, then: Then) {
        match then {
            Then::NewSong => self.replace_song(Project::empty(), None, 120.0),
            Then::Demo => self.replace_song(Project::demo(), None, 120.0),
            Then::Open(Opened { path, bytes }) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                match file::from_bytes(&name, bytes) {
                    Ok(song) => {
                        #[cfg(not(target_arch = "wasm32"))]
                        file::remember(&path);
                        self.replace_song(song.project, Some(path), song.bpm);
                    }
                    Err(e) => self.load_error = Some(e),
                }
            }
            Then::Quit => {
                self.may_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn replace_song(&mut self, project: Project, file: Option<PathBuf>, bpm: f32) {
        self.shared.playing.store(false, Relaxed);
        self.selected = first_melodic(&project);
        self.project = project;
        self.selected_group = None;
        self.picked.clear();
        self.shared.edit_pattern.store(0, Relaxed);
        self.shared.set_bpm(bpm);
        self.publish();
        self.undo.reset(self.project.clone());
        self.file = file;
        self.modified = false;
        self.saved_bpm = bpm;
        self.load_error = None;
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open_dialog(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .add_filter("daw song", &[file::EXTENSION])
            .pick_file();
        if let Some(path) = picked {
            match std::fs::read(&path) {
                Ok(bytes) => self.request(ctx, Then::Open(Opened { path, bytes })),
                Err(e) => self.load_error = Some(format!("Couldn't open the song: {e}")),
            }
        }
    }

    /// The browser's picker answers later, through `inbox`.
    #[cfg(target_arch = "wasm32")]
    fn open_dialog(&mut self, ctx: &egui::Context) {
        web::pick(ctx, self.inbox_tx.clone(), true);
    }

    /// Saves to the current file, or asks where. Returns false if cancelled
    /// or it failed.
    fn save(&mut self) -> bool {
        match self.file.clone() {
            Some(path) => self.write_song(path),
            None => self.save_as(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save_as(&mut self) -> bool {
        let picked = rfd::FileDialog::new()
            .add_filter("daw song", &[file::EXTENSION])
            .set_file_name(format!("{}.{}", self.song_name(), file::EXTENSION))
            .save_file();
        match picked {
            Some(path) => self.write_song(path.with_extension(file::EXTENSION)),
            None => false,
        }
    }

    /// Browsers decide where downloads go, so this just names the file.
    #[cfg(target_arch = "wasm32")]
    fn save_as(&mut self) -> bool {
        let name = format!("{}.{}", self.song_name(), file::EXTENSION);
        self.write_song(PathBuf::from(name))
    }

    /// Asks where to export, then renders on a background thread so the
    /// app stays responsive.
    #[cfg(not(target_arch = "wasm32"))]
    fn export(&mut self, ctx: &egui::Context, kind: ExportKind) {
        let (label, ext) = match kind {
            ExportKind::Wav => ("WAV audio", "wav"),
            ExportKind::Midi => ("MIDI file", "mid"),
        };
        let picked = rfd::FileDialog::new()
            .add_filter(label, &[ext])
            .set_file_name(format!("{}.{ext}", self.song_name()))
            .save_file();
        let Some(path) = picked else { return };
        let path = path.with_extension(ext);
        let project = self.project.clone();
        let bpm = self.shared.bpm();
        let pattern = self.edit_pattern();
        let tx = self.export_tx.clone();
        let ctx = ctx.clone();
        self.exporting = true;
        self.notice = None;
        thread::spawn(move || {
            let bytes = match kind {
                ExportKind::Wav => export::wav(&project, bpm, pattern),
                ExportKind::Midi => Ok(export::midi(&project, bpm, pattern)),
            };
            let result = bytes.and_then(|bytes| {
                std::fs::write(&path, bytes).map_err(|e| format!("Couldn't export: {e}"))
            });
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let _ = tx.send(result.map(|()| format!("Exported {name}")));
            ctx.request_repaint();
        });
    }

    /// Renders right away (browsers have no background threads here) and
    /// downloads the result.
    #[cfg(target_arch = "wasm32")]
    fn export(&mut self, _ctx: &egui::Context, kind: ExportKind) {
        let bpm = self.shared.bpm();
        let pattern = self.edit_pattern();
        let (bytes, ext, mime) = match kind {
            ExportKind::Wav => (export::wav(&self.project, bpm, pattern), "wav", "audio/wav"),
            ExportKind::Midi => (
                Ok(export::midi(&self.project, bpm, pattern)),
                "mid",
                "audio/midi",
            ),
        };
        let name = format!("{}.{ext}", self.song_name());
        match bytes.and_then(|bytes| web::download(&name, &bytes, mime)) {
            Ok(()) => self.notice = Some(format!("Exported {name}")),
            Err(e) => self.load_error = Some(e),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn write_song(&mut self, path: PathBuf) -> bool {
        let bpm = self.shared.bpm();
        match file::save(&path, &self.project, bpm) {
            Ok(()) => {
                file::remember(&path);
                self.file = Some(path);
                self.modified = false;
                self.saved_bpm = bpm;
                self.load_error = None;
                true
            }
            Err(e) => {
                self.load_error = Some(e);
                false
            }
        }
    }

    /// Downloads the song, and keeps it in the browser for next visit.
    #[cfg(target_arch = "wasm32")]
    fn write_song(&mut self, path: PathBuf) -> bool {
        let bpm = self.shared.bpm();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let saved = file::to_bytes(&self.project, bpm)
            .and_then(|bytes| web::download(&name, &bytes, "application/zip"));
        match saved {
            Ok(()) => {
                self.file = Some(path);
                self.modified = false;
                self.saved_bpm = bpm;
                self.load_error = None;
                self.autosave();
                true
            }
            Err(e) => {
                self.load_error = Some(e);
                false
            }
        }
    }

    /// Keeps the song in the browser, so it's back after a reload or next
    /// visit. Runs a moment after each change.
    #[cfg(target_arch = "wasm32")]
    fn autosave(&mut self) {
        let bpm = self.shared.bpm();
        self.autosaved = (self.edits, bpm);
        let name = self
            .file
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let kept =
            file::to_bytes(&self.project, bpm).and_then(|bytes| web::store_song(&name, &bytes));
        if let Err(e) = kept {
            self.load_error = Some(e);
        }
    }

    fn preview(&self, track: usize, lane: usize) {
        let _ = self.events.try_send(Event::Preview { track, lane });
    }

    /// Rec: writes a played note into the pattern at the step that's playing.
    fn record(&mut self, track: usize, note: u8, velocity: u8) {
        let pattern = self.edit_pattern();
        let step = self.shared.current_step.load(Relaxed);
        if let Some(t) = self.project.tracks.get_mut(track) {
            let lane = t.lane_for(note);
            t.patterns[pattern][lane][step] = Some(Step {
                velocity,
                ..Step::HIT
            });
            self.dirty = true;
        }
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
            self.midi.track_deleted(i);
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
                *v = View {
                    others: v.others,
                    ..View::ALL
                };
            }
        });
    }

    /// Opens the file picker and loads the chosen audio files.
    #[cfg(not(target_arch = "wasm32"))]
    fn pick_samples(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .add_filter("Audio", &sample::EXTENSIONS)
            .pick_files();
        for path in picked.into_iter().flatten() {
            self.load_file(ctx, path);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn pick_samples(&mut self, ctx: &egui::Context) {
        web::pick(ctx, self.inbox_tx.clone(), false);
    }

    /// Decodes audio that's already in memory, e.g. dropped in a browser.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn load_bytes(&mut self, name: &str, bytes: Vec<u8>) {
        match sample::decode(name, bytes) {
            Ok(sample) => self.add_track(Track::sampler(Arc::new(sample), false)),
            Err(e) => self.load_error = Some(e),
        }
    }

    /// Decodes a file on a background thread so the UI doesn't freeze.
    #[cfg(not(target_arch = "wasm32"))]
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
            track.patterns[edit] = EMPTY_GRID;
        }
        self.dirty = true;
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                let item = |ui: &mut egui::Ui, label: &str, keys: &str| {
                    let clicked = ui
                        .add(egui::Button::new(label).shortcut_text(keys))
                        .clicked();
                    if clicked {
                        ui.close_menu();
                    }
                    clicked
                };
                if item(ui, "New song", "Ctrl+N") {
                    self.request(ctx, Then::NewSong);
                }
                if item(ui, "Open…", "Ctrl+O") {
                    self.open_dialog(ctx);
                }
                if item(ui, "Save", "Ctrl+S") {
                    self.save();
                }
                // In a browser, Save downloads the song, so Save as is the same.
                #[cfg(not(target_arch = "wasm32"))]
                if item(ui, "Save as…", "Ctrl+Shift+S") {
                    self.save_as();
                }
                ui.separator();
                // Exports the Song row, or the current pattern four times if it's empty.
                if item(ui, "Export WAV…", "") {
                    self.export(ctx, ExportKind::Wav);
                }
                if item(ui, "Export MIDI…", "") {
                    self.export(ctx, ExportKind::Midi);
                }
                ui.separator();
                if item(ui, "Load demo song", "") {
                    self.request(ctx, Then::Demo);
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            });
            ui.menu_button("Edit", |ui| {
                let undo = egui::Button::new("Undo  (Ctrl+Z)");
                if ui.add_enabled(self.undo.can_undo(), undo).clicked() {
                    self.undo();
                    ui.close_menu();
                }
                let redo = egui::Button::new("Redo  (Ctrl+Shift+Z)");
                if ui.add_enabled(self.undo.can_redo(), redo).clicked() {
                    self.redo();
                    ui.close_menu();
                }
                ui.separator();
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
                ui.checkbox(&mut v.others, "Other tracks' notes in piano roll");
                ui.separator();
                ui.label(RichText::new("Focus on one thing").small().weak());
                if ui.button("Tracks only").clicked() {
                    *v = View {
                        song: false,
                        tracks: true,
                        piano: false,
                        panel: false,
                        ..*v
                    };
                    ui.close_menu();
                }
                if ui.button("Piano roll and sound").clicked() {
                    *v = View {
                        song: false,
                        tracks: false,
                        piano: true,
                        panel: true,
                        ..*v
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

    /// Lists MIDI inputs: tick to connect, and pick where each one's notes go.
    fn midi_inputs_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_midi_inputs;
        let tracks: Vec<String> = self.project.tracks.iter().map(|t| t.name.clone()).collect();
        let groups: Vec<(u32, String)> = self
            .project
            .groups
            .iter()
            .map(|g| (g.id, g.name.clone()))
            .collect();
        let describe = |route: Route| match route {
            Route::Selected => "Selected track".to_string(),
            Route::Track(i) => tracks.get(i).cloned().unwrap_or_default(),
            Route::Group(id) => groups
                .iter()
                .find(|(g, _)| *g == id)
                .map_or_else(String::new, |(_, name)| format!("{name} (group)")),
        };
        egui::Window::new("MIDI inputs")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                let midi = &mut self.midi;
                if ui.button("⟳ Rescan").clicked() {
                    midi.refresh();
                }
                if midi.in_ports.is_empty() {
                    ui.weak("No MIDI inputs found. Plug one in and press Rescan.");
                }
                ui.add_space(4.0);
                egui::Grid::new("inputs").num_columns(2).show(ui, |ui| {
                    for name in midi.in_ports.clone() {
                        let route = midi.route(&name);
                        let mut on = route.is_some();
                        if ui.checkbox(&mut on, &name).changed() {
                            midi.set_connected(&name, on);
                        }
                        if let Some(route) = route {
                            let mut picked = route;
                            egui::ComboBox::from_id_salt(&name)
                                .selected_text(describe(route))
                                .width(170.0)
                                .show_ui(ui, |ui| {
                                    let mut option = |ui: &mut egui::Ui, r: Route| {
                                        ui.selectable_value(&mut picked, r, describe(r));
                                    };
                                    option(ui, Route::Selected);
                                    for &(id, _) in &groups {
                                        option(ui, Route::Group(id));
                                    }
                                    for i in 0..tracks.len() {
                                        option(ui, Route::Track(i));
                                    }
                                });
                            if picked != route {
                                midi.set_route(&name, picked);
                            }
                        }
                        ui.end_row();
                    }
                });
                ui.add_space(4.0);
                ui.weak(
                    "Selected track follows your clicks. A group sends each drum\n\
                         note to its matching track: the kick pad plays Kick.",
                );
            });
        self.show_midi_inputs = open;
    }

    /// "Save changes first?", shown before anything that would lose them.
    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(then) = self.confirm.clone() else {
            return;
        };
        #[derive(PartialEq)]
        enum Choice {
            Save,
            Discard,
            Cancel,
        }
        let mut choice = None;
        egui::Window::new("Save changes?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Save the changes to \"{}\" first?",
                    self.song_name()
                ));
                ui.weak("If you don't, they'll be lost.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(Choice::Save);
                    }
                    if ui.button("Don't save").clicked() {
                        choice = Some(Choice::Discard);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(Choice::Cancel);
                    }
                });
            });
        match choice {
            Some(Choice::Save) => {
                // If saving is cancelled or fails, stay put and keep asking.
                if self.save() {
                    self.confirm = None;
                    self.carry_out(ctx, then);
                }
            }
            Some(Choice::Discard) => {
                self.confirm = None;
                self.carry_out(ctx, then);
            }
            Some(Choice::Cancel) => self.confirm = None,
            None => {}
        }
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
            let rec_label = if s.counting_in.load(Relaxed) {
                "⏺ Count-in"
            } else {
                "⏺ Rec"
            };
            let rec = egui::Button::new(RichText::new(rec_label).color(rec_color))
                .selected(self.recording);
            let rec_hint = "While playing, notes you play on the keyboard\n\
                            are written into the selected track.\n\
                            Play counts in one bar first.";
            if ui.add(rec).on_hover_text(rec_hint).clicked() {
                self.recording = !self.recording;
            }
            let metronome = s.metronome.load(Relaxed);
            if ui
                .selectable_label(metronome, "Click")
                .on_hover_text("Metronome: a click on every beat")
                .clicked()
            {
                s.metronome.store(!metronome, Relaxed);
            }
            if ui
                .selectable_label(self.show_fx, "FX")
                .on_hover_text("Reverb and delay settings")
                .clicked()
            {
                self.show_fx = !self.show_fx;
            }
            ui.separator();
            ui.weak("Space = play · A–K = notes · Z/X = octave · F1 = help");
        });
    }

    /// Plays the selected track from the computer keyboard, like a MIDI
    /// keyboard, and records into the pattern when Rec is on.
    /// `rolling`: playing and past any count-in, so Rec writes notes.
    fn computer_keyboard(&mut self, ctx: &egui::Context, rolling: bool) {
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
        let base = track.base_note();
        let mut octave_changed = false;
        let mut to_record = Vec::new();
        for (key, pressed) in keys {
            match key {
                egui::Key::Z if pressed => {
                    track.octave = track.octave.saturating_sub(1);
                    octave_changed = true;
                }
                egui::Key::X if pressed => {
                    track.octave = (track.octave + 1).min(7);
                    octave_changed = true;
                }
                _ => {
                    let Some(semitone) = key_semitone(key) else {
                        continue;
                    };
                    if pressed {
                        if let Some((_, t, note)) = self.held_key.take() {
                            let _ = self.events.try_send(Event::Release { track: t, note });
                        }
                        let note = base + semitone;
                        self.held_key = Some((key, self.selected, note));
                        let _ = self.events.try_send(Event::Hold {
                            track: self.selected,
                            note,
                            velocity: NORMAL,
                        });
                        if self.recording && rolling {
                            to_record.push(note);
                        }
                    } else if let Some((_, t, note)) = self.held_key.filter(|&(k, _, _)| k == key) {
                        self.held_key = None;
                        let _ = self.events.try_send(Event::Release { track: t, note });
                    }
                }
            }
        }
        if octave_changed {
            self.dirty = true;
        }
        for note in to_record {
            self.record(self.selected, note, NORMAL);
        }
    }

    fn midi_row(&mut self, ui: &mut egui::Ui) {
        let s = self.shared.clone();
        ui.horizontal(|ui| {
            let m = &mut self.midi;
            if let Some(pick) = port_picker(ui, "MIDI out", &m.out_ports, &m.out_name) {
                m.select_out(pick);
            }
            let inputs = format!("MIDI in: {} connected", m.connected_inputs());
            let hint = "Connect controllers and choose which track each one plays";
            if ui.button(inputs).on_hover_text(hint).clicked() {
                self.show_midi_inputs = !self.show_midi_inputs;
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
            for (step, on) in lit.iter_mut().enumerate() {
                *on |= t.hit_at(pattern, step);
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
                    let _ = self.events.try_send(Event::Preview { track: i, lane: 0 });
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
            let lanes = track.lanes();
            let on_color = if melodic { NOTE_ON } else { DRUM_ON };
            let grid = &mut track.patterns[pattern];
            // A step is a column across every lane, so walk it by index.
            #[allow(clippy::needless_range_loop)]
            for step in 0..STEPS {
                // Every lane with something at this step, lowest note first.
                let notes: Vec<usize> = (0..lanes).filter(|&l| grid[l][step].is_some()).collect();
                let first = notes.iter().copied().find(|&l| Step::is_hit(&grid[l][step]));
                // Melodic steps show their lowest note and how many more
                // make the chord; groove settings show as marks.
                let text = match first.and_then(|l| grid[l][step].map(|s| (l, s))) {
                    Some((l, s)) => {
                        let name = if melodic { NOTE_NAMES[l] } else { "" };
                        let more = match notes.len() {
                            1 => String::new(),
                            n => format!("+{}", n - 1),
                        };
                        format!("{name}{more}{}", step_marks(&s))
                    }
                    None if !notes.is_empty() => "–".to_string(),
                    None => String::new(),
                };
                let fill = match first.and_then(|l| grid[l][step]) {
                    Some(s) => lit_fill(on_color, s.velocity),
                    None if !notes.is_empty() => NOTE_HELD,
                    None => empty_fill(step, false),
                };
                let pad = egui::Button::new(RichText::new(text).small().color(Color32::BLACK))
                    .fill(fill)
                    .stroke(playhead(current == Some(step)))
                    .min_size(Vec2::new(28.0, 26.0));
                let resp = ui.add(pad);
                if resp.clicked() {
                    if ui.input(|i| i.modifiers.shift) {
                        if first.is_none() {
                            grid[0][step] = None;
                            toggle_ghost(&mut grid[0][step]);
                        }
                        for &l in &notes {
                            if Step::is_hit(&grid[l][step]) {
                                toggle_ghost(&mut grid[l][step]);
                            }
                        }
                    } else if notes.is_empty() {
                        grid[0][step] = Some(Step::HIT);
                    } else {
                        for &l in &notes {
                            grid[l][step] = None;
                        }
                    }
                    self.dirty = true;
                }
                if let Some(l) = first {
                    resp.context_menu(|ui| {
                        let mut edited = grid[l][step].unwrap_or(Step::HIT);
                        if step_menu(ui, &mut edited) {
                            // Apply to every note of a chord at once.
                            for &n in &notes {
                                if Step::is_hit(&grid[n][step]) {
                                    grid[n][step] = Some(edited);
                                }
                            }
                            self.dirty = true;
                        }
                    });
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
        // Notes other melodic tracks play in this pattern, by pitch (C..B),
        // shown faintly so parts can be lined up.
        let mut others = [[false; STEPS]; LANES];
        if self.view.others {
            for (j, t) in self.project.tracks.iter().enumerate() {
                if j == i || !t.is_melodic() {
                    continue;
                }
                for (lane, steps) in t.patterns[pattern].iter().enumerate() {
                    for (step, value) in steps.iter().enumerate() {
                        if Step::is_hit(value) {
                            others[lane][step] = true;
                        }
                    }
                }
            }
        }
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
            ui.weak("Click: add/remove · several per column = chord · right-click after a note: hold · right-click a note: groove");
        });
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        let grid = &mut track.patterns[pattern];
        // Highest note on top, like a piano.
        for lane in (0..LANES).rev() {
            let name = NOTE_NAMES[lane];
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
                    let _ = self.events.try_send(Event::Preview { track: i, lane });
                }
                ui.add_space(52.0); // line up with the steps above
                for step in 0..STEPS {
                    let value = grid[lane][step];
                    let fill = match value {
                        Some(s) if s.tie => NOTE_HELD,
                        Some(s) => lit_fill(NOTE_ON, s.velocity),
                        None if others[lane][step] => OTHER_NOTE,
                        None => empty_fill(step, sharp),
                    };
                    let text = match value {
                        Some(s) if !s.tie => step_marks(&s),
                        _ => String::new(),
                    };
                    let cell = egui::Button::new(RichText::new(text).small().color(Color32::BLACK))
                        .fill(fill)
                        .stroke(playhead(current == Some(step)))
                        .min_size(Vec2::new(28.0, 17.0));
                    let resp = ui.add(cell);
                    let lane_steps = &mut grid[lane];
                    if resp.clicked() {
                        if ui.input(|i| i.modifiers.shift) {
                            if !Step::is_hit(&value) {
                                lane_steps[step] = None;
                            }
                            toggle_ghost(&mut lane_steps[step]);
                        } else if value.is_some() {
                            lane_steps[step] = None;
                            // Drop the ties that held this note on.
                            for later in lane_steps.iter_mut().skip(step + 1) {
                                if !later.is_some_and(|s| s.tie) {
                                    break;
                                }
                                *later = None;
                            }
                        } else {
                            lane_steps[step] = Some(Step::HIT);
                            let _ = self.events.try_send(Event::Preview { track: i, lane });
                        }
                        self.dirty = true;
                    } else if Step::is_hit(&value) {
                        if let Some(s) = lane_steps[step].as_mut() {
                            resp.context_menu(|ui| self.dirty |= step_menu(ui, s));
                        }
                    } else if resp.secondary_clicked()
                        && value.is_none()
                        && step > 0
                        && lane_steps[step - 1].is_some()
                    {
                        // Hold the note before through this step.
                        lane_steps[step] = Some(Step::TIE);
                        self.dirty = true;
                    }
                }
            });
        }
    }

    /// The shared reverb and delay. Tracks send to them with their Reverb
    /// and Delay knobs.
    fn fx_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_fx;
        let fx = &mut self.project.effects;
        let mut changed = false;
        egui::Window::new("Effects")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.weak("Each track's Reverb and Delay knobs send to these.");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Reverb").small().strong());
                        ui.horizontal(|ui| {
                            changed |= knob(ui, &REVERB_SIZE, &mut fx.reverb_size);
                            changed |= knob(ui, &REVERB_TONE, &mut fx.reverb_tone);
                        });
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Delay").small().strong());
                        ui.horizontal(|ui| {
                            changed |= knob(ui, &DELAY_TIME, &mut fx.delay_steps);
                            changed |= knob(ui, &DELAY_FEEDBACK, &mut fx.delay_feedback);
                        });
                    });
                });
            });
        self.show_fx = open;
        if changed {
            fx.delay_steps = fx.delay_steps.round();
            self.dirty = true;
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
                changed |= knob(ui, &HUMANIZE, &mut track.humanize);
            }
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Mix").small().strong());
        ui.horizontal(|ui| {
            changed |= knob(ui, &TRACK_VOLUME, &mut track.volume);
            changed |= knob(ui, &PAN, &mut track.pan);
            changed |= knob(ui, &REVERB_SEND, &mut track.reverb);
            changed |= knob(ui, &DELAY_SEND, &mut track.delay);
        });
        if changed {
            self.dirty = true;
        }

        ui.add_space(12.0);
        if ui.button("Play a note").clicked() {
            self.preview(self.selected, 0);
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
