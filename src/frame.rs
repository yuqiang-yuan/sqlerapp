use std::path::PathBuf;

use gpui_kit::base::{StyledExt, h_resizable, resizable_panel};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::label::Label;
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{ActiveTheme, IconName, Theme, ThemeMode};
use gpui_kit::component::TitleBar;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

#[cfg(not(target_os = "macos"))]
use gpui_kit::base::GlobalState;

use crate::actions::{New, Open, Quit, Save};
use crate::db::DialectName;
use crate::document::Document;
use crate::settings::WindowState;

/// Application-wide state. Holds the active editor view so the global action
/// listeners (which only receive `&mut App`) can reach back into `FrameView`
/// to mutate the open document.
pub struct AppState {
    /// The currently active editor view, if any.
    pub frame: Option<Entity<FrameView>>,
    /// Latest window geometry snapshot, refreshed every render. Bounds
    /// changes trigger `bounds_changed` → `refresh` → render, so this stays
    /// current. Read at quit time, because by then the window has already
    /// been removed from `cx.windows()` (on `LastWindowClosed` platforms the
    /// window is dropped before `on_app_quit` fires).
    pub last_window_state: Option<WindowState>,
}

impl Global for AppState {}

pub struct FrameView {
    focus_handle: FocusHandle,
    menu_bar: Entity<AppMenuBar>,
    /// The open document. `None` means no document is open — the welcome
    /// screen is shown in its place. A document with `path == None` is an
    /// untitled new document and **does** show the editor.
    document: Option<Document>,
    /// Kept alive so the theme-change observer stays registered for the view's
    /// lifetime. `Theme::change` writes the Base-layer `Theme` global via
    /// `set_global`, which notifies this observer; we then `notify()` so the
    /// view re-renders (the toggle button swaps its sun/moon icon and any
    /// stale hover/active element state is rebuilt).
    _theme_observer: Subscription,
}

impl FrameView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        Self::set_menus(cx);
        let menu_bar = AppMenuBar::new(cx);

        // `Theme::change` (used by the title-bar toggle) writes the Base-layer
        // `Theme` global with `set_global`, which fires this. Re-rendering the
        // view here — rather than relying on `window.refresh()` alone — ensures
        // the toggle button rebuilds with the new icon and a clean hit/hover
        // state, so repeated clicks register reliably.
        let _theme_observer =
            cx.observe_global::<gpui_kit::base::Theme>(|_view, cx| cx.notify());

        Self {
            focus_handle,
            menu_bar,
            document: None,
            _theme_observer,
        }
    }

    fn set_menus(cx: &mut App) {
        register_actions(cx);

        #[cfg(target_os = "macos")]
        {
            let app_menu = Menu::new("sqlerapp").items(vec![
                MenuItem::os_submenu("Services", gpui::SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit", Quit),
            ]);
            let file_menu = Menu::new("File").items(vec![
                MenuItem::action("New", New),
                MenuItem::action("Open", Open),
                MenuItem::action("Save", Save),
            ]);
            cx.set_menus(vec![app_menu, file_menu]);
        }

        #[cfg(not(target_os = "macos"))]
        {
            let file_menu = Menu::new("File").items(vec![
                MenuItem::action("New", New),
                MenuItem::action("Open", Open),
                MenuItem::action("Save", Save),
            ]);
            GlobalState::global_mut(cx).set_app_menus(vec![file_menu.owned()]);
        }
    }

    fn on_open(_: &Open, cx: &mut App) {
        println!("open action");
        // For now: placeholder flow. Once the file-open dialog + JSON load are
        // implemented, this will read the `.sqler` file and replace the
        // document on the active frame.
        cx.spawn(async move |cx| {
            let path: PathBuf = open_file_dialog().await;
            let _ = cx.update(|cx| {
                let _ = with_frame(cx, |frame, cx| {
                    frame.open_document(path, cx);
                });
            });
        })
        .detach();
    }

    /// `New` handler — create a new untitled document and switch to the
    /// editor. Defaults to the MySQL dialect for now; a dialect picker will
    /// replace this once the New dialog is designed.
    fn on_new(_: &New, cx: &mut App) {
        println!("new action");
        let _ = with_frame(cx, |frame, cx| {
            frame.new_document(cx);
        });
    }

    /// `Save` handler.
    fn on_save(_: &Save, cx: &mut App) {
        println!("save action");
        let _ = with_frame(cx, |frame, _cx| {
            if frame.document.is_some() {
                // TODO: serialize to .sqler JSON and write to `document.path`.
                println!("saving document");
            } else {
                println!("nothing to save");
            }
        });
    }

    fn on_quit(_: &Quit, cx: &mut App) {
        cx.quit();
    }

    /// Create a new untitled document and enter the editor.
    fn new_document(&mut self, cx: &mut Context<Self>) {
        // TODO: let the user pick the dialect. MySQL is the placeholder.
        self.document = Some(Document::new(DialectName::MySql));
        cx.notify();
    }

    /// Load a document from a `.sqler` file path. TODO: parse the JSON.
    fn open_document(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // TODO: read + deserialize the .sqler JSON into a Document.
        let mut doc = Document::new(DialectName::MySql);
        doc.path = Some(path);
        self.document = Some(doc);
        cx.notify();
    }
}

/// Register the app actions as global listeners, so the native menu bar
/// validates them as available (and dispatches them) regardless of focus.
fn register_actions(cx: &mut App) {
    if cx.has_global::<ListenerRegistered>() {
        return;
    }
    cx.set_global(ListenerRegistered);
    if !cx.has_global::<AppState>() {
        cx.set_global(AppState {
            frame: None,
            last_window_state: None,
        });
    }

    cx.on_action(FrameView::on_open);
    cx.on_action(FrameView::on_new);
    cx.on_action(FrameView::on_save);
    cx.on_action(FrameView::on_quit);
}

/// Marker global so action listeners are registered exactly once.
struct ListenerRegistered;
impl Global for ListenerRegistered {}

/// Run `f` against the active frame view, if there is one.
fn with_frame(cx: &mut App, f: impl FnOnce(&mut FrameView, &mut Context<FrameView>)) -> Result<(), ()> {
    let frame = cx.global::<AppState>().frame.clone();
    let Some(frame) = frame else {
        return Err(());
    };
    frame.update(cx, f);
    Ok(())
}

impl Render for FrameView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Register this view as the active frame so global action listeners
        // can reach it. Cheap to repeat each render; set_global replaces.
        cx.global_mut::<AppState>().frame = Some(cx.entity());

        // Snapshot the live window geometry. Bounds changes drive
        // `bounds_changed` → `refresh` → render, so this stays current and is
        // available at quit time (when the window is already gone from
        // `cx.windows()`).
        cx.global_mut::<AppState>().last_window_state =
            Some(WindowState::from_window_bounds(_window.window_bounds()));

        let has_document = self.document.is_some();

        div()
            .v_flex()
            .size_full()
            .child(
                TitleBar::new()
                    .when(cfg!(not(target_os = "macos")), |title_bar| {
                        title_bar.child(self.menu_bar.clone())
                    })
                    .child(
                        // `ml_auto` pins this to the right edge of the bar, just
                        // left of the window controls (min/max/close).
                        div().ml_auto().child(theme_toggle_button(cx)),
                    ),
            )
            .child(if has_document {
                editor_panel()
            } else {
                welcome_screen()
            })
    }
}

/// The editor layout shown when a document is open: a horizontally resizable
/// left/right split with an empty status bar pinned to the bottom.
fn editor_panel() -> Div {
    div()
        .v_flex()
        .flex_1()
        .child(
            // Wrap the split so its `size_full` resolves against a bounded,
            // shrinkable height (the group itself is not `Styled`).
            div()
                .flex_1()
                .min_h(px(0.))
                .child(
                    h_resizable("editor-split")
                        .child(
                            resizable_panel()
                                .size(px(300.))
                                .flex_none() // hold 300px; still drag-resizable
                                .child(left_pane()),
                        )
                        .child(resizable_panel().child(right_pane())),
                ),
        )
        .child(StatusBar::new().left("Ready"))
}

/// Placeholder for the left pane (e.g. canvas / table list). Empty for now.
fn left_pane() -> Div {
    div().size_full()
}

/// Placeholder for the right pane (e.g. inspector / properties). Empty for now.
fn right_pane() -> Div {
    div().size_full()
}

/// A title-bar button that toggles between the light and dark themes. Shows a
/// sun in dark mode (click → light) and a moon in light mode (click → dark).
fn theme_toggle_button(cx: &App) -> Button {
    let is_dark = cx.theme().is_dark();
    Button::new("theme-toggle")
        .ghost()
        .icon(if is_dark { IconName::Sun } else { IconName::Moon })
        .tooltip(if is_dark { "Switch to light theme" } else { "Switch to dark theme" })
        .on_click(|_, window, cx| {
            let mode = if cx.theme().is_dark() {
                ThemeMode::Light
            } else {
                ThemeMode::Dark
            };
            Theme::change(mode, Some(&mut *window), cx);
        })
}

/// The welcome screen shown when no document is open: a centered app title
/// with New and Open buttons.
fn welcome_screen() -> Div {
    div()
        .flex()
        .size_full()
        .items_center()
        .justify_center()
        .child(
            div()
                .v_flex()
                .items_center()
                .gap_4()
                .child(
                    Label::new("sqlerapp")
                        .text_2xl()
                        .font_weight(FontWeight::BOLD),
                )
                .child(
                    div()
                        .mt_4()
                        .h_flex()
                        .gap_3()
                        .child(
                            Button::new("welcome-new")
                                .label("New")
                                .primary()
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(New), cx)
                                }),
                        )
                        .child(
                            Button::new("welcome-open")
                                .label("Open")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(Open), cx)
                                }),
                        ),
                ),
        )
}

/// Placeholder for the file-open dialog.
async fn open_file_dialog() -> PathBuf {
    PathBuf::from("/tmp/example.sqler")
}
