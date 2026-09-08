//! The neutral logical ER model — the canvas/edit source of truth.
//!
//! A [`Schema`] owns tables, their columns, constraints and indexes, plus
//! schema-level type definitions (PG `CREATE TYPE ... AS ENUM`, etc.). The
//! database dialect is fixed per-document and lives on the document, not here.
//!
//! Key design decisions (from prior discussion):
//!
//! - `Column.ty` is [`crate::db::DialectType`] — one concrete type regardless
//!   of dialect; there is no neutral canonical type.
//! - Only object-bound comments are stored (`Table.comment`, `Column.comment`).
//!   Free-form inline `--` / `/* */` comments are ignored by the parser.
//! - Layout (canvas coordinates) is **not** here — it lives in a separate
//!   `GraphLayout` keyed by `TableId`, so `Table` is pure logical structure.
//! - `Relation` is a **derived** view over foreign-key constraints, not the
//!   source of truth; FKs live in [`Table::constraints`].
//! - Version differences are **not** modeled — what the user writes is stored
//!   and emitted verbatim.
//! - Enum types that PG requires to be declared via `CREATE TYPE` live in
//!   [`Schema::types`], referenced by name from columns. MySQL `ENUM` stays
//!   inline in `MySqlType::Enum`.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::db::DialectType;

// —— New-type identifiers ——————————————————————————————————————————————
// Wrapping a UUID string prevents mixing a table id with a column id, etc.
// The value is a UUID (string form), so no counter is needed — callers mint
// a fresh UUID when constructing an entity.

/// Identifier of a table within a schema.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TableId(pub String);

impl TableId {
    /// Mint a fresh table id.
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

/// Identifier of a column within a table.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ColumnId(pub String);

impl ColumnId {
    /// Mint a fresh column id.
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

/// Identifier of a constraint within a table.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConstraintId(pub String);

impl ConstraintId {
    /// Mint a fresh constraint id.
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

/// Identifier of an index within a table.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IndexId(pub String);

impl IndexId {
    /// Mint a fresh index id.
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for TableId {
    fn default() -> Self {
        Self::new()
    }
}
impl Default for ColumnId {
    fn default() -> Self {
        Self::new()
    }
}
impl Default for ConstraintId {
    fn default() -> Self {
        Self::new()
    }
}
impl Default for IndexId {
    fn default() -> Self {
        Self::new()
    }
}

/// Name key for a schema-level type definition.
pub type TypeName = String;

/// A logical ER schema — the source of truth for the canvas.
///
/// Dialect-agnostic in structure (the dialect is a document property); the
/// chosen dialect only constrains which arm of [`DialectType`] appears on
/// columns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schema {
    pub tables: BTreeMap<TableId, Table>,
    /// Schema-level type definitions: PG `CREATE TYPE name AS ENUM (...)`,
    /// composites, domains, ranges. MySQL `ENUM`/`SET` stay inline on the
    /// column type and never appear here.
    pub types: BTreeMap<TypeName, TypeDef>,
}

impl Schema {
    pub fn new() -> Self {
        Self::default()
    }
}

/// A schema-level type definition.
#[derive(Clone, Debug, PartialEq)]
pub enum TypeDef {
    /// `CREATE TYPE name AS ENUM ('a','b')` (PG).
    Enum { values: Vec<String> },

    /// Any type kind not yet modeled explicitly (PG range, composite, domain).
    /// `body` holds the verbatim definition for lossless round-trip.
    Other { body: String },
}

/// A table — the primary logical structure.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub id: TableId,
    pub name: String,
    pub columns: Vec<Column>,
    pub constraints: Vec<Constraint>,
    pub indexes: Vec<Index>,
    pub comment: Option<String>,
}

/// A column.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub id: ColumnId,
    pub name: String,
    pub ty: DialectType,
    pub nullable: bool,
    /// Default value stored as the raw token string (e.g.
    /// `CURRENT_TIMESTAMP`, `now()`, `''`, `'literal'`).
    pub default: Option<String>,
    /// MySQL `AUTO_INCREMENT`. PG expresses auto-increment via the
    /// `Serial`/`BigSerial` type variant instead, so this flag mainly serves
    /// MySQL and stays `false` for PG serial columns.
    pub auto_increment: bool,
    pub comment: Option<String>,
}

/// A table-level integrity constraint (parsed from `CREATE TABLE`).
#[derive(Clone, Debug, PartialEq)]
pub enum Constraint {
    /// `PRIMARY KEY (cols)` / `CONSTRAINT name PRIMARY KEY (cols)`.
    PrimaryKey {
        id: ConstraintId,
        name: Option<String>,
        columns: Vec<String>,
    },
    /// `UNIQUE (cols)` / `CONSTRAINT name UNIQUE (cols)`.
    Unique {
        id: ConstraintId,
        name: Option<String>,
        columns: Vec<String>,
    },
    /// A foreign-key declaration. The referenced table need not exist in the
    /// schema — that's a document-level validation, not this declaration's
    /// concern.
    ForeignKey {
        id: ConstraintId,
        name: Option<String>,
        columns: Vec<String>,
        referenced_table: String,
        referenced_columns: Vec<String>,
        on_delete: Option<ReferentialAction>,
        on_update: Option<ReferentialAction>,
    },
    /// `CHECK (expr)`.
    Check {
        id: ConstraintId,
        name: Option<String>,
        expr: String,
    },
}

impl Constraint {
    pub fn id(&self) -> ConstraintId {
        match self {
            Constraint::PrimaryKey { id, .. }
            | Constraint::Unique { id, .. }
            | Constraint::ForeignKey { id, .. }
            | Constraint::Check { id, .. } => id.clone(),
        }
    }
}

/// ON DELETE / ON UPDATE behavior of a foreign key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferentialAction {
    Cascade,
    SetNull,
    SetDefault,
    Restrict,
    NoAction,
}

/// A secondary index (`INDEX`) — a performance structure, not an integrity
/// constraint. A `UNIQUE INDEX` is folded into [`Index::unique`] rather than
/// a [`Constraint::Unique`].
#[derive(Clone, Debug, PartialEq)]
pub struct Index {
    pub id: IndexId,
    pub name: Option<String>,
    pub columns: Vec<IndexColumn>,
    pub unique: bool,
    pub comment: Option<String>,
}

/// A column targeted by an index. MySQL allows a prefix length like
/// `name(10)`; `prefix_length` captures it when present.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexColumn {
    pub name: String,
    pub prefix_length: Option<u32>,
}

/// A derived ER relation — a **view** over [`Constraint::ForeignKey`], not a
/// source of truth. May be recomputed from the schema at render time.
#[derive(Clone, Debug, PartialEq)]
pub struct Relation {
    /// The id of the source FK constraint this relation is derived from.
    pub from_constraint: ConstraintId,
    pub kind: RelationKind,
    /// The referencing table.
    pub from_table: TableId,
    /// The referenced table.
    pub to_table: TableId,
}

/// Relationship cardinality for display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelationKind {
    OneToOne,
    OneToMany,
}

/// Map of [`TableId`] → canvas position, stored separately from [`Table`]
/// so logical structure is layout-independent.
#[derive(Clone, Debug, Default)]
pub struct GraphLayout {
    pub positions: BTreeMap<TableId, (f32, f32)>,
}
