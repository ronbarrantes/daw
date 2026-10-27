//! MIDI in and out.
//!
//! The audio thread queues outgoing messages on a channel; a small sender
//! thread writes them to whichever output port is selected. Several inputs
//! can be connected at once, each routed to the selected track or to a
//! fixed track or group. Incoming notes go to the engine as events, and to
//! the UI so Rec can write them into the pattern.

use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

use crate::audio::{Event, Shared};
use crate::drums;
use crate::project::Project;

pub const CLOCK: u8 = 0xF8;
pub const START: u8 = 0xFA;
pub const CONTINUE: u8 = 0xFB;
pub const STOP: u8 = 0xFC;

/// A short MIDI message, stored inline so queuing it never allocates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Msg {
    data: [u8; 3],
    len: u8,
}

impl Msg {
    pub fn one(a: u8) -> Self {
        Self {
            data: [a, 0, 0],
            len: 1,
        }
    }

    pub fn three(a: u8, b: u8, c: u8) -> Self {
        Self {
            data: [a, b, c],
            len: 3,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }
}

/// Where an input's notes go.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Route {
    /// Whichever track is selected in the UI; follows your clicks.
    Selected,
    /// Always this track, by index.
    Track(usize),
    /// Always this group: each note plays the track in it that sends the
    /// same note (kick pad → Kick), or else its first melodic track.
    Group(u32),
}

/// A note played on a controller, for Rec to write into the pattern.
pub struct Played {
    pub track: usize,
    pub note: u8,
    pub velocity: u8,
}

/// The track a note should play, or `None` if the route has nowhere to go.
pub fn target(project: &Project, selected: usize, route: Route, note: u8) -> Option<usize> {
    let track = match route {
        Route::Selected => selected,
        Route::Track(i) => i,
        Route::Group(id) => {
            let drum = drums::canonical_note(note);
            let in_group = |t: &&crate::project::Track| t.group == Some(id);
            let tracks = &project.tracks;
            tracks
                .iter()
                .position(|t| in_group(&t) && !t.is_melodic() && t.midi_note == drum)
                .or_else(|| tracks.iter().position(|t| in_group(&t) && t.is_melodic()))?
        }
    };
    (track < project.tracks.len()).then_some(track)
}

type OutSlot = Arc<Mutex<Option<MidiOutputConnection>>>;

struct Input {
    name: String,
    route: Arc<Mutex<Route>>,
    /// Kept alive to stay connected; dropping it disconnects.
    _conn: MidiInputConnection<()>,
}

pub struct Midi {
    shared: Arc<Shared>,
    events: SyncSender<Event>,
    played: Sender<Played>,
    out: OutSlot,
    inputs: Vec<Input>,
    pub out_ports: Vec<String>,
    pub in_ports: Vec<String>,
    pub out_name: Option<String>,
    pub error: Option<String>,
    /// Outgoing messages, sent each frame (the web version has no threads).
    #[cfg(target_arch = "wasm32")]
    queue: Receiver<Msg>,
}

impl Midi {
    pub fn new(
        shared: Arc<Shared>,
        events: SyncSender<Event>,
        played: Sender<Played>,
        queue: Receiver<Msg>,
    ) -> Self {
        let out: OutSlot = Arc::default();
        // The web version has no threads; `send_queued` runs every frame.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let slot = out.clone();
            thread::spawn(move || {
                for msg in queue {
                    if let Some(conn) = slot.lock().unwrap().as_mut() {
                        let _ = conn.send(msg.bytes());
                    }
                }
            });
        }
        let mut midi = Self {
            shared,
            events,
            played,
            out,
            inputs: Vec::new(),
            out_ports: Vec::new(),
            in_ports: Vec::new(),
            out_name: None,
            error: None,
            #[cfg(target_arch = "wasm32")]
            queue,
        };
        midi.refresh();
        midi
    }

    /// Sends what the engine queued for MIDI out.
    #[cfg(target_arch = "wasm32")]
    pub fn send_queued(&self) {
        let mut slot = self.out.lock().unwrap();
        for msg in self.queue.try_iter() {
            if let Some(conn) = slot.as_mut() {
                let _ = conn.send(msg.bytes());
            }
        }
    }

    /// Re-scans for devices, e.g. after plugging something in.
    pub fn refresh(&mut self) {
        self.out_ports = MidiOutput::new("daw")
            .map(|o| {
                o.ports()
                    .iter()
                    .filter_map(|p| o.port_name(p).ok())
                    .collect()
            })
            .unwrap_or_default();
        self.in_ports = MidiInput::new("daw")
            .map(|i| {
                i.ports()
                    .iter()
                    .filter_map(|p| i.port_name(p).ok())
                    .collect()
            })
            .unwrap_or_default();
    }

    pub fn select_out(&mut self, name: Option<String>) {
        let mut slot = self.out.lock().unwrap();
        if let Some(conn) = slot.take() {
            conn.close();
        }
        self.out_name = None;
        let Some(name) = name else { return };
        match connect_out(&name) {
            Ok(conn) => {
                *slot = Some(conn);
                self.out_name = Some(name);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("MIDI out: {e}")),
        }
    }

    pub fn connected_inputs(&self) -> usize {
        self.inputs.len()
    }

    /// The input's route, or `None` if it isn't connected.
    pub fn route(&self, name: &str) -> Option<Route> {
        let input = self.inputs.iter().find(|i| i.name == name)?;
        Some(*input.route.lock().unwrap())
    }

    /// Connects or disconnects an input.
    pub fn set_connected(&mut self, name: &str, on: bool) {
        self.inputs.retain(|i| i.name != name);
        if !on {
            return;
        }
        let route = Arc::new(Mutex::new(Route::Selected));
        let ctx = InputContext {
            shared: self.shared.clone(),
            events: self.events.clone(),
            played: self.played.clone(),
            route: route.clone(),
            sounding: [None; 128],
        };
        match connect_in(name, ctx) {
            Ok(conn) => {
                self.inputs.push(Input {
                    name: name.to_string(),
                    route,
                    _conn: conn,
                });
                self.error = None;
            }
            Err(e) => self.error = Some(format!("MIDI in: {e}")),
        }
    }

    /// Changes where an input's notes go; takes effect on the next note.
    pub fn set_route(&mut self, name: &str, route: Route) {
        if let Some(input) = self.inputs.iter().find(|i| i.name == name) {
            *input.route.lock().unwrap() = route;
        }
    }

    /// Keeps fixed-track routes pointing at the same tracks after one is
    /// deleted; routes to the deleted track fall back to the selected track.
    /// A track was inserted at `at`: routes to later tracks move with them.
    pub fn track_inserted(&mut self, at: usize) {
        for input in &self.inputs {
            let mut route = input.route.lock().unwrap();
            if let Route::Track(i) = *route {
                if i >= at {
                    *route = Route::Track(i + 1);
                }
            }
        }
    }

    /// Two tracks swapped places: routes follow them.
    pub fn tracks_swapped(&mut self, a: usize, b: usize) {
        for input in &self.inputs {
            let mut route = input.route.lock().unwrap();
            *route = match *route {
                Route::Track(i) if i == a => Route::Track(b),
                Route::Track(i) if i == b => Route::Track(a),
                other => other,
            };
        }
    }

    pub fn track_deleted(&mut self, deleted: usize) {
        for input in &self.inputs {
            let mut route = input.route.lock().unwrap();
            if let Route::Track(i) = *route {
                *route = match i.cmp(&deleted) {
                    std::cmp::Ordering::Less => Route::Track(i),
                    std::cmp::Ordering::Equal => Route::Selected,
                    std::cmp::Ordering::Greater => Route::Track(i - 1),
                };
            }
        }
    }
}

fn connect_out(name: &str) -> Result<MidiOutputConnection, String> {
    let out = MidiOutput::new("daw").map_err(|e| e.to_string())?;
    let port = out
        .ports()
        .into_iter()
        .find(|p| out.port_name(p).as_deref() == Ok(name))
        .ok_or("device not found")?;
    out.connect(&port, "daw-out").map_err(|e| e.to_string())
}

/// Everything one input's callback needs.
struct InputContext {
    shared: Arc<Shared>,
    events: SyncSender<Event>,
    played: Sender<Played>,
    route: Arc<Mutex<Route>>,
    /// The track each held note went to, so its note-off goes there too
    /// even if the route or selection changed meanwhile.
    sounding: [Option<usize>; 128],
}

fn connect_in(name: &str, mut ctx: InputContext) -> Result<MidiInputConnection<()>, String> {
    let mut input = MidiInput::new("daw").map_err(|e| e.to_string())?;
    // Keep clock messages; drop SysEx and active sensing.
    input.ignore(Ignore::SysexAndActiveSense);
    let port = input
        .ports()
        .into_iter()
        .find(|p| input.port_name(p).as_deref() == Ok(name))
        .ok_or("device not found")?;
    input
        .connect(&port, "daw-in", move |_, msg, _| ctx.handle(msg), ())
        .map_err(|e| e.to_string())
}

impl InputContext {
    fn handle(&mut self, msg: &[u8]) {
        let shared = &self.shared;
        let external = shared.external_sync.load(Relaxed);
        match *msg {
            [CLOCK] if external => {
                shared.ext_ticks.fetch_add(1, Relaxed);
            }
            [START] if external => {
                shared.ext_start.store(true, Relaxed);
                shared.playing.store(true, Relaxed);
            }
            [CONTINUE] if external => shared.playing.store(true, Relaxed),
            [STOP] if external => shared.playing.store(false, Relaxed),
            [status, note, velocity] if status & 0xF0 == 0x90 && velocity > 0 => {
                let note = note & 0x7F;
                let project = shared.project();
                let selected = shared.selected_track.load(Relaxed);
                let route = *self.route.lock().unwrap();
                if let Some(track) = target(&project, selected, route, note) {
                    self.sounding[note as usize] = Some(track);
                    let _ = self.events.try_send(Event::Hold {
                        track,
                        note,
                        velocity,
                    });
                    let _ = self.played.send(Played {
                        track,
                        note,
                        velocity,
                    });
                }
            }
            // Note-off, or note-on with velocity 0 (which means the same).
            [status, note, _] if status & 0xF0 == 0x80 || status & 0xF0 == 0x90 => {
                let note = note & 0x7F;
                if let Some(track) = self.sounding[note as usize].take() {
                    let _ = self.events.try_send(Event::Release { track, note });
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNARE: usize = 1;
    const LEAD: usize = 4;

    #[test]
    fn routes_pick_the_right_track() {
        let p = Project::demo();
        let drums = p.groups[0].id;
        assert_eq!(target(&p, LEAD, Route::Selected, 60), Some(LEAD));
        assert_eq!(target(&p, LEAD, Route::Track(SNARE), 60), Some(SNARE));
        // A drum kit's snare (38, or the 40 variant) finds the Snare track.
        assert_eq!(target(&p, LEAD, Route::Group(drums), 40), Some(SNARE));
        // A note no drum sends has nowhere to go in an all-drum group.
        assert_eq!(target(&p, LEAD, Route::Group(drums), 60), None);
        assert_eq!(target(&p, 99, Route::Selected, 60), None);
    }
}
