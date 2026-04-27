//! Bounded undo / redo stack with optional coalescing.
//!
//! Generic over any cloneable state `T`. Callers snapshot the state
//! *before* mutating it via [`push`](UndoStack::push); [`undo`] swaps the
//! current state with the most recent snapshot (and pushes the supplanted
//! state onto the redo side), [`redo`] does the reverse.
//!
//! Coalescing: when the same logical edit fires many times in a row
//! (e.g. dragging a font-size spinbutton), pass a [`CoalesceKey`] to
//! [`push_coalesced`]. The stack only records the first push of any
//! contiguous run of the same key, so a single Ctrl+Z reverts the whole
//! run rather than just the last increment.

const DEFAULT_CAPACITY: usize = 200;

#[derive(Debug)]
pub struct UndoStack<T> {
    past: Vec<T>,
    future: Vec<T>,
    last_key: Option<CoalesceKey>,
    capacity: usize,
}

/// Identifies a logical edit so consecutive identical keys can collapse
/// into one undo step. Distinct keys (or `None`, set after non-coalescing
/// pushes) always force a fresh entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoalesceKey {
    FontFamily(usize),
    FontSize(usize),
    FontColor(usize),
    StrokeColor(usize),
    StrokeWidth(usize),
    StrokeStyle(usize),
    /// Default-side font edits (no FreeText selected): tweaks to
    /// `current_font` etc. don't appear in undo history at all, but if a
    /// caller wants to coalesce them they can reuse this variant.
    FontDefault,
}

impl<T: Clone> UndoStack<T> {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            past: Vec::new(),
            future: Vec::new(),
            last_key: None,
            capacity: capacity.max(1),
        }
    }

    /// Snapshot a non-coalescing edit. Always records, clears the redo
    /// branch, and resets the coalesce key.
    pub fn push(&mut self, snapshot: T) {
        self.past.push(snapshot);
        if self.past.len() > self.capacity {
            self.past.remove(0);
        }
        self.future.clear();
        self.last_key = None;
    }

    /// Snapshot a coalescable edit. Records only when `key` differs from
    /// the previous coalesced push; otherwise treats this as part of the
    /// same logical edit and skips.
    pub fn push_coalesced(&mut self, key: CoalesceKey, snapshot: T) {
        if self.last_key == Some(key) {
            return;
        }
        self.past.push(snapshot);
        if self.past.len() > self.capacity {
            self.past.remove(0);
        }
        self.future.clear();
        self.last_key = Some(key);
    }

    /// Pop the most recent snapshot. The caller is expected to swap it
    /// into the live state and pass the *displaced* state via
    /// [`push_redo`] so it can be re-applied.
    pub fn pop_undo(&mut self, current: T) -> Option<T> {
        let prev = self.past.pop()?;
        self.future.push(current);
        self.last_key = None;
        Some(prev)
    }

    /// Pop the most recent redo snapshot, mirroring [`pop_undo`].
    pub fn pop_redo(&mut self, current: T) -> Option<T> {
        let next = self.future.pop()?;
        self.past.push(current);
        self.last_key = None;
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    pub fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
        self.last_key = None;
    }
}

impl<T: Clone> Default for UndoStack<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_then_undo_restores_previous_state() {
        let mut s: UndoStack<i32> = UndoStack::new();
        s.push(1);
        s.push(2);
        let cur = 3;
        let restored = s.pop_undo(cur).expect("undo");
        assert_eq!(restored, 2);
    }

    #[test]
    fn redo_inverts_undo() {
        let mut s: UndoStack<i32> = UndoStack::new();
        s.push(1);
        let after_undo = s.pop_undo(2).expect("undo");
        assert_eq!(after_undo, 1);
        let after_redo = s.pop_redo(after_undo).expect("redo");
        assert_eq!(after_redo, 2);
    }

    #[test]
    fn fresh_push_clears_redo_branch() {
        let mut s: UndoStack<i32> = UndoStack::new();
        s.push(1);
        let _ = s.pop_undo(2);
        assert!(s.can_redo());
        s.push(99);
        assert!(!s.can_redo(), "new edit must invalidate redo");
    }

    #[test]
    fn coalesce_collapses_same_key_runs() {
        let mut s: UndoStack<i32> = UndoStack::new();
        s.push_coalesced(CoalesceKey::FontSize(0), 10);
        s.push_coalesced(CoalesceKey::FontSize(0), 11);
        s.push_coalesced(CoalesceKey::FontSize(0), 12);
        // Three calls, but only the first snapshot was recorded.
        let restored = s.pop_undo(13).expect("undo");
        assert_eq!(restored, 10);
        assert!(!s.can_undo());
    }

    #[test]
    fn coalesce_breaks_on_different_key() {
        let mut s: UndoStack<i32> = UndoStack::new();
        s.push_coalesced(CoalesceKey::FontSize(0), 10);
        s.push_coalesced(CoalesceKey::FontFamily(0), 20);
        s.push_coalesced(CoalesceKey::FontSize(0), 30);
        // Three logical edits, three undos.
        assert_eq!(s.pop_undo(40).unwrap(), 30);
        assert_eq!(s.pop_undo(30).unwrap(), 20);
        assert_eq!(s.pop_undo(20).unwrap(), 10);
        assert!(!s.can_undo());
    }

    #[test]
    fn non_coalescing_push_resets_key() {
        let mut s: UndoStack<i32> = UndoStack::new();
        s.push_coalesced(CoalesceKey::FontSize(0), 10);
        s.push(20); // unrelated edit
        s.push_coalesced(CoalesceKey::FontSize(0), 30);
        // Three distinct undos because push() reset the key.
        assert_eq!(s.pop_undo(40).unwrap(), 30);
        assert_eq!(s.pop_undo(30).unwrap(), 20);
        assert_eq!(s.pop_undo(20).unwrap(), 10);
    }

    #[test]
    fn capacity_evicts_oldest() {
        let mut s: UndoStack<i32> = UndoStack::with_capacity(2);
        s.push(1);
        s.push(2);
        s.push(3);
        // Capacity 2 → pushing a 3rd evicts 1.
        assert_eq!(s.pop_undo(4).unwrap(), 3);
        assert_eq!(s.pop_undo(3).unwrap(), 2);
        assert!(!s.can_undo());
    }
}
