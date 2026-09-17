//! An open ER document — the editable unit shown in the editor window.
//!
//! A [`Document`] bundles the logical [`Schema`], its fixed [`DialectName`],
//! the canvas [`GraphLayout`], the on-disk [`path`](Document::path), and a
//! [`dirty`](Document::dirty) flag. It replaces the loose `path` +
//! `AppState.selected_path` pair that previously stood in for "the open
//! document" on the old `FrameView` (now removed).
//!
//! - `path == None` means untitled / never saved (the welcome screen is shown
//!   when there is *no* document at all, not when a document has no path).
//! - `dirty` tracks unsaved edits; it drives the title-bar `*` marker and the
//!   "save before close?" prompt.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::db::DialectName;
use crate::model::{GraphLayout, Schema};

/// An open ER document.
#[derive(Clone, Debug)]
pub struct ErDocument {
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

/// On-disk serialization wrapper. A `.sqler` file is JSON carrying a `format`
/// marker and a `version` for future migration. `path` and `dirty` are runtime
/// state and are intentionally not persisted. See memory
/// `document-file-format`.
#[derive(Serialize, Deserialize)]
struct SqlerFile {
    format: String,
    version: u32,
    dialect: DialectName,
    schema: Schema,
    layout: GraphLayout,
}

impl ErDocument {
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

    /// Serialize the document (dialect + schema + layout) to pretty JSON.
    pub fn to_json(&self) -> serde_json::Result<String> {
        let file = SqlerFile {
            format: "sqler".to_string(),
            version: 1,
            dialect: self.dialect,
            schema: self.schema.clone(),
            layout: self.layout.clone(),
        };
        serde_json::to_string_pretty(&file)
    }

    /// Serialize and write the document to `path`. Only the dialect, schema,
    /// and layout are written — `path` and `dirty` never go to disk.
    pub fn save_to_path(&self, path: &Path) -> std::io::Result<()> {
        let json = self.to_json().map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Read and deserialize a `.sqler` file at `path` into a clean document
    /// rooted at that path. The on-disk `format` marker is validated so a
    /// non-SQLER JSON file fails loudly rather than silently loading as an
    /// empty-ish document. Errors are returned as display strings so the Open
    /// action can pass them straight to a notification.
    pub fn open_from_path(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let file: SqlerFile = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        if file.format != "sqler" {
            return Err(format!(
                "Not a SQLER document (format: {})",
                file.format
            ));
        }
        Ok(Self {
            dialect: file.dialect,
            schema: file.schema,
            layout: file.layout,
            path: Some(path.to_path_buf()),
            dirty: false,
        })
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
