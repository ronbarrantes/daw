# daw

A tiny, lightweight DAW in Rust: a step sequencer with built-in drums,
synths and sample tracks, patterns chained into blocks, MIDI blocks with a
full piano roll, a timeline to arrange songs, and MIDI for hardware like the
M-VAVE FM-1 and Roland AIRA Compacts.

## Download

| Windows | Mac (Apple Silicon) | Linux |
| --- | --- | --- |
| **[daw-windows-x86_64.zip](https://github.com/ronbarrantes/daw/releases/latest/download/daw-windows-x86_64.zip)** | **[daw-macos-arm64.app.zip](https://github.com/ronbarrantes/daw/releases/latest/download/daw-macos-arm64.app.zip)** | **[daw-linux-x86_64.tar.gz](https://github.com/ronbarrantes/daw/releases/latest/download/daw-linux-x86_64.tar.gz)** |

Install steps for each are below. Or [build it yourself](#build-it-yourself).

> **No install? Try the [web version](https://ronbarrantes.github.io/daw/).**
> It's the full app in your browser: Chrome, Edge, Firefox or Safari (even on
> iPhone). [More about it below](#use-it-in-a-browser).

## Install on Windows

1. Download **[daw-windows-x86_64.zip](https://github.com/ronbarrantes/daw/releases/latest/download/daw-windows-x86_64.zip)**.
2. Right-click it, choose **Extract All…**, then **Extract**.
3. In the new **daw** folder, double-click **daw.exe**.
4. The first time, Windows may say **"Windows protected your PC"**, because
   the app isn't signed with a paid developer certificate. Click **More
   info**, then **Run anyway**. You only need to do this once per update.

To keep it handy, right-click **daw.exe** and choose **Pin to Start** (or
**Show more options → Send to → Desktop** for a shortcut). To update,
download the new zip and replace the folder.

## Install on Mac (Apple Silicon)

1. Download **[daw-macos-arm64.app.zip](https://github.com/ronbarrantes/daw/releases/latest/download/daw-macos-arm64.app.zip)**
   and open it to unzip (Safari may do this for you). You get **daw.app**.
2. Drag **daw.app** into **Applications**.
3. Open it. The first time, macOS says it can't check the developer, because
   the app isn't signed with a paid Apple developer account. To allow it:
   - **macOS 15 and later:** click **Done**, then open **System Settings →
     Privacy & Security**, scroll down to the message about daw and click
     **Open Anyway**.
   - **macOS 14 and earlier:** Control-click **daw.app**, choose **Open**,
     then **Open** again.
   - **Or, in Terminal:** `xattr -dr com.apple.quarantine /Applications/daw.app`

   You only need to do this once per update.

## Install on Linux (Arch, Hyprland and others)

```sh
curl -L https://github.com/ronbarrantes/daw/releases/latest/download/daw-linux-x86_64.tar.gz | tar xz
./daw/install.sh
```

This puts `daw` in `~/.local/bin` and adds an icon and a menu entry, so it
shows up in app launchers (wofi, rofi, fuzzel and so on). No sudo needed;
you only need `alsa-lib`, which any desktop already has.

- **Update:** run the same two lines again.
- **Uninstall:** `./daw/install.sh --uninstall`
- **Hyprland:** the window class is `daw`, e.g.
  `windowrulev2 = float, class:^(daw)$`

## Other downloads

The links above get the newest numbered version. Every merge to `main` also
updates a **latest** build with the newest changes; all builds are on the
[Releases](https://github.com/ronbarrantes/daw/releases) page.

## Build it yourself

### Setup (once)

**Arch Linux**

```sh
sudo pacman -S --needed rustup base-devel alsa-lib
rustup default stable
```

**macOS**

```sh
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then open a new terminal.

**Windows**

Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
with **Desktop development with C++**, then [rustup](https://rustup.rs).
Use Git Bash or PowerShell for the commands below.

### Run

```sh
git clone https://github.com/ronbarrantes/daw.git
cd daw
cargo run --release
```

The first build takes a few minutes. After that it's fast.

## Use it in a browser

**[ronbarrantes.github.io/daw](https://ronbarrantes.github.io/daw/)** runs
the same app in your browser, nothing to install. It's updated with every
merge to `main`. It works in Chrome, Edge, Firefox and Safari, including
Safari on iPhone.

It works like the desktop app, with a few differences:

- **Sound starts on your first click or key press** (browsers require it).
- **Save** (Ctrl+S) downloads the `.daw` file; **Open…** uploads one. The
  same files open in the desktop app and vice versa.
- **Export WAV / MIDI** download the file.
- Your song is **kept in the browser** and is back when you return, even
  without saving. Songs with long samples may be too big for that (about
  5 MB); then use Save.
- **MIDI gear** works in Chrome and Edge (they'll ask for permission), not
  in Safari.
- **If the app doesn't respond to clicks**, a browser extension is probably
  in the way. Try a private (Incognito) window, then turn extensions off one
  at a time to find which.
- Ctrl+N opens a new browser window instead; use **File → New song**.
- On older or slower computers, the downloadable app runs much better.

To build it yourself: `rustup target add wasm32-unknown-unknown`,
`cargo install wasm-bindgen-cli` (the version in `Cargo.lock`), then
`./web/build.sh` and serve `web/dist`, e.g. `python3 -m http.server -d web/dist`.

## Use

Press **F1** (or **Help → How to use**) for the built-in guide.

- **Space** or **▶ Play** starts and stops. Drag **BPM** to change the tempo.
- **Master** (top bar) is the volume of the whole song. The meter next to
  it shows the output level, green to yellow to red. A **limiter** keeps
  the mix from ever clipping; **LIMIT** lights up when it's working hard,
  a sign to turn Master or some tracks down a little. Exports get the same
  volume and limiter.
- **Ctrl+Z** (Cmd+Z on Mac) undoes; **Ctrl+Shift+Z** or **Ctrl+Y** redoes.
  Also in the **Edit** menu. A whole knob drag is one step.

### Saving songs

- **File → Save** (Ctrl+S, Cmd+S on Mac) saves a `.daw` file. **Save as…**
  (Ctrl+Shift+S) picks a new name; **Open…** (Ctrl+O) opens one.
- Samples you loaded are saved inside the file, so songs work on any computer.
- The song you had open reopens next time you start the app.
- A dot in the window title means unsaved changes; daw asks before anything
  would lose them.

A `.daw` file is a zip (like `.docx`) holding `song.json`, which is readable
and editable by hand, plus `samples/*.wav` for any loaded samples.

### Exporting

- **File → Export WAV…** saves the sound, ready to share or post.
- **File → Export MIDI…** saves the notes as a standard MIDI file for other
  DAWs or hardware synths. Each MIDI channel becomes its own track; drums
  go on channel 10, and synths without a channel get a free one.
- Both export the **timeline**; if it's empty, the current block's chain,
  or else the current pattern four times. Muted tracks are left out.

### Tracks

Each row is a track: its name, **M** (mute), **S** (solo) and 16 steps.
Tracks work like layers: mute turns one off, and solo plays only the
soloed tracks so you can hear a part on its own.

- The **Tool** bar (next to + Synth) picks what clicks and drags do:
  **Draw** adds notes and turns steps on or off, **Erase** removes them.
  In MIDI blocks there's also **Select**. On a touch screen the tools do
  what Ctrl, Alt and right-click do with a mouse.
- Click a step to turn it on or off. **Drag along a row** to paint steps
  on (starting on an empty step) or erase them (starting on a lit one),
  like a pencil and an eraser: a full row of hi-hats is one swipe.
- Click a track's name to select it and hear it. The right panel shows its
  settings; melodic tracks also get a **piano roll** below the rows.
- **+ Synth** adds a synth track. **+ Drum** adds a built-in drum: kick,
  snare, hat, clap, clave, bongos, congas, toms, shaker or cowbell (it goes
  in the Drums group if there is one). **+ Sample…** loads WAV, MP3, FLAC
  or OGG files as sample tracks, or just drop files onto the window.
- Right-click a track's name to delete it.
- Rename a track by editing its name at the top of the right panel.
- **Mix** (right panel): **Volume**, **Pan** (left or right in the stereo
  mix; double-click to centre), and **Reverb** and **Delay**: how much of
  the track goes to the shared effects.

### Effects

Every track can send to one shared reverb and one delay. Turn up a
track's **Reverb** or **Delay** knob, then press **FX** in the top bar to
shape them:

- **Reverb:** **Size** (how long it rings) and **Tone** (muffled to airy).
- **Delay:** **Time** between echoes in 16th notes (3/16, a dotted eighth,
  is a classic) and **Feedback** (how many echoes). Echoes bounce between
  left and right and follow the tempo.

The demo's pad has some reverb on it.

### Groups

Groups are folders of tracks, like layer groups in Photoshop. The demo puts
the four drums in a **Drums** group.

- **Ctrl-click** (Cmd on Mac) track names to pick several, then **Ctrl+G**.
  Or right-click a track name for *Group*, *Move to group* and *Remove from group*.
- **▶/▼** collapses or expands a group; collapsed, it shows all its hits on
  one line.
- The group's **M** / **S** mute or solo all its tracks. Click its name for a
  group volume.

### Showing and hiding

Hiding a part never deletes it or stops it playing.

- **View** menu, or **F5** chain and patterns, **F6** tracks, **F7** piano roll,
  **F8** instrument panel.
- **View → Focus** shows just the tracks, or just the piano roll and sound.
- **F9** shows everything again.

### Groove

- **Shift-click** a step for a ghost note (a quiet hit). Quieter steps look dimmer.
- **Right-click** a step for its settings: **Velocity** (Ghost / Normal /
  Accent), **Chance** (plays only sometimes), **Ratchet** (repeats 2–4 times
  in the step, like a roll) and **Nudge** (a little early or late).
- **Swing** (top bar) delays every other 16th note; 66% is a triplet shuffle.
- **Humanize** (right panel) adds small random changes to a track's timing
  and loudness.

All of this goes out over MIDI too, with velocity.

### Piano roll

- Click a cell to add a note; click it again to remove it. Several notes in
  one column make a **chord**. Steps show the chord's name (Cm, G7, Dm7,
  C/E for an inversion); long ones end with a dot (BM7·). Hover a step for
  the full name, and the piano roll's title shows the chord under the
  pointer or playing.
- **Drag from an empty cell** along a row to paint notes; **Ctrl-drag**
  (Cmd on Mac) erases them.
- **Drag a note sideways** to make it longer or shorter (the held part is a
  darker blue). Longer notes let the synth's **Sustain** and **Release** be
  heard. Right-clicking the cell after a note holds it one step longer too.
- **Right-click** a note for its groove settings; **shift-click** for a ghost note.
- Other tracks' notes show faintly behind, to help line parts up (View menu
  to turn off).
- Each track plays up to 16 notes at once, so long releases, chords and
  notes played from a keyboard can overlap. Past that, the oldest note
  gives way.

### Play from your computer keyboard

No MIDI keyboard needed: your laptop keys play the selected track.

```
 W E   T Y U   O P        ← black keys
A S D F G H J K L         ← white keys (A = C)
```

- **Z** / **X** move down or up an octave.
- Turn on **⏺ Rec** and press Play: notes you play are written into the
  pattern at the step that's playing. Play first clicks one bar (the
  count-in) so you can come in on time.
- **Click** (top bar) turns on the metronome: a click on every beat, higher
  on beat 1.

### Sample tracks

- **Pitched off:** plays as recorded, like a drum hit.
- **Pitched on:** each note plays the sample higher or lower, like an
  instrument. C at octave 4 is the original pitch.

On Arch, the **+ Sample…** file picker needs `xdg-desktop-portal` and
`xdg-desktop-portal-gtk`. Drag and drop always works.

### Jam and Arrange

The app has two halves, picked with the **Jam** and **Arrange** tabs at the
top left:

- **Jam** is the groovebox: make patterns, chain them into blocks, shape
  sounds. It's the home screen.
- **Arrange** is the timeline, where blocks are put together into a song.
  Later it will also hold audio (vocals, guitars) and recording.

The units, from small to big: **pattern** (one bar of steps), **chain**
(patterns bar by bar), **block** (a chain you can place on the timeline;
like a clip in other DAWs), **song** (blocks on the timeline). A **MIDI
block** is the other kind of block: free notes for one track, made in
Arrange.

### Patterns (Jam)

The **Patterns** row is your pattern library. Click a pattern to edit it in
the grid below; each holds every track's steps.

- **+** adds a pattern (up to 64). Right-click one to **rename**,
  **duplicate** or **delete** it.
- **Copy** / **Paste** / **Clear** work on the pattern you're editing.
- **Pattern** mode (top bar) loops the pattern you're editing.

### Chains and blocks (Jam)

The **Chain** row shows the block you're editing, one square per bar.

- Click a square to put the selected pattern in that bar; click it again
  (or right-click) to empty it.
- **Chain** mode (top bar) plays the chain bar by bar, round and round.
- Pick another block from the drop-down, **+** starts an empty one, and
  **Save as new block** copies this chain into a new block to keep working
  on. Right-click the drop-down to rename, duplicate or delete the block.

### Timeline (Arrange)

Arrange plays the timeline, from the cursor.

- **Drag a block** from the **Blocks** drawer onto the timeline, or
  **double-click** the timeline to place the current block. Rows are just
  for layout: blocks on different rows play together.
- **Drag** a block to move it; drag its **edges** to trim or stretch it. A
  block stretched past its length repeats. **Alt-drag** makes a copy.
- Blocks on the timeline are **linked**, like Figma instances: edit the
  block in Jam and every copy changes (a copy that plays it once through
  grows or shrinks with it). Right-click a copy and choose **Make unique**
  to give it its own block. Copies of the block you're editing are brighter.
- **Double-click** a block (on the timeline or in the drawer) to edit it in
  Jam. A MIDI block opens in the note editor below instead, with one click.
- Click the **ruler** to set where playback starts; drag along it to mark a
  **loop**, and toggle **Loop** on or off.
- **C** splits the selected block at the cursor (like Premiere's razor),
  **Ctrl+D** duplicates it, **Delete** removes it.
- **Snap** sets what blocks line up to (bar, beat or step); hold **Shift**
  while dragging for single steps. **Ctrl+scroll** or **-** / **+** zoom.

Songs from v0.12 and v0.13 open with each pattern on the timeline turned
into a one-bar block; older songs' Song row becomes a block called *Song*.
Songs saved now need v0.14 or later to open (v0.16 or later if they have
MIDI blocks).

### MIDI blocks (Arrange)

A MIDI block is free notes for one track: any pitch, any length, starting
anywhere, each with its own velocity. Good for melodies, bass lines and
chords that don't fit a one-bar pattern.

- **+ MIDI** in the Blocks drawer makes one for the selected track. Click a
  MIDI block (in the drawer or on the timeline) to open its notes below.
- **Plays** picks the track; **Bars** sets how long it is. Drawing past the
  end makes it longer by itself.
- The **Tool** bar at the left of its toolbar: **Draw** (click or drag empty
  space to add notes), **Select** (click notes or drag a box around them)
  and **Erase** (click or drag over notes to remove them). With any tool,
  drag a note to move it and its **end** to make it longer or shorter.
- With a mouse, shortcuts work whatever the tool: **right-click** deletes a
  note, **right-drag** erases, **Ctrl-drag** draws, **Alt-drag** copies.
- **Ctrl-drag** (Cmd on Mac) draws notes along the way, one per grid
  cell, like a pencil; **right-drag** erases every note it passes.
- **Drag empty space** to select notes; **Shift-click** adds or removes one.
  **Alt-drag** copies. **Ctrl+A** selects all, **Delete** deletes,
  **Ctrl+D** duplicates, the **arrows** move them (Shift: by octave or bar),
  **Esc** deselects.
- **Snap** goes from 1/4 to 1/32, with **1/8 and 1/16 triplets**, or Off;
  hold **Shift** while dragging to go off the grid.
- **Length** sets how long new notes are, from 1/16 to 4 bars (Grid: one
  snap step), and changes the selected notes to it. Handy for long notes
  for the arpeggiator, and on a touch screen where dragging a note's end
  is fiddly.
- **Velocity** sets how loud the selected notes are (or new ones).
- Scroll to move around, **Ctrl+scroll** or **-** / **+** to zoom.

### Synth

Select a synth track to see its knobs in the **instrument panel** on the
right (F8 shows or hides it). Drag a knob up or down; double-click to reset
it. Hover for what each one does. To keep scrolling down:

- **Click a section's heading** (Scope, Oscillators, Filter, Envelope, LFO,
  Mix, Theremin) to fold it; click again to unfold it.
- **×** on a heading closes the section completely. **+ Section** (at the
  bottom of the panel) or **View → Instrument panel sections** brings it
  back. Closed sections are remembered next time you open the app.

- **Preset:** shows which preset the knobs match (Custom once you've
  changed one). Pick a starting sound: Piano (synth), Electric piano, Guitar
  pluck, Bass, Sub bass, Acid bass, Strings, Violin lead, Brass, Organ,
  Flute, Bell, Warm pad or Shaker (noise). They're synth versions of the
  instruments; every knob can be adjusted from there, and Ctrl+Z undoes it.
- **Oscillators:** two of them. **Wave 1** and **Wave 2** each go from
  **sine** (smooth) through **triangle**, **saw** (buzzy) and **square**
  (hollow) to **noise** (hiss); in between blends two shapes. **Level 2**
  is how loud the second one is (0 = just the first), **Pitch 2** moves it
  in semitones (+12 an octave up, +7 a fifth), **Detune** pulls it slightly
  out of tune to thicken the sound.
- **Voices:** **Poly** plays chords. **Mono** plays one note at a time.
  **Legato** plays one at a time, and a note played while another is held
  just changes pitch without restarting. **Glide** slides between notes in
  Mono and Legato (portamento). **Humanize** varies timing and loudness.
- **Arpeggiator** (folded; click it open): plays held notes one at a time
  in a pattern instead of together. **Arp** picks the pattern (**Up**,
  **Down**, **Up/Down**, **Random**, **As played**, or **Off** for normal
  chords), **Arp rate** how often (1/4 to 1/32, with triplets, in time with
  the tempo), **Octaves** how many octaves it climbs, **Arp gate** how long
  each note lasts. It works on anything held: keys, a MIDI controller, long
  notes in a MIDI block, tied notes in the piano roll, or the Drone.
  **Arp notes** picks what it plays: **Held** (the notes you hold), or a
  chord (**Major**, **Minor**, **7th**, **Maj7**, **Min7**, **Sus4**) or
  scale (**Major scale**, **Minor scale**, **Pentatonic**) built on the
  lowest held note, so one finger plays a whole pattern. **Latch** keeps
  it going after you let go, until you play a new chord or turn Latch off.
- **Play a note** and **Drone** are at the top of the panel. Play a note
  plays one short note. **Drone** plays a note nonstop, so you can turn
  knobs and hear the sound change; click again to stop. Its slider sets the pitch anywhere, sliding
  smoothly between notes too (it shows the Hz and the nearest note). For a
  drone in a song, put one long note in a MIDI block.
- **Theremin** (folded at the bottom of the panel; click it open): hold the
  mouse down on the pad and move. Left to right slides the pitch smoothly
  over four octaves, C2 to C6, with no steps; bottom to top opens the
  filter. Let go to stop.
- **Filter:** **Cutoff** and **Reso** set the tone; **Env amt** and
  **F decay** make each note "pluck" by opening the filter and closing it.
- **Envelope (ADSR):** **Attack** fades in, **Decay** falls to the
  **Sustain** level while the note is held, **Release** fades out after.
- **LFO:** a slow wobble. **LFO to** picks what it moves: **Pitch**
  (vibrato), **Filter** (wah), **Volume** (tremolo) or **Pan** (the sound
  sweeps left and right, called auto-pan). **LFO wave** is its
  shape; **Random** jumps to a new value each cycle (on pitch, by whole
  semitones: random tones). **LFO amt** is how much (0 = off), **LFO rate**
  how fast, and **LFO sync** ties the speed to the tempo instead (1/4 = once
  a beat).

Try:
- Cutoff down, Reso up, Env amt up, Decay short: squelchy acid bass. Add
  Legato and some Glide for a 303-style slide.
- Wave 1 on Noise, Level 2 at 0, Cutoff high, Attack short, Decay and
  Sustain low: a shaker or hi-hat.
- LFO to Pitch, LFO wave Random, LFO amt about 50%, LFO sync 1/16: random
  bleeps.
- Wave 2 on Sine, Pitch 2 at -12: a sub-bass under the main sound.
- LFO to Pan, LFO sync 1/4: the sound bounces left and right on the beat.

**Scope:** the green screen at the top of the panel is an oscilloscope of
the selected track, so you can watch a knob change the wave. A saw is a
ramp, a square is a square, and the filter rounds off the sharp corners.
Click it to show more or less time. **Pop out** moves it into its own
window you can drag anywhere and make as big as you like; close that window
to put it back. The blue drawing under the envelope knobs shows the note's
volume over time. Turn both off in **View**.

### MIDI

Plug gear in over USB (or a USB-MIDI cable), then press **⟳** to rescan.

- **MIDI out:** sends clock and Start/Stop so gear follows the tempo. Each
  track's **MIDI ch** (right panel) sends its notes on that channel; the
  built-in drums use channel 10 with General MIDI notes (Kick 36, Snare 38,
  Hat 42, Clap 39, Clave 75, Bongos 60/61, Congas 63/64, Toms 50/47/45,
  Shaker 70, Cowbell 56).
- **MIDI in:** click it to connect controllers, several at once. Each plays
  the **selected track** (follows your clicks) or a fixed **track** or
  **group**. Route a drum kit to the Drums group and each pad plays its
  matching drum. With **⏺ Rec** on and playing, what you play is written into
  the pattern, chords included.
- **Follow MIDI clock:** the incoming clock sets the tempo, and the other
  device's Start/Stop controls playback.

## Roadmap

Done:

- ~~Drum step sequencer~~
- ~~MIDI clock and notes for the FM-1 and AIRA Compacts~~
- ~~Built-in synth with knobs~~
- ~~Patterns and song arrangement~~
- ~~Add/remove tracks, per-track volume, sample tracks~~
- ~~Groove: swing, velocity/ghost notes, humanize, chance, ratchets, nudge; solo~~
- ~~Track groups, View menu and focus mode~~
- ~~Chords, other tracks' notes in the piano roll, routed MIDI inputs with recording~~
- ~~Save and load songs~~
- ~~Export to WAV and MIDI~~
- ~~Pan, reverb and delay, metronome, undo~~
- ~~Web version, Windows build~~
- ~~Timeline, pattern library, Jam and Arrange views with chains and blocks~~
- ~~Real polyphony: 16 voices per track~~
- ~~MIDI blocks with a full piano roll and triplet grids~~
- ~~Clave, bongos, congas, toms, shaker and cowbell; + Drum~~
- ~~Drag to paint and erase steps and notes~~
- ~~Synth: two oscillators from sine to noise, LFO, mono/legato, glide, drone~~
- ~~Foldable instrument panel, pop-out scope, LFO on pan~~
- ~~Free pitch: Drone slider and theremin pad; closable panel sections~~
- ~~Synth presets: piano, basses, strings, brass, organ, bell, pad and more~~
- ~~Arpeggiator~~
- ~~Master volume, level meter and limiter~~
- ~~Arp latch, and chords and scales from one note~~
- ~~Chord names on steps~~
- ~~Tool bar: Draw, Select and Erase~~

Next:

1. Import .mid files as MIDI blocks (say, sheet music turned into MIDI)
2. Record a take from a MIDI keyboard or the computer keys into a MIDI block
3. Audio blocks on the timeline: vocals and full tracks, with trim, split,
   fades and clip volume
4. Sampled instruments (SoundFont / SFZ), for real piano and other sounds
5. Recording audio onto the timeline
6. Record a jam (pattern and chain changes) straight into the timeline
7. Time signatures: 3/4, 6/8, 5/4, 13/8 and others, and pattern lengths
   other than 16 steps
8. Clip overrides: mute a track, transpose or change the volume of one
   block without unlinking it
9. Sample editor: chop, trim, ADSR, fades, reverse
10. A full mixer
11. Freesound.org sample browser
12. Mod matrix: connect any source (LFOs, envelopes, velocity) to any knob,
    with an amount, e.g. an LFO on reverb or a second LFO on the filter
13. Patch view: a full page of modules chained together for generative
    music, e.g. a random source, a quantizer that keeps it on a scale, a
    clock that sets how often it picks a note, a range clamp and chance;
    then record the result for as long as you like into a MIDI block
14. Fine tune: shift a whole track slightly off the usual notes, in cents
15. Other tunings and scales: 24 notes per octave (quarter tones), just
    intonation or your own, and switching between them and free pitch;
    the piano roll and keyboard follow
16. Save your own synth presets, and share them between songs

See [docs/WORKFLOW.md](docs/WORKFLOW.md) for how changes get pushed, merged and released.
