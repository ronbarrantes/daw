# Roadmap

What's being built now, what's planned, and what's done. Every PR updates
this file: tick off what it finishes, and drop or reword anything that's no
longer the plan.

## Now: the new layout

Worked out with mockups on a design canvas. Always in view: the top bar,
the tracks (name, volume, M, S, steps) and a status line. Everything else
is a drawer that opens when you need it and folds to one line.

1. [x] **Instrument drawer** (v0.56). The instrument panel moves from the
   right edge to a drawer along the bottom, its sections side by side; I or
   F8 opens and shuts it. A status line along the bottom shows errors and
   the audio load.
2. [x] **Track rows** (v0.57). Volume in each row by the name, a level
   meter after the steps, and a Mix toggle that adds pan, reverb and delay
   columns. The piano roll opens under its own track. Bars sits next to
   the pattern picker.
4. [x] **Slimmer** (v0.59). The first pass moved panels without shrinking
   anything, so it got busier. This one makes it fit the Air with no
   scrolling: one row along the top (menus and transport; MIDI moves to a
   Keys & MIDI menu), small square steps, 20 px rows, a 10 px piano roll,
   smaller knobs, no helper text in the drawer, and the drawer's controls
   on one line so six sections fit across.
5. [x] **Like the mockups** (v0.60). Built to match the chosen flow on the
   design canvas: patterns, Bars and a display (position, tempo, swing) in
   the top bar; rows with a color dot, name, volume, M S, two bars of steps
   side by side in the track's color, and a meter; the drawer's sections as
   boxes with a drawing and one row of knobs; the shut drawer shows small
   drawings of the sound; a flat dark look.
6. [x] **Things stay put** (v0.61). Hidden parts fold to a line with a
   ▾/▸ switch instead of vanishing; clicking a synth always opens its
   piano roll; the drawer's header, the tracks' header and the tools stay
   in view while only what's under them scrolls; scroll areas fill the
   window; patterns scroll sideways when there are many.
7. [x] **Tracks things on the tracks** (v0.62). The patterns, Length and
   what Plays move from the top bar to the tracks' top line, so they sit
   by what they change; the top bar is just for playing. Steps stretch to
   fill the window, the piano roll's keys reach its grid, and the meters
   and Mix sit at the right edge.
3. [x] **Themes** (v0.58). View > Theme: follow the computer, dark, light,
   a black-and-white LED look, and a black-and-orange hardware look.

## Then: synth modules

Holding a drone and turning knobs is the heart of the app, so this comes
first. The plan came out of reading what people find easy and painful in
popular synths (Vital, Serum, Drift, Minilogue: you see the sound move, and
drag an LFO onto a knob; Surge, Massive X, Wavestate: clutter, clicks and
menus).

1. [x] **Modules** (v0.33). The synth is now modules (Oscillators, Voice,
   Filter, Envelope, LFO, Arpeggiator) that all follow one contract: their
   knobs, how they're laid out, and the code that reads them. Same sound,
   same knobs; old songs load the same. A new module gets a panel section,
   pop-out, saving and automation for free. How to write one:
   [MODULES.md](MODULES.md).
2. [x] **Drag-and-drop modulation** (v0.34). Drag a source (LFO 1, LFO 2,
   the Mod envelope, Velocity) onto any knob; a ring in its colour shows
   how far it moves the knob, a dot shows where it is now, right-click to
   change or remove it. LFO 1 keeps its own "LFO to" too. Next for it: a
   preview while hovering, and dragging the ring to set the amount.
3. [ ] **New modules**, each on its own:
   - [x] Pluck and Mallet engines (v0.35): a plucked string
     (Karplus-Strong) and a struck bar, glass or bell (modal), picked with
     Voice > Engine, with presets. Written from scratch, small and light.
   - [x] FM engine (v0.36): two operators with ratio, depth, a decay that
     mellows each note, and feedback; four presets
   - [x] Wavetable engine (v0.37): four built-in tables (Basic, Vocal,
     Hollow, Metal) with Position and Sub, band-limited per octave;
     presets that come with modulation on Position
   - [x] Visuals (v0.38): a filter curve (with where modulation has it
     now), each LFO's shape with a moving dot, and a spectrum view
   - [x] Mutate (v0.39): nudge knobs by an amount, with sections left
     alone; Keep A / Keep B and a slider that morphs between them. Later:
     an X/Y pad to morph between four sounds, and saving A and B with the
     song.
4. [ ] **Sound packs.** A file with presets, patches built from daw's own
   modules and visual styles. Import it and it works; share it or keep a
   library of them. (This is also "save your own presets".)

## Next

- **Your own wavetables:** right now the Wavetable engine has four
  built-in tables. Next: draw a few waves of your own and Position morphs
  between them, then open Serum/Vital-style `.wav` wavetables (thousands
  free online).
- **More drums:** more built-in synth drums (rimshot, ride, crash, 808
  kick and others) for the drum browser, made in code like the rest.
- **Your own themes:** a small plain-text file, like Ghostty's config
  (`~/.config/daw/theme` on Mac and Linux, the matching folder on
  Windows), with one `name = color` per line: background, panels, text,
  accent, the step and track colors. Anything left out keeps the built-in
  color, so a theme can be three lines. It shows up in View > Theme, and
  the app picks up changes to the file while it runs. On the web, load a
  theme file instead (no folders there).

## Later

- **DX7 patches:** open DX7 `.syx` banks (32 sounds each, thousands free
  online) and play them on a new 6-operator FM engine, likely from the
  Rust port of Mutable Instruments' Plaits 1.2 (MIT), which has one. Read
  the files, add the engine, then a bank browser and a few knobs over the
  whole patch. Check its weight on the Air first.
- **Code packs:** modules made by anyone, loaded from a file, using
  WebAssembly (a sealed box: works on desktop and web, can't touch files or
  crash the app). Tools like Faust could make them. Test speed on the Air
  first.
- **Patch view:** a full page of modules wired together, e.g. for
  generative music (a random source, a quantizer that keeps it on a scale,
  a clock, chance), recorded into a MIDI block.
- Import .mid files as MIDI blocks (say, sheet music turned into MIDI)
- Record a take from a MIDI keyboard or the computer keys into a MIDI block
- Record a jam (pattern and chain changes) straight into the timeline
- Audio blocks on the timeline: vocals and full tracks, with trim, split,
  fades and clip volume
- Recording audio onto the timeline
- Sampled instruments (SoundFont / SFZ), for real piano and other sounds
  modules can't fake
- Time signatures: 3/4, 6/8, 5/4, 13/8 and others, and bars other than
  16 steps
- Linked parts ("components"): one track's part, or a group's, shared by
  several patterns, so editing it once changes it everywhere
- Clip overrides: mute a track, transpose or change the volume of one block
  without unlinking it
- Sample editor: chop a sample into slices
- A full mixer
- Freesound.org sample browser
- Fine tune: shift a whole track slightly off the usual notes, in cents
- Other tunings and scales: quarter tones, just intonation or your own; the
  piano roll and keyboard follow
- Phone mode: the synth panel and an on-screen keyboard, for iPhone
- Plugin hosting (CLAP) on desktop, maybe

## Done

Newest first, by release.

- v0.62.2: steps a modest size (no stretching, spare room on the right),
  the meter and Mix right after the steps, no Level label, and meters
  that show even when silent
- v0.62.1: the Tool (Draw, Erase) sits over the track names, and + Synth,
  + Drum and + Sample under the last track, so they read as the tracks'
- v0.62: patterns, Length and Plays on the tracks' top line; steps stretch
  to fill the window; meters and Mix at the right edge
- v0.61: things stay put: ▾/▸ switches on Tracks, Piano roll, Chain and
  Instrument (nothing needs a key to come back); clicking a synth opens its
  piano roll; pinned headers and tools; full-width scrolling
- v0.60.2: the guides read through for the new layout: a "The screen" map
  at the top of F1 and the README, and the View menu's names match
- v0.60.1: a drawer section's whole title line folds and unfolds it, with
  a triangle showing which (clicks on the name were being eaten)
- v0.60: the layout from the design canvas: patterns and Bars in the top
  bar, colored track rows with two bars side by side, the drawer's
  sections as boxes, a shut drawer that sketches the sound, a flat look
- v0.59: slimmer: menus and transport in one row, small square steps,
  compact rows and piano roll, smaller knobs, a one-line drawer header;
  Jam fits the Air without scrolling
- v0.58: View > Theme: the computer's light or dark, LED (white on black)
  or Hardware (black and orange)
- v0.57: each track row has its volume and a level meter, and Mix adds
  pan, reverb and delay; the piano roll opens under its track; Bars sits
  next to the patterns
- v0.56: the instrument is a drawer along the bottom (I or F8), its
  sections side by side; a status line shows errors and the audio load
- v0.55.2: the LFOs sit by the sound engine and Mod envelope by the
  envelope in the panel; the drawer's arrows are triangles like the rest
- v0.55.1: the Mac app builds on GitHub's current macOS runners (the old
  ones stopped, so v0.55.0 had no release); includes v0.55.0's Bend
- v0.55: Bend and Bend time (Voice): every note slides its pitch by a set
  amount, a pitch envelope
- v0.54: sample Fade out: a trimmed hit dies away instead of stopping dead
- v0.53: the instrument panel slides shut to a strip on the right edge
  and open again, like a drawer
- v0.52: a pitch wheel on the computer keyboard (turn on Bend): hold
  1 / 2 to bend the held notes down / up
- v0.51.1: the Jam view's editor moved into its own module; UI smoke
  tests click through menus and track actions headless
- v0.51: Help > Diagnostics: audio load, dropouts, device and a log of
  recent errors (also in a small file on desktop), with Copy all
- v0.50.2: copies of a song (undo steps, the engine's snapshot) share
  unchanged bars, so long patterns don't eat memory
- v0.50.1: fix: right-clicking the piano roll could add or drag a note
  instead of opening its menu or holding the note before; a right-click
  may drift a little before it counts as a drag
- v0.50: Drone slider on its own wide row with Notes / Free and semitone
  steps; Play a note is now a small Tap; selecting a track is silent
- v0.49: ties across the bar line in multi-bar patterns
- v0.48: salsa drums (timbales, cascara, guiro) and a Grand piano preset
  made from the string engine
- v0.47.1: fix: deleting a track from its right-click menu crashed the
  app when tracks came after it
- v0.47: patterns up to 8 bars, with each track its own length (shorter
  parts repeat); multi-bar patterns fill a chain square per bar
- v0.46: View > Low power: redraw less often while playing, to save
  battery
- v0.45.2: README: Hyprland is a window manager, not a distro
- v0.45.1: the Velocity chip moves to the Voice section
- v0.45: piano rolls up to 4 octaves tall; recording keeps the octave
- v0.44: drag piano-roll notes to move them; fix: the arpeggiator kept
  playing erased or ended tied chords; shortening a note added a new one
- v0.43.2: fix: deleting the pattern being edited crashed the app
- v0.43.1: roadmap: your own wavetables and more drums are next
- v0.43: searchable drum sound browser; swap sounds while keeping patterns
- v0.42.1: instrument panel and synth UI extracted into their own module;
  controls, sound and saved layouts behave as before
- v0.42: Click wave; the Drone goes from 0.1 to 20,000 Hz
- v0.41.2: drawings sit above the knobs
- v0.41.1: smaller pop-out arrows
- v0.41: presets reworked for the new engines, in groups, with modulation
  and velocity
- v0.40.1: an icon for the web version on the iPad home screen
- v0.40: sustain pedal; LFO retrigger
- v0.39.2: fix: the computer keys play chords and feed the arpeggiator
- v0.39.1: fix: the popped-out scope window kept growing
- v0.39: Mutate, and A/B sounds with a morph slider
- v0.38: filter curve, LFO shapes and a spectrum view
- v0.37: Wavetable engine; presets can bring their own modulation
- v0.36: FM engine (electric piano, bells, basses, glassy keys)
- v0.35: Pluck and Mallet engines (strings, marimba, vibes, glass, bells)
- v0.34: drag-and-drop modulation; LFO 2 and the Mod envelope
- v0.33: the synth is made of modules (same sound); new Voice section
- v0.32: pop any panel section out into its own window
- v0.31: automation: draw knob changes over the song
- v0.30: sample trim, fade in, release and reverse
- v0.29: duplicate and reorder tracks
- v0.28: tool bar: Draw, Select and Erase
- v0.27: chord names on steps
- v0.26: arp latch; chords and scales from one note
- v0.25: master volume, level meter and limiter
- v0.24: new tracks start silent; Play a note and Drone on top; note length
- v0.23: arpeggiator
- v0.22: synth presets
- v0.21: free pitch (Drone slider, theremin pad); closable panel sections
- v0.20: foldable instrument panel, pop-out scope, LFO on pan
- v0.19: two oscillators from sine to noise, LFO, mono/legato, glide, drone
- v0.18: drag to paint and erase steps and notes
- v0.17: clave, bongos, congas, toms, shaker, cowbell; + Drum
- v0.16: MIDI blocks with a full piano roll
- v0.15: real polyphony, 16 voices per track
- v0.14: Jam and Arrange views, with chains and blocks
- v0.13: drag notes longer or shorter; oscilloscope and envelope drawing
- v0.12: timeline with linked pattern clips and a pattern library
- v0.11: Windows build; the web version works in Safari
- v0.10: web version
- v0.9: undo and redo
- v0.8: shared reverb and delay
- v0.7: metronome and count-in
- v0.6: pan per track, stereo mix
- v0.5: export songs as WAV and MIDI; .daw song files
- v0.4: save and open songs
- v0.3: Mac app bundle, Linux installer and app icon
- v0.2: chords, routed MIDI inputs, GitHub builds and releases
- v0.1: drum step sequencer, MIDI clock and notes for the FM-1 and AIRA
  Compacts, built-in synth with knobs, sample tracks, groove, groups,
  patterns and song mode
