//! Undo/redo. Pixel edits are stored as compact before/after patches of the
//! touched rectangle only; structural edits (layers, canvas size) store
//! copy-on-write document snapshots. History is bounded by memory, not by a
//! fixed step count, and every operation (including canvas resizes and
//! selection changes) is undoable.

use crate::doc::{DocSnapshot, Document};
use crate::raster::{IRect, Mask, Rgba};

pub enum Edit {
    Pixels { layer_id: u64, rect: IRect, before: Vec<Rgba>, after: Vec<Rgba> },
    Selection { before: Option<Mask>, after: Option<Mask> },
    Doc { before: Box<DocSnapshot>, after: Box<DocSnapshot> },
    Group(Vec<Edit>),
}

impl Edit {
    fn byte_size(&self) -> usize {
        match self {
            Edit::Pixels { before, after, .. } => (before.len() + after.len()) * 4,
            Edit::Selection { before, after } => {
                before.as_ref().map_or(0, |m| m.data.len()) + after.as_ref().map_or(0, |m| m.data.len())
            }
            // Snapshots share layer pixels with the live document and with each
            // other, so count only one side as an estimate.
            Edit::Doc { after, .. } => after.byte_size() / 2,
            Edit::Group(v) => v.iter().map(Edit::byte_size).sum(),
        }
    }

    fn apply(&self, doc: &mut Document, undo: bool) {
        match self {
            Edit::Pixels { layer_id, rect, before, after } => {
                if let Some(li) = doc.layer_index(*layer_id) {
                    doc.layers[li].img_mut().write_rect(*rect, if undo { before } else { after });
                    doc.mark_dirty(*rect);
                }
            }
            Edit::Selection { before, after } => {
                doc.selection = if undo { before.clone() } else { after.clone() };
                doc.selection_changed = true;
            }
            Edit::Doc { before, after } => {
                doc.restore(if undo { (**before).clone() } else { (**after).clone() });
            }
            Edit::Group(v) => {
                if undo {
                    v.iter().rev().for_each(|e| e.apply(doc, true));
                } else {
                    v.iter().for_each(|e| e.apply(doc, false));
                }
            }
        }
    }
}

pub struct Entry {
    pub label: String,
    pub edit: Edit,
    size: usize,
}

pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    pub budget_bytes: usize,
    /// Bumps on every change so views can notice.
    pub version: u64,
}

impl Default for History {
    fn default() -> Self {
        let budget_bytes = if cfg!(target_arch = "wasm32") { 256 << 20 } else { 1024 << 20 };
        Self { undo: Vec::new(), redo: Vec::new(), budget_bytes, version: 0 }
    }
}

impl History {
    pub fn push(&mut self, label: &str, edit: Edit) {
        let size = edit.byte_size();
        self.undo.push(Entry { label: label.to_string(), edit, size });
        self.redo.clear();
        self.version += 1;
        self.trim();
    }

    /// Like `push`, but merges into the previous step if it has the same
    /// label (used for continuous slider drags).
    pub fn push_coalesce(&mut self, label: &str, edit: Edit) {
        if let (Some(last), Edit::Doc { after, .. }) = (self.undo.last_mut(), &edit)
            && last.label == label && self.redo.is_empty()
                && let Edit::Doc { after: prev_after, .. } = &mut last.edit {
                    *prev_after = after.clone();
                    self.version += 1;
                    return;
                }
        self.push(label, edit);
    }

    fn trim(&mut self) {
        let mut total: usize = self.undo.iter().map(|e| e.size).sum();
        let mut drop = 0;
        while total > self.budget_bytes && drop + 1 < self.undo.len() {
            total -= self.undo[drop].size;
            drop += 1;
        }
        if drop > 0 {
            self.undo.drain(..drop);
        }
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.version += 1;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, doc: &mut Document) -> Option<String> {
        let e = self.undo.pop()?;
        e.edit.apply(doc, true);
        let label = e.label.clone();
        self.redo.push(e);
        self.version += 1;
        doc.modified = true;
        Some(label)
    }

    pub fn redo(&mut self, doc: &mut Document) -> Option<String> {
        let e = self.redo.pop()?;
        e.edit.apply(doc, false);
        let label = e.label.clone();
        self.undo.push(e);
        self.version += 1;
        doc.modified = true;
        Some(label)
    }

    /// Labels of undoable steps (oldest first) and redoable steps (next first).
    pub fn labels(&self) -> (Vec<&str>, Vec<&str>) {
        (
            self.undo.iter().map(|e| e.label.as_str()).collect(),
            self.redo.iter().rev().map(|e| e.label.as_str()).collect(),
        )
    }

    /// Jump so that exactly `n` steps are applied.
    pub fn goto(&mut self, doc: &mut Document, n: usize) {
        while self.undo.len() > n && self.undo(doc).is_some() {}
        while self.undo.len() < n && self.redo(doc).is_some() {}
    }

    pub fn memory_used(&self) -> usize {
        self.undo.iter().chain(self.redo.iter()).map(|e| e.size).sum()
    }
}
