use std::path::PathBuf;

use gpui_kit::base::{GlobalState, StyledExt};
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::component::TitleBar;
use gpui_kit::*;

use crate::actions::{Open, Save};

pub struct FrameView {
    focus_handle: FocusHandle,
    selected_path: Option<PathBuf>,
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
            selected_path: None,
            menu_bar,
        }
    }

    /// Define the top-level application menus.
    fn set_menus(cx: &mut App) {
        let file_menu = Menu::new("File").items(vec![
            MenuItem::action("Open", Open),
            MenuItem::action("Save", Save),
        ]);

        GlobalState::global_mut(cx).set_app_menus(vec![file_menu.owned()]);
    }

    fn on_open(&mut self, _: &Open, _window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| {
            // Your existing dialog logic — returns the chosen path.
            let path: PathBuf = open_file_dialog().await;

            // Hand the result to the FrameView instance.
            let _ = view.update(cx, |view, cx| {
                view.selected_path = Some(path);
                cx.notify(); // re-render so the UI reflects the new path
            });
        })
        .detach();
    }

    /// `Save` handler — `self` is the FrameView; read state directly.
    fn on_save(&mut self, _: &Save, _window: &mut Window, _cx: &mut Context<Self>) {
        match &self.selected_path {
            Some(path) => println!("Saving to {}", path.display()),
            None => println!("Nothing to save"),
        }
    }
}

impl Render for FrameView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .size_full()
            .on_action(cx.listener(Self::on_open))
            .on_action(cx.listener(Self::on_save))
            .child(TitleBar::new().child(self.menu_bar.clone()))
    }
}

/// Placeholder for your existing file-open dialog.
async fn open_file_dialog() -> PathBuf {
    PathBuf::from("/tmp/example.txt")
}
