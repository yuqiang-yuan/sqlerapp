//! An open ER document — the editable unit shown in the editor window.
//!
//! A [`Document`] bundles the logical [`Schema`], its fixed [`DialectName`],
//! the canvas [`GraphLayout`], the on-disk [`path`](Document::path), and a
//! [`dirty`](Document::dirty) flag. It replaces the loose `path` +
//! `AppState.selected_path` pair that previously stood in for "the open
//! document" on `FrameView`.
//!
//! - `path == None` means untitled / never saved (the welcome screen is shown
//!   when there is *no* document at all, not when a document has no path).
//! - `dirty` tracks unsaved edits; it drives the title-bar `*` marker and the
//!   "save before close?" prompt.

use std::path::PathBuf;

use crate::db::DialectName;
use crate::model::{GraphLayout, Schema};

/// An open ER document.
#[derive(Clone, Debug)]
pub struct Document {
    /// The document's dialect, fixed at creation and never switched.
    pub dialect: DialectName,
    /// The logical model — source of truth for the canvas.
    pub schema: Schema,
    /// Canvas positions, separate from the logical schema.
    pub layout: GraphLayout,
    /// Where the document was loaded from / will be saved to. `None` means
    /// untitled (new, never saved).
    pub path: Option<PathBuf>,
    /// Whether the document has unsaved changes.
    pub dirty: bool,
}

impl Document {
    /// Create a new untitled document for the given dialect.
    pub fn new(dialect: DialectName) -> Self {
        Self {
            dialect,
            schema: Schema::new(),
            layout: GraphLayout::default(),
            path: None,
            dirty: false,
        }
    }

    /// A display title for the document: the file name, or "Untitled" if it
    /// has never been saved. Prefixes `*` when there are unsaved changes.
    pub fn title(&self) -> String {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".to_string());
        if self.dirty {
            format!("*{}", name)
        } else {
            name
        }
    }

    /// Mark the document as having unsaved changes.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Clear the dirty flag (after a save).
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Whether the document has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}
