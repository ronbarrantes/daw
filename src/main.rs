// No console window behind the app on Windows (debug builds keep it for logs).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod chords;
mod drum_browser;
mod drums;
mod export;
mod file;
mod fx;
mod help;
mod jam;
mod log;
mod midi;
mod notes;
mod project;
mod sample;
mod synth;
mod synth_ui;
mod theme;
mod timeline;
#[cfg(test)]
mod ui_tests;
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

use eframe::egui::{self, Color32, RichText, Vec2};

use audio::{Event, Shared};
use jam::{NoteDrag, Paint, Playhead, Tool};
use midi::{Midi, Played, Route};
use project::{Effects, Kind, Part, Project, Sample, Step, Track, MAX_TRACKS, NORMAL};
use synth::{Param, Unit};
use synth_ui::{section_titles, Snapshot};
use undo::Undo;
use widgets::{knob, meter, port_picker};

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
        // Matches daw.desktop, so Linux desktops and window managers
        // (e.g. Hyprland) show the right name and icon for the window.
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
        Box::new(move |cc| {
            loose_clicks(&cc.egui_ctx);
            Ok(Box::new(App::new(project, path, bpm)))
        }),
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
                Box::new(move |cc| {
                    loose_clicks(&cc.egui_ctx);
                    Ok(Box::new(App::new(project, path, bpm)))
                }),
            )
            .await
            .expect("couldn't start daw");
    });
}

/// Lets a click drift a little further before it counts as a drag, so a
/// two-finger trackpad click still opens a right-click menu.
fn loose_clicks(ctx: &egui::Context) {
    ctx.options_mut(|o| o.input_options.max_click_dist = 12.0);
}

/// The two halves of the app.
#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Jam,
    Arrange,
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
    drum_browser: drum_browser::Browser,
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
    clipboard: Option<Vec<Part>>,
    /// The bar of the pattern shown in the editor, from 0.
    edit_bar: usize,
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
    /// The computer-keyboard keys holding notes (several at once, for
    /// chords and the arpeggiator): each key, its track and MIDI note.
    held_keys: Vec<(egui::Key, usize, u8)>,
    /// The keyboard's pitch wheel: how far notes played on it are bent
    /// now, in semitones (1 and 2 bend down and up while held).
    bend: f32,
    /// How far 1 and 2 bend, in semitones.
    bend_range: f32,
    /// Whether 1 and 2 bend at all.
    bend_keys: bool,
    show_help: bool,
    show_about: bool,
    /// Help > Diagnostics.
    show_diagnostics: bool,
    /// The busiest the audio has been lately, 0-1 of its time, for
    /// Diagnostics.
    audio_load: f32,
    /// The errors last shown, so each new one is logged once.
    logged: [Option<String>; 3],
    show_fx: bool,
    /// Jam (make patterns and blocks) or Arrange (the timeline).
    tab: Tab,
    timeline: timeline::State,
    /// The note editor for MIDI blocks, in Arrange.
    notes: notes::State,
    /// A piano-roll note being dragged.
    note_drag: Option<NoteDrag>,
    /// The note a synth track is droning (playing nonstop), if any:
    /// (track, MIDI note).
    drone: Option<(usize, u8)>,
    /// The Drone's pitch, in Hz: anywhere, not just on the 12 notes.
    drone_hz: f32,
    /// The Drone's slider moves freely in Hz instead of note by note.
    drone_free: bool,
    /// The theremin pad's note while it's held: (track, MIDI note).
    theremin: Option<(usize, u8)>,
    /// The piano-roll column the pointer was over last frame, for naming
    /// its chord.
    roll_hover: Option<usize>,
    /// Steps being painted or erased by dragging across them.
    paint: Option<Paint>,
    /// The tool bar's tool.
    tool: Tool,
    /// The oscilloscope and envelope drawing in the instrument panel.
    show_scope: bool,
    /// Pan and the effect sends as columns on each track row.
    show_mix: bool,
    /// Each track's level for its meter, falling slowly between peaks.
    track_levels: [f32; MAX_TRACKS],
    /// Where each shown bar's first step sits on screen, for the piano
    /// roll to line up with.
    bar_x: [f32; 2],
    /// A step's size and a piano-roll lane's height, fitted to the window.
    cell: Vec2,
    lane_h: f32,
    /// Redraw less often while playing, to save battery.
    low_power: bool,
    /// View > Theme, and whether it's been put on yet.
    theme: theme::Theme,
    theme_on: bool,
    /// Instrument panel sections popped out into their own windows, by
    /// title, with where each window was (x, y, width, height).
    popped: Vec<(String, Option<[f32; 4]>)>,
    /// `popped` changed and should be remembered once the mouse is up.
    popped_changed: bool,
    /// What the level meter shows: recent peaks, falling slowly, and how
    /// hard the limiter is working.
    meter_levels: [f32; 2],
    meter_limiting: f32,
    /// Instrument panel sections closed with their ×, by title.
    closed_sections: Vec<String>,
    /// `closed_sections` changed and should be remembered.
    sections_changed: bool,
    /// Which of `SCOPE_SPANS` the scope shows.
    scope_span: usize,
    /// The scope shows the spectrum (loudness per pitch) instead.
    spectrum: bool,
    /// How far Mutate moves each knob (0-1 of its sweep), the modules it
    /// leaves alone, and its xorshift state.
    mutate_amount: f32,
    mutate_locked: Vec<&'static str>,
    rng: u32,
    /// Each synth track's two remembered sounds (not saved with the song).
    snapshots: Vec<Snapshot>,
    /// The scope had something to show last frame, so keep it moving.
    scope_busy: bool,
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
        log::add(&format!(
            "daw {} started ({} {})",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
        match &audio_error {
            Some(e) => log::add(&format!("Audio didn't start: {e}")),
            None => log::add(&format!(
                "Audio: {}, {} Hz",
                shared.device.lock().map(|d| d.clone()).unwrap_or_default(),
                shared.sample_rate.load(Relaxed)
            )),
        }
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
            drum_browser: Default::default(),
            shared,
            project,
            history: VecDeque::from([snapshot]),
            undo,
            dirty: false,
            events,
            midi,
            audio_error: audio_error.clone(),
            clipboard: None,
            edit_bar: 0,
            selected,
            load_tx,
            load_rx,
            loading: 0,
            load_error: None,
            recording: false,
            held_keys: Vec::new(),
            bend: 0.0,
            bend_range: file::bend_range(),
            bend_keys: file::bend_keys(),
            show_help: false,
            show_about: false,
            show_diagnostics: false,
            audio_load: 0.0,
            // The audio error is already logged above.
            logged: [audio_error.clone(), None, None],
            show_fx: false,
            tab: Tab::Jam,
            timeline: timeline::State::default(),
            notes: notes::State::default(),
            note_drag: None,
            paint: None,
            tool: Tool::Draw,
            roll_hover: None,
            drone: None,
            drone_hz: synth::midi_to_hz(48),
            drone_free: false,
            theremin: None,
            show_scope: true,
            show_mix: false,
            track_levels: [0.0; MAX_TRACKS],
            bar_x: [0.0; 2],
            cell: Vec2::splat(18.0),
            lane_h: 10.0,
            low_power: file::low_power(),
            theme: file::theme(),
            theme_on: false,
            popped: file::popped_sections(),
            popped_changed: false,
            meter_levels: [0.0; 2],
            meter_limiting: 0.0,
            closed_sections: file::closed_sections(),
            sections_changed: false,
            scope_span: 1,
            spectrum: false,
            mutate_amount: 0.15,
            mutate_locked: vec!["Voice", "Arpeggiator"],
            rng: 0x9e37_79b9,
            snapshots: Vec::new(),
            scope_busy: false,
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.draw(ctx);
    }
}

impl App {
    /// One frame of the whole app: everything `update` does, without
    /// eframe's frame, so tests can drive it.
    fn draw(&mut self, ctx: &egui::Context) {
        let playing = self.shared.playing.load(Relaxed);
        // Undo or deleting a pattern or block can leave the editor
        // pointing past the last one.
        let last = self.project.patterns.len() - 1;
        if self.shared.edit_pattern.load(Relaxed) as usize > last {
            self.shared.edit_pattern.store(last as u8, Relaxed);
        }
        self.shared.edit_block.store(self.edit_block(), Relaxed);
        // Arrange plays the timeline; Jam plays a pattern or a chain.
        self.shared
            .song_mode
            .store(self.tab == Tab::Arrange, Relaxed);
        if self
            .timeline
            .selected
            .is_some_and(|i| i >= self.project.clips.len())
        {
            self.timeline.selected = None;
        }
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
                self.set_playing(!playing);
            }
            if ctx.input(|i| i.key_pressed(egui::Key::F1)) {
                self.show_help = !self.show_help;
            }
            self.view_keys(ctx);
            self.undo_keys(ctx);
            // With notes selected, Delete and the rest are the note editor's.
            if self.tab == Tab::Arrange && !self.note_keys(ctx) {
                self.timeline_keys(ctx);
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::G)) {
                self.group_picked();
            }
            self.computer_keyboard(ctx, rolling);
        }
        self.shared.selected_track.store(self.selected, Relaxed);
        for (i, level) in self.track_levels.iter_mut().enumerate() {
            *level = self.shared.take_track_meter(i).max(*level * 0.85);
        }
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

        if !self.theme_on {
            theme::apply(ctx, self.theme);
            self.theme_on = true;
        }
        // One row along the top: the menus, then the transport.
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            ui.add_space(2.0);
            self.menu_bar(ui, ctx, playing);
            ui.add_space(2.0);
        });

        // Errors, notices and how hard the audio is working, always in view.
        egui::TopBottomPanel::bottom("status")
            .exact_height(20.0)
            .show(ctx, |ui| self.status_line(ui));

        // The instrument is a drawer along the bottom: shut, it's one line
        // saying what's selected; open (I or F8), it lays every section
        // out side by side.
        if self.view.panel {
            egui::TopBottomPanel::bottom("instrument")
                .resizable(true)
                .default_height(290.0)
                .height_range(120.0..=(ctx.screen_rect().height() * 0.75).max(130.0))
                .show(ctx, |ui| match self.selected_group {
                    Some(id) => {
                        self.drawer_bar(ui, true);
                        egui::ScrollArea::vertical()
                            .auto_shrink(false)
                            .show(ui, |ui| self.group_panel(ui, id));
                    }
                    // Its header stays put; only the sections scroll.
                    None => self.instrument_panel(ui),
                });
        } else {
            egui::TopBottomPanel::bottom("instrument drawer")
                .exact_height(24.0)
                .show(ctx, |ui| self.drawer_bar(ui, false));
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.tab == Tab::Arrange {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        self.timeline(ui, playing);
                        ui.add_space(8.0);
                        self.block_drawer(ui);
                        if let Some(b) = self.open_notes() {
                            ui.add_space(8.0);
                            self.note_editor(ui, b, playing);
                        }
                    });
            } else {
                self.jam_view(ui, ctx, playing);
            }
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
        self.log_errors();
        self.diagnostics_window(ctx);
        self.fx_window(ctx);
        self.popped_windows(ctx);
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
        match ctx.input(|i| i.pointer.primary_down().then(|| i.pointer.interact_pos())) {
            Some(pos) => {
                if let Some(p) = &mut self.paint {
                    p.last_x = pos.map(|pos| pos.x).or(p.last_x);
                }
            }
            None => {
                self.paint = None;
                // The theremin's note ends when the mouse comes up, even if
                // its pad was hidden mid-drag.
                if let Some((track, note)) = self.theremin.take() {
                    let _ = self.events.try_send(Event::Release { track, note });
                }
            }
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
        // The engine only feeds the scope while it's showing.
        let in_panel = self.view.panel && self.selected_group.is_none();
        let scope = self.show_scope && (in_panel || self.is_popped("Scope"));
        self.shared.scope_on.store(scope, Relaxed);
        // Only redraw continuously while playing (or the scope has sound
        // to show), to keep CPU low when idle.
        // Knobs that modulation moves keep moving on screen.
        let modulating = in_panel && self.shared.mod_live(0).is_some();
        if playing || (scope && self.scope_busy) || modulating {
            ctx.request_repaint_after(self.frame());
        }
    }
}

impl App {
    /// Jam: the chain (in Chain mode), the tracks with their header and
    /// tools pinned and the rows scrolling between them, and the selected
    /// synth's piano roll under its row. Anything hidden folds to a line
    /// with a ▸ to bring it back.
    fn jam_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, playing: bool) {
        let edit = self.shared.edit_pattern.load(Relaxed) as usize;
        self.edit_bar = self.edit_bar.min(self.project.pattern_bars(edit) - 1);
        // Only show the playhead on the pattern that's actually playing.
        let current =
            (playing && self.shared.play_pattern.load(Relaxed) as usize == edit).then(|| {
                Playhead {
                    bar: self.shared.current_bar.load(Relaxed),
                    step: self.shared.current_step.load(Relaxed),
                }
            });
        // The chain only matters while playing chains; the patterns are in
        // the top bar.
        if self.shared.chain_mode.load(Relaxed) {
            let open = self.view.song;
            let name = if open {
                "Hide the chain"
            } else {
                "Show the chain"
            };
            if widgets::fold_toggle(ui, open, "Chain", name)
                .on_hover_text("Shortcut: F5")
                .clicked()
            {
                self.view.song = !open;
            }
            if open {
                self.chain_row(ui, playing);
            }
            ui.add_space(4.0);
        }
        self.fit_steps(ui.available_width(), edit);
        // Patterns, length and what plays sit on the tracks' own top line.
        self.pattern_line(ui, edit);
        if !self.view.tracks {
            let melodic = self
                .project
                .tracks
                .get(self.selected)
                .is_some_and(Track::is_melodic);
            if melodic {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| self.piano_roll_or_bar(ui, edit, current));
            }
            return;
        }
        self.tracks_header(ui, edit, current);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                self.track_list(ui, edit, current);
                // Adding a track sits right under the last one.
                ui.add_space(4.0);
                self.add_row(ui, ctx);
            });
    }

    /// How long to wait between redraws while something moves.
    fn frame(&self) -> Duration {
        Duration::from_millis(if self.low_power { 66 } else { 30 })
    }

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
        let last = self.project.patterns.len() - 1;
        if self.edit_pattern() > last {
            self.shared.edit_pattern.store(last as u8, Relaxed);
        }
        if self
            .selected_group
            .is_some_and(|id| self.project.group(id).is_none())
        {
            self.selected_group = None;
        }
        self.picked.clear();
        self.notes.deselect();
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
        self.stop_drone();
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
        let edit = self.edit_now();
        let tx = self.export_tx.clone();
        let ctx = ctx.clone();
        self.exporting = true;
        self.notice = None;
        thread::spawn(move || {
            let bytes = match kind {
                ExportKind::Wav => export::wav(&project, bpm, edit),
                ExportKind::Midi => Ok(export::midi(&project, bpm, edit)),
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
        let edit = self.edit_now();
        let (bytes, ext, mime) = match kind {
            ExportKind::Wav => (export::wav(&self.project, bpm, edit), "wav", "audio/wav"),
            ExportKind::Midi => (
                Ok(export::midi(&self.project, bpm, edit)),
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

    /// Briefly plays one lane of a track.
    fn preview(&self, track: usize, lane: usize) {
        if let Some(t) = self.project.tracks.get(track) {
            let note = t.note(lane);
            let _ = self.events.try_send(Event::Preview { track, note });
        }
    }

    /// What the Jam view has open, for exports.
    fn edit_now(&self) -> audio::Edit {
        audio::Edit {
            pattern: self.edit_pattern(),
            block: self.edit_block(),
        }
    }

    /// Rec: writes a played note into the pattern at the step that's playing.
    fn record(&mut self, track: usize, note: u8, velocity: u8) {
        let pattern = self.edit_pattern();
        let step = self.shared.current_step.load(Relaxed);
        let bar = self.shared.current_bar.load(Relaxed);
        if let Some(t) = self.project.tracks.get_mut(track) {
            let lane = t.lane_for(note);
            if let Some(grid) = t.grid_mut(pattern, bar) {
                grid[lane][step] = Some(Step {
                    velocity,
                    ..Step::HIT
                });
                self.dirty = true;
            }
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
        self.project.add_track(track);
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

    /// Adds built-in drum `i`, on the drum channel, into the Drums group
    /// if there is one.
    fn add_drum(&mut self, i: usize) {
        let Some(sample) = project::builtin_drums().into_iter().nth(i) else {
            return;
        };
        let mut track = Track::sampler(sample, false);
        track.midi_channel = Some(9);
        track.midi_note = drums::NOTES[i];
        track.group = self
            .project
            .groups
            .iter()
            .find(|g| g.name == "Drums")
            .map(|g| g.id);
        self.add_track(track);
    }

    fn delete_selected(&mut self) {
        self.delete_track(self.selected);
    }

    /// Stops the Drone button's note.
    fn stop_drone(&mut self) {
        if let Some((track, note)) = self.drone.take() {
            let _ = self.events.try_send(Event::Release { track, note });
        }
    }

    fn delete_track(&mut self, i: usize) {
        if i < self.project.tracks.len() {
            // The engine's voices don't move with the tracks, so a drone
            // can't follow its track to a new place.
            self.stop_drone();
            self.project.remove_track(i);
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
            if i.key_pressed(I) && i.modifiers.is_none() {
                v.panel = !v.panel;
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
                .map(|t| t.patterns[edit].clone())
                .collect(),
        );
    }

    fn paste_pattern(&mut self) {
        let edit = self.edit_pattern();
        if let Some(copied) = &self.clipboard {
            for (track, part) in self.project.tracks.iter_mut().zip(copied) {
                track.patterns[edit] = part.clone();
            }
            self.dirty = true;
        }
    }

    fn clear_pattern(&mut self) {
        let edit = self.edit_pattern();
        for track in &mut self.project.tracks {
            track.patterns[edit].clear();
        }
        self.dirty = true;
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, playing: bool) {
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
                // Exports the timeline, or the current pattern four times if it's empty.
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
                ui.checkbox(&mut v.song, "Chain  (F5)");
                ui.checkbox(&mut v.tracks, "Tracks  (F6)");
                ui.checkbox(&mut v.piano, "Piano roll  (F7)");
                ui.checkbox(&mut v.panel, "Instrument drawer  (I or F8)");
                ui.checkbox(&mut v.others, "Other tracks' notes in piano roll");
                ui.checkbox(&mut self.show_scope, "Scope and drawings");
                ui.menu_button("Instrument sections", |ui| {
                    for t in section_titles() {
                        let mut on = !self.closed_sections.iter().any(|c| c == t);
                        if ui.checkbox(&mut on, t).changed() {
                            self.closed_sections.retain(|c| c != t);
                            if !on {
                                self.closed_sections.push(t.to_string());
                            }
                            file::remember_closed_sections(&self.closed_sections);
                        }
                    }
                });
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
                ui.separator();
                if ui
                    .checkbox(&mut self.show_fx, "Effects (reverb and delay)")
                    .changed()
                {
                    ui.close_menu();
                }
                let low = ui
                    .checkbox(&mut self.low_power, "Low power")
                    .on_hover_text("Redraw less often while playing, to save battery");
                if low.changed() {
                    file::remember_low_power(self.low_power);
                }
                ui.menu_button("Theme", |ui| {
                    for t in theme::Theme::ALL {
                        let picked = ui
                            .radio_value(&mut self.theme, t, t.name())
                            .on_hover_text(t.hint());
                        if picked.clicked() {
                            self.theme_on = false;
                            file::remember_theme(t);
                            ui.close_menu();
                        }
                    }
                });
            });
            ui.menu_button("Help", |ui| {
                if ui.button("How to use (F1)").clicked() {
                    self.show_help = true;
                    ui.close_menu();
                }
                if ui.button("Diagnostics").clicked() {
                    self.show_diagnostics = true;
                    ui.close_menu();
                }
                if ui.button("About").clicked() {
                    self.show_about = true;
                    ui.close_menu();
                }
            });
            ui.menu_button("MIDI", |ui| self.midi_menu(ui));
            ui.separator();
            self.transport_row(ui, playing);
        });
    }

    /// The line along the very bottom: any error (or what just happened),
    /// then how hard the audio is working. Click the load for Diagnostics.
    fn status_line(&mut self, ui: &mut egui::Ui) {
        // The busiest callback lately, falling slowly so peaks stay readable.
        self.audio_load = self.shared.take_load().max(self.audio_load * 0.97);
        ui.horizontal_centered(|ui| {
            let errors: Vec<&String> = [&self.audio_error, &self.midi.error, &self.load_error]
                .into_iter()
                .flatten()
                .collect();
            if !errors.is_empty() {
                for err in errors {
                    ui.colored_label(Color32::LIGHT_RED, RichText::new(err).small());
                }
            } else if self.exporting {
                ui.spinner();
                ui.label(RichText::new("Exporting…").small());
            } else if let Some(notice) = &self.notice {
                ui.colored_label(Color32::LIGHT_GREEN, RichText::new(notice).small());
            } else {
                ui.label(
                    RichText::new("Space plays · A to K play notes · Z / X octave · F1 help")
                        .small()
                        .weak(),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dropouts = self.shared.dropouts.load(Relaxed);
                let load = format!("Audio {:.0}%", self.audio_load * 100.0);
                let text = if dropouts > 0 {
                    RichText::new(format!("{load} · {dropouts} dropouts"))
                        .small()
                        .color(Color32::from_rgb(230, 170, 90))
                } else {
                    RichText::new(load).small().weak()
                };
                let resp = ui
                    .add(egui::Label::new(text).sense(egui::Sense::click()))
                    .on_hover_text("How hard the audio is working. Click for Diagnostics.");
                if resp.clicked() {
                    self.show_diagnostics = true;
                }
            });
        });
    }

    /// The instrument drawer's top line: an arrow to open or shut it, and
    /// what's selected, so even shut it says what you'd be shaping.
    /// The drawer's switch: ▾ Instrument hides it, ▸ Instrument shows it.
    pub(crate) fn drawer_toggle(&mut self, ui: &mut egui::Ui) {
        let open = self.view.panel;
        let name = if open {
            "Hide the instrument"
        } else {
            "Show the instrument"
        };
        if widgets::fold_toggle(ui, open, "Instrument", name)
            .on_hover_text("Shortcut: I or F8")
            .clicked()
        {
            self.view.panel = !open;
        }
    }

    fn drawer_bar(&mut self, ui: &mut egui::Ui, open: bool) {
        ui.horizontal(|ui| {
            self.drawer_toggle(ui);
            if open {
                return;
            }
            if let Some(id) = self.selected_group {
                if let Some(g) = self.project.groups.iter().find(|g| g.id == id) {
                    ui.label(RichText::new(format!("{} (group)", g.name)).small());
                }
            } else if let Some(t) = self.project.tracks.get(self.selected) {
                ui.label(RichText::new(&t.name).small());
                let what = match &t.kind {
                    Kind::Synth(v) => format!("Synth · {}", synth_ui::preset_name(v)),
                    Kind::Sampler { sample, .. } => format!("Sample · {}", sample.name),
                };
                ui.label(RichText::new(what).small().weak());
                // The sound at a glance: small drawings of its envelope,
                // filter and LFO, and where the cutoff is.
                if let Kind::Synth(v) = &t.kind {
                    let size = Vec2::new(48.0, 16.0);
                    widgets::envelope_sized(
                        ui,
                        size,
                        v[synth::ATTACK],
                        v[synth::DECAY],
                        v[synth::SUSTAIN],
                        v[synth::RELEASE],
                    );
                    widgets::filter_curve_sized(
                        ui,
                        size,
                        v[synth::CUTOFF],
                        v[synth::RESONANCE],
                        None,
                    );
                    widgets::lfo_shape_sized(
                        ui,
                        size,
                        synth::choice(v[synth::LFO_SHAPE], synth::LFO_SHAPES.len()),
                        None,
                        widgets::source_color(synth::Source::Lfo1),
                    );
                    let cutoff = &synth::PARAMS[synth::CUTOFF];
                    ui.label(
                        RichText::new(format!(
                            "Cutoff {}",
                            widgets::format_value(cutoff, v[synth::CUTOFF])
                        ))
                        .small()
                        .weak(),
                    );
                }
                if ui
                    .small_button("Tap")
                    .on_hover_text("Play one short note")
                    .clicked()
                {
                    self.preview(self.selected, 0);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new("I to open").small().weak());
            });
        });
    }

    /// Logs each error the app shows, once, as it appears.
    fn log_errors(&mut self) {
        let now = [
            self.audio_error.clone(),
            self.load_error.clone(),
            self.midi.error.clone(),
        ];
        for (seen, error) in self.logged.iter_mut().zip(now) {
            if *seen != error {
                if let Some(e) = &error {
                    log::add(e);
                }
                *seen = error;
            }
        }
    }

    /// Help > Diagnostics: how the audio is doing, and the recent log, to
    /// copy into a bug report.
    fn diagnostics_window(&mut self, ctx: &egui::Context) {
        if !self.show_diagnostics {
            return;
        }
        let s = &self.shared;
        let sr = s.sample_rate.load(Relaxed).max(1);
        let buffer = s.buffer.load(Relaxed);
        let facts = [
            ("Version", env!("CARGO_PKG_VERSION").to_string()),
            (
                "System",
                format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            ),
            (
                "Audio device",
                s.device.lock().map(|d| d.clone()).unwrap_or_default(),
            ),
            ("Sample rate", format!("{sr} Hz")),
            (
                "Buffer",
                format!(
                    "{buffer} frames ({:.1} ms)",
                    buffer as f32 * 1000.0 / sr as f32
                ),
            ),
            ("Audio load", format!("{:.0}%", self.audio_load * 100.0)),
            ("Dropouts", s.dropouts.load(Relaxed).to_string()),
            ("Audio errors", s.stream_errors.load(Relaxed).to_string()),
            (
                "Low power",
                if self.low_power { "on" } else { "off" }.into(),
            ),
        ];
        let lines = log::lines();
        let mut open = true;
        egui::Window::new("Diagnostics")
            .open(&mut open)
            .default_width(420.0)
            .show(ctx, |ui| {
                egui::Grid::new("diagnostics")
                    .num_columns(2)
                    .show(ui, |ui| {
                        for (name, value) in &facts {
                            ui.label(*name);
                            ui.label(value);
                            ui.end_row();
                        }
                    });
                ui.weak(
                    "Audio load is how much of its time the sound engine needs; \
                     near 100% it starts to crackle. Dropouts count the times it \
                     ran late.",
                );
                ui.separator();
                ui.horizontal(|ui| {
                    ui.strong("Recent log");
                    if ui.button("Copy all").clicked() {
                        let mut report: Vec<String> =
                            facts.iter().map(|(n, v)| format!("{n}: {v}")).collect();
                        report.push(String::new());
                        report.extend(lines.iter().cloned());
                        ui.ctx().copy_text(report.join("\n"));
                    }
                });
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(path) = log::path() {
                    ui.weak(format!("Also saved in {}", path.display()));
                }
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &lines {
                            ui.label(RichText::new(line).monospace().small());
                        }
                    });
            });
        self.show_diagnostics = open;
        ctx.request_repaint_after(Duration::from_millis(250));
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

    /// The transport, in the menu row: Jam and Arrange, play and record,
    /// a display with the position, tempo and swing, then the click,
    /// master volume and meter. (Patterns live on the tracks' top line.)
    fn transport_row(&mut self, ui: &mut egui::Ui, playing: bool) {
        let s = self.shared.clone();
        for (tab, name, hint) in [
            (Tab::Jam, "Jam", "Make patterns and chain them into blocks"),
            (
                Tab::Arrange,
                "Arrange",
                "Put blocks on the timeline to make a song",
            ),
        ] {
            if ui
                .selectable_label(self.tab == tab, name)
                .on_hover_text(hint)
                .clicked()
                && self.tab != tab
            {
                self.tab = tab;
                if tab == Tab::Arrange && playing {
                    // Pick up the song from the cursor.
                    s.seek.store(self.timeline.cursor, Relaxed);
                }
            }
        }
        ui.separator();
        let play = if playing { "■" } else { "▶" };
        if ui
            .selectable_label(playing, play)
            .on_hover_text("Play or stop (Space)")
            .clicked()
        {
            self.set_playing(!playing);
        }
        let rec_color = if self.recording {
            RECORDING
        } else {
            Color32::GRAY
        };
        let rec = if s.counting_in.load(Relaxed) {
            "⏺ 1 bar"
        } else {
            "⏺"
        };
        let rec_hint = "Record: while playing, notes you play on the keyboard\n\
                        are written into the selected track.\n\
                        Play counts in one bar first.";
        if ui
            .selectable_label(self.recording, RichText::new(rec).color(rec_color))
            .on_hover_text(rec_hint)
            .clicked()
        {
            self.recording = !self.recording;
        }
        // The display: where it is, the tempo and the swing.
        let display = ui.visuals().extreme_bg_color;
        egui::Frame::none()
            .fill(display)
            .inner_margin(egui::Margin::symmetric(8.0, 1.0))
            .rounding(2.0)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let mono = |t: String| RichText::new(t).monospace();
                let (bar, step) = if playing {
                    (s.current_bar.load(Relaxed), s.current_step.load(Relaxed))
                } else {
                    (0, 0)
                };
                ui.label(mono(format!(
                    "{}.{}.{}",
                    bar + 1,
                    step / 4 + 1,
                    step % 4 + 1
                )))
                .on_hover_text("Bar, beat and step playing");
                ui.weak("/");
                let mut bpm = s.bpm();
                let drag = egui::DragValue::new(&mut bpm)
                    .range(40.0..=240.0)
                    .speed(0.5)
                    .suffix(" BPM");
                if ui.add(drag).on_hover_text("Tempo: drag or type").changed() {
                    s.set_bpm(bpm);
                }
                ui.weak("/");
                let mut swing = self.project.swing * 100.0;
                let drag = egui::DragValue::new(&mut swing)
                    .range(0.0..=100.0)
                    .speed(0.5)
                    .prefix("swing ")
                    .suffix("%");
                let hint =
                    "Delays every other 16th note for a shuffled feel.\n66% is a triplet swing.";
                if ui.add(drag).on_hover_text(hint).changed() {
                    self.project.swing = swing / 100.0;
                    self.dirty = true;
                }
            });
        ui.separator();
        if self.tab == Tab::Arrange {
            ui.weak("Plays the timeline");
        }
        ui.separator();
        let metronome = s.metronome.load(Relaxed);
        if ui
            .selectable_label(metronome, "Click")
            .on_hover_text("Metronome: a click on every beat")
            .clicked()
        {
            s.metronome.store(!metronome, Relaxed);
        }
        let mut master = self.project.master * 100.0;
        let drag = egui::DragValue::new(&mut master)
            .range(0.0..=100.0)
            .speed(0.5)
            .suffix("%");
        let hint = "Master: the volume of the whole song. A limiter after it\n\
                    keeps the mix from clipping.";
        if ui.add(drag).on_hover_text(hint).changed() {
            self.project.master = master / 100.0;
            self.dirty = true;
        }
        let (peak, limiting) = s.take_meter();
        for (shown, new) in self.meter_levels.iter_mut().zip(peak) {
            *shown = new.max(*shown * 0.85);
        }
        self.meter_limiting = limiting.max(self.meter_limiting * 0.9);
        meter(ui, self.meter_levels, self.meter_limiting);
        if self.meter_levels.iter().any(|l| *l > 0.001) {
            // Keep the meter falling after the sound stops.
            ui.ctx().request_repaint_after(self.frame());
        }
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
                        if self.held_keys.iter().any(|&(k, _, _)| k == key) {
                            continue;
                        }
                        let note = base + semitone;
                        self.held_keys.push((key, self.selected, note));
                        let _ = self.events.try_send(Event::Hold {
                            track: self.selected,
                            note,
                            velocity: NORMAL,
                        });
                        // Played while the wheel is pushed: starts bent.
                        if self.bend != 0.0 {
                            let _ = self.events.try_send(Event::Bend {
                                track: self.selected,
                                note,
                                semis: self.bend,
                                filter: 0.0,
                            });
                        }
                        if self.recording && rolling {
                            to_record.push(note);
                        }
                    } else if let Some(i) = self.held_keys.iter().position(|&(k, _, _)| k == key) {
                        let (_, t, note) = self.held_keys.remove(i);
                        let _ = self.events.try_send(Event::Release { track: t, note });
                    }
                }
            }
        }
        // A key let go while the window wasn't listening (it lost focus,
        // say) never sends its release: let go of it here instead, so no
        // note gets stuck.
        let (focused, down) = ctx.input(|i| (i.focused, i.keys_down.clone()));
        self.held_keys.retain(|&(k, t, note)| {
            let still = focused && down.contains(&k);
            if !still {
                let _ = self.events.try_send(Event::Release { track: t, note });
            }
            still
        });
        self.bend_wheel(ctx, focused, &down);
        if octave_changed {
            self.dirty = true;
        }
        for note in to_record {
            self.record(self.selected, note, NORMAL);
        }
    }

    /// The keyboard's pitch wheel: holding 1 slides the notes played on
    /// the keys down by `bend_range` semitones, 2 slides them up, and
    /// letting go slides them back, like a wheel springing to the middle.
    fn bend_wheel(
        &mut self,
        ctx: &egui::Context,
        focused: bool,
        down: &std::collections::HashSet<egui::Key>,
    ) {
        let held = |k| self.bend_keys && focused && down.contains(&k);
        let target = match (held(egui::Key::Num1), held(egui::Key::Num2)) {
            (true, false) => -self.bend_range,
            (false, true) => self.bend_range,
            _ => 0.0,
        };
        if self.bend == target {
            return;
        }
        // A quick sweep, about a fifth of a second end to end.
        let step = self.bend_range * ctx.input(|i| i.stable_dt).min(0.1) / 0.2;
        self.bend = if (target - self.bend).abs() <= step {
            target
        } else {
            self.bend + step * (target - self.bend).signum()
        };
        for &(_, track, note) in &self.held_keys {
            let _ = self.events.try_send(Event::Bend {
                track,
                note,
                semis: self.bend,
                filter: 0.0,
            });
        }
        ctx.request_repaint();
    }

    /// Keys & MIDI: MIDI gear in and out, and bending with the keyboard.
    fn midi_menu(&mut self, ui: &mut egui::Ui) {
        let s = self.shared.clone();
        ui.vertical(|ui| {
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
            ui.separator();
            let hint = "Turn on to bend with the keyboard: while you play, hold 1\n\
                        to bend the notes down, 2 to bend them up";
            if ui
                .checkbox(&mut self.bend_keys, "Bend with keys 1 and 2")
                .on_hover_text(hint)
                .changed()
            {
                file::remember_bend_keys(self.bend_keys);
            }
            let range = egui::DragValue::new(&mut self.bend_range)
                .range(1.0..=12.0)
                .speed(0.05)
                .fixed_decimals(0)
                .suffix(" st");
            let hint = "While you play on the computer keyboard, hold 1 to bend\n\
                        the notes down this many semitones, 2 to bend them up";
            if ui
                .add_enabled(self.bend_keys, range)
                .on_hover_text(hint)
                .changed()
            {
                self.bend_range = self.bend_range.round();
                file::remember_bend_range(self.bend_range);
            }
        });
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
}
