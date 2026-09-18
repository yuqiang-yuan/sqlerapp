//! The ER diagram canvas — a hybrid "nodes are DOM, edges are canvas" view.
//!
//! Tables render as absolutely-positioned `div` cards (so text, theme colors,
//! and later click/edit come for free). Foreign-key relations render on a
//! `canvas` layer behind the cards using `PathBuilder::stroke` cubic Béziers
//! (via lyon tessellation) + a filled triangle arrow head — `Path` (the scene
//! primitive) fills triangles, so a stroked edge needs `PathBuilder`; `Svg` is
//! single-color alpha-mask, unsuited to a multi-color ER diagram, so edges use
//! `canvas` + `paint_path` instead.
//!
//! State lives entirely on `ErDocument`'s `GraphLayout` (positions, pan
//! offset, zoom), so the view holds only the in-progress drag gesture, the
//! current selection, and the canvas origin (for edge hit-testing), and
//! everything else persists with the document. Coordinates:
//!   `screen = logical * scale + offset`
//! and dragging a table writes back `logical = (screen - offset) / scale`.
//!
//! The zoom/pan is a canvas-wide transform: card positions, card sizes, card
//! fonts, and edge stroke widths all go through the same mapping (every
//! dimension × `scale`), so zooming scales tables and FK edges as a unit
//! instead of just moving their corners. GPUI divs have no transform matrix
//! (only `Svg` does, and transforms there skip hit-testing), so the scaling is
//! applied per card rather than via one container transform. `GraphLayout`
//! serializes `offset`/`scale` into the `.sqler` file, so the viewport
//! survives save/reload.

use std::collections::BTreeMap;

use gpui_kit::component::ActiveTheme;
use gpui_kit::gpui::{
    BoxShadow, Canvas, Context, Entity, EventEmitter, IntoElement, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point, Render, ScrollDelta,
    ScrollWheelEvent, Styled, Window, canvas, div, hsla, point, px,
};
use gpui_kit::base::{ElementExt, StyledExt};
use gpui_kit::prelude::FluentBuilder;
// Bring in the GPUI element traits (ParentElement, StatefulInteractiveElement,
// Styled, etc.) the same way main.rs does via the glob.
use gpui_kit::*;

use crate::db::{DialectType, MySqlType, PostgresType};
use crate::document::ErDocument;
use crate::model::{Constraint, ConstraintId, GraphLayout, Table, TableId};

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

/// What is currently selected on the canvas. Transient view state — not
/// persisted with the document — so reloading a file starts with nothing
/// selected. Single selection: clicking a card selects the table, clicking an
/// edge selects the edge, clicking empty canvas clears it. Public so the SQL
/// panel can read the selection and subscribe to [`SelectionChanged`].
#[derive(Clone, Debug, PartialEq)]
pub enum Selection {
    /// No selection.
    None,
    /// A table card.
    Table(TableId),
    /// A foreign-key edge (identified by its constraint id).
    Edge(ConstraintId),
}

/// Emitted from [`ErCanvas::select`] whenever the selection changes, so a
/// sibling view (the SQL panel) can react without polling. Carries the new
/// selection; [`ErCanvas`] implements [`EventEmitter`] for this.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectionChanged(pub Selection);

impl Default for Selection {
    fn default() -> Self {
        Self::None
    }
}

/// The geometry of one foreign-key edge, in canvas-local screen coordinates
/// (i.e. `to_screen` output — scaled + panned, but **before** the canvas
/// element's own origin is added). Edge painting (adds the origin) and edge
/// hit-testing (subtracts the origin from the click) share this, so the line
/// you see is the line you can click.
struct EdgeGeom {
    id: ConstraintId,
    from: TableId,
    to: TableId,
    start: Point<Pixels>,
    end: Point<Pixels>,
    cp_a: Point<Pixels>,
    cp_b: Point<Pixels>,
    /// Arrowhead direction along x: +1 points right (into a card on the
    /// right), -1 points left.
    tip_dir: f32,
}

/// The ER canvas view. Holds only the drag gesture and selection — geometry
/// and viewport live on the document's `GraphLayout`.
pub struct ErCanvas {
    doc: Entity<ErDocument>,
    drag: Option<Drag>,
    selection: Selection,
    /// The canvas element's window origin, captured each frame via
    /// `on_prepaint`. Edge hit-testing subtracts this from the click position
    /// to compare against canvas-local edge geometry.
    canvas_origin: Point<Pixels>,
}

impl ErCanvas {
    pub fn new(doc: Entity<ErDocument>) -> Self {
        Self {
            doc,
            drag: None,
            selection: Selection::None,
            canvas_origin: point(px(0.), px(0.)),
        }
    }

    /// Card height in logical units for the given column count: header + one
    /// row per column (the card DOM must mirror this — see `table_card`).
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
        // the press landed on the canvas itself. First try to hit-test a
        // foreign-key edge under the cursor — clicking an edge selects it (and
        // does not start a pan). Otherwise clear the selection and start a pan.
        let local = point(
            ev.position.x - self.canvas_origin.x,
            ev.position.y - self.canvas_origin.y,
        );
        if let Some(id) = self.edge_at(local, cx) {
            self.select(Selection::Edge(id), cx);
            return;
        }
        self.select(Selection::None, cx);
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
                    // The offset is persisted with the document, so a pan is
                    // an edit just like dragging a table — mark dirty so the
                    // on-screen viewport can't silently diverge from the file.
                    doc.mark_dirty();
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
        // Wheel zoom toward the cursor, clamped to [0.25, 3.0]. The offset is
        // part of the same transform, so it rebalances around the cursor to
        // keep the logical point under it stationary.
        let dy = match ev.delta {
            ScrollDelta::Pixels(p) => p.y.as_f32(),
            ScrollDelta::Lines(l) => l.y * 40.0,
        };
        let cursor = ev.position;
        let canvas_origin = self.canvas_origin;
        self.doc.update(cx, |doc, cx| {
            let layout = &mut doc.layout;
            let old_scale = layout.scale;
            let new_scale = (old_scale * (1.0 - dy * 0.0015)).clamp(0.25, 3.0);
            if new_scale == old_scale {
                return;
            }
            // The wheel handler lives on the outer pane, but the transform is
            // relative to the canvas element itself: work in canvas-local
            // coordinates so the anchor point survives the pane moving or
            // being resized.
            let cx_local = cursor.x.as_f32() - canvas_origin.x.as_f32();
            let cy_local = cursor.y.as_f32() - canvas_origin.y.as_f32();
            // Keep the cursor anchored: logical = (cursor - offset) / old_scale,
            // then offset = cursor - logical * new_scale.
            let logical_x = (cx_local - layout.offset.0) / old_scale;
            let logical_y = (cy_local - layout.offset.1) / old_scale;
            layout.scale = new_scale;
            layout.offset.0 = cx_local - logical_x * new_scale;
            layout.offset.1 = cy_local - logical_y * new_scale;
            doc.mark_dirty();
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

    /// The current canvas selection (table, edge, or none).
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Set the selection, notifying only when it actually changes so re-clicking
    /// the same card doesn't churn the view. Emits [`SelectionChanged`] so a
    /// sibling SQL panel can react without polling.
    fn select(&mut self, sel: Selection, cx: &mut Context<Self>) {
        if self.selection != sel {
            self.selection = sel;
            cx.emit(SelectionChanged(self.selection.clone()));
            cx.notify();
        }
    }

    /// Find the nearest foreign-key edge within the click radius of `local`
    /// (canvas-local screen coordinates, i.e. already with the canvas origin
    /// subtracted). Returns its constraint id so the caller can select it.
    fn edge_at(&self, local: Point<Pixels>, cx: &App) -> Option<ConstraintId> {
        const HIT_RADIUS: f32 = 7.0;
        let doc = self.doc.read(cx);
        let mut best: Option<(f32, ConstraintId)> = None;
        for g in Self::local_edges(&doc.layout, &doc.schema.tables) {
            let d = point_to_bezier_dist(local, &g);
            if d <= HIT_RADIUS && best.as_ref().map_or(true, |(bd, _)| d < *bd) {
                best = Some((d, g.id));
            }
        }
        best.map(|(_, id)| id)
    }

    /// Compute every foreign-key edge's geometry in canvas-local screen
    /// coordinates (logical × scale + offset). Endpoints sit on the *scaled*
    /// card borders because `to_screen` scales the card-corner math, matching
    /// cards that are laid out at `CARD_W * scale` × `card_height * scale`.
    /// Shared by edge painting (which adds the canvas origin) and edge
    /// hit-testing, so the visible line and the clickable line stay in
    /// lockstep at any zoom.
    fn local_edges(layout: &GraphLayout, tables: &BTreeMap<TableId, Table>) -> Vec<EdgeGeom> {
        let mut out = Vec::new();
        for (from_id, from_table) in tables {
            let Some(&from_pos) = layout.positions.get(from_id) else {
                continue;
            };
            let from_h = Self::card_height(from_table.columns.len());
            for c in &from_table.constraints {
                let Constraint::ForeignKey {
                    id,
                    referenced_table,
                    ..
                } = c
                else {
                    continue;
                };
                let to_id = tables
                    .iter()
                    .find(|(_, t)| t.name == *referenced_table)
                    .map(|(tid, _)| tid.clone());
                let Some(to_id) = to_id else { continue };
                let Some(&to_pos) = layout.positions.get(&to_id) else {
                    continue;
                };
                let to_h = Self::card_height(
                    tables.get(&to_id).map(|t| t.columns.len()).unwrap_or(1),
                );

                // Pick the connecting edges from the cards' relative horizontal
                // positions, so the arrowhead always points into the referenced
                // (to) card regardless of where the tables happen to be laid out.
                let from_cx = from_pos.0 + CARD_W / 2.0;
                let to_cx = to_pos.0 + CARD_W / 2.0;
                let from_cy = from_pos.1 + from_h / 2.0;
                let to_cy = to_pos.1 + to_h / 2.0;
                let (start, end, tip_dir) = if from_cx <= to_cx {
                    // from is left of to: out the right edge, into the left
                    // edge, arrowhead points right (+x).
                    (
                        Self::to_screen((from_pos.0 + CARD_W, from_cy), layout),
                        Self::to_screen((to_pos.0, to_cy), layout),
                        1.0,
                    )
                } else {
                    // from is right of to: out the left edge, into the right
                    // edge, arrowhead points left (-x).
                    (
                        Self::to_screen((from_pos.0, from_cy), layout),
                        Self::to_screen((to_pos.0 + CARD_W, to_cy), layout),
                        -1.0,
                    )
                };

                // The S-curve edge: a stroked cubic Bézier. Two control points
                // — at the 1/3 and 2/3 horizontal marks — bias the curve toward
                // `start.y` leaving the source and toward `end.y` entering the
                // target, giving a smoother S than a single control point.
                let mid_x = (start.x.as_f32() + end.x.as_f32()) / 2.0;
                let cp_a = point(
                    px(start.x.as_f32() + (mid_x - start.x.as_f32()) * 0.6),
                    start.y,
                );
                let cp_b = point(
                    px(end.x.as_f32() - (end.x.as_f32() - mid_x) * 0.6),
                    end.y,
                );
                out.push(EdgeGeom {
                    id: id.clone(),
                    from: from_id.clone(),
                    to: to_id,
                    start,
                    end,
                    cp_a,
                    cp_b,
                    tip_dir,
                });
            }
        }
        out
    }

    /// Build the canvas layer that paints foreign-key edges. Edges sit behind
    /// the cards (this child comes first), so a card's opaque background
    /// naturally clips the line where it would cross a table body.
    ///
    /// Styling by selection state:
    /// - A **selected** edge lights up — a wide, low-alpha primary "halo"
    ///   behind a thicker solid primary stroke, plus a larger arrowhead. The
    ///   halo's alpha is tuned per theme so it glows on both light and dark.
    /// - An edge **connected to the selected table** (either endpoint) draws
    ///   in primary at normal width, so selecting a table highlights its
    ///   relations without the full glow.
    /// - Otherwise the edge is a neutral gray that contrasts with the canvas
    ///   background on both themes.
    fn edges_layer(&self) -> Canvas<()> {
        let doc = self.doc.clone();
        let selection = self.selection.clone();
        canvas(
            |_bounds, _window, _cx| {},
            move |bounds, _state, window, cx| {
                let theme = cx.theme();
                let layout = doc.read(cx).layout.clone();
                let tables = doc.read(cx).schema.tables.clone();
                // `paint_path` paints in window-content-absolute coordinates,
                // clipped by this element's content_mask. `local_edges` returns
                // coords local to the canvas pane, so offset every point by the
                // canvas element's own origin (cf. gpui-component
                // separator.rs:98).
                let origin = bounds.origin;
                let to_win = |p: Point<Pixels>| point(p.x + origin.x, p.y + origin.y);

                let sel_edge = match &selection {
                    Selection::Edge(id) => Some(id.clone()),
                    _ => None,
                };
                let sel_table = match &selection {
                    Selection::Table(id) => Some(id.clone()),
                    _ => None,
                };

                // Stroke widths are logical dimensions: scale them with the
                // canvas so the lines and arrowheads grow/shrink with the
                // cards instead of staying screen-fixed. A small floor keeps
                // them visible when zoomed far out.
                let scale = layout.scale.max(0.25);
                for g in Self::local_edges(&layout, &tables) {
                    let start = to_win(g.start);
                    let end = to_win(g.end);
                    let cp_a = to_win(g.cp_a);
                    let cp_b = to_win(g.cp_b);

                    let is_selected = sel_edge.as_ref() == Some(&g.id);
                    let is_connected = sel_table
                        .as_ref()
                        .map_or(false, |t| t == &g.from || t == &g.to);

                    // Stroke color + width by selection state. Selected and
                    // connected edges both use the theme accent; selected is
                    // thicker.
                    let stroke_color = if is_selected || is_connected {
                        theme.primary
                    } else if theme.is_dark() {
                        hsla(0.0, 0.0, 0.7, 1.0)
                    } else {
                        hsla(0.0, 0.0, 0.4, 1.0)
                    };
                    let stroke_w = if is_selected {
                        2.5
                    } else if is_connected {
                        2.0
                    } else {
                        1.5
                    } * scale;

                    // Glow halo only on the selected edge — a wide, low-alpha
                    // primary stroke behind the solid line so it reads as lit
                    // up. Alpha is higher on dark so the glow is visible.
                    if is_selected {
                        let halo_color =
                            theme.primary.opacity(if theme.is_dark() { 0.35 } else { 0.25 });
                        let mut halo = PathBuilder::stroke(px(6.0 * scale));
                        halo.move_to(start);
                        halo.cubic_bezier_to(end, cp_a, cp_b);
                        if let Ok(halo_path) = halo.build() {
                            window.paint_path(halo_path, halo_color);
                        }
                    }

                    let mut edge = PathBuilder::stroke(px(stroke_w));
                    edge.move_to(start);
                    edge.cubic_bezier_to(end, cp_a, cp_b);
                    if let Ok(edge_path) = edge.build() {
                        window.paint_path(edge_path, stroke_color);
                    }

                    // Solid triangular arrowhead at `end`, colored to match the
                    // stroke, larger when selected. Scaled like the stroke.
                    let s = if is_selected { 10.0 } else { 8.0 } * scale;
                    let base_x = end.x.as_f32() - g.tip_dir * s;
                    let base_l = point(px(base_x), px(end.y.as_f32() - s / 2.0));
                    let base_r = point(px(base_x), px(end.y.as_f32() + s / 2.0));
                    let mut arrow = PathBuilder::fill();
                    arrow.move_to(end);
                    arrow.line_to(base_l);
                    arrow.line_to(base_r);
                    arrow.line_to(end);
                    if let Ok(arrow_path) = arrow.build() {
                        window.paint_path(arrow_path, stroke_color);
                    }
                }
            },
        )
        // Fill the pane so `bounds` (and thus `bounds.origin`) reflects the
        // canvas element's real position; a zero-size canvas paints at the
        // wrong origin and its paths get clipped to nothing.
        .size_full()
    }

    /// One table card: absolutely positioned by `GraphLayout`. The whole
    /// card — position, size, paddings, fonts, border — is multiplied by
    /// `layout.scale`, so zooming scales the card as a unit and the edge
    /// layer's scaled-corner math keeps landing exactly on the card borders.
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
        let scale = layout.scale;
        // Every logical dimension × scale. Explicit height keeps it in
        // lockstep with `card_height` (the same formula the edge layer uses to
        // estimate card bounds), so edges land exactly on the card edges — and
        // taffy sizes the absolute element instead of collapsing it to zero.
        let card_w = CARD_W * scale;
        let card_h = Self::card_height(table.columns.len()) * scale;
        let header_h = CARD_HEADER_H * scale;
        let row_h = CARD_ROW_H * scale;
        let pad_x = px(8.0 * scale);
        // Font sizes: the card used Tailwind text classes (rem-based) before;
        // reproduce their rem ratios explicitly so the canvas zoom still
        // applies on top of the theme's global font size (rem base).
        let rem = view.theme().font_size.as_f32();
        let font_header = px(rem * scale);
        let font_col = px(rem * 0.875 * scale);
        let font_ty = px(rem * 0.75 * scale);

        let theme = view.theme();
        let id_clone = id.clone();
        let is_selected = self.selection == Selection::Table(id.clone());

        let rows = table.columns.iter().map(|col| {
            div()
                .px(pad_x)
                .h(px(row_h))
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_size(font_col).child(col.name.clone()))
                .child(
                    div()
                        .text_size(font_ty)
                        .text_color(theme.muted_foreground)
                        .child(type_label(&col.ty)),
                )
        });

        div()
            .id(id.0.clone())
            .absolute()
            .left(screen.x)
            .top(screen.y)
            .w(px(card_w))
            .h(px(card_h))
            .bg(theme.background)
            .border(px(scale.max(0.5)))
            .border_color(if is_selected { theme.primary } else { theme.border })
            .rounded(px(6.0 * scale))
            .overflow_hidden()
            // Selected card: an accent glow (a primary-colored box-shadow with
            // blur) layered over a subtle drop shadow, so the selection reads
            // on both light and dark. The glow alpha is tuned per theme —
            // brighter on dark so it shows up against the dark canvas.
            .when(is_selected, |this| {
                let glow = theme.primary.opacity(if theme.is_dark() { 0.5 } else { 0.35 });
                this.shadow(vec![
                    BoxShadow::new(
                        px(0.),
                        px(4.),
                        hsla(0., 0., 0., if theme.is_dark() { 0.45 } else { 0.12 }),
                    )
                    .blur_radius(px(6.))
                    .spread_radius(px(-1.)),
                    BoxShadow::new(px(0.), px(0.), glow)
                        .blur_radius(px(10.))
                        .spread_radius(px(2.)),
                ])
            })
            .when(!is_selected, |this| this.shadow_sm())
            .on_mouse_down(
                gpui_kit::gpui::MouseButton::Left,
                view.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    // Selecting on mouse-down (not click) keeps it in lockstep
                    // with drag: press selects, drag moves, release ends.
                    this.select(Selection::Table(id_clone.clone()), cx);
                    this.begin_table_drag(id_clone.clone(), ev, window, cx);
                }),
            )
            .child(
                div()
                    .h(px(header_h))
                    .px(pad_x)
                    .flex()
                    .items_center()
                    .bg(theme.primary)
                    .text_color(theme.primary_foreground)
                    .text_size(font_header)
                    .font_bold()
                    .child(table.name.clone()),
            )
            .children(rows)
    }
}

/// `ErCanvas` emits [`SelectionChanged`] whenever the selection changes, so a
/// sibling SQL panel can react (regenerate the editor's SQL) without polling.
impl EventEmitter<SelectionChanged> for ErCanvas {}

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
            // Capture the canvas element's window origin each frame so
            // `on_mouse_down` can convert a window-coordinate click into
            // canvas-local coordinates for edge hit-testing.
            .on_prepaint({
                let view = cx.entity();
                move |bounds, _window, cx| {
                    let _ = view.update(cx, |this, _cx| this.canvas_origin = bounds.origin);
                }
            })
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

/// Sample a cubic Bézier at `t` ∈ [0, 1].
fn bezier_point(
    t: f32,
    p0: Point<Pixels>,
    p1: Point<Pixels>,
    p2: Point<Pixels>,
    p3: Point<Pixels>,
) -> Point<Pixels> {
    let u = 1.0 - t;
    let a = u * u * u;
    let b = 3.0 * u * u * t;
    let c = 3.0 * u * t * t;
    let d = t * t * t;
    point(
        px(a * p0.x.as_f32() + b * p1.x.as_f32() + c * p2.x.as_f32() + d * p3.x.as_f32()),
        px(a * p0.y.as_f32() + b * p1.y.as_f32() + c * p2.y.as_f32() + d * p3.y.as_f32()),
    )
}

/// Minimum distance from `p` to the cubic Bézier edge, by sampling the curve
/// into short segments and taking the point-to-segment distance. 24 samples
/// is plenty at card scale.
fn point_to_bezier_dist(p: Point<Pixels>, g: &EdgeGeom) -> f32 {
    const N: usize = 24;
    let mut prev = g.start;
    let mut min = f32::MAX;
    for i in 1..=N {
        let t = i as f32 / N as f32;
        let cur = bezier_point(t, g.start, g.cp_a, g.cp_b, g.end);
        let d = dist_to_segment(p, prev, cur);
        if d < min {
            min = d;
        }
        prev = cur;
    }
    min
}

/// Distance from point `p` to segment `a`–`b`.
fn dist_to_segment(p: Point<Pixels>, a: Point<Pixels>, b: Point<Pixels>) -> f32 {
    let (px, py) = (p.x.as_f32(), p.y.as_f32());
    let (ax, ay) = (a.x.as_f32(), a.y.as_f32());
    let (bx, by) = (b.x.as_f32(), b.y.as_f32());
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (ax + t * dx, ay + t * dy);
    let (ex, ey) = (px - cx, py - cy);
    (ex * ex + ey * ey).sqrt()
}
