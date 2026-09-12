//! Application actions bound to menu items.
//!
//! Each menu item carries a GPUI [`Action`]; selecting the item dispatches it.
//! These are registered as *global* action listeners (see [`crate::frame`]),
//! so the macOS native menu bar treats them as always-available (enabled)
//! regardless of window focus, matching the pattern in GPUI's own `set_menus`
//! example.

use gpui_kit::gpui::Action;

/// About.
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = sqlerapp)]
pub struct About;

/// Open a file.
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = sqlerapp)]
pub struct Open;

/// Create a new (untitled) document.
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = sqlerapp)]
pub struct New;

/// Save a file.
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = sqlerapp)]
pub struct Save;

/// Quit the application (macOS App menu).
#[derive(Action, Clone, PartialEq, Eq)]
#[action(namespace = sqlerapp)]
pub struct Quit;
