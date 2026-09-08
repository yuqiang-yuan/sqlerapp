//! Database type system.
//!
//! One strongly-typed enum per dialect — there is **no** neutral/canonical
//! `DataType`. A document's dialect is fixed at creation and never switches,
//! so each enum only ever serves its own documents and there is no
//! cross-dialect translation loss.
//!
//! Every enum has an [`Other`] escape hatch: a syntactically valid type the
//! tool doesn't recognize is stored verbatim and round-tripped losslessly,
//! rather than rejected. Version differences are **not** modeled — whatever
//! the user writes is stored and emitted as-is (including deprecated syntax
//! such as MySQL `display_width`).

pub mod mysql;
pub mod postgres;

pub use mysql::MySqlType;
pub use postgres::PostgresType;

/// The dialect a document is bound to, fixed at creation and never switched.
///
/// This is the **name** of the dialect; `DialectType` carries the actual type
/// enum for a column. One day each arm may dispatch to a `Dialect` impl that
/// parses/emits DDL for that dialect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DialectName {
    MySql,
    Postgres,
}

/// The dialect a document is bound to, fixed at creation.
///
/// Wraps the per-dialect type enum so `Column.ty` has one concrete type
/// regardless of dialect, without paying for a neutral canonical type.
#[derive(Clone, Debug, PartialEq)]
pub enum DialectType {
    MySql(MySqlType),
    Postgres(PostgresType),
}
