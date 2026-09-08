use std::path::PathBuf;

use gpui_kit::base::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::label::Label;
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::component::TitleBar;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

#[cfg(not(target_os = "macos"))]
use gpui_kit::base::GlobalState;

use crate::actions::{New, Open, Quit, Save};

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
    /// The currently open document path. `None` means no document is open, in
    /// which case the welcome screen is shown instead of the editor body.
    path: Option<PathBuf>,
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
            path: None,
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
        println!("open action");
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

    /// `New` handler — creates a new untitled document. Logic TBD.
    fn on_new(_: &New, _cx: &mut App) {
        println!("new action");
    }

    /// `Save` handler — reads state from the global app state.
    fn on_save(_: &Save, cx: &mut App) {
        println!("menu save clicked");
        match &cx.global::<AppState>().selected_path {
            Some(path) => println!("Saving to {}", path.display()),
            None => println!("Nothing to save"),
        }
    }

    fn on_quit(_: &Quit, cx: &mut App) {
        cx.quit();
    }
}

fn register_actions(cx: &mut App) {
    if cx.has_global::<ListenerRegistered>() {
        return;
    }
    cx.set_global(ListenerRegistered);
    if !cx.has_global::<AppState>() {
        cx.set_global(AppState::default());
    }

    cx.on_action(FrameView::on_open);
    cx.on_action(FrameView::on_new);
    cx.on_action(FrameView::on_save);
    cx.on_action(FrameView::on_quit);
}

/// Marker global so action listeners are registered exactly once.
struct ListenerRegistered;
impl Global for ListenerRegistered {}

impl Render for FrameView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let has_document = self.path.is_some();

        div()
            .v_flex()
            .size_full()
            .child(
                TitleBar::new().when(cfg!(not(target_os = "macos")), |title_bar| {
                    title_bar.child(self.menu_bar.clone())
                }),
            )
            .child(if has_document {
                // Editor body — populated once document rendering is implemented.
                div().size_full()
            } else {
                welcome_screen()
            })
    }
}

/// The welcome screen shown when no document is open: a centered app title
/// with New and Open buttons. Button logic is not wired up yet.
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

/// Placeholder for your existing file-open dialog.
async fn open_file_dialog() -> PathBuf {
    PathBuf::from("/tmp/example.txt")
}
