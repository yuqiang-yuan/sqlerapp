//! Application actions bound to menu items.
//!
//! Each menu item carries a GPUI [`Action`]; selecting the item dispatches it.
//! Views register `on_action` handlers for these to actually do the work.

use gpui_kit::gpui::Action;

/// Open a file.
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = sqlerapp)]
pub struct Open;

/// Save the current file.
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = sqlerapp)]
pub struct Save;
