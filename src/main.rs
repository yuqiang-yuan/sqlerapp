use std::path::{Path, PathBuf};

use gpui_kit::component::{button::{Button, ButtonVariants}, dialog::DialogClose, notification::NotificationType};
use gpui_kit::component::dialog::{DialogAction, DialogFooter};
use gpui_kit::component::list::{List, ListState};
use gpui_kit::component::menu::*;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use gpui_kit::gpui::KeyBinding;
use rust_embed::Embed;
use sqlerapp::actions::{
    About, New, NewDialogConfirmed, NewRelationship, NewTable, Open, Quit, Redo, Save, Undo,
};
use sqlerapp::db::{DialectName, DialectType, MySqlType, PostgresType};
use sqlerapp::document::ErDocument;
use sqlerapp::er_canvas::ErCanvas;
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
    /// The ER canvas view. Held as an entity (not rebuilt each render) so its
    /// in-progress `drag` gesture survives re-renders; recreated when a
    /// document is loaded/created.
    er_canvas: Option<Entity<ErCanvas>>,
    objects_list: Option<Entity<ListState<ObjectsListDelegate>>>,
    /// Backs `h_resizable` in the editor; owned here so the close hook can
    /// read the split position without reaching back into the widget.
    resizable_state: Entity<ResizableState>,
    /// Saved left-panel width, fed back as the panel's `initial_size`.
    left_panel_width: Option<Pixels>,
    /// Last known window geometry, cached each frame so the close hook can
    /// snapshot it after the window is already gone.
    last_window_bounds: Option<WindowBounds>,
    /// Recently opened files, most-recent-first, capped at 10. Shown on the
    /// welcome screen; persisted via `snapshot_settings`. Updated on open.
    recent_files: Vec<PathBuf>,
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

        // Bind keyboard shortcuts for the menu actions so they can be
        // triggered and so their keystrokes show on the right of each menu
        // item. `secondary` is the platform-portable modifier — Cmd on macOS,
        // Ctrl on Windows/Linux (parsed in gpui's `Keystroke::parse`).
        //
        // Redo follows each platform's convention: Cmd-Shift-Z on macOS,
        // Ctrl-Y on Windows/Linux. The rest are uniform across platforms.
        #[cfg(target_os = "macos")]
        let redo_binding = KeyBinding::new("secondary-shift-z", Redo, None);
        #[cfg(not(target_os = "macos"))]
        let redo_binding = KeyBinding::new("secondary-y", Redo, None);

        cx.bind_keys([
            KeyBinding::new("secondary-n", New, None),
            KeyBinding::new("secondary-o", Open { path: None }, None),
            KeyBinding::new("secondary-s", Save, None),
            KeyBinding::new("secondary-q", Quit, None),
            KeyBinding::new("secondary-z", Undo, None),
            redo_binding,
            KeyBinding::new("secondary-t", NewTable, None),
            KeyBinding::new("secondary-r", NewRelationship, None),
        ]);

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
        let close_sub = cx.on_window_closed(move |cx, _| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let _ = app.read(cx).snapshot_settings(cx).save();
        });

        let left_panel_width = settings.left_panel_width;
        let recent_files = settings.recent_files;

        Self {
            focus_handle,
            menu_bar: AppMenuBar::new(cx),
            current_file: None,
            document: None,
            er_canvas: None,
            objects_list: None,
            resizable_state,
            left_panel_width,
            last_window_bounds: None,
            recent_files,
            _close_sub: close_sub,
        }
    }

    pub fn on_new_action(&mut self, _: &New, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.new(|_| NewDocumentDialog {
            // Default to MySQL so New → Ok creates a document without forcing
            // the user to click a radio first (previously the dialog closed
            // silently when Ok was pressed with no selection).
            selected_option: Some(0),
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
        let schema = sample_schema(&act.dialect_name);
        let layout = sample_layout(&schema);
        let doc = cx.new(|_| ErDocument {
            dialect: act.dialect_name.clone(),
            schema,
            layout,
            path: None,
            dirty: false,
        });

        let list = cx.new(|cx| {
            ListState::new(ObjectsListDelegate { doc: doc.clone() }, window, cx)
        });
        let er_canvas = cx.new(|_| ErCanvas::new(doc.clone()));

        self.document = Some(doc);
        self.objects_list = Some(list);
        self.er_canvas = Some(er_canvas);

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

    /// Snapshot the live state into an [`AppSettings`] for persistence. The
    /// single source of truth for what gets saved — used by both the
    /// on-close hook and `add_recent`'s immediate write, so they can never
    /// drift apart.
    fn snapshot_settings(&self, cx: &App) -> AppSettings {
        AppSettings {
            window: self.last_window_bounds.map(WindowState::from_window_bounds),
            theme_mode: Theme::global(cx).mode,
            font_size: Some(Theme::global(cx).font_size),
            left_panel_width: self.resizable_state.read(cx).sizes().get(0).copied(),
            recent_files: self.recent_files.clone(),
        }
    }

    /// Record a just-opened (or saved) path as most-recent. Dedups against any
    /// prior entry, caps the list at 10, and persists immediately so a crash
    /// doesn't lose the recent list.
    fn add_recent(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.recent_files.retain(|p| p != &path);
        self.recent_files.insert(0, path);
        if self.recent_files.len() > 10 {
            self.recent_files.truncate(10);
        }
        let _ = self.snapshot_settings(cx).save();
        cx.notify();
    }

    /// File → Open. `Open { path: None }` (the menu item) pops a native file
    /// dialog; `Open { path: Some }` (a welcome-screen recent entry) opens
    /// that path directly. Both deserialize the `.sqler` file into a document,
    /// build the objects list, swap in the new document, record the path as
    /// most-recent, and re-render into the editor view.
    ///
    /// The dialog branch has to await `prompt_for_paths` off the UI thread, so
    /// it runs the deserialize + entity build inside a spawned task. Pure file
    /// I/O (`ErDocument::open_from_path`) is done before the `cx.update` so the
    /// app borrow is held only for the entity/list build + state mutation.
    pub fn on_open_action(&mut self, act: &Open, window: &mut Window, cx: &mut Context<Self>) {
        match &act.path {
            // Recent entry → open directly. Synchronous: we own `self`.
            Some(path) => match ErDocument::open_from_path(path) {
                Ok(doc) => self.load_document(doc, window, cx),
                Err(err) => {
                    window.push_notification(
                        (NotificationType::Error, format!("Open failed: {err}")),
                        cx,
                    );
                }
            },
            // Menu item → file dialog. The receiver resolves async, so the
            // rest runs in a task.
            None => {
                let view = cx.entity();
                let rx = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: None,
                });
                window.spawn(cx, async move |cx| {
                    let Ok(result) = rx.await else {
                        // Channel dropped (app shutting down) — nothing to do.
                        return;
                    };
                    let path = match result {
                        Ok(Some(mut paths)) => match paths.pop() {
                            Some(p) => p,
                            // Dialog returned an empty list — treat as cancel.
                            None => return,
                        },
                        // User cancelled the dialog.
                        Ok(None) => return,
                        Err(err) => {
                            let _ = cx.update(|window, cx| {
                                window.push_notification(
                                    (NotificationType::Error, format!("Open failed: {err}")),
                                    cx,
                                );
                            });
                            return;
                        }
                    };

                    // Pure I/O + deserialize — no app borrow needed here.
                    let doc_data = ErDocument::open_from_path(&path);
                    let _ = cx.update(|window, cx| match doc_data {
                        Ok(erdoc) => {
                            let path_for_recent = erdoc.path.clone();
                            let doc = cx.new(|_| erdoc);
                            let list = cx.new(|cx| {
                                ListState::new(
                                    ObjectsListDelegate { doc: doc.clone() },
                                    window,
                                    cx,
                                )
                            });
                            view.update(cx, |app, cx| {
                                app.document = Some(doc);
                                app.objects_list = Some(list);
                                if let Some(p) = path_for_recent {
                                    app.add_recent(p, cx);
                                }
                                cx.notify();
                            });
                            window.push_notification(
                                (NotificationType::Success, "Opened"),
                                cx,
                            );
                        }
                        Err(err) => {
                            window.push_notification(
                                (NotificationType::Error, format!("Open failed: {err}")),
                                cx,
                            );
                        }
                    });
                })
                .detach();
            }
        }
    }

    /// Finish opening an already-deserialized document: build the doc + list
    /// entities, swap them in, record the path as most-recent, and re-render.
    /// The synchronous half of [`on_open_action`] (direct open / recent entry).
    fn load_document(
        &mut self,
        doc: ErDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = doc.path.clone();
        let doc = cx.new(|_| doc);
        let list = cx
            .new(|cx| ListState::new(ObjectsListDelegate { doc: doc.clone() }, window, cx));
        let er_canvas = cx.new(|_| ErCanvas::new(doc.clone()));
        self.document = Some(doc);
        self.objects_list = Some(list);
        self.er_canvas = Some(er_canvas);
        if let Some(p) = path {
            self.add_recent(p, cx);
        }
        cx.notify();
    }

    /// Edit → New Table. Inserts an empty, auto-named table (`table_<n>` where
    /// `n` is one past the current table count) into the document's schema,
    /// marks the document dirty, and refreshes the objects list so the new row
    /// appears in the "Tables" section. `ListState` caches row counts and has no
    /// public refresh, so we trigger a re-render by notifying the list state —
    /// see memory `gpui-kit-list-refresh`.
    pub fn on_new_table_action(
        &mut self,
        _: &NewTable,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.document.as_ref() else {
            window.push_notification((NotificationType::Warning, "No document open"), cx);
            return;
        };

        doc.update(cx, |doc, _| {
            let name = format!("table_{}", doc.schema.tables.len() + 1);
            let table = Table {
                id: TableId::new(),
                name,
                columns: vec![],
                constraints: vec![],
                indexes: vec![],
                comment: None,
            };
            doc.schema.tables.insert(table.id.clone(), table);
            doc.mark_dirty();
        });

        if let Some(list) = self.objects_list.as_ref() {
            list.update(cx, |_, cx| cx.notify());
        }
    }

    /// Edit → New Relationship. Placeholder: relationship creation needs the
    /// canvas/interaction work that isn't built yet, so this just notifies the
    /// user rather than doing nothing silently.
    pub fn on_new_relationship_action(
        &mut self,
        _: &NewRelationship,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.push_notification((NotificationType::Info, "New relationship is not implemented yet"), cx);
    }

    /// Edit → Undo. Placeholder: a real history system is a dedicated later
    /// task. For now the menu item and shortcut exist, and the handler just
    /// tells the user it isn't wired up yet.
    pub fn on_undo_action(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        window.push_notification((NotificationType::Info, "Undo is not implemented yet"), cx);
    }

    /// Edit → Redo. See [`on_undo_action`].
    pub fn on_redo_action(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        window.push_notification((NotificationType::Info, "Redo is not implemented yet"), cx);
    }

    /// File → Save. If the document already has a path, write it in place.
    /// Otherwise pop a native save dialog, then write to the chosen path, set
    /// `path`, and clear `dirty`. `prompt_for_new_path` resolves
    /// asynchronously, so the untitled branch runs its write + state update
    /// inside a task that still has window access (to push the outcome
    /// notification). See memory `document-file-format` for the `.sqler` JSON
    /// format.
    pub fn on_save_action(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.document.clone() else {
            window.push_notification((NotificationType::Warning, "No document open"), cx);
            return;
        };

        // Already saved somewhere → write in place. No path mutation, just
        // clear the dirty flag.
        if let Some(path) = doc.read(cx).path.clone() {
            write_document(&doc, &path, window, cx);
            return;
        }

        // Untitled → ask for a location. The receiver resolves off the UI
        // thread, so the post-prompt work (serialize/write, set path, mark
        // clean, notify) runs in a spawned task. `window.spawn` hands the task
        // an `AsyncWindowContext`, whose `update` yields both the window and
        // the app — enough to mutate the doc and push a notification.
        let dir = save_directory();
        let rx = cx.prompt_for_new_path(&dir, Some("Untitled.sqler"));
        window.spawn(cx, async move |cx| {
            let Ok(path) = rx.await else {
                // Channel dropped (e.g. app shutting down) — nothing to do.
                return;
            };
            let path = match path {
                Ok(Some(path)) => ensure_sqler_ext(path),
                // User cancelled the dialog.
                Ok(None) => return,
                Err(err) => {
                    let _ = cx.update(|window, cx| {
                        window.push_notification((
                            NotificationType::Error,
                            format!("Save failed: {err}")
                        ), cx);
                    });
                    return;
                }
            };

            let _ = cx.update(|window, cx| {
                match doc.read(cx).save_to_path(&path) {
                    Ok(()) => {
                        doc.update(cx, |d, cx| {
                            d.path = Some(path);
                            d.mark_clean();
                            cx.notify();
                        });
                        window.push_notification((
                            NotificationType::Success,
                            "Saved",
                        ), cx);
                    }
                    Err(err) => window.push_notification((
                        NotificationType::Error,
                        format!("Save failed: {err}")
                    ), cx),
                }
            });
        })
        .detach();
    }
}

impl Render for MyApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.last_window_bounds = Some(window.window_bounds());
        let dialog_layer = Root::render_dialog_layer(window, cx);
        // Root does not auto-mount its overlay layers (see memory
        // `gpui-dialog-layer-must-mount`), so mount the notification layer
        // here too — the Edit menu's placeholder handlers push notifications
        // via `window.push_notification`, which need it to display.
        let notification_layer = Root::render_notification_layer(window, cx);
        // Only one of welcome/editor is shown; build just that branch as an
        // owned element so the welcome screen can borrow `recent_files`
        // without conflicting with the editor view's self borrow.
        let content: AnyElement = if self.document.is_none() {
            welcome_screen(&self.recent_files).into_any_element()
        } else {
            self.editor_view(cx).into_any_element()
        };
        div()
            .id("main-window")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_new_action))
            .on_action(cx.listener(Self::on_new_dialog_confirmed))
            .on_action(cx.listener(Self::on_open_action))
            .on_action(cx.listener(Self::on_about_action))
            .on_action(cx.listener(Self::on_quit_action))
            .on_action(cx.listener(Self::on_save_action))
            .on_action(cx.listener(Self::on_new_table_action))
            .on_action(cx.listener(Self::on_new_relationship_action))
            .on_action(cx.listener(Self::on_undo_action))
            .on_action(cx.listener(Self::on_redo_action))
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
            .child(content)
            .children(dialog_layer)
            .children(notification_layer)
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

/// A simple grid layout for the sample schema, so the canvas shows the tables
/// and their foreign-key edge on first open. Tables are placed left-to-right,
/// wrapping every 3 columns; spacing exceeds the fixed card geometry so cards
/// never overlap. The layout stays on `GraphLayout` (not on `Table`), so it
/// is layout-independent and persists with the document.
fn sample_layout(schema: &Schema) -> GraphLayout {
    const COLS: usize = 3;
    const DX: f32 = 300.0;
    const DY: f32 = 220.0;
    const X0: f32 = 60.0;
    const Y0: f32 = 60.0;
    let mut layout = GraphLayout::default();
    for (i, id) in schema.tables.keys().enumerate() {
        let col = (i % COLS) as f32;
        let row = (i / COLS) as f32;
        layout
            .positions
            .insert(id.clone(), (X0 + col * DX, Y0 + row * DY));
    }
    layout
}

/// The no-document welcome screen: a New/Open prompt plus a list of the
/// most-recent files. Clicking a recent entry dispatches `Open { path: Some
/// }` to open it directly; the Open button dispatches `Open { path: None }`
/// for the file dialog.
fn welcome_screen(recent: &[PathBuf]) -> impl IntoElement {
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
                .child(
                    Button::new("open")
                        .label("Open")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(Open { path: None }), cx);
                        }),
                ),
        )
        .when(!recent.is_empty(), |this| {
            this.child(
                div()
                    .mt_8()
                    .v_flex()
                    .gap_1()
                    .w(px(320.))
                    .child(div().text_sm().child("Recent files"))
                    .children(recent.iter().enumerate().map(|(i, path)| {
                        let owned_path = path.clone();
                        let label = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.to_string_lossy().into_owned());
                        Button::new(SharedString::from(format!("recent-{i}")))
                            .ghost()
                            .label(label)
                            .w_full()
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(
                                    Box::new(Open {
                                        path: Some(owned_path.clone()),
                                    }),
                                    cx,
                                );
                            })
                    })),
            )
        })
}

/// Serialize `doc` to `path`, clear its dirty flag, and push a notification
/// with the outcome. Used by the Save action when the document already has a
/// path (no dialog). The dirty flag is the only state that changes here — the
/// path stays as it was.
fn write_document(
    doc: &Entity<ErDocument>,
    path: &Path,
    window: &mut Window,
    cx: &mut App,
) {
    match doc.read(cx).save_to_path(path) {
        Ok(()) => {
            doc.update(cx, |d, cx| {
                d.mark_clean();
                cx.notify();
            });
            window.push_notification((NotificationType::Success, "Saved"), cx);
        }
        Err(err) => window.push_notification((NotificationType::Error, format!("Save failed: {err}")), cx),
    }
}

/// Initial directory for the save dialog: the documents dir, home, or cwd.
fn save_directory() -> PathBuf {
    dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Append the `.sqler` extension if the chosen path has none, matching the
/// document format convention (memory `document-file-format`). A path the user
/// explicitly gave an extension is left untouched.
fn ensure_sqler_ext(mut path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        path.set_extension("sqler");
    }
    path
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
    fn editor_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                        // The right pane is itself a vertical resizable split:
                        // the ER render image on top, a reserved strip below for
                        // future content (placeholder text for now). The group
                        // owns no caller state, so its divider position lives in
                        // `window.use_keyed_state` under the "v-resizer" id —
                        // enough to survive re-renders without disk persistence.
                        v_resizable("v-resizer")
                            .child(
                                resizable_panel()
                                    .size_range(px(200.0)..px(4000.0))
                                    .child(
                                        div()
                                            .id("er-canvas-pane")
                                            .size_full()
                                            .when_some(self.er_canvas.as_ref(), |pane, canvas| {
                                                pane.child(canvas.clone())
                                            }),
                                    ),
                            )
                            .child(
                                resizable_panel()
                                    .size(px(180.0))
                                    .flex_none()
                                    .size_range(px(100.0)..px(500.0))
                                    .child(
                                        div()
                                            .id("render-bottom-pane")
                                            .size_full()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(
                                                div()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child("Placeholder"),
                                            ),
                                    ),
                            ),
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
                MenuItem::action("Open", Open { path: None }),
                MenuItem::action("Save", Save),
                MenuItem::separator(),
                MenuItem::action("Quit", Quit),
            ],
            disabled: false,
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::action("Undo", Undo),
                MenuItem::action("Redo", Redo),
                MenuItem::separator(),
                MenuItem::action("New Table", NewTable),
                MenuItem::action("New Relationship", NewRelationship),
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
