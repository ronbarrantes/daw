# daw

A tiny, lightweight DAW in Rust: synth and sample tracks, 8 patterns you can
arrange into a song, and MIDI for hardware like the M-VAVE FM-1 and Roland
AIRA Compacts.

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

- Click a step to turn it on or off.
- Click a track's name to select it and hear it. The right panel shows its
  settings; melodic tracks also get a **piano roll** below the rows.
- **+ Synth** adds a synth track. **+ Sample…** loads WAV, MP3, FLAC or OGG
  files as sample tracks, or just drop files onto the window.
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

- **View** menu, or **F5** song and patterns, **F6** tracks, **F7** piano roll,
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
  one column make a **chord**.
- **Drag a note sideways** to make it longer or shorter (the held part is a
  darker blue). Longer notes let the synth's **Sustain** and **Release** be
  heard. Right-clicking the cell after a note holds it one step longer too.
- **Right-click** a note for its groove settings; **shift-click** for a ghost note.
- Other tracks' notes show faintly behind, to help line parts up (View menu
  to turn off).

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
like a clip in other DAWs), **song** (blocks on the timeline).

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
  Jam.
- Click the **ruler** to set where playback starts; drag along it to mark a
  **loop**, and toggle **Loop** on or off.
- **C** splits the selected block at the cursor (like Premiere's razor),
  **Ctrl+D** duplicates it, **Delete** removes it.
- **Snap** sets what blocks line up to (bar, beat or step); hold **Shift**
  while dragging for single steps. **Ctrl+scroll** or **-** / **+** zoom.

Songs from v0.12 and v0.13 open with each pattern on the timeline turned
into a one-bar block; older songs' Song row becomes a block called *Song*.
Songs saved now need v0.14 or later to open.

### Synth

Select a synth track to see its knobs. Drag a knob up or down; double-click
to reset it. Hover for what each one does.

- **Sound:** **Wave** saw → square. **Detune** thickens. **Humanize**.
- **Filter:** **Cutoff** and **Reso** set the tone; **Env amt** and
  **F decay** make each note "pluck" by opening the filter and closing it.
- **Envelope (ADSR):** **Attack** fades in, **Decay** falls to the
  **Sustain** level while the note is held, **Release** fades out after.

Try: Cutoff down, Reso up, Env amt up, Decay short → squelchy acid bass.

**Scope:** the green screen at the top of the panel is an oscilloscope of
the selected track, so you can watch a knob change the wave. A saw is a
ramp, a square is a square, and the filter rounds off the sharp corners.
Click it to show more or less time. The blue drawing under the envelope
knobs shows the note's volume over time. Turn both off in **View**.

### MIDI

Plug gear in over USB (or a USB-MIDI cable), then press **⟳** to rescan.

- **MIDI out:** sends clock and Start/Stop so gear follows the tempo. Each
  track's **MIDI ch** (right panel) sends its notes on that channel; the
  built-in drums use channel 10 (Kick 36, Snare 38, Hat 42, Clap 39).
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

Next:

1. **MIDI blocks:** a third kind of block, a free-length list of notes
   (any pitch, any length, exact timing and velocity) placed on the
   timeline and played by any track. Comes with real polyphony (a pool of
   voices per track instead of one per note name) and a full multi-octave
   piano roll to draw, move and resize notes, with triplet grids.
2. Import .mid files as MIDI blocks (say, sheet music turned into MIDI)
3. Record a take from a MIDI keyboard or the computer keys into a MIDI block
4. Audio blocks on the timeline: vocals and full tracks, with trim, split,
   fades and clip volume
5. Sampled instruments (SoundFont / SFZ), for real piano and other sounds
6. Recording audio onto the timeline
7. Record a jam (pattern and chain changes) straight into the timeline
8. Time signatures: 3/4, 6/8, 5/4, 13/8 and others, and pattern lengths
   other than 16 steps
9. Clip overrides: mute a track, transpose or change the volume of one
   block without unlinking it
10. Sample editor: chop, trim, ADSR, fades, reverse
11. A full mixer
12. Freesound.org sample browser

See [docs/WORKFLOW.md](docs/WORKFLOW.md) for how changes get pushed, merged and released.
