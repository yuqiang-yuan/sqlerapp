//! Application actions bound to menu items.
//!
//! Each menu item carries a GPUI [`Action`]; selecting the item dispatches it.
//! These are registered as *global* action listeners (see [`crate::frame`]),
//! so the macOS native menu bar treats them as always-available (enabled)
//! regardless of window focus, matching the pattern in GPUI's own `set_menus`
//! example.


use std::path::PathBuf;

use gpui_kit::gpui::Action;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::db::DialectName;

/// About.
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = sqlerapp)]
pub struct About;

/// Open a file.
///
/// `path == None` opens the platform file dialog (the Open menu item).
/// `path == Some` opens that path directly — used by the recent-files list on
/// the welcome screen, which dispatches one entry per click.
#[derive(Action, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[action(namespace = sqlerapp)]
pub struct Open {
    pub path: Option<PathBuf>,
}

/// Create a new (untitled) document.
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = sqlerapp)]
pub struct New;

#[derive(Action, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[action(namespace = sqlerapp)]
pub struct NewDialogConfirmed {
    pub dialect_name: DialectName,
}

/// Save a file.
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = sqlerapp)]
pub struct Save;

/// Quit the application (macOS App menu).
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = sqlerapp)]
pub struct Quit;

/// Edit → Undo. Not yet backed by a history system; the menu item is wired
/// so the shortcut and menu entry exist, with a placeholder handler.
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = sqlerapp)]
pub struct Undo;

/// Edit → Redo. See [`Undo`].
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = sqlerapp)]
pub struct Redo;

/// Edit → New Table. Inserts an empty, auto-named table into the current
/// document's schema and refreshes the objects list.
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = sqlerapp)]
pub struct NewTable;

/// Edit → New Relationship. Not yet implemented; the menu item is wired as a
/// placeholder so the shortcut exists, with a notification handler.
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = sqlerapp)]
pub struct NewRelationship;
