//! Drag-and-drop modulation: a source (an LFO, the mod envelope, how hard
//! a note was played) connected to a knob, with an amount. A knob can
//! have several; they add up.

use super::{Values, PARAMS};

/// Something that can move knobs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Lfo1,
    Lfo2,
    ModEnv,
    Velocity,
}

impl Source {
    pub const ALL: [Source; 4] = [Source::Lfo1, Source::Lfo2, Source::ModEnv, Source::Velocity];

    /// Its name, shown on its chip and saved in songs.
    pub fn name(self) -> &'static str {
        match self {
            Source::Lfo1 => "LFO 1",
            Source::Lfo2 => "LFO 2",
            Source::ModEnv => "Mod env",
            Source::Velocity => "Velocity",
        }
    }

    pub fn named(name: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|s| s.name() == name)
    }

    /// Its slot in a `[f32; 4]` of source values.
    pub fn index(self) -> usize {
        self as usize
    }

    /// LFOs swing both ways (-1 to 1); the others go 0 to 1.
    pub fn bipolar(self) -> bool {
        matches!(self, Source::Lfo1 | Source::Lfo2)
    }
}

/// One connection: `source` moves knob `knob` (by index into `PARAMS`)
/// by up to `amount` of the knob's full sweep (-1 to 1).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Link {
    pub source: Source,
    pub knob: usize,
    pub amount: f32,
}

/// The knobs `base` with `links` applied, given each source's value now
/// (`sources[Source::index]`). Each link moves its knob along the knob's
/// sweep (so a log knob like Cutoff moves by ratio), within its range.
pub fn apply(base: &Values, links: &[Link], sources: [f32; 4]) -> Values {
    let mut out = *base;
    let mut moved = [0.0_f32; PARAMS.len()];
    for l in links.iter().filter(|l| l.knob < PARAMS.len()) {
        moved[l.knob] += l.amount * sources[l.source.index()];
    }
    for (k, m) in moved.iter().enumerate() {
        if *m != 0.0 {
            let p = &PARAMS[k];
            out[k] = p.at_unit((p.to_unit(base[k]) + m).clamp(0.0, 1.0));
        }
    }
    out
}

/// Whether any link uses `source`.
pub fn uses(links: &[Link], source: Source) -> bool {
    links.iter().any(|l| l.source == source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{CUTOFF, PARAMS, RESONANCE};

    #[test]
    fn links_move_knobs_along_their_sweep_and_add_up() {
        let base = PARAMS.map(|p| p.default);
        let link = |source, knob, amount| Link {
            source,
            knob,
            amount,
        };
        let links = [
            link(Source::Lfo1, CUTOFF, 0.5),
            link(Source::ModEnv, CUTOFF, 0.25),
            link(Source::Velocity, RESONANCE, -1.0),
        ];
        let still = apply(&base, &links, [0.0; 4]);
        assert_eq!(still, base, "sources at rest move nothing");
        let moved = apply(&base, &links, [0.2, 0.0, 1.0, 1.0]);
        let unit = |k: usize, v: &Values| PARAMS[k].to_unit(v[k]);
        let want = unit(CUTOFF, &base) + 0.5 * 0.2 + 0.25;
        assert!((unit(CUTOFF, &moved) - want).abs() < 1e-4, "adds up");
        assert_eq!(moved[RESONANCE], PARAMS[RESONANCE].min, "stays in range");
    }
}
