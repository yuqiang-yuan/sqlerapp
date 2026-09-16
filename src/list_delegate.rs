//! List delegate for the editor's left panel: lists the tables and
//! relationships of the open [`ErDocument`], in two sections.
//!
//! The delegate is a thin view over the document — it holds an
//! [`Entity<ErDocument>`] handle and reads the current schema on every
//! `items_count` / `render_item` call. The document stays the single source of
//! truth; this type owns no copy of the data.

use gpui_kit::component::list::{ListDelegate, ListItem, ListState};
use gpui_kit::component::{ActiveTheme, IndexPath};
use gpui_kit::gpui::{App, Context, Entity, IntoElement, ParentElement, Styled, Window, div};

use crate::document::ErDocument;
use crate::model::Constraint;

/// Section 0 = tables, section 1 = relationships.
const SECTION_TABLES: usize = 0;
const SECTION_RELATIONSHIPS: usize = 1;

/// The list data source for the objects panel. Holds a borrowed handle to the
/// document; `ErDocument` remains pure data and is owned by `MyApp`.
pub struct ObjectsListDelegate {
    pub doc: Entity<ErDocument>,
}

impl ListDelegate for ObjectsListDelegate {
    type Item = ListItem;

    fn sections_count(&self, _cx: &App) -> usize {
        2
    }

    fn items_count(&self, section: usize, cx: &App) -> usize {
        let doc = self.doc.read(cx);
        match section {
            SECTION_TABLES => doc.schema.tables.len(),
            SECTION_RELATIONSHIPS => doc
                .schema
                .tables
                .values()
                .map(|t| {
                    t.constraints
                        .iter()
                        .filter(|c| matches!(c, Constraint::ForeignKey { .. }))
                        .count()
                })
                .sum(),
            _ => 0,
        }
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let doc = self.doc.read(cx);
        match ix.section {
            SECTION_TABLES => {
                let table = doc.schema.tables.values().nth(ix.row)?;
                Some(
                    ListItem::new(ix)
                        .child(table.name.clone()),
                )
            }
            SECTION_RELATIONSHIPS => {
                // Walk tables in order, yielding each foreign key as a flat row.
                let mut row = 0usize;
                for table in doc.schema.tables.values() {
                    for c in &table.constraints {
                        if let Constraint::ForeignKey {
                            referenced_table, ..
                        } = c
                        {
                            if row == ix.row {
                                let label =
                                    format!("{} → {}", table.name, referenced_table);
                                return Some(ListItem::new(ix).child(label));
                            }
                            row += 1;
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn render_section_header(
        &mut self,
        section: usize,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<impl IntoElement> {
        let title = match section {
            SECTION_TABLES => "Tables",
            SECTION_RELATIONSHIPS => "Relationships",
            _ => return None,
        };
        Some(
            div()
                .px_3()
                .py_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(title)
                .into_any_element(),
        )
    }

    fn set_selected_index(
        &mut self,
        _ix: Option<IndexPath>,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) {
        cx.notify();
    }
}
