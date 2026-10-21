//! MIDI in and out.
//!
//! The audio thread queues outgoing messages on a channel; a small sender
//! thread writes them to whichever output port is selected. Incoming
//! messages update `Shared` from midir's own callback thread.

use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread;

use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

use crate::audio::Shared;
use crate::drums;

pub const CLOCK: u8 = 0xF8;
pub const START: u8 = 0xFA;
pub const CONTINUE: u8 = 0xFB;
pub const STOP: u8 = 0xFC;
pub const NOTE_ON_CH10: u8 = 0x99;
pub const NOTE_OFF_CH10: u8 = 0x89;

/// A short MIDI message, stored inline so queuing it never allocates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Msg {
    data: [u8; 3],
    len: u8,
}

impl Msg {
    pub fn one(a: u8) -> Self {
        Self { data: [a, 0, 0], len: 1 }
    }

    pub fn three(a: u8, b: u8, c: u8) -> Self {
        Self { data: [a, b, c], len: 3 }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }
}

type OutSlot = Arc<Mutex<Option<MidiOutputConnection>>>;

pub struct Midi {
    shared: Arc<Shared>,
    out: OutSlot,
    input: Option<MidiInputConnection<()>>,
    pub out_ports: Vec<String>,
    pub in_ports: Vec<String>,
    pub out_name: Option<String>,
    pub in_name: Option<String>,
    pub error: Option<String>,
}

impl Midi {
    pub fn new(shared: Arc<Shared>, queue: Receiver<Msg>) -> Self {
        let out: OutSlot = Arc::default();
        let slot = out.clone();
        thread::spawn(move || {
            for msg in queue {
                if let Some(conn) = slot.lock().unwrap().as_mut() {
                    let _ = conn.send(msg.bytes());
                }
            }
        });
        let mut midi = Self {
            shared,
            out,
            input: None,
            out_ports: Vec::new(),
            in_ports: Vec::new(),
            out_name: None,
            in_name: None,
            error: None,
        };
        midi.refresh();
        midi
    }

    /// Re-scans for devices, e.g. after plugging something in.
    pub fn refresh(&mut self) {
        self.out_ports = MidiOutput::new("daw")
            .map(|o| o.ports().iter().filter_map(|p| o.port_name(p).ok()).collect())
            .unwrap_or_default();
        self.in_ports = MidiInput::new("daw")
            .map(|i| i.ports().iter().filter_map(|p| i.port_name(p).ok()).collect())
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

    pub fn select_in(&mut self, name: Option<String>) {
        if let Some(conn) = self.input.take() {
            conn.close();
        }
        self.in_name = None;
        let Some(name) = name else { return };
        match connect_in(&name, self.shared.clone()) {
            Ok(conn) => {
                self.input = Some(conn);
                self.in_name = Some(name);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("MIDI in: {e}")),
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

fn connect_in(name: &str, shared: Arc<Shared>) -> Result<MidiInputConnection<()>, String> {
    let mut input = MidiInput::new("daw").map_err(|e| e.to_string())?;
    // Keep clock messages; drop SysEx and active sensing.
    input.ignore(Ignore::SysexAndActiveSense);
    let port = input
        .ports()
        .into_iter()
        .find(|p| input.port_name(p).as_deref() == Ok(name))
        .ok_or("device not found")?;
    input
        .connect(&port, "daw-in", move |_, msg, _| handle_input(&shared, msg), ())
        .map_err(|e| e.to_string())
}

fn handle_input(shared: &Shared, msg: &[u8]) {
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
        // Note-on from any channel plays the matching drum.
        [status, note, velocity] if status & 0xF0 == 0x90 && velocity > 0 => {
            if let Some(track) = drums::track_for_note(note) {
                shared.preview(track);
            }
        }
        _ => {}
    }
}
