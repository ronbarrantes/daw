//! Undo and redo, by keeping whole copies of the song.
//!
//! A song is small (samples are shared `Arc`s), so copies are cheap. Edits
//! are grouped into steps: a knob drag or a burst of typing is one step,
//! settled once the mouse button is up and no text field has focus.

/// How many steps back you can go.
const LIMIT: usize = 100;

pub struct Undo<T> {
    /// States before each step, oldest first.
    done: Vec<T>,
    /// States undone, most recently undone last.
    undone: Vec<T>,
    /// The state when the last step settled.
    base: T,
    /// Something changed since `base` that isn't a step yet.
    pending: bool,
}

impl<T: Clone> Undo<T> {
    pub fn new(base: T) -> Self {
        Self {
            done: Vec::new(),
            undone: Vec::new(),
            base,
            pending: false,
        }
    }

    /// Forgets all history, e.g. when another song is opened.
    pub fn reset(&mut self, base: T) {
        *self = Self::new(base);
    }

    /// Notes that the state has changed; `settle` makes it a step.
    pub fn changed(&mut self) {
        self.pending = true;
    }

    /// Turns pending changes into one undo step.
    pub fn settle(&mut self, current: &T) {
        if self.pending {
            let before = std::mem::replace(&mut self.base, current.clone());
            self.done.push(before);
            if self.done.len() > LIMIT {
                self.done.remove(0);
            }
            self.undone.clear();
            self.pending = false;
        }
    }

    pub fn can_undo(&self) -> bool {
        self.pending || !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.pending && !self.undone.is_empty()
    }

    /// The state before the last step, if there is one.
    pub fn undo(&mut self, current: &T) -> Option<T> {
        self.settle(current);
        let before = self.done.pop()?;
        self.undone.push(current.clone());
        self.base = before.clone();
        Some(before)
    }

    /// The state the last undo went back from, if nothing changed since.
    pub fn redo(&mut self, current: &T) -> Option<T> {
        if self.pending {
            return None;
        }
        let after = self.undone.pop()?;
        self.done.push(current.clone());
        self.base = after.clone();
        Some(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Makes an edit the way the app does: change, then settle.
    fn edit(u: &mut Undo<i32>, state: &mut i32, to: i32) {
        *state = to;
        u.changed();
        u.settle(state);
    }

    #[test]
    fn undo_and_redo_walk_the_steps() {
        let mut state = 0;
        let mut u = Undo::new(state);
        assert!(!u.can_undo());
        edit(&mut u, &mut state, 1);
        edit(&mut u, &mut state, 2);
        state = u.undo(&state).unwrap();
        assert_eq!(state, 1);
        state = u.undo(&state).unwrap();
        assert_eq!(state, 0);
        assert_eq!(u.undo(&state), None, "nothing left");
        state = u.redo(&state).unwrap();
        state = u.redo(&state).unwrap();
        assert_eq!(state, 2);
        assert_eq!(u.redo(&state), None);
    }

    #[test]
    fn a_new_edit_clears_redo() {
        let mut state = 0;
        let mut u = Undo::new(state);
        edit(&mut u, &mut state, 1);
        state = u.undo(&state).unwrap();
        edit(&mut u, &mut state, 5);
        assert!(!u.can_redo());
        assert_eq!(u.undo(&state), Some(0));
    }

    #[test]
    fn unsettled_changes_are_one_step() {
        // Like a knob drag: many changes before the mouse comes up.
        let mut state = 0;
        let mut u = Undo::new(state);
        for v in 1..=10 {
            state = v;
            u.changed();
        }
        assert!(u.can_undo(), "undo works mid-drag too");
        assert_eq!(u.undo(&state), Some(0), "all the way back at once");
        assert_eq!(u.redo(&0), Some(10));
    }

    #[test]
    fn history_is_limited() {
        let mut state = 0;
        let mut u = Undo::new(state);
        for v in 1..=LIMIT as i32 + 20 {
            edit(&mut u, &mut state, v);
        }
        let mut steps = 0;
        while let Some(s) = u.undo(&state) {
            state = s;
            steps += 1;
        }
        assert_eq!(steps, LIMIT);
        assert_eq!(state, 20, "the oldest steps are dropped");
    }
}
