//! Persisted application settings.
//!
//! Stored as JSON under the OS config dir (`<config_dir>/sqlerapp/settings.json`).
//! On launch [`AppSettings::load`] restores the saved window geometry and theme;
//! on quit [`AppSettings::save`] snapshots the live state back to disk. Loading
//! and saving are best-effort — a missing or malformed file falls back to the
//! defaults rather than failing the app.

use std::path::PathBuf;

use gpui_kit::component::ThemeMode;
use gpui_kit::{App, Bounds, Pixels, Size, WindowBounds};
use serde::{Deserialize, Serialize};

const APP_DIR: &str = "sqlerapp";
const SETTINGS_FILE: &str = "settings.json";

/// The settings persisted across launches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// Saved window geometry, if a window was ever open.
    #[serde(default)]
    pub window: Option<WindowState>,

    /// The color theme mode.
    #[serde(default = "default_theme_mode")]
    pub theme_mode: ThemeMode,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            window: None,
            theme_mode: default_theme_mode(),
        }
    }
}

fn default_theme_mode() -> ThemeMode {
    ThemeMode::Light
}

/// Persisted window geometry: just the size and whether the window was
/// maximized. The position is intentionally not stored — on reopen the window
/// is re-centered on screen. Captured from [`WindowBounds`] (which is not
/// itself `Serialize`) so the geometry round-trips through JSON.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WindowState {
    /// The restore size of the window.
    pub size: Size<Pixels>,
    /// Whether the window was maximized.
    pub maximized: bool,
}

impl WindowState {
    /// Capture the window state from the live [`WindowBounds`].
    pub fn from_window_bounds(bounds: WindowBounds) -> Self {
        let maximized = matches!(bounds, WindowBounds::Maximized(_));
        Self {
            size: bounds.get_bounds().size,
            maximized,
        }
    }

    /// Reconstruct the [`WindowBounds`] for restoring the window on open.
    /// The position is centered on the primary display, since we don't store
    /// it.
    pub fn to_window_bounds(self, cx: &App) -> WindowBounds {
        let bounds = Bounds::centered(None, self.size, cx);
        if self.maximized {
            WindowBounds::Maximized(bounds)
        } else {
            WindowBounds::Windowed(bounds)
        }
    }
}

impl AppSettings {
    /// Load settings from the config file, falling back to defaults on any
    /// error (missing file, parse error, or no config dir available).
    pub fn load() -> Self {
        let Some(path) = settings_path() else {
            return Self::default();
        };
        let Ok(content) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&content).unwrap_or_default()
    }

    /// Write settings to the config file. Returns the error so the caller can
    /// log it; never panics.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = settings_path() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// The [`WindowBounds`] to open the main window with, if any were saved.
    /// Needs the app to center the window on screen.
    pub fn window_bounds(&self, cx: &App) -> Option<WindowBounds> {
        self.window.map(|w| w.to_window_bounds(cx))
    }
}

/// The full path to the settings file, or `None` if the OS provides no
/// config dir.
fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(APP_DIR).join(SETTINGS_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Bounds, Point, Size, px};

    fn bounds_800x600() -> Bounds<Pixels> {
        Bounds {
            origin: Point::new(px(10.), px(20.)),
            size: Size::new(px(800.), px(600.)),
        }
    }

    #[test]
    fn from_window_bounds_captures_size_and_maximized() {
        let bounds = bounds_800x600();
        let maximized = WindowState::from_window_bounds(WindowBounds::Maximized(bounds));
        assert!(maximized.maximized);
        assert_eq!(maximized.size, Size::new(px(800.), px(600.)));

        let windowed = WindowState::from_window_bounds(WindowBounds::Windowed(bounds));
        assert!(!windowed.maximized);
    }

    #[test]
    fn settings_serde_round_trip() {
        let settings = AppSettings {
            window: Some(WindowState {
                size: Size::new(px(800.), px(600.)),
                maximized: true,
            }),
            theme_mode: ThemeMode::Dark,
        };
        let json = serde_json::to_string(&settings).unwrap();
        let back: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.theme_mode, ThemeMode::Dark);
        let window = back.window.expect("window should survive round trip");
        assert!(window.maximized);
        assert_eq!(window.size, Size::new(px(800.), px(600.)));
    }

    #[test]
    fn load_missing_file_falls_back_to_defaults() {
        // No config dir manipulation here; just assert defaults are sane.
        let default = AppSettings::default();
        assert_eq!(default.theme_mode, ThemeMode::Light);
        assert!(default.window.is_none());
    }
}
