//! The stress test (Help > Diagnostics): holds more and more notes on
//! synth tracks, with and without the costly parts (a second oscillator,
//! LFOs, reverb and delay, other engines), and times the real engine
//! making the sound. The answer is how much of one processor core each
//! takes; above 100% it would crackle.
//!
//! It runs one case per frame, so the window can show how far it's got;
//! the web version has no threads to run it on.

use crate::audio;
use crate::project::{Kind, Project, Track, MAX_TRACKS};
use crate::synth::{self, Link, Source};

const SAMPLE_RATE: f32 = 48_000.0;
/// How much sound each case makes and times, in seconds.
const SECONDS: f32 = 0.25;
/// Each case is timed this many times and the quickest kept, as other
/// programs can slow any one try down.
const TRIES: usize = 3;
/// Voices one track can play at once (the engine's `POLYPHONY`).
const PER_TRACK: usize = 16;
pub const MAX_VOICES: usize = MAX_TRACKS * PER_TRACK;
/// How many voices the "what each part costs" cases play.
const COMPARE_VOICES: usize = 64;
/// The voice counts the full patch is tried at.
const SWEEP: [usize; 6] = [1, 16, 32, 64, 128, MAX_VOICES];
/// The sweep stops once a case needs this much of a core.
const GIVE_UP: f64 = 1.5;
/// The share of a core the sound can have and still leave room for the
/// screen (which, on the web, runs on the same core).
const SAFE: f64 = 0.7;

/// How a case's synth is set up. Each adds to the one before, up to
/// `Full`; the engines are tried on their own.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Patch {
    /// One saw and the filter.
    Basic,
    /// Plus the second oscillator, detuned.
    TwoOsc,
    /// Plus LFO 1 on the filter and LFO 2 and the mod envelope moving
    /// knobs in every voice.
    Lfos,
    /// Plus sends to the reverb and delay.
    Full,
    Fm,
    Wavetable,
}

impl Patch {
    pub fn name(self) -> &'static str {
        match self {
            Patch::Basic => "1 osc + filter",
            Patch::TwoOsc => "+ 2nd osc",
            Patch::Lfos => "+ LFOs",
            Patch::Full => "+ reverb & delay",
            Patch::Fm => "FM engine",
            Patch::Wavetable => "Wavetable engine",
        }
    }

    fn track(self, n: usize) -> Track {
        let mut t = Track::synth(&format!("Stress {}", n + 1));
        let Kind::Synth(v) = &mut t.kind else {
            unreachable!("a synth track")
        };
        v[synth::LEVEL_2] = 0.0;
        let at_least = |p: Patch| self as u8 >= p as u8 && (self as u8) <= Patch::Full as u8;
        if at_least(Patch::TwoOsc) {
            v[synth::LEVEL_2] = 0.8;
            v[synth::DETUNE] = 0.3;
        }
        if at_least(Patch::Lfos) {
            v[synth::LFO_TARGET] = 1.0; // the filter
            v[synth::LFO_AMOUNT] = 0.5;
            v[synth::LFO_RATE] = 3.0;
            t.mods = vec![
                Link {
                    source: Source::Lfo2,
                    knob: synth::RESONANCE,
                    amount: 0.3,
                },
                Link {
                    source: Source::ModEnv,
                    knob: synth::DETUNE,
                    amount: 0.4,
                },
            ];
        }
        if at_least(Patch::Full) {
            t.reverb = 0.3;
            t.delay = 0.3;
        }
        match self {
            Patch::Fm => v[synth::ENGINE] = synth::FM as f32,
            Patch::Wavetable => v[synth::ENGINE] = synth::WAVETABLE as f32,
            _ => {}
        }
        t
    }
}

/// What the sweep's rows are called.
pub const FULL: &str = "Everything on";

/// One case: a patch at a number of voices.
#[derive(Clone, Copy, Debug)]
pub struct Case {
    pub patch: Patch,
    pub voices: usize,
}

/// What a case measured.
#[derive(Clone, Copy, Debug)]
pub struct Result {
    pub case: Case,
    /// How much of one core it needs, 1.0 being all of it.
    pub load: f64,
}

/// Times one case.
pub fn run(case: Case) -> Result {
    let tracks = case.voices.div_ceil(PER_TRACK).max(1);
    let mut project = Project::empty();
    project.tracks = (0..tracks).map(|n| case.patch.track(n)).collect();
    // Spread over the tracks, different notes on each so none share.
    let notes: Vec<(usize, u8)> = (0..case.voices)
        .map(|i| (i / PER_TRACK, 40 + (i % PER_TRACK) as u8 * 2))
        .collect();
    let samples = (SECONDS * SAMPLE_RATE) as usize;
    let seconds = (0..TRIES)
        .map(|_| audio::time_held(project.clone(), &notes, SAMPLE_RATE, samples))
        .fold(f64::INFINITY, f64::min);
    Result {
        case,
        load: seconds / SECONDS as f64,
    }
}

/// The patches compared at `COMPARE_VOICES`, before the sweep.
const COMPARE: [Patch; 6] = [
    Patch::Basic,
    Patch::TwoOsc,
    Patch::Lfos,
    Patch::Full,
    Patch::Fm,
    Patch::Wavetable,
];

/// The cases, in the order they run.
fn cases() -> Vec<Case> {
    let compare = COMPARE.map(|patch| Case {
        patch,
        voices: COMPARE_VOICES,
    });
    let sweep = SWEEP.map(|voices| Case {
        patch: Patch::Full,
        voices,
    });
    compare.into_iter().chain(sweep).collect()
}

/// A stress test in progress, or done.
#[derive(Default)]
pub struct Test {
    pub results: Vec<Result>,
    /// The cases still to run, next first.
    todo: Vec<Case>,
    total: usize,
}

impl Test {
    pub fn start(&mut self) {
        self.todo = cases();
        self.todo.reverse();
        self.total = self.todo.len();
        self.results.clear();
    }

    pub fn stop(&mut self) {
        self.todo.clear();
    }

    pub fn running(&self) -> bool {
        !self.todo.is_empty()
    }

    /// (cases done, cases in all).
    pub fn progress(&self) -> (usize, usize) {
        (self.total - self.todo.len(), self.total)
    }

    /// Runs the next case, if any. Call once a frame.
    pub fn step(&mut self) {
        let Some(case) = self.todo.pop() else {
            return;
        };
        let r = run(case);
        self.results.push(r);
        // No point making a slow computer grind through bigger ones.
        if self.results.len() > COMPARE.len() && r.load > GIVE_UP {
            self.todo.clear();
        }
    }

    /// The results split into what each part costs and the voice sweep.
    pub fn parts(&self) -> (Vec<Result>, Vec<Result>) {
        let n = self.results.len().min(COMPARE.len());
        (self.results[..n].to_vec(), self.results[n..].to_vec())
    }

    /// About how many voices of the full patch fit with room to spare:
    /// between the two sweep points either side of `SAFE`, or past the
    /// last two if it never got there.
    pub fn fit(&self) -> Option<usize> {
        let (_, sweep) = self.parts();
        if sweep.len() < 2 {
            return None;
        }
        let over = sweep.iter().position(|r| r.load > SAFE);
        let i = over.unwrap_or(sweep.len() - 1).max(1);
        let (a, b) = (sweep[i - 1], sweep[i]);
        let per_voice = (b.load - a.load) / (b.case.voices - a.case.voices) as f64;
        let more = (SAFE - a.load) / per_voice.max(1e-9);
        let fit = (a.case.voices as f64 + more).floor().max(0.0) as usize;
        Some(fit.min(MAX_VOICES))
    }

    /// The results as plain text, for Copy all and the console.
    pub fn report(&self) -> Vec<String> {
        if self.results.is_empty() {
            return Vec::new();
        }
        let line = |name: &str, r: &Result| {
            format!(
                "  {name:<18} {:>3} voices  {:>4.0}%",
                r.case.voices,
                r.load * 100.0
            )
        };
        let (compare, sweep) = self.parts();
        let mut out = vec!["Stress test (load is the share of one core):".to_string()];
        out.extend(compare.iter().map(|r| line(r.case.patch.name(), r)));
        out.extend(sweep.iter().map(|r| line(FULL, r)));
        if let Some(fit) = self.fit() {
            out.push(format!(
                "  About {fit} voices of the full patch fit, keeping {:.0}% spare.",
                (1.0 - SAFE) * 100.0
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn more_voices_take_more_time() {
        let one = run(Case {
            patch: Patch::Full,
            voices: 1,
        });
        let many = run(Case {
            patch: Patch::Full,
            voices: 32,
        });
        assert!(many.load > one.load, "{} > {}", many.load, one.load);
    }

    #[test]
    fn a_test_runs_every_case_once() {
        let mut t = Test::default();
        t.start();
        // Fewer voices; the whole thing is slow in a debug build.
        for c in &mut t.todo {
            c.voices = c.voices.min(4);
        }
        while t.running() {
            t.step();
        }
        assert_eq!(t.results.len(), COMPARE.len() + SWEEP.len());
        assert_eq!(t.parts().0.len(), COMPARE.len());
        assert!(!t.report().is_empty());
    }

    /// The whole test, printed. Run it in a release build:
    /// `cargo test --release stress_report -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn stress_report() {
        let mut t = Test::default();
        t.start();
        while t.running() {
            t.step();
        }
        for line in t.report() {
            println!("{line}");
        }
    }
}
