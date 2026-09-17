//! The ER diagram canvas — a hybrid "nodes are DOM, edges are canvas" view.
//!
//! Tables render as absolutely-positioned `div` cards (so text, theme colors,
//! and later click/edit come for free). Foreign-key relations render on a
//! `canvas` layer behind the cards using `Path::curve_to` + a triangle arrow
//! head — `Svg` is single-color alpha-mask (see memory
//! `gpui-secondary-modifier-portable-keybinding` context), unsuited to a
//! multi-color ER diagram, so edges use `canvas` + `paint_path` instead.
//!
//! State lives entirely on `ErDocument`'s `GraphLayout` (positions, pan
//! offset, zoom scale), so the view holds only the in-progress drag gesture
//! and everything else persists with the document. Coordinates:
//!   `screen = logical * scale + offset`
//! and dragging a table writes back `logical = (screen - offset) / scale`.

use std::collections::BTreeMap;

use gpui_kit::component::ActiveTheme;
use gpui_kit::gpui::{
    Canvas, Context, Entity, IntoElement, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Path,
    Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, Styled, Window, canvas, div, point, px,
};
use gpui_kit::base::StyledExt;
// Bring in the GPUI element traits (ParentElement, StatefulInteractiveElement,
// Styled, etc.) the same way main.rs does via the glob.
use gpui_kit::*;

use crate::db::{DialectType, MySqlType, PostgresType};
use crate::document::ErDocument;
use crate::model::{Constraint, GraphLayout, TableId};

/// Fixed card geometry so the canvas-edge layer can estimate each card's
/// bounds from `GraphLayout.positions` without measuring the DOM (the canvas
/// paints before the card children lay out). Keep these in sync with the
/// card rendering below.
const CARD_W: f32 = 200.0;
const CARD_HEADER_H: f32 = 32.0;
const CARD_ROW_H: f32 = 22.0;

/// What the pointer is currently dragging on the canvas.
#[derive(Clone, Debug)]
enum Drag {
    /// Panning the whole canvas.
    Pan { last: Point<Pixels> },
    /// Moving a table card; write its new logical position on each move.
    Table { id: TableId, last: Point<Pixels> },
}

/// The ER canvas view. Holds only the drag gesture — geometry and viewport
/// live on the document's `GraphLayout`.
pub struct ErCanvas {
    doc: Entity<ErDocument>,
    drag: Option<Drag>,
}

impl ErCanvas {
    pub fn new(doc: Entity<ErDocument>) -> Self {
        Self { doc, drag: None }
    }

    /// Card height for the given column count: header + one row per column.
    fn card_height(columns: usize) -> f32 {
        CARD_HEADER_H + CARD_ROW_H * columns.max(1) as f32
    }

    /// `screen = logical * scale + offset`, as a `Point<Pixels>`.
    fn to_screen(logical: (f32, f32), layout: &GraphLayout) -> Point<Pixels> {
        point(
            px(logical.0 * layout.scale + layout.offset.0),
            px(logical.1 * layout.scale + layout.offset.1),
        )
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        // A card's `on_mouse_down` stops propagation, so reaching here means
        // the press landed on empty canvas → start a pan.
        self.drag = Some(Drag::Pan { last: ev.position });
        cx.notify();
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.clone() else {
            return;
        };
        match drag {
            Drag::Pan { last } => {
                let dx = (ev.position.x - last.x).as_f32();
                let dy = (ev.position.y - last.y).as_f32();
                self.doc.update(cx, |doc, cx| {
                    doc.layout.offset.0 += dx;
                    doc.layout.offset.1 += dy;
                    cx.notify();
                });
                self.drag = Some(Drag::Pan { last: ev.position });
            }
            Drag::Table { id, last } => {
                // Convert the screen delta to logical units before writing back.
                let scale = self.doc.read(cx).layout.scale;
                let dx = (ev.position.x - last.x).as_f32() / scale;
                let dy = (ev.position.y - last.y).as_f32() / scale;
                self.doc.update(cx, |doc, cx| {
                    if let Some(pos) = doc.layout.positions.get_mut(&id) {
                        pos.0 += dx;
                        pos.1 += dy;
                    }
                    doc.mark_dirty();
                    cx.notify();
                });
                self.drag = Some(Drag::Table { id, last: ev.position });
            }
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }

    fn on_scroll_wheel(&mut self, ev: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        // Wheel zoom toward the cursor, clamped to [0.25, 3.0].
        let dy = match ev.delta {
            ScrollDelta::Pixels(p) => p.y.as_f32(),
            ScrollDelta::Lines(l) => l.y * 40.0,
        };
        let cursor = ev.position;
        self.doc.update(cx, |doc, cx| {
            let layout = &mut doc.layout;
            let old_scale = layout.scale;
            let new_scale = (old_scale * (1.0 - dy * 0.0015)).clamp(0.25, 3.0);
            if new_scale == old_scale {
                return;
            }
            // Keep `cursor` anchored: logical = (cursor - offset) / old_scale,
            // then offset = cursor - logical * new_scale.
            let logical_x = (cursor.x.as_f32() - layout.offset.0) / old_scale;
            let logical_y = (cursor.y.as_f32() - layout.offset.1) / old_scale;
            layout.scale = new_scale;
            layout.offset.0 = cursor.x.as_f32() - logical_x * new_scale;
            layout.offset.1 = cursor.y.as_f32() - logical_y * new_scale;
            cx.notify();
        });
    }

    /// Start dragging a table card. Called from the card's `on_mouse_down`,
    /// which stops propagation so the canvas pan handler never fires.
    fn begin_table_drag(
        &mut self,
        id: TableId,
        ev: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drag = Some(Drag::Table { id, last: ev.position });
        cx.notify();
    }

    /// Build the canvas layer that paints foreign-key edges. Edges sit behind
    /// the cards (this child comes first), so a card's opaque background
    /// naturally clips the line where it would cross a table body.
    fn edges_layer(&self) -> Canvas<()> {
        let doc = self.doc.clone();
        canvas(
            |_bounds, _window, _cx| {},
            move |bounds, _state, window, cx| {
                let _theme = cx.theme();
                // TODO: derive edge/arrow colors from the theme once the edge
                // layer is stable; high-contrast placeholders for now.
                let edge_color = gpui_kit::gpui::red();
                let arrow_color = gpui_kit::gpui::black();
                let layout = doc.read(cx).layout.clone();
                let tables = doc.read(cx).schema.tables.clone();
                // `paint_path` paints in window-content-absolute coordinates,
                // clipped by this element's content_mask (see gpui-pre
                // window.rs:4356). `to_screen` returns coords local to the
                // canvas pane, so offset every point by the canvas element's
                // own origin (cf. gpui-component separator.rs:98).
                let origin = bounds.origin;
                let to_win = |p: Point<Pixels>| point(p.x + origin.x, p.y + origin.y);

                for (from_id, from_table) in &tables {
                    let from_pos = match layout.positions.get(from_id) {
                        Some(p) => *p,
                        None => continue,
                    };
                    let from_h = Self::card_height(from_table.columns.len());
                    for c in &from_table.constraints {
                        let Constraint::ForeignKey {
                            referenced_table, ..
                        } = c
                        else {
                            continue;
                        };
                        // Find the referenced table by name.
                        let to_id = tables
                            .iter()
                            .find(|(_, t)| t.name == *referenced_table)
                            .map(|(id, _)| id.clone());
                        let Some(to_id) = to_id else { continue };
                        let Some(to_pos) = layout.positions.get(&to_id) else {
                            continue;
                        };
                        let to_h = Self::card_height(
                            tables.get(&to_id).map(|t| t.columns.len()).unwrap_or(1),
                        );

                        // Pick the connecting edges from the cards' relative
                        // horizontal positions, so the arrowhead always points
                        // into the referenced (to) card regardless of where the
                        // tables happen to be laid out (BTreeMap iteration order
                        // is by UUID, not by placement).
                        let from_cx = from_pos.0 + CARD_W / 2.0;
                        let to_cx = to_pos.0 + CARD_W / 2.0;
                        let from_cy = from_pos.1 + from_h / 2.0;
                        let to_cy = to_pos.1 + to_h / 2.0;

                        let (start, end, tip_dir) = if from_cx <= to_cx {
                            // from is left of to: out the right edge, into the
                            // left edge, arrowhead points right (+x).
                            (
                                to_win(Self::to_screen((from_pos.0 + CARD_W, from_cy), &layout)),
                                to_win(Self::to_screen((to_pos.0, to_cy), &layout)),
                                1.0,
                            )
                        } else {
                            // from is right of to: out the left edge, into the
                            // right edge, arrowhead points left (-x).
                            (
                                to_win(Self::to_screen((from_pos.0, from_cy), &layout)),
                                to_win(Self::to_screen((to_pos.0 + CARD_W, to_cy), &layout)),
                                -1.0,
                            )
                        };

                        // Smooth S-curve: a single control point at the
                        // horizontal midpoint, biased toward the start's y.
                        let cp = point(
                            px((start.x.as_f32() + end.x.as_f32()) / 2.0),
                            start.y,
                        );
                        let mut path = Path::new(start);
                        path.curve_to(end, cp);
                        window.paint_path(path, edge_color);

                        // Arrow head at `end`, pointing into the to card. The
                        // base sits on the opposite side of the tip from the
                        // travel direction.
                        let s = 6.0;
                        let base_x = end.x.as_f32() - tip_dir * s;
                        let base_l = point(px(base_x), px(end.y.as_f32() - s / 2.0));
                        let base_r = point(px(base_x), px(end.y.as_f32() + s / 2.0));
                        let mut arrow = Path::new(end);
                        arrow.line_to(base_l);
                        arrow.line_to(base_r);
                        arrow.line_to(end);
                        window.paint_path(arrow, arrow_color);
                    }
                }
            },
        )
        // Fill the pane so `bounds` (and thus `bounds.origin`) reflects the
        // canvas element's real position; a zero-size canvas paints at the
        // wrong origin and its paths get clipped to nothing.
        .size_full()
    }

    /// One table card: absolutely positioned by `GraphLayout`, scaled + panned.
    fn table_card(&self, id: &TableId, view: &Context<Self>) -> impl IntoElement {
        let doc = self.doc.read(view);
        let layout = doc.layout.clone();
        let table = doc
            .schema
            .tables
            .get(id)
            .expect("card id must reference an existing table");
        let pos = layout.positions.get(id).copied().unwrap_or((40.0, 40.0));
        let screen = Self::to_screen(pos, &layout);
        // Explicit height keeps it in lockstep with `card_height` (the same
        // formula the edge layer uses to estimate card bounds), so edges land
        // exactly on the card edges — and taffy sizes the absolute element
        // instead of collapsing it to zero.
        let card_h = Self::card_height(table.columns.len());
        let theme = view.theme();
        let id_clone = id.clone();

        let rows = table.columns.iter().map(|col| {
            div()
                .px_2()
                .h(px(CARD_ROW_H))
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_sm().child(col.name.clone()))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(type_label(&col.ty)),
                )
        });

        div()
            .id(id.0.clone())
            .absolute()
            .left(screen.x)
            .top(screen.y)
            .w(px(CARD_W))
            .h(px(card_h))
            // TODO: theme the card background; placeholder yellow for now.
            .bg(gpui_kit::gpui::yellow())
            .border_1()
            .border_color(gpui_kit::gpui::black())
            .rounded_md()
            .overflow_hidden()
            .shadow_sm()
            .on_mouse_down(
                gpui_kit::gpui::MouseButton::Left,
                view.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.begin_table_drag(id_clone.clone(), ev, window, cx);
                }),
            )
            .child(
                div()
                    .h(px(CARD_HEADER_H))
                    .px_2()
                    .flex()
                    .items_center()
                    .bg(theme.primary)
                    .text_color(theme.primary_foreground)
                    .font_bold()
                    .child(table.name.clone()),
            )
            .children(rows)
    }
}

impl Render for ErCanvas {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let positions: BTreeMap<TableId, (f32, f32)> =
            self.doc.read(cx).layout.positions.clone();
        let ids: Vec<TableId> = positions.keys().cloned().collect();
        let edges = self.edges_layer();
        let theme = cx.theme();

        div()
            .id("er-canvas")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.muted)
            .on_mouse_down(
                gpui_kit::gpui::MouseButton::Left,
                cx.listener(Self::on_mouse_down),
            )
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(
                gpui_kit::gpui::MouseButton::Left,
                cx.listener(Self::on_mouse_up),
            )
            .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
            .child(edges)
            // Build the cards eagerly — `table_card` borrows `self` and `cx`
            // per call, which the `.children(iter)` map closure can't hold.
            .children({
                let mut cards = Vec::new();
                for id in &ids {
                    cards.push(self.table_card(id, cx));
                }
                cards
            })
    }
}

/// A one-line label for a column type, shown in the card's right column.
fn type_label(ty: &DialectType) -> String {
    match ty {
        DialectType::MySql(m) => mysql_label(m),
        DialectType::Postgres(p) => pg_label(p),
    }
}

fn mysql_label(t: &MySqlType) -> String {
    use MySqlType::*;
    match t {
        TinyInt { .. } => "TINYINT".into(),
        SmallInt { .. } => "SMALLINT".into(),
        Int { .. } => "INT".into(),
        MediumInt { .. } => "MEDIUMINT".into(),
        BigInt { .. } => "BIGINT".into(),
        Decimal { .. } => "DECIMAL".into(),
        Float { .. } => "FLOAT".into(),
        Double { .. } => "DOUBLE".into(),
        Boolean => "BOOLEAN".into(),
        Char { .. } => "CHAR".into(),
        Varchar { .. } => "VARCHAR".into(),
        Text { .. } => "TEXT".into(),
        Enum { .. } => "ENUM".into(),
        Set { .. } => "SET".into(),
        Binary { .. } => "BINARY".into(),
        VarBinary { .. } => "VARBINARY".into(),
        Blob { .. } => "BLOB".into(),
        Date => "DATE".into(),
        Time { .. } => "TIME".into(),
        DateTime { .. } => "DATETIME".into(),
        Timestamp { .. } => "TIMESTAMP".into(),
        Year => "YEAR".into(),
        Bit { .. } => "BIT".into(),
        Other { text } => text.clone(),
    }
}

fn pg_label(t: &PostgresType) -> String {
    use PostgresType::*;
    match t {
        SmallInt => "SMALLINT".into(),
        Integer => "INTEGER".into(),
        BigInt => "BIGINT".into(),
        Serial => "SERIAL".into(),
        BigSerial => "BIGSERIAL".into(),
        Numeric { .. } => "NUMERIC".into(),
        Money => "MONEY".into(),
        Real => "REAL".into(),
        DoublePrecision => "DOUBLE PRECISION".into(),
        Char { .. } => "CHAR".into(),
        Varchar { .. } => "VARCHAR".into(),
        Text => "TEXT".into(),
        Bytea => "BYTEA".into(),
        Date => "DATE".into(),
        Time { .. } => "TIME".into(),
        Timestamp { .. } => "TIMESTAMP".into(),
        Interval { .. } => "INTERVAL".into(),
        Boolean => "BOOLEAN".into(),
        Uuid => "UUID".into(),
        Json => "JSON".into(),
        Jsonb => "JSONB".into(),
        Bit { .. } => "BIT".into(),
        Cidr => "CIDR".into(),
        Inet => "INET".into(),
        MacAddr => "MACADDR".into(),
        MacAddr8 => "MACADDR8".into(),
        Geometric { kind } => format!("{kind:?}"),
        Enum { .. } => "ENUM".into(),
        Array { inner } => format!("{}[]", pg_label(inner)),
        Other { text } => text.clone(),
    }
}
