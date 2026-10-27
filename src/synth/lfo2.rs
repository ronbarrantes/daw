//! The LFO 2 module: a second wobble, one per track. It has no target of
//! its own: drag it onto any knob to move that knob (see `modulation`).

use super::arp::OFF_ON;
use super::lfo::{SHAPES, SYNCS};
use super::{param, ModuleInfo, Unit};

pub const SHAPE: usize = 0;
pub const RATE: usize = 1;
pub const SYNC: usize = 2;
pub const RETRIG: usize = 3;

pub const INFO: ModuleInfo = ModuleInfo {
    name: "LFO 2",
    knobs: &[
        param("LFO 2 wave", 0.0, 4.0, 0.0, false, Unit::Choice(&SHAPES)),
        param("LFO 2 rate", 0.05, 20.0, 1.0, true, Unit::Hz),
        param("LFO 2 sync", 0.0, 7.0, 0.0, false, Unit::Choice(&SYNCS)),
        param("LFO 2 retrig", 0.0, 1.0, 0.0, false, Unit::Choice(&OFF_ON)),
    ],
    rows: &[&[SHAPE, RATE, SYNC, RETRIG]],
    open: true,
    engine: None,
};
