use std::path::PathBuf;

use gpui_kit::component::{button::{Button, ButtonVariants}, dialog::DialogClose};
use gpui_kit::component::dialog::{DialogAction, DialogFooter};
use gpui_kit::component::list::{List, ListState};
use gpui_kit::component::menu::*;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_embed::Embed;
use sqlerapp::actions::{About, New, NewDialogConfirmed, Open, Quit, Save};
use sqlerapp::db::{DialectName, DialectType, MySqlType, PostgresType};
use sqlerapp::document::ErDocument;
use sqlerapp::list_delegate::ObjectsListDelegate;
use sqlerapp::model::{
    Column, ColumnId, Constraint, ConstraintId, GraphLayout, Schema, Table, TableId,
};
use sqlerapp::settings::{AppSettings, WindowState};

#[allow(dead_code)]
pub struct MyApp {
    focus_handle: FocusHandle,
    menu_bar: Entity<AppMenuBar>,
    current_file: Option<PathBuf>,
    document: Option<Entity<ErDocument>>,
    objects_list: Option<Entity<ListState<ObjectsListDelegate>>>,
    /// Backs `h_resizable` in the editor; owned here so the close hook can
    /// read the split position without reaching back into the widget.
    resizable_state: Entity<ResizableState>,
    /// Saved left-panel width, fed back as the panel's `initial_size`.
    left_panel_width: Option<Pixels>,
    /// Last known window geometry, cached each frame so the close hook can
    /// snapshot it after the window is already gone.
    last_window_bounds: Option<WindowBounds>,
    /// Keeps the on-window-closed save hook alive for this view's lifetime.
    _close_sub: Subscription,
}

impl MyApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, settings: AppSettings) -> Self {
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

        // `Theme::change` (in open_main_window) resets font_size to the theme
        // config's value, so re-apply the saved size here — after that, before
        // the first render picks it up via Root's set_rem_size.
        if let Some(fs) = settings.font_size {
            Theme::global_mut(cx).font_size = fs;
        }

        let resizable_state = cx.new(|_| ResizableState::default());

        // Snapshot window geometry, theme, and the resizer split on close so
        // the next launch restores them. `on_window_closed` fires on every
        // close path — the platform close (Alt+F4, the macOS traffic light)
        // and the title-bar close button's `remove_window` — so it is the
        // single reliable hook. The window is already gone by then, so the
        // geometry comes from `last_window_bounds`, cached each frame in
        // `render`.
        let weak = cx.entity().downgrade();
        let resizable_for_close = resizable_state.clone();
        let close_sub = cx.on_window_closed(move |cx, _| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let bounds = app.read(cx).last_window_bounds.clone();
            let theme_mode = Theme::global(cx).mode;
            let font_size = Some(Theme::global(cx).font_size);
            let left = resizable_for_close.read(cx).sizes().get(0).copied();
            let _ = AppSettings {
                window: bounds.map(WindowState::from_window_bounds),
                theme_mode,
                font_size,
                left_panel_width: left,
            }
            .save();
        });

        let left_panel_width = settings.left_panel_width;

        Self {
            focus_handle,
            menu_bar: AppMenuBar::new(cx),
            current_file: None,
            document: None,
            objects_list: None,
            resizable_state,
            left_panel_width,
            last_window_bounds: None,
            _close_sub: close_sub,
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
                        .child(DialogAction::new().child(Button::new("ok").primary().label("Ok")))
                        .child(
                            DialogClose::new().child(
                                Button::new("cancel")
                                    .label("Close")
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                        ),
                )
                .on_ok(move |_, window, app| {
                    match view_clone.read(app).selected_option {
                        Some(0) => {
                            window.dispatch_action(
                                Box::new(NewDialogConfirmed {
                                    dialect_name: DialectName::MySql,
                                }),
                                app,
                            );
                        }
                        Some(1) => {
                            window.dispatch_action(
                                Box::new(NewDialogConfirmed {
                                    dialect_name: DialectName::Postgres,
                                }),
                                app,
                            );
                        }
                        _ => {}
                    };

                    true
                })
        })
    }

    pub fn on_new_dialog_confirmed(
        &mut self,
        act: &NewDialogConfirmed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        println!("selected database type: {:?}", act.dialect_name);
        let schema = sample_schema(&act.dialect_name);
        let doc = cx.new(|_| ErDocument {
            dialect: act.dialect_name.clone(),
            schema,
            layout: GraphLayout::default(),
            path: None,
            dirty: false,
        });

        let list = cx.new(|cx| {
            ListState::new(ObjectsListDelegate { doc: doc.clone() }, window, cx)
        });

        self.document = Some(doc);
        self.objects_list = Some(list);

        cx.notify();
    }

    /// Show the About dialog. `MenuItem::action("About", About)` dispatches
    /// here; the dialog is a plain info box with a Close button.
    pub fn on_about_action(&mut self, _: &About, window: &mut Window, cx: &mut Context<Self>) {
        window.open_dialog(cx, |dialog, _, _| {
            dialog
                .title("About")
                .child(
                    div()
                        .v_flex()
                        .gap_2()
                        .py_3()
                        .child(div().text_lg().font_bold().child("SQLER"))
                        .child(div().text_sm().child("A visual ER diagram tool for SQL tables and relations."))
                        .child(div().text_sm().child("Built with Rust · GPUI · gpui-kit")),
                )
                .footer(
                    DialogFooter::new().child(
                        DialogClose::new().child(
                            Button::new("about-close")
                                .label("Close")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        ),
                    ),
                )
        });
    }

    /// Quit the application. `MenuItem::action("Quit", Quit)` dispatches
    /// here; `cx.quit()` goes through the platform's standard quit routine,
    /// so the `on_window_closed` save hook still fires for every window
    /// before the process exits.
    pub fn on_quit_action(&mut self, _: &Quit, _window: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }
}

impl Render for MyApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.last_window_bounds = Some(window.window_bounds());
        let dialog_layer = Root::render_dialog_layer(window, cx);
        div()
            .id("main-window")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_new_action))
            .on_action(cx.listener(Self::on_new_dialog_confirmed))
            .on_action(cx.listener(Self::on_about_action))
            .on_action(cx.listener(Self::on_quit_action))
            .size_full()
            .v_flex()
            .child(
                TitleBar::new()
                    .child(self.menu_bar.clone())
                    .child(
                        h_flex()
                            .gap_1()
                            .child(font_size_button())
                            .child(theme_button(cx)),
                    ),
            )
            .when(self.document.is_none(), |div| div.child(welcome_screen()))
            .when(self.document.is_some(), |div| div.child(self.editor_view(cx)))
            .children(dialog_layer)
    }
}

struct NewDocumentDialog {
    selected_option: Option<usize>,
}

impl Render for NewDocumentDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().p_1().child(
            RadioGroup::vertical("dialect-type-radio-group")
                .children(["MySQL", "PostgreSQL"])
                .selected_index(self.selected_option)
                .on_change(cx.listener(|view, selected_idx, _, cx| {
                    println!("selected index: {}", *selected_idx);
                    view.selected_option = Some(*selected_idx);

                    cx.notify();
                })),
        )
    }
}

/// Build a small sample schema (users + posts, with posts → users FK) so the
/// objects list has something to show before Open/Save are wired. The column
/// type follows the chosen dialect.
fn sample_schema(dialect: &DialectName) -> Schema {
    let int_ty = match dialect {
        DialectName::MySql => DialectType::MySql(MySqlType::Int {
            unsigned: false,
            display_width: None,
        }),
        DialectName::Postgres => DialectType::Postgres(PostgresType::Integer),
    };

    let users = Table {
        id: TableId::new(),
        name: "users".into(),
        columns: vec![
            Column {
                id: ColumnId::new(),
                name: "id".into(),
                ty: int_ty.clone(),
                nullable: false,
                default: None,
                auto_increment: matches!(dialect, DialectName::MySql),
                comment: None,
            },
            Column {
                id: ColumnId::new(),
                name: "name".into(),
                ty: int_ty.clone(),
                nullable: true,
                default: None,
                auto_increment: false,
                comment: None,
            },
        ],
        constraints: vec![],
        indexes: vec![],
        comment: None,
    };

    let posts = Table {
        id: TableId::new(),
        name: "posts".into(),
        columns: vec![Column {
            id: ColumnId::new(),
            name: "user_id".into(),
            ty: int_ty,
            nullable: false,
            default: None,
            auto_increment: false,
            comment: None,
        }],
        constraints: vec![Constraint::ForeignKey {
            id: ConstraintId::new(),
            name: Some("posts_user_id_fkey".into()),
            columns: vec!["user_id".into()],
            referenced_table: "users".into(),
            referenced_columns: vec!["id".into()],
            on_delete: None,
            on_update: None,
        }],
        indexes: vec![],
        comment: None,
    };

    let mut schema = Schema::default();
    schema.tables.insert(users.id.clone(), users);
    schema.tables.insert(posts.id.clone(), posts);
    schema
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

/// Title-bar font-size picker. The global `Theme::font_size` is the rem base:
/// `Root::render` calls `window.set_rem_size(theme.font_size)`, so changing it
/// and refreshing the window scales the whole UI (text + spacing) like a
/// browser zoom. Each item sets the size directly in its `on_click` — no action
/// dispatch, so it works regardless of the focus path. See memory
/// `gpui-kit-global-font-size`.
fn font_size_button() -> impl IntoElement {
    Button::new("font-size")
        .ghost()
        .label("Aa")
        .tooltip("Font size")
        .dropdown_menu(|menu, _window, cx| {
            // The DropdownMenu rebuilds its PopupMenu each time it opens
            // (dismiss clears the cached menu entity), so re-reading the
            // current size here keeps the check mark fresh.
            let current = cx.theme().font_size.as_f32().round() as i32;
            menu.item(
                PopupMenuItem::new("Small")
                    .checked(current == 14)
                    .on_click(|_, window, cx| {
                        Theme::global_mut(cx).font_size = px(14.);
                        window.refresh();
                    }),
            )
            .item(
                PopupMenuItem::new("Medium")
                    .checked(current == 16)
                    .on_click(|_, window, cx| {
                        Theme::global_mut(cx).font_size = px(16.);
                        window.refresh();
                    }),
            )
            .item(
                PopupMenuItem::new("Large")
                    .checked(current == 18)
                    .on_click(|_, window, cx| {
                        Theme::global_mut(cx).font_size = px(18.);
                        window.refresh();
                    }),
            )
        })
}

/// Title-bar light/dark theme toggle. `Theme::change` swaps the mode and calls
/// `window.refresh()` itself, so no manual refresh is needed here.
fn theme_button(cx: &App) -> impl IntoElement {
    let is_dark = cx.theme().is_dark();
    Button::new("theme-toggle")
        .ghost()
        .tooltip("Toggle theme")
        // Show the mode you'll switch TO: sun when dark (→ light),
        // moon when light (→ dark).
        .icon(if is_dark { IconName::Sun } else { IconName::Moon })
        .on_click(|_, window, cx| {
            let mode = if cx.theme().is_dark() {
                ThemeMode::Light
            } else {
                ThemeMode::Dark
            };
            Theme::change(mode, Some(window), cx);
        })
}

impl MyApp {
    fn editor_view(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .v_flex()
            .child(
                h_resizable("h-resizer")
                    .with_state(&self.resizable_state)
                    .child(
                        resizable_panel()
                            .size_range(px(100.0)..px(400.0))
                            .when_some(self.left_panel_width, |panel, w| panel.size(w))
                            .child(
                                div()
                                    .id("objects-list-box")
                                    .size_full()
                                    .p_1()
                                    .overflow_scroll()
                                    .when_some(self.objects_list.as_ref(), |panel, list| {
                                        panel.child(List::new(list))
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .child("Bottom Panel")
                            .into_any_element()
                    ),
            )
            .child(StatusBar::new().left("Ready"))
    }
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
                MenuItem::separator(),
                MenuItem::action("Quit", Quit),
            ],
            disabled: false,
        },
        Menu {
            name: "Help".into(),

            items: vec![
                MenuItem::action("About", About),
            ],
            disabled: false,
        }
    ]
}

fn open_main_window(settings: AppSettings, cx: &mut App) {
    // Fall back to a centered default when no geometry was saved.
    let window_bounds = settings
        .window_bounds(cx)
        .unwrap_or_else(|| WindowBounds::centered(size(px(800.), px(600.)), cx));

    let options = WindowOptions {
        window_bounds: Some(window_bounds),
        kind: WindowKind::Normal,
        #[cfg(target_os = "linux")]
        window_decorations: Some(WindowDecorations::Client),
        ..TitleBar::window_options()
    };

    cx.open_window(options, |window, cx| {
        // Restore the saved theme before building views, so Root::render
        // picks it up on the first frame.
        Theme::change(settings.theme_mode, Some(window), cx);
        let view = cx.new(|cx| MyApp::new(window, cx, settings));
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

        let settings = AppSettings::load();

        cx.spawn(async move |cx| {
            cx.update(|cx| {
                open_main_window(settings, cx);
            });
        })
        .detach();
    });
}
