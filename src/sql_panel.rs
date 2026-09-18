//! The SQL panel — a code editor that mirrors the canvas selection and writes
//! edits back into the [`ErDocument`] on Apply.
//!
//! Selecting a table card loads its `CREATE TABLE`; selecting a foreign-key
//! edge loads its `ALTER TABLE … ADD CONSTRAINT … FOREIGN KEY …`. Both use
//! [`crate::sql`] to emit canonical SQL for the dialect and to parse edits back.
//! Editing the SQL flips `modified` and reveals an Apply button (checkmark) in
//! the bottom-right; Apply parses the edited SQL and folds the result into the
//! document's schema, preserving ids, indexes, comments, and `CHECK`
//! constraints (which the SQL doesn't round-trip).
//!
//! Two subscriptions drive the view:
//! - the canvas's [`SelectionChanged`] reloads the editor;
//! - the editor's [`InputEvent`] flips `modified`.
//!
//! `set_value` runs with `emit_events = false` internally (gpui-base
//! `input/base/state.rs`), so reloading the editor on a selection change does
//! **not** re-fire `Change` and wrongly mark the panel modified — only real
//! user edits do. `set_value` needs a `&mut Window`, which a `subscribe`
//! handler doesn't receive, so the selection handler reaches the window via
//! [`App::active_window`] + [`AnyWindowHandle::update`].

use gpui_kit::base::input::{EditorState, InputEvent};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Editor;
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::document::ErDocument;
use crate::er_canvas::{ErCanvas, Selection, SelectionChanged};
use crate::model::{Constraint, ConstraintId, TableId};
use crate::sql;

/// The bottom pane of the editor split: a SQL editor bound to the canvas
/// selection, with an Apply round-trip back into the document.
pub struct SqlPanel {
    doc: Entity<ErDocument>,
    er_canvas: Entity<ErCanvas>,
    editor: Entity<EditorState>,
    /// The SQL text last loaded into the editor — the baseline for
    /// `modified` detection.
    last_generated: String,
    /// Whether the editor text differs from `last_generated` (a user edit
    /// not yet applied). Drives the Apply button's visibility.
    modified: bool,
    /// Canvas selection → reload the editor.
    _sel_sub: Subscription,
    /// Editor text → recompute `modified`.
    _change_sub: Subscription,
}

impl SqlPanel {
    /// Build the panel bound to `doc` (the SQL source/sink) and `er_canvas`
    /// (the selection source). The editor starts empty until a selection
    /// arrives.
    pub fn new(
        doc: Entity<ErDocument>,
        er_canvas: Entity<ErCanvas>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                .default_value("")
        });

        // Reload the editor whenever the canvas selection changes.
        let sel_sub = cx.subscribe(&er_canvas, |this, _canvas, event: &SelectionChanged, cx| {
            this.on_selection_changed(&event.0, cx);
        });

        // Recompute `modified` on real user edits only — `set_value` (used to
        // reload on selection change) suppresses `Change`, so programmatic
        // updates never flip this.
        let change_sub = cx.subscribe(&editor, |this, editor, _event: &InputEvent, cx| {
            let text = editor.read(cx).text().to_string();
            this.modified = text != this.last_generated;
            cx.notify();
        });

        Self {
            doc,
            er_canvas,
            editor,
            last_generated: String::new(),
            modified: false,
            _sel_sub: sel_sub,
            _change_sub: change_sub,
        }
    }

    /// Emit the SQL for `sel` against the current document (empty for no
    /// selection). Table → `CREATE TABLE`; edge → the owning table's
    /// `ALTER TABLE … FOREIGN KEY …`.
    fn generate_sql(&self, sel: &Selection, cx: &App) -> String {
        let doc = self.doc.read(cx);
        let dialect = doc.dialect;
        match sel {
            Selection::None => String::new(),
            Selection::Table(id) => doc
                .schema
                .tables
                .get(id)
                .map(|t| sql::emit_table_sql(t, dialect))
                .unwrap_or_default(),
            Selection::Edge(cid) => {
                for table in doc.schema.tables.values() {
                    for c in &table.constraints {
                        if let Constraint::ForeignKey { id, .. } = c {
                            if id == cid {
                                return sql::emit_fk_sql(table, c, dialect);
                            }
                        }
                    }
                }
                String::new()
            }
        }
    }

    /// Reload the editor with the SQL for `sel`, reset the modified baseline,
    /// and re-render. `set_value` needs a `&mut Window`; the selection
    /// subscription handler has none, so it reaches the window via
    /// [`App::active_window`] (the panel lives in the single app window).
    fn on_selection_changed(&mut self, sel: &Selection, cx: &mut Context<Self>) {
        let sql = self.generate_sql(sel, cx);
        self.last_generated = sql.clone();
        self.modified = false;
        if let Some(handle) = cx.active_window() {
            let editor = self.editor.clone();
            let _ = handle.update(cx, |_view, window, app| {
                editor.update(app, |state, cx| state.set_value(sql, window, cx));
            });
        }
        cx.notify();
    }

    /// Parse the edited SQL and fold it back into the document. Dispatched from
    /// the Apply button (which has a `&mut Window`).
    fn apply_sql(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sel = self.er_canvas.read(cx).selection().clone();
        match &sel {
            Selection::None => {}
            Selection::Table(id) => self.apply_table(id, window, cx),
            Selection::Edge(cid) => self.apply_edge(cid, window, cx),
        }
    }

    /// Apply an edited `CREATE TABLE`. Replaces the table's name, columns, and
    /// PK/Unique/FK constraints with the parsed ones (which carry fresh ids),
    /// while preserving `CHECK` constraints (not round-tripped by the SQL),
    /// `indexes`, and `comment`. The table `id` is kept so the card keeps its
    /// position.
    fn apply_table(&mut self, id: &TableId, window: &mut Window, cx: &mut Context<Self>) {
        let dialect = self.doc.read(cx).dialect;
        let sql_text = self.editor.read(cx).text().to_string();
        let parsed = match sql::parse_table_sql(&sql_text, dialect) {
            Ok(p) => p,
            Err(e) => {
                window.push_notification(
                    (NotificationType::Error, format!("Parse error: {e}")),
                    cx,
                );
                return;
            }
        };
        self.doc.update(cx, |doc, cx| {
            if let Some(table) = doc.schema.tables.get_mut(id) {
                table.name = parsed.name;
                table.columns = parsed.columns;
                // Keep CHECK constraints (and any other non-round-tripped kind)
                // with their existing ids; PK/Unique/FK come back from the parse.
                let preserved: Vec<Constraint> = table
                    .constraints
                    .iter()
                    .filter(|c| matches!(c, Constraint::Check { .. }))
                    .cloned()
                    .collect();
                table.constraints = parsed.constraints;
                table.constraints.extend(preserved);
                doc.mark_dirty();
                cx.notify();
            }
        });
        self.reload_editor(&Selection::Table(id.clone()), window, cx);
        window.push_notification((NotificationType::Success, "Applied"), cx);
    }

    /// Apply an edited `ALTER TABLE … FOREIGN KEY`. Replaces just the selected
    /// FK constraint, **keeping its id** so the edge selection stays valid and
    /// the canvas re-renders the same edge.
    fn apply_edge(&mut self, cid: &ConstraintId, window: &mut Window, cx: &mut Context<Self>) {
        let dialect = self.doc.read(cx).dialect;
        let sql_text = self.editor.read(cx).text().to_string();
        let parsed = match sql::parse_fk_sql(&sql_text, dialect) {
            Ok(p) => p,
            Err(e) => {
                window.push_notification(
                    (NotificationType::Error, format!("Parse error: {e}")),
                    cx,
                );
                return;
            }
        };
        self.doc.update(cx, |doc, cx| {
            let mut found = false;
            for table in doc.schema.tables.values_mut() {
                if let Some(idx) = table.constraints.iter().position(|c| c.id() == *cid) {
                    table.constraints[idx] = Constraint::ForeignKey {
                        id: cid.clone(),
                        name: parsed.name,
                        columns: parsed.columns,
                        referenced_table: parsed.referenced_table,
                        referenced_columns: parsed.referenced_columns,
                        on_delete: parsed.on_delete,
                        on_update: parsed.on_update,
                    };
                    found = true;
                    break;
                }
            }
            if found {
                doc.mark_dirty();
                cx.notify();
            }
        });
        self.reload_editor(&Selection::Edge(cid.clone()), window, cx);
        window.push_notification((NotificationType::Success, "Applied"), cx);
    }

    /// Regenerate the SQL for `sel` from the (now-updated) document, load it
    /// back into the editor as the new baseline, and clear `modified` — the
    /// canonical emit may differ from what the user typed, so this keeps the
    /// editor and document in sync.
    fn reload_editor(&mut self, sel: &Selection, window: &mut Window, cx: &mut Context<Self>) {
        let sql = self.generate_sql(sel, cx);
        self.last_generated = sql.clone();
        self.modified = false;
        self.editor
            .update(cx, |state, cx| state.set_value(sql, window, cx));
        cx.notify();
    }
}

impl Render for SqlPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let show_apply = self.modified;
        let view = cx.entity();
        div()
            .id("sql-panel")
            .relative()
            .size_full()
            .v_flex()
            .child(
                // The styled `Editor` (gpui-component) applies the theme's
                // monospace family, a code line height, the editor color scheme,
                // and — via `Input::render`'s `ensure_highlighter_factory` — the
                // tree-sitter highlighter for the `sql` language set on the state.
                // Its built-in `.text_size` is the theme's fixed `mono_font_size`
                // (13px), which does NOT track the app's Small/Medium/Large font
                // setting, so override it with a rem-based size: `rem_size` is
                // `Theme::font_size` (set by Root::render), so `rems(0.875)`
                // scales the editor with that setting (≈14px at Medium). The
                // editor reads `window.text_style()` each frame, so a font-size
                // change + `window.refresh()` re-flows it.
                //
                // The editor's element sets `flex_grow = 1.0` and
                // `size.height = relative(1.)`, so it fills its container — but
                // only if the container is a flex column (flex_grow needs a flex
                // parent). Hence the `.v_flex()` above plus `.h(relative(1.))`
                // here, mirroring how gpui-kit's own inspector embeds an editor.
                Editor::new(&self.editor)
                    .h(relative(1.))
                    .text_size(rems(0.875))
                    .bordered(false),
            )
            .when(show_apply, |this| {
                this.child(
                    div()
                        .absolute()
                        .bottom_3()
                        .right_3()
                        .child(
                            Button::new("apply-sql")
                                .primary()
                                .icon(IconName::Check)
                                .label("Apply")
                                .tooltip("Parse the edited SQL back into the document")
                                .on_click(move |_, window, app| {
                                    view.update(app, |panel, cx| panel.apply_sql(window, cx));
                                }),
                        ),
                )
            })
    }
}
