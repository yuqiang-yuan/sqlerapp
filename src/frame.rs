use std::path::PathBuf;

use gpui_kit::base::StyledExt;
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::component::TitleBar;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

#[cfg(not(target_os = "macos"))]
use gpui_kit::base::GlobalState;

use crate::actions::{Open, Quit, Save};

/// Application-wide state, so global action listeners (registered without a
/// view in scope) can read/write the selected path — the same place the
/// `FrameView` reads from when rendering.
#[derive(Default)]
pub struct AppState {
    pub selected_path: Option<PathBuf>,
}

impl Global for AppState {}

pub struct FrameView {
    focus_handle: FocusHandle,
    menu_bar: Entity<AppMenuBar>,
}

impl FrameView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        Self::set_menus(cx);
        let menu_bar = AppMenuBar::new(cx);

        Self {
            focus_handle,
            menu_bar,
        }
    }

    /// Define the top-level application menus and register the action handlers.
    ///
    /// On macOS the menus are handed to GPUI's native `App::set_menus`, which
    /// populates the screen-top menu bar (`NSMainMenu`). On Windows/Linux the
    /// app draws its own in-window menu bar via `GlobalState::set_app_menus`,
    /// read by `AppMenuBar` in the `TitleBar`.
    ///
    /// The `Open`/`Save`/`Quit` handlers are registered as *global* action
    /// listeners with `cx.on_action`. The macOS native menu bar validates each
    /// item with `is_action_available`, which returns `true` when a matching
    /// global listener exists — so the items stay enabled regardless of window
    /// focus. (`cx.listener`, by contrast, only registers a handler on a view's
    /// focus path, which the native-menu validation does not reliably hit.)
    fn set_menus(cx: &mut App) {
        register_actions(cx);

        #[cfg(target_os = "macos")]
        {
            // The first menu is the macOS Application menu (bold, titled with the
            // app name); AppKit treats whatever comes first as the App menu, so
            // we give it the app name and the conventional app-level items.
            // `File` follows as a normal menu.
            let app_menu = Menu::new("sqlerapp").items(vec![
                MenuItem::os_submenu("Services", gpui::SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit", Quit),
            ]);
            let file_menu = Menu::new("File").items(vec![
                MenuItem::action("Open", Open),
                MenuItem::action("Save", Save),
            ]);
            cx.set_menus(vec![app_menu, file_menu]);
        }

        #[cfg(not(target_os = "macos"))]
        {
            let file_menu = Menu::new("File").items(vec![
                MenuItem::action("Open", Open),
                MenuItem::action("Save", Save),
            ]);
            GlobalState::global_mut(cx).set_app_menus(vec![file_menu.owned()]);
        }
    }

    fn on_open(_: &Open, cx: &mut App) {
        cx.spawn(async move |cx| {
            // Your existing dialog logic — returns the chosen path.
            let path: PathBuf = open_file_dialog().await;

            // Hand the result to the global app state.
            let _ = cx.update(|cx| {
                cx.global_mut::<AppState>().selected_path = Some(path);
            });
        })
        .detach();
    }

    /// `Save` handler — reads state from the global app state.
    fn on_save(_: &Save, cx: &mut App) {
        match &cx.global::<AppState>().selected_path {
            Some(path) => println!("Saving to {}", path.display()),
            None => println!("Nothing to save"),
        }
    }

    fn on_quit(_: &Quit, cx: &mut App) {
        cx.quit();
    }
}

/// Register the app actions as global listeners, so the native menu bar
/// validates them as available (and dispatches them) regardless of focus.
fn register_actions(cx: &mut App) {
    // Guard with a global flag so re-registering on multiple windows or
    // re-renders does not stack duplicate listeners.
    if cx.has_global::<ListenerRegistered>() {
        return;
    }
    cx.set_global(ListenerRegistered);
    cx.on_action(FrameView::on_open);
    cx.on_action(FrameView::on_save);
    cx.on_action(FrameView::on_quit);
}

/// Marker global so action listeners are registered exactly once.
struct ListenerRegistered;
impl Global for ListenerRegistered {}

impl Render for FrameView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .size_full()
            // Keep the listener registration on the focus path too, so the
            // in-window menu bar (Linux/Windows) and any focus-based dispatch
            // still work alongside the global listeners.
            .on_action(cx.listener(|_, _: &Open, _, cx| FrameView::on_open(&Open, cx)))
            .on_action(cx.listener(|_, _: &Save, _, cx| FrameView::on_save(&Save, cx)))
            .child(
                TitleBar::new().when(cfg!(not(target_os = "macos")), |title_bar| {
                    title_bar.child(self.menu_bar.clone())
                }),
            )
    }
}

/// Placeholder for your existing file-open dialog.
async fn open_file_dialog() -> PathBuf {
    PathBuf::from("/tmp/example.txt")
}
