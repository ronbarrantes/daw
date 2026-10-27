//! Names for chords: works out what a set of notes is called, like Dm7 or
//! G7/B, for the step rows and the piano roll.
//!
//! Each note set is tried with every one of its notes as the root, against
//! the shapes below. A root that's also the lowest note wins; otherwise
//! the name gets a slash and its bass note (an inversion). The same notes
//! can have two right names (C6 is also Am7/C); the earlier shape in the
//! list is the one picked. Sets that match nothing get no name.

use crate::widgets::NOTE_NAMES;

/// A chord shape: semitones above the root, its short suffix and its name.
struct Shape {
    intervals: &'static [u8],
    short: &'static str,
    long: &'static str,
    /// The fifth (7) may be left out, as players often do in big chords.
    fifth_optional: bool,
}

const fn shape(
    intervals: &'static [u8],
    short: &'static str,
    long: &'static str,
    fifth_optional: bool,
) -> Shape {
    Shape {
        intervals,
        short,
        long,
        fifth_optional,
    }
}

/// Most common first, so it wins when the same notes fit two shapes.
const SHAPES: &[Shape] = &[
    shape(&[0, 4, 7], "", "major", false),
    shape(&[0, 3, 7], "m", "minor", false),
    shape(&[0, 4, 7, 10], "7", "7", true),
    shape(&[0, 3, 7, 10], "m7", "minor 7", true),
    shape(&[0, 4, 7, 11], "M7", "major 7", true),
    shape(&[0, 7], "5", "power chord", false),
    shape(&[0, 5, 7], "sus4", "sus4", false),
    shape(&[0, 2, 7], "sus2", "sus2", false),
    shape(&[0, 3, 6], "dim", "diminished", false),
    shape(&[0, 4, 8], "aug", "augmented", false),
    shape(&[0, 4, 7, 9], "6", "6", false),
    shape(&[0, 3, 7, 9], "m6", "minor 6", false),
    shape(&[0, 3, 6, 10], "m7b5", "half-diminished", false),
    shape(&[0, 3, 6, 9], "dim7", "diminished 7", false),
    shape(&[0, 3, 7, 11], "mM7", "minor-major 7", true),
    shape(&[0, 5, 7, 10], "7sus4", "7 sus4", true),
    shape(&[0, 2, 4, 7], "add9", "add 9", false),
    shape(&[0, 2, 3, 7], "madd9", "minor add 9", false),
    shape(&[0, 4, 8, 10], "7#5", "7 sharp 5", false),
    shape(&[0, 4, 6, 10], "7b5", "7 flat 5", false),
    shape(&[0, 4, 8, 11], "M7#5", "major 7 sharp 5", false),
    shape(&[0, 2, 4, 7, 10], "9", "9", true),
    shape(&[0, 2, 3, 7, 10], "m9", "minor 9", true),
    shape(&[0, 2, 4, 7, 11], "M9", "major 9", true),
    shape(&[0, 1, 4, 7, 10], "7b9", "7 flat 9", true),
    shape(&[0, 3, 4, 7, 10], "7#9", "7 sharp 9", true),
    shape(&[0, 2, 4, 5, 7, 10], "11", "11", true),
    shape(&[0, 2, 3, 5, 7, 10], "m11", "minor 11", true),
    shape(&[0, 2, 4, 7, 9, 10], "13", "13", true),
    shape(&[0, 2, 4, 7, 9], "6/9", "6/9", false),
];

/// A chord's name, short (for a step: "Dm7", "G7/B") and long (for a
/// tooltip: "D minor 7", "G 7 over B").
#[derive(Debug, PartialEq)]
pub struct Chord {
    pub short: String,
    pub long: String,
}

/// The name of the chord these notes make (MIDI note numbers, any order,
/// any octave), if they make a known one. Needs at least two notes.
pub fn name(notes: &[u8]) -> Option<Chord> {
    let bass = *notes.iter().min()?;
    let mut classes: Vec<u8> = notes.iter().map(|n| n % 12).collect();
    classes.sort_unstable();
    classes.dedup();
    if classes.len() < 2 {
        return None;
    }
    let mut best: Option<(usize, bool, u8)> = None;
    for &root in &classes {
        let mut intervals: Vec<u8> = classes.iter().map(|c| (c + 12 - root) % 12).collect();
        intervals.sort_unstable();
        let Some(i) = SHAPES.iter().position(|s| fits(s, &intervals)) else {
            continue;
        };
        let in_bass = root == bass % 12;
        // A root in the bass beats a slash chord; then the commoner shape.
        let better = match best {
            None => true,
            Some((j, j_bass, _)) => (in_bass && !j_bass) || (in_bass == j_bass && i < j),
        };
        if better {
            best = Some((i, in_bass, root));
        }
    }
    let (i, in_bass, root) = best?;
    let s = &SHAPES[i];
    let root_name = NOTE_NAMES[root as usize];
    let mut short = format!("{root_name}{}", s.short);
    let mut long = format!("{root_name} {}", s.long);
    if !in_bass {
        let b = NOTE_NAMES[(bass % 12) as usize];
        short.push_str(&format!("/{b}"));
        long.push_str(&format!(" over {b}"));
    }
    Some(Chord { short, long })
}

/// Whether the intervals (sorted, from the root) are this shape, or this
/// shape without its fifth when that's allowed.
fn fits(s: &Shape, intervals: &[u8]) -> bool {
    if s.intervals == intervals {
        return true;
    }
    if !s.fifth_optional || intervals.contains(&7) {
        return false;
    }
    let without: Vec<u8> = s.intervals.iter().copied().filter(|&i| i != 7).collect();
    without == intervals
}

/// A short name cut to fit a step cell: long names end with a dot so it's
/// clear there's more (shown on hover).
pub fn fit(short: &str, room: usize) -> String {
    if short.chars().count() <= room {
        short.to_string()
    } else {
        let cut: String = short.chars().take(room.saturating_sub(1)).collect();
        format!("{cut}·")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn short(notes: &[u8]) -> String {
        name(notes).map_or("?".into(), |c| c.short)
    }

    #[test]
    fn common_chords_get_their_names() {
        assert_eq!(short(&[60, 64, 67]), "C");
        assert_eq!(short(&[62, 65, 69]), "Dm");
        assert_eq!(short(&[62, 65, 69, 72]), "Dm7");
        assert_eq!(short(&[67, 71, 74, 77]), "G7");
        assert_eq!(short(&[60, 64, 67, 71]), "CM7");
        assert_eq!(short(&[71, 74, 77]), "Bdim");
        assert_eq!(short(&[60, 65, 67]), "Csus4");
        assert_eq!(short(&[48, 55]), "C5");
        assert_eq!(short(&[59, 63, 67, 70]), "BM7#5");
        assert_eq!(short(&[60, 64, 67, 70, 74]), "C9");
        assert_eq!(short(&[60, 64, 70]), "C7", "no fifth");
        let long = name(&[62, 65, 69, 72]).unwrap().long;
        assert_eq!(long, "D minor 7");
    }

    #[test]
    fn inversions_get_a_slash_and_odd_sets_no_name() {
        assert_eq!(short(&[64, 67, 72]), "C/E");
        assert_eq!(short(&[59, 62, 65, 67]), "G7/B");
        assert_eq!(name(&[59, 62, 65, 67]).unwrap().long, "G 7 over B");
        assert_eq!(short(&[60, 61, 62]), "?", "a cluster has no name");
        assert_eq!(short(&[60, 72]), "?", "one note in two octaves");
    }

    #[test]
    fn long_names_are_cut_to_fit() {
        assert_eq!(fit("Dm7", 4), "Dm7");
        assert_eq!(fit("BM7#5", 4), "BM7·");
    }
}
