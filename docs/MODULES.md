# Synth modules

The built-in synth is made of modules. Each one is a small piece of code
that follows the same contract, so a new module (a Pluck engine, an FM
engine, a visual, an effect) plugs in without touching the rest of the app.

## What's there now

| Module | File | What it does |
| --- | --- | --- |
| Voice | `src/synth/play.rs` | the Engine, Poly / Mono / Legato, Glide, the synth's volume |
| Oscillators | `src/synth/osc.rs` | the Analog engine: two oscillators, sine to noise and click |
| Pluck | `src/synth/pluck.rs` | the Pluck engine: a plucked string (Karplus-Strong) |
| Mallet | `src/synth/mallet.rs` | the Mallet engine: struck bar, glass or bell (modal) |
| FM | `src/synth/fm.rs` | the FM engine: two sine operators, with feedback |
| Wavetable | `src/synth/wavetable.rs` | the Wavetable engine: built-in tables, band-limited (built by `warm_up` before the engine starts) |
| Filter | `src/synth/filter.rs` | resonant low-pass with its own decay envelope |
| Envelope (ADSR) | `src/synth/envelope.rs` | each note's volume over time |
| LFO | `src/synth/lfo.rs` | a wobble on pitch, filter, volume or pan (one per track) |
| LFO 2 | `src/synth/lfo2.rs` | a second wobble, only a modulation source |
| Mod envelope | `src/synth/modenv.rs` | a second envelope per voice, only a modulation source |
| Arpeggiator | `src/synth/arp.rs` | knobs only; the engine (`src/audio.rs`) runs it |

`src/synth/mod.rs` lists them in `MODULES` and wires one voice together
(`Voice`).

## The contract

A module file has:

1. **Its knobs, numbered from 0**, as constants (`pub const CUTOFF: usize = 0;`).
2. **A `ModuleInfo`** called `INFO`:
   - `name`: the heading of its section in the instrument panel;
   - `knobs`: a `Param` per knob (name, range, default, log or linear,
     unit). **Knob names must be unique across all modules**, because song
     files save knobs by name. A test checks this.
   - `rows`: how the panel lays the knobs out, as rows of knob numbers. A
     knob in no row is hidden but can still be automated.
   - `open`: whether its section starts unfolded.
   - `engine`: for a sound engine, which of `play::ENGINES` it is; its
     section only shows while that engine is picked.
3. **For modules that make or shape sound**, a struct holding one voice's
   state, implementing `Module::set_knobs(&mut self, knobs, sr)`: it gets
   just its own knobs, numbered as in its `ModuleInfo`, and works out
   anything it needs (coefficients and so on) there, not per sample.

What a module gets for free, once it's in `MODULES`:

- its knobs in every synth track (`PARAMS`, `Values`), with defaults;
- saving and loading in songs (by knob name; old songs without the knobs
  get the defaults);
- a panel section that folds, closes and pops out into its own window;
- automation lanes for its knobs (all but the pick-one knobs);
- modulation: any source chip can be dragged onto its knobs (all but the
  pick-one knobs).

## Adding a module

1. Write `src/synth/<name>.rs` with the knobs, `INFO` and (if it makes or
   shapes sound) its struct.
2. Add `mod <name>;` and `&<name>::INFO` to `MODULES` in
   `src/synth/mod.rs`.
3. If it makes or shapes sound, add its struct to `Voice` and call it in
   `Voice::set_params` and `Voice::next_sample`.
4. Write a test that it makes the sound it should.
5. Update the README's *Synth* section, the F1 guide (`src/help.rs`) and
   [ROADMAP.md](ROADMAP.md).

A new engine also needs a name in `play::ENGINES` (and the Engine knob's
range), and a branch in `Voice::note_on` and `Voice::next_sample`.

## Modulation

`src/synth/modulation.rs` has the sources (`Source`: LFO 1, LFO 2, Mod
env, Velocity) and the links (`Link`: a source, a knob, an amount), which
live on each track (`Track::mods`) and are saved by name. Every
`UPDATE_INTERVAL` samples the engine works out each sounding voice's knobs
with the links applied (`modulation::apply`) and gives them to the voice
(`Voice::set_params`), and publishes the selected track's for the knobs to
draw. A new source needs a `Source` variant, a colour (`widgets::source_color`),
a chip in its module's section (`section` in `main.rs`) and its value in
`Engine::apply_modulation`.

## Rules for the sound code

These come from the audio thread (see `src/audio.rs`):

- **Never allocate, lock or block** while making sound. A module that
  needs memory (a delay line for a plucked string, say) gets it up front:
  a fixed-size array in its struct, made when the voice is made.
- Work per sample, and do the expensive maths in `set_knobs`.
- Keep it light: up to 16 voices per track have to run on a 2012 dual-core
  MacBook Air.

## Keeping the sound the same

When the synth was split into modules (v0.33), `src/synth/legacy.rs` kept
a copy of the old code, and a test plays every preset (plus odd settings)
through both and checks they match exactly, sample for sample. When
restructuring sound code again, do the same: keep the old code beside the
new until a test shows they match.
