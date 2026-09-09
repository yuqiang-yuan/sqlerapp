use std::collections::HashSet;
use std::path::PathBuf;

use gpui_kit::base::{StyledExt, h_resizable, resizable_panel, TreeItem};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::label::Label;
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable, Theme, ThemeMode};
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
    /// State for the left-pane schema tree (Tables / Relationships). Holds
    /// test data for now; will be driven by the document's schema later.
    tree_state: Entity<SchemaTreeState>,
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
        let tree_state = cx.new(|_cx| SchemaTreeState::new(test_tree_items()));

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
            tree_state,
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
            .child(if let Some(document) = self.document.as_ref() {
                editor_panel(document, &self.tree_state, cx)
            } else {
                welcome_screen()
            })
    }
}

/// The editor layout shown when a document is open: a horizontally resizable
/// left/right split with a status bar pinned to the bottom. The status bar
/// shows "Ready", a separator, and the open document's file name (or
/// "Untitled" for a never-saved new document).
fn editor_panel(document: &Document, tree_state: &Entity<SchemaTreeState>, cx: &App) -> Div {
    let file_name = document
        .path
        .as_ref()
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".to_string());

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
                                .child(left_pane(tree_state, cx)),
                        )
                        .child(resizable_panel().child(right_pane())),
                ),
        )
        .child(
            StatusBar::new()
                .left("Ready")
                .left(Separator::vertical())
                .left(file_name),
        )
}

/// The left pane: a two-level schema tree. Top level groups are `Tables` and
/// `Relationships`; their children are the individual tables / relationships.
/// Test data is long enough to exercise vertical scrolling, and some names are
/// deliberately very long to exercise horizontal overflow.
///
/// This is a hand-rolled `uniform_list` rather than `gpui_kit`'s `tree()`: the
/// kit's tree wraps a `uniform_list` with `ListHorizontalSizingBehavior::FitList`
/// (rows clipped to the pane width) and exposes no way to switch to
/// `Unconstrained`, so over-wide names could never scroll horizontally.
/// Rendering our own list with `Unconstrained` + `with_width_from_item` lets
/// long names scroll instead of being clipped.
fn left_pane(tree_state: &Entity<SchemaTreeState>, cx: &App) -> impl IntoElement {
    let view = tree_state.read(cx);
    let count = view.flat.len();
    // `uniform_list` derives the list width by measuring a single row (at
    // max-content). Point it at the widest row so that row — and therefore the
    // list — can exceed the pane width and engage horizontal scrolling.
    let widest = view.widest_index();
    let scroll_handle = view.scroll_handle.clone();
    // `view` (borrowed via `tree_state.read`) is no longer used after this;
    // NLL releases it, so `tree_state.clone()` below is fine.

    let state = tree_state.clone();

    div()
        .id("schema-tree")
        .size_full()
        .child(
            uniform_list("schema-tree-list", count, move |range, _window, cx| {
                let view = state.read(cx);
                range
                    .map(|ix| {
                        let entry = &view.flat[ix];
                        let selected = view.selected_id.as_ref() == Some(&entry.id);
                        render_schema_row(ix, entry, selected, &state, cx)
                    })
                    .collect::<Vec<_>>()
            })
            .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
            .with_width_from_item(Some(widest))
            .flex_grow_1()
            .size_full()
            .track_scroll(&scroll_handle),
        )
        .vertical_scrollbar(&scroll_handle)
        .horizontal_scrollbar(&scroll_handle)
}

/// Render one visible row of the schema tree: indentation, a fold chevron for
/// folders, and the label (never wrapped, so long names overflow right and are
/// revealed by horizontal scrolling). Clicking a folder toggles it; clicking a
/// leaf selects it.
fn render_schema_row(
    ix: usize,
    entry: &FlatEntry,
    selected: bool,
    state: &Entity<SchemaTreeState>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let chevron = if entry.is_folder {
        let icon = if entry.is_expanded {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        };
        Some(Icon::new(icon).small().text_color(theme.muted_foreground))
    } else {
        None
    };
    let label_color = if selected {
        theme.accent_foreground
    } else if entry.is_folder {
        theme.foreground
    } else {
        theme.muted_foreground
    };
    let state = state.clone();
    div()
        .id(("tree-row", ix))
        .h(px(28.))
        // Fill the list's available width so the *entire* row is a click
        // target, not just the label. `uniform_list` lays each item out with a
        // definite available width but leaves the item's own width at `auto`;
        // a flex row with `auto` width shrinks to its content. `w_full` makes
        // short rows span the full width. Measurement still runs at
        // max-content (where `100%` is treated as content-based), so the
        // widest row keeps defining the horizontal scroll range.
        .w_full()
        .flex()
        .items_center()
        .gap_1()
        .pl(px(entry.depth as f32 * 12. + 8.))
        .pr(px(8.))
        .when(selected, |r| r.bg(theme.accent))
        .cursor_pointer()
        .children(chevron)
        .child(
            div()
                .flex_none()
                .whitespace_nowrap()
                .text_color(label_color)
                .child(entry.label.clone()),
        )
        .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
            state.update(cx, |s, cx| s.on_click(ix, cx));
        })
        .into_any_element()
}

/// Placeholder for the right pane (e.g. inspector / properties). Empty for now.
fn right_pane() -> Div {
    div().size_full()
}

/// A flattened, visible row in the schema tree. `depth` drives indentation;
/// `is_folder` / `is_expanded` drive the chevron and whether children show.
#[derive(Clone)]
struct FlatEntry {
    id: SharedString,
    label: SharedString,
    depth: usize,
    is_folder: bool,
    is_expanded: bool,
}

/// UI state for the left-pane schema tree: the root items (Tables,
/// Relationships, …), which folders are expanded, the selected row (by id), and
/// the scroll handle shared between the virtual list and its scrollbars.
///
/// This owns a flattened `flat` list of the currently-visible entries,
/// rebuilt whenever a folder is toggled — the `uniform_list` renders straight
/// out of it. See [`left_pane`] for why we don't use `gpui_kit`'s `tree()`.
pub struct SchemaTreeState {
    items: Vec<TreeItem>,
    expanded: HashSet<SharedString>,
    selected_id: Option<SharedString>,
    flat: Vec<FlatEntry>,
    scroll_handle: UniformListScrollHandle,
}

impl SchemaTreeState {
    /// New state from root items. All top-level folders start expanded.
    pub fn new(items: Vec<TreeItem>) -> Self {
        let expanded = items
            .iter()
            .filter(|i| i.is_folder())
            .map(|i| i.id.clone())
            .collect();
        let mut state = Self {
            items,
            expanded,
            selected_id: None,
            flat: Vec::new(),
            scroll_handle: UniformListScrollHandle::new(),
        };
        state.rebuild_flat();
        state
    }

    /// Index of the widest-label row. `uniform_list` measures this single row
    /// (at max-content) to size the list, so it must be the row we want the
    /// horizontal scroll range to be based on.
    fn widest_index(&self) -> usize {
        self.flat
            .iter()
            .enumerate()
            .max_by_key(|(_, e)| e.label.len())
            .map(|(ix, _)| ix)
            .unwrap_or(0)
    }

    /// Handle a row click: toggle the folder, or select the leaf.
    fn on_click(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.flat.get(ix).cloned() else {
            return;
        };
        if entry.is_folder {
            if !self.expanded.remove(&entry.id) {
                self.expanded.insert(entry.id.clone());
            }
            // Selection is by id, so it survives the index reshuffle.
            self.rebuild_flat();
        } else {
            self.selected_id = Some(entry.id.clone());
        }
        cx.notify();
    }

    /// Rebuild the flattened visible-entries list from `items` + `expanded`.
    fn rebuild_flat(&mut self) {
        self.flat.clear();
        // Clone the roots so the immutable walk of `items` doesn't alias the
        // mutable push into `flat` (and its `expanded` lookups).
        for item in self.items.clone() {
            self.add_flat(&item, 0);
        }
    }

    fn add_flat(&mut self, item: &TreeItem, depth: usize) {
        let is_expanded = self.expanded.contains(&item.id);
        self.flat.push(FlatEntry {
            id: item.id.clone(),
            label: item.label.clone(),
            depth,
            is_folder: item.is_folder(),
            is_expanded,
        });
        if is_expanded {
            for child in &item.children {
                self.add_flat(child, depth + 1);
            }
        }
    }
}

/// Build the test tree: `Tables` and `Relationships` at the top level, each
/// expanded, with a batch of children — some with very long names to exercise
/// horizontal overflow. This is throwaway data; the real tree will be derived
/// from the document's [`Schema`](crate::model::Schema).
fn test_tree_items() -> Vec<TreeItem> {
    let tables = TreeItem::new("tables", "Tables")
        .expanded(true)
        .children(
            (0..60)
                .map(|i| {
                    let label = if i % 7 == 0 {
                        format!(
                            "very_long_table_name_that_exceeds_the_pane_width_for_testing_{i}"
                        )
                    } else {
                        format!("table_{i}")
                    };
                    TreeItem::new(format!("table-{i}"), label)
                })
                .collect::<Vec<_>>(),
        );

    let relationships = TreeItem::new("relationships", "Relationships")
        .expanded(true)
        .children(
            (0..40)
                .map(|i| {
                    let label = if i % 5 == 0 {
                        format!("fk_{i}_references_some_other_very_long_table_name_column")
                    } else {
                        format!("relationship_{i}")
                    };
                    TreeItem::new(format!("rel-{i}"), label)
                })
                .collect::<Vec<_>>(),
        );

    vec![tables, relationships]
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
