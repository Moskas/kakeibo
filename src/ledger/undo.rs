use super::model::Journal;
use super::ops::{Splice, apply};

#[derive(Debug, Default)]
pub struct UndoStack {
  undo: Vec<Splice>,
  redo: Vec<Splice>,
}

impl UndoStack {
  /// Apply an edit and record it.
  pub fn perform(&mut self, journal: &mut Journal, splice: Splice) {
    apply(journal, &splice);
    self.undo.push(splice);
    self.redo.clear();
  }

  pub fn undo(&mut self, journal: &mut Journal) -> bool {
    let Some(s) = self.undo.pop() else {
      return false;
    };
    apply(journal, &s.inverse());
    self.redo.push(s);
    true
  }

  pub fn redo(&mut self, journal: &mut Journal) -> bool {
    let Some(s) = self.redo.pop() else {
      return false;
    };
    apply(journal, &s);
    self.undo.push(s);
    true
  }

  pub fn clear(&mut self) {
    self.undo.clear();
    self.redo.clear();
  }

  /// Number of edits since the stack was last cleared, used to track the dirty state.
  pub fn depth(&self) -> usize {
    self.undo.len()
  }
}
