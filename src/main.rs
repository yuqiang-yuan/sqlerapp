use std::path::PathBuf;

use gpui_kit::assets::Assets;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::menu::*;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_embed::Embed;
use sqlerapp::actions::{About, New, NewDialogConfirmed, Open, Quit, Save};
use sqlerapp::db::DialectName;
use sqlerapp::document::ErDocument;
use sqlerapp::frame::FrameView;

pub struct MyApp {
    focus_handle: FocusHandle,
    menu_bar: Entity<AppMenuBar>,
    current_file: Option<PathBuf>,
    document: Option<ErDocument>,
}

impl MyApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        #[cfg(target_os = "macos")]
        {
            cx.set_menus(build_menus());
        }

        #[cfg(not(target_os = "macos"))]
        {
            let menus = build_menus().into_iter().map(|menu| menu.owned()).collect();
            GlobalState::global_mut(cx).set_app_menus(menus);
        }

        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        Self {
            focus_handle,
            menu_bar: AppMenuBar::new(cx),
            current_file: None,
            document: None,
        }
    }

    pub fn on_new_action(&mut self, _: &New, window: &mut Window, cx: &mut Context<Self>) {
        println!("new action executed");

        let view = cx.new(|_| NewDocumentDialog {
            selected_option: None,
        });

        window.open_dialog(cx, move |dialog, _, _| {
            let view_clone = view.clone();
            dialog
                .title("Select Database")
                .overlay_closable(false)
                .child(view.clone())
                .footer(
                    DialogFooter::new()
                        .child(DialogAction::new().child(
                            Button::new("ok")
                                .primary()
                                .label("Ok")
                        ))
                        .child(DialogClose::new().child(
                            Button::new("cancel")
                                .label("Close")
                        ))
                )
                .on_ok(move |_, window, app| {
                    match view_clone.read(app).selected_option {
                        Some(0) => {
                            window.dispatch_action(Box::new(NewDialogConfirmed {
                                dialect_name: DialectName::MySql,
                            }), app);
                        },
                        Some(1) => {
                            window.dispatch_action(Box::new(NewDialogConfirmed {
                                dialect_name: DialectName::Postgres,
                            }), app);
                        },
                        _ => {}
                    };

                    true
                })
        })
    }

    pub fn on_new_dialog_confirmed(&mut self, act: &NewDialogConfirmed, window: &mut Window, cx: &mut Context<Self>) {
        println!("selected database type: {:?}", act.dialect_name);
    }
}

impl Render for MyApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dialog_layer = Root::render_dialog_layer(window, cx);
        div()
            .id("main-window")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_new_action))
            .on_action(cx.listener(Self::on_new_dialog_confirmed))
            .size_full()
            .v_flex()
            .child(TitleBar::new().child(self.menu_bar.clone()))
            .when(self.document.is_none(), |div| div.child(welcome_screen()))
            .when(self.document.is_some(), |div| div.child(main_view()))
            .children(dialog_layer)
    }
}

struct NewDocumentDialog {
    selected_option: Option<usize>,
}

impl Render for NewDocumentDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .p_1()
            .child(
                RadioGroup::vertical("dialect-type-radio-group")
                    .children(["MySQL", "PostgreSQL"])
                    .selected_index(self.selected_option)
                    .on_change(cx.listener(|view, selected_idx, _, cx| {
                        println!("selected index: {}", *selected_idx);
                        view.selected_option = Some(*selected_idx);

                        cx.notify();
                    }))
            )
    }
}

fn welcome_screen() -> impl IntoElement {
    div()
        .size_full()
        .v_flex()
        .items_center()
        .justify_center()
        .child(div().text_lg().font_bold().child("Welcome to SQLER"))
        .child(
            div()
                .mt_8()
                .h_flex()
                .gap_4()
                .child(
                    Button::new("new")
                        .primary()
                        .label("New")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(New), cx);
                        }),
                )
                .child(Button::new("open").label("Open")),
        )
}

fn main_view() -> impl IntoElement {
    div().size_full()
}

#[derive(Embed)]
#[folder = "assets"]
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some(file) = AppAssets::get(path) {
            return Ok(Some(file.data));
        }

        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut files = AppAssets::iter()
            .filter(|f| f.starts_with(path))
            .map(SharedString::from)
            .collect::<Vec<_>>();
        files.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(files)
    }
}

fn build_menus() -> Vec<Menu> {
    vec![
        #[cfg(target_os = "macos")]
        {
            Menu {
                name: "SQLER".into(),
                items: vec![
                    MenuItem::action("About", About),
                    MenuItem::separator(),
                    MenuItem::action("Quit", Quit),
                ],
                disabled: false,
            }
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New", New),
                MenuItem::action("Open", Open),
                MenuItem::action("Save", Save),
            ],
            disabled: false,
        },
    ]
}

fn open_main_window(window_bounds: Option<WindowBounds>, cx: &mut App) {
    // Fall back to a centered default when no geometry was saved.
    let window_bounds =
        window_bounds.unwrap_or_else(|| WindowBounds::centered(size(px(800.), px(600.)), cx));

    let options = WindowOptions {
        window_bounds: Some(window_bounds),
        kind: WindowKind::Normal,
        #[cfg(target_os = "linux")]
        window_decorations: Some(WindowDecorations::Client),
        ..TitleBar::window_options()
    };

    cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| MyApp::new(window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    })
    .expect("Failed to open window");

    cx.activate(true);
}

fn main() {
    let app = gpui_kit::application().with_assets(AppAssets);

    app.run(move |cx| {
        gpui_kit::init(cx);
        cx.set_app_identity("top.sqler.app", "SQLER");

        let window_bounds = WindowBounds::centered(size(px(800.0), px(600.0)), cx);

        cx.spawn(async move |cx| {
            cx.update(|cx| {
                open_main_window(Some(window_bounds), cx);
            });
        })
        .detach();
    });
}
