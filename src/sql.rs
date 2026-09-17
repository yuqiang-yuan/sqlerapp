//! SQL emit + parse for the canvas SQL panel.
//!
//! The panel shows a table's `CREATE TABLE` (or an edge's `ALTER TABLE … ADD
//! CONSTRAINT … FOREIGN KEY …`) and, on Apply, parses the edited SQL back into
//! the [`crate::model`]. Emit is hand-written canonical SQL (dialect-appropriate
//! identifier quoting); parse uses [`sqlparser`] and maps its AST onto our
//! per-dialect type enums. Anything unrecognized round-trips through each type
//! enum's `Other { text }` escape hatch so Apply never loses data.

use sqlparser::ast::{
    self as sqlast, ColumnDef, ColumnOption, DataType, ExactNumberInfo, ObjectName, Statement,
    TableConstraint,
};
use sqlparser::dialect::{Dialect, MySqlDialect, PostgreSqlDialect};
use sqlparser::parser::Parser;

use crate::db::{DialectName, DialectType, MySqlType, PostgresType};
use crate::model::{Column, ColumnId, Constraint, ConstraintId, ReferentialAction, Table};

/// A `CREATE TABLE` parsed back from the editor, ready to fold into the schema.
/// Columns and constraints carry fresh ids — the caller swaps them in for the
/// selected table's, preserving the table id (so the card keeps its position).
pub struct ParsedTable {
    pub name: String,
    pub columns: Vec<Column>,
    /// Primary-key, unique, and foreign-key constraints parsed from the SQL.
    /// (`CHECK` constraints are not emitted, so they never appear here.)
    pub constraints: Vec<Constraint>,
}

/// An `ALTER TABLE … ADD CONSTRAINT … FOREIGN KEY …` parsed from the editor.
/// The caller assigns the selected edge's constraint id when swapping it in.
pub struct ParsedFk {
    pub table: String,
    pub name: Option<String>,
    pub columns: Vec<String>,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    pub on_delete: Option<ReferentialAction>,
    pub on_update: Option<ReferentialAction>,
}

// ——————————————————————————————————————————————————————————————————————————
// Emit (model → SQL)
// ——————————————————————————————————————————————————————————————————————————

fn quote_char(dialect: DialectName) -> char {
    match dialect {
        DialectName::MySql => '`',
        DialectName::Postgres => '"',
    }
}

/// Quote an identifier for the dialect, doubling any embedded quote char
/// (the SQL standard escaping rule).
fn quote_ident(name: &str, dialect: DialectName) -> String {
    let q = quote_char(dialect);
    let escaped = name.replace(q, format!("{q}{q}").as_str());
    format!("{q}{escaped}{q}")
}

fn ident_list(names: &[String], dialect: DialectName) -> String {
    let q = quote_char(dialect);
    names
        .iter()
        .map(|n| format!("{q}{}{q}", n.replace(q, format!("{q}{q}").as_str())))
        .collect::<Vec<_>>()
        .join(", ")
}

fn referential_action_sql(a: ReferentialAction) -> &'static str {
    match a {
        ReferentialAction::Cascade => "CASCADE",
        ReferentialAction::SetNull => "SET NULL",
        ReferentialAction::SetDefault => "SET DEFAULT",
        ReferentialAction::Restrict => "RESTRICT",
        ReferentialAction::NoAction => "NO ACTION",
    }
}

fn on_clause(prefix: &str, action: Option<ReferentialAction>) -> String {
    match action {
        Some(a) => format!("ON {prefix} {}", referential_action_sql(a)),
        None => String::new(),
    }
}

/// `DEFAULT <expr>` — the model stores the default as the raw token string, so
/// emit it verbatim (it already reads as SQL: `'abc'`, `0`, `CURRENT_TIMESTAMP`).
fn default_clause(default: &Option<String>) -> String {
    match default {
        Some(d) if !d.trim().is_empty() => format!("DEFAULT {d}"),
        _ => String::new(),
    }
}

/// Emit a per-dialect type in its canonical SQL form **with** args (unlike the
/// display-only `type_label` in `er_canvas.rs`, which drops them).
pub fn type_to_sql(ty: &DialectType) -> String {
    match ty {
        DialectType::MySql(m) => mysql_type_to_sql(m),
        DialectType::Postgres(p) => pg_type_to_sql(p),
    }
}

fn opt_width(w: Option<u32>) -> String {
    match w {
        Some(n) => format!("({n})"),
        None => String::new(),
    }
}

fn mysql_type_to_sql(t: &MySqlType) -> String {
    use MySqlType::*;
    match t {
        TinyInt { unsigned, display_width } => {
            int_sql("TINYINT", *unsigned, *display_width)
        }
        SmallInt { unsigned, display_width } => {
            int_sql("SMALLINT", *unsigned, *display_width)
        }
        Int { unsigned, display_width } => int_sql("INT", *unsigned, *display_width),
        MediumInt { unsigned, display_width } => {
            int_sql("MEDIUMINT", *unsigned, *display_width)
        }
        BigInt { unsigned, display_width } => int_sql("BIGINT", *unsigned, *display_width),
        Decimal { precision, scale } => dec_sql("DECIMAL", *precision, *scale),
        Float { precision, scale } => dec_sql("FLOAT", *precision, *scale),
        Double { precision, scale } => dec_sql("DOUBLE", *precision, *scale),
        Boolean => "BOOLEAN".into(),
        Char { length } => format!("CHAR{}", opt_width(*length)),
        Varchar { length } => format!("VARCHAR{}", opt_width(*length)),
        Text { size } => text_size_sql(size).into(),
        Enum { values } => enum_sql("ENUM", values),
        Set { values } => enum_sql("SET", values),
        Binary { length } => format!("BINARY{}", opt_width(*length)),
        VarBinary { length } => format!("VARBINARY{}", opt_width(*length)),
        Blob { size } => blob_size_sql(size).into(),
        Date => "DATE".into(),
        Time { precision } => format!("TIME{}", opt_width(*precision)),
        DateTime { precision } => format!("DATETIME{}", opt_width(*precision)),
        Timestamp { precision } => format!("TIMESTAMP{}", opt_width(*precision)),
        Year => "YEAR".into(),
        Bit { length } => format!("BIT{}", opt_width(*length)),
        Other { text } => text.clone(),
    }
}

fn int_sql(name: &str, unsigned: bool, display_width: Option<u32>) -> String {
    let mut s = name.to_string();
    if let Some(w) = display_width {
        s.push_str(&format!("({w})"));
    }
    if unsigned {
        s.push_str(" UNSIGNED");
    }
    s
}

fn dec_sql(name: &str, precision: Option<u32>, scale: Option<u32>) -> String {
    match (precision, scale) {
        (Some(p), Some(s)) => format!("{name}({p},{s})"),
        (Some(p), None) => format!("{name}({p})"),
        _ => name.to_string(),
    }
}

fn text_size_sql(size: &crate::db::mysql::TextSize) -> &'static str {
    use crate::db::mysql::TextSize::*;
    match size {
        Tiny => "TINYTEXT",
        Normal => "TEXT",
        Medium => "MEDIUMTEXT",
        Long => "LONGTEXT",
    }
}

fn blob_size_sql(size: &crate::db::mysql::BlobSize) -> &'static str {
    use crate::db::mysql::BlobSize::*;
    match size {
        Tiny => "TINYBLOB",
        Normal => "BLOB",
        Medium => "MEDIUMBLOB",
        Long => "LONGBLOB",
    }
}

fn enum_sql(name: &str, values: &[String]) -> String {
    let inner = values
        .iter()
        .map(|v| format!("'{}'", v.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");
    format!("{name}({inner})")
}

fn pg_type_to_sql(t: &PostgresType) -> String {
    use PostgresType::*;
    match t {
        SmallInt => "SMALLINT".into(),
        Integer => "INTEGER".into(),
        BigInt => "BIGINT".into(),
        Serial => "SERIAL".into(),
        BigSerial => "BIGSERIAL".into(),
        Numeric { precision, scale } => dec_sql("NUMERIC", *precision, *scale),
        Money => "MONEY".into(),
        Real => "REAL".into(),
        DoublePrecision => "DOUBLE PRECISION".into(),
        Char { length } => format!("CHAR{}", opt_width(*length)),
        Varchar { length } => format!("VARCHAR{}", opt_width(*length)),
        Text => "TEXT".into(),
        Bytea => "BYTEA".into(),
        Date => "DATE".into(),
        Time { precision, with_tz } => {
            let mut s = format!("TIME{}", opt_width(*precision));
            if *with_tz {
                s.push_str(" WITH TIME ZONE");
            }
            s
        }
        Timestamp { precision, with_tz } => {
            let mut s = format!("TIMESTAMP{}", opt_width(*precision));
            if *with_tz {
                s.push_str(" WITH TIME ZONE");
            }
            s
        }
        Interval { fields } => match fields {
            Some(f) => format!("INTERVAL {f}"),
            None => "INTERVAL".into(),
        },
        Boolean => "BOOLEAN".into(),
        Uuid => "UUID".into(),
        Json => "JSON".into(),
        Jsonb => "JSONB".into(),
        Bit { length, varying } => {
            let base = if *varying { "BIT VARYING" } else { "BIT" };
            format!("{base}{}", opt_width(*length))
        }
        Cidr => "CIDR".into(),
        Inet => "INET".into(),
        MacAddr => "MACADDR".into(),
        MacAddr8 => "MACADDR8".into(),
        Geometric { kind } => format!("{kind:?}").to_uppercase(),
        Enum { values } => enum_sql("ENUM", values),
        Array { inner } => format!("{}[]", pg_type_to_sql(inner)),
        Other { text } => text.clone(),
    }
}

/// Emit a full `CREATE TABLE` for a table — columns, then `PRIMARY KEY` /
/// `UNIQUE` / `FOREIGN KEY` table constraints. `CHECK` constraints and indexes
/// are intentionally not emitted: they're not round-tripped by Apply, so
/// showing them would be misleading.
pub fn emit_table_sql(table: &Table, dialect: DialectName) -> String {
    let mut lines: Vec<String> = Vec::new();

    for col in &table.columns {
        let mut parts = vec![
            quote_ident(&col.name, dialect),
            type_to_sql(&col.ty),
        ];
        if !col.nullable {
            parts.push("NOT NULL".into());
        }
        let dc = default_clause(&col.default);
        if !dc.is_empty() {
            parts.push(dc);
        }
        if col.auto_increment {
            parts.push("AUTO_INCREMENT".into());
        }
        lines.push(format!("  {}", parts.join(" ")));
    }

    // Table-level constraints. (Column-level PRIMARY KEY / UNIQUE are folded up
    // to table level for a canonical, consistent emit.)
    for c in &table.constraints {
        match c {
            Constraint::PrimaryKey { name, columns, .. } => {
                let cname = name
                    .as_ref()
                    .map(|n| format!("CONSTRAINT {} ", quote_ident(n, dialect)))
                    .unwrap_or_default();
                lines.push(format!(
                    "  {cname}PRIMARY KEY ({})",
                    ident_list(columns, dialect)
                ));
            }
            Constraint::Unique { name, columns, .. } => {
                let cname = name
                    .as_ref()
                    .map(|n| format!("CONSTRAINT {} ", quote_ident(n, dialect)))
                    .unwrap_or_default();
                lines.push(format!(
                    "  {cname}UNIQUE ({})",
                    ident_list(columns, dialect)
                ));
            }
            Constraint::ForeignKey {
                name,
                columns,
                referenced_table,
                referenced_columns,
                on_delete,
                on_update,
                ..
            } => {
                lines.push(format!("  {}", fk_clause(name, columns, referenced_table, referenced_columns, on_delete, on_update, dialect)));
            }
            Constraint::Check { .. } => {}
        }
    }

    format!(
        "CREATE TABLE {} (\n{}\n);",
        quote_ident(&table.name, dialect),
        lines.join(",\n")
    )
}

/// The `CONSTRAINT name FOREIGN KEY (…) REFERENCES … (…) [ON DELETE …] [ON UPDATE …]`
/// clause, shared by the table emit and the standalone `ALTER TABLE` emit.
fn fk_clause(
    name: &Option<String>,
    columns: &[String],
    referenced_table: &str,
    referenced_columns: &[String],
    on_delete: &Option<ReferentialAction>,
    on_update: &Option<ReferentialAction>,
    dialect: DialectName,
) -> String {
    let cname = name
        .as_ref()
        .map(|n| format!("CONSTRAINT {} ", quote_ident(n, dialect)))
        .unwrap_or_default();
    let mut s = format!(
        "{cname}FOREIGN KEY ({}) REFERENCES {} ({})",
        ident_list(columns, dialect),
        quote_ident(referenced_table, dialect),
        ident_list(referenced_columns, dialect),
    );
    let od = on_clause("DELETE", *on_delete);
    if !od.is_empty() {
        s.push_str(&format!(" {od}"));
    }
    let ou = on_clause("UPDATE", *on_update);
    if !ou.is_empty() {
        s.push_str(&format!(" {ou}"));
    }
    s
}

/// Emit a standalone `ALTER TABLE … ADD CONSTRAINT … FOREIGN KEY …` for a
/// selected edge.
pub fn emit_fk_sql(
    table: &Table,
    fk: &Constraint,
    dialect: DialectName,
) -> String {
    let Constraint::ForeignKey {
        name,
        columns,
        referenced_table,
        referenced_columns,
        on_delete,
        on_update,
        ..
    } = fk
    else {
        return String::new();
    };
    format!(
        "ALTER TABLE {} ADD {};",
        quote_ident(&table.name, dialect),
        fk_clause(
            name,
            columns,
            referenced_table,
            referenced_columns,
            on_delete,
            on_update,
            dialect
        )
    )
}

// ——————————————————————————————————————————————————————————————————————————
// Parse (SQL → model)
// ——————————————————————————————————————————————————————————————————————————

fn dialect_for(dialect: DialectName) -> Box<dyn Dialect> {
    match dialect {
        DialectName::MySql => Box::new(MySqlDialect {}),
        DialectName::Postgres => Box::new(PostgreSqlDialect {}),
    }
}

/// The last segment of a (possibly schema-qualified) [`ObjectName`] as a plain
/// string — `public.users` → `users`.
fn object_name_last(name: &ObjectName) -> String {
    name.0.last().map(|i| i.value.clone()).unwrap_or_default()
}

fn map_referential_action(a: &sqlast::ReferentialAction) -> ReferentialAction {
    match a {
        sqlast::ReferentialAction::Restrict => ReferentialAction::Restrict,
        sqlast::ReferentialAction::Cascade => ReferentialAction::Cascade,
        sqlast::ReferentialAction::SetNull => ReferentialAction::SetNull,
        sqlast::ReferentialAction::NoAction => ReferentialAction::NoAction,
        sqlast::ReferentialAction::SetDefault => ReferentialAction::SetDefault,
    }
}

/// Parse a [`sqlparser`] type onto the matching per-dialect enum arm. Unknown
/// types fall through to `Other { text }` (the verbatim SQL text) so Apply
/// round-trips them losslessly instead of failing.
fn parse_type(dt: &DataType, dialect: DialectName) -> DialectType {
    match dialect {
        DialectName::MySql => DialectType::MySql(parse_mysql_type(dt)),
        DialectName::Postgres => DialectType::Postgres(parse_pg_type(dt)),
    }
}

fn char_len(opt: &Option<sqlast::CharacterLength>) -> Option<u32> {
    match opt {
        Some(sqlast::CharacterLength::IntegerLength { length, .. }) => Some(*length as u32),
        _ => None,
    }
}

fn exact_num(info: &ExactNumberInfo) -> (Option<u32>, Option<u32>) {
    match info {
        ExactNumberInfo::None => (None, None),
        ExactNumberInfo::Precision(p) => (Some(*p as u32), None),
        ExactNumberInfo::PrecisionAndScale(p, s) => (Some(*p as u32), Some(*s as u32)),
    }
}

fn parse_mysql_type(dt: &DataType) -> MySqlType {
    use MySqlType::*;
    use crate::db::mysql::{BlobSize, TextSize};
    let signed_int = |w: Option<u64>| MySqlType::Int {
        unsigned: false,
        display_width: w.map(|x| x as u32),
    };
    match dt {
        DataType::TinyInt(w) => TinyInt { unsigned: false, display_width: w.map(|x| x as u32) },
        DataType::UnsignedTinyInt(w) => TinyInt { unsigned: true, display_width: w.map(|x| x as u32) },
        DataType::SmallInt(w) => SmallInt { unsigned: false, display_width: w.map(|x| x as u32) },
        DataType::UnsignedSmallInt(w) => SmallInt { unsigned: true, display_width: w.map(|x| x as u32) },
        DataType::MediumInt(w) => MediumInt { unsigned: false, display_width: w.map(|x| x as u32) },
        DataType::UnsignedMediumInt(w) => MediumInt { unsigned: true, display_width: w.map(|x| x as u32) },
        DataType::Int(w) | DataType::Integer(w) => signed_int(*w),
        DataType::UnsignedInt(w) | DataType::UnsignedInteger(w) => MySqlType::Int {
            unsigned: true,
            display_width: w.map(|x| x as u32),
        },
        DataType::BigInt(w) => BigInt { unsigned: false, display_width: w.map(|x| x as u32) },
        DataType::UnsignedBigInt(w) => BigInt { unsigned: true, display_width: w.map(|x| x as u32) },
        DataType::Decimal(info) | DataType::Numeric(info) => {
            let (p, s) = exact_num(info);
            Decimal { precision: p, scale: s }
        }
        DataType::Float(w) => Float { precision: w.map(|x| x as u32), scale: None },
        DataType::Double(_) | DataType::DoublePrecision => Double { precision: None, scale: None },
        DataType::Bool | DataType::Boolean => Boolean,
        DataType::Char(opt) | DataType::Character(opt) => Char { length: char_len(opt) },
        DataType::Varchar(opt)
        | DataType::CharVarying(opt)
        | DataType::CharacterVarying(opt) => Varchar { length: char_len(opt) },
        DataType::Text => Text { size: TextSize::Normal },
        DataType::TinyText => Text { size: TextSize::Tiny },
        DataType::MediumText => Text { size: TextSize::Medium },
        DataType::LongText => Text { size: TextSize::Long },
        DataType::Binary(w) => Binary { length: w.map(|x| x as u32) },
        DataType::Varbinary(w) => VarBinary { length: w.map(|x| x as u32) },
        DataType::Blob(w) => Blob {
            size: w.map(|_| BlobSize::Normal).unwrap_or(BlobSize::Normal),
        },
        DataType::TinyBlob => Blob { size: BlobSize::Tiny },
        DataType::MediumBlob => Blob { size: BlobSize::Medium },
        DataType::LongBlob => Blob { size: BlobSize::Long },
        DataType::Date => Date,
        DataType::Time(w, _) => Time { precision: w.map(|x| x as u32) },
        DataType::Datetime(w) => DateTime { precision: w.map(|x| x as u32) },
        DataType::Timestamp(w, _) => Timestamp { precision: w.map(|x| x as u32) },
        DataType::Bit(w) => Bit { length: w.map(|x| x as u32) },
        DataType::Enum(members, _) => Enum {
            values: members
                .iter()
                .map(|m| match m {
                    sqlast::EnumMember::Name(n) | sqlast::EnumMember::NamedValue(n, _) => n.clone(),
                })
                .collect(),
        },
        DataType::Set(members) => Set { values: members.clone() },
        _ => Other { text: dt.to_string() },
    }
}

fn parse_pg_type(dt: &DataType) -> PostgresType {
    use PostgresType::*;
    match dt {
        DataType::SmallInt(_) => SmallInt,
        DataType::Int(_) | DataType::Integer(_) => Integer,
        DataType::BigInt(_) => BigInt,
        DataType::Decimal(info) | DataType::Numeric(info) => {
            let (p, s) = exact_num(info);
            Numeric { precision: p, scale: s }
        }
        DataType::Real => Real,
        DataType::DoublePrecision => DoublePrecision,
        DataType::Char(opt) | DataType::Character(opt) => Char { length: char_len(opt) },
        DataType::Varchar(opt)
        | DataType::CharVarying(opt)
        | DataType::CharacterVarying(opt) => Varchar { length: char_len(opt) },
        DataType::Text => Text,
        DataType::Bytea => Bytea,
        DataType::Date => Date,
        DataType::Time(w, tz) => Time {
            precision: w.map(|x| x as u32),
            with_tz: matches!(tz, sqlast::TimezoneInfo::WithTimeZone),
        },
        DataType::Timestamp(w, tz) => Timestamp {
            precision: w.map(|x| x as u32),
            with_tz: matches!(tz, sqlast::TimezoneInfo::WithTimeZone),
        },
        DataType::Interval => Interval { fields: None },
        DataType::Bool | DataType::Boolean => Boolean,
        DataType::Uuid => Uuid,
        DataType::JSON => Json,
        DataType::JSONB => Jsonb,
        DataType::Bit(w) => Bit { length: w.map(|x| x as u32), varying: false },
        DataType::BitVarying(w) => Bit { length: w.map(|x| x as u32), varying: true },
        DataType::Array(sqlast::ArrayElemTypeDef::AngleBracket(inner))
        | DataType::Array(sqlast::ArrayElemTypeDef::SquareBracket(inner, _))
        | DataType::Array(sqlast::ArrayElemTypeDef::Parenthesis(inner)) => {
            PostgresType::Array { inner: Box::new(parse_pg_type(inner)) }
        }
        // `SERIAL` / `BIGSERIAL` aren't first-class `DataType` variants in
        // sqlparser — they parse as `Custom`. Map them back to the dedicated
        // arms so they round-trip instead of falling through to `Other`.
        DataType::Custom(name, _) => {
            match object_name_last(name).to_uppercase().as_str() {
                "SERIAL" => Serial,
                "BIGSERIAL" => BigSerial,
                _ => Other { text: dt.to_string() },
            }
        }
        _ => Other { text: dt.to_string() },
    }
}

/// Map a [`ColumnDef`]'s options onto (nullable, default, auto_increment) and
/// fold any inline `PRIMARY KEY` / `UNIQUE` into table constraints.
fn column_from_def(
    def: &ColumnDef,
    dialect: DialectName,
    out_constraints: &mut Vec<Constraint>,
) -> Column {
    let mut nullable = true;
    let mut default = None;
    let mut auto_increment = false;

    for opt in &def.options {
        match &opt.option {
            ColumnOption::Null => nullable = true,
            ColumnOption::NotNull => nullable = false,
            ColumnOption::Default(expr) => default = Some(expr.to_string()),
            // MySQL's `AUTO_INCREMENT` parses as a dialect-specific option; the
            // string form contains "AUTO_INCREMENT" regardless of casing.
            ColumnOption::DialectSpecific(_) => {
                if opt.to_string().to_uppercase().contains("AUTO_INCREMENT") {
                    auto_increment = true;
                }
            }
            ColumnOption::Unique { is_primary, .. } => {
                if *is_primary {
                    out_constraints.push(Constraint::PrimaryKey {
                        id: ConstraintId::new(),
                        name: opt.name.as_ref().map(|n| n.value.clone()),
                        columns: vec![def.name.value.clone()],
                    });
                } else {
                    out_constraints.push(Constraint::Unique {
                        id: ConstraintId::new(),
                        name: opt.name.as_ref().map(|n| n.value.clone()),
                        columns: vec![def.name.value.clone()],
                    });
                }
            }
            _ => {}
        }
    }

    Column {
        id: ColumnId::new(),
        name: def.name.value.clone(),
        ty: parse_type(&def.data_type, dialect),
        nullable,
        default,
        auto_increment,
        comment: None,
    }
}

/// Parse the first `CREATE TABLE` statement in `sql`.
pub fn parse_table_sql(sql: &str, dialect: DialectName) -> Result<ParsedTable, String> {
    let d = dialect_for(dialect);
    let stmts = Parser::parse_sql(d.as_ref(), sql).map_err(|e| format!("{e}"))?;
    let create = stmts
        .into_iter()
        .find_map(|s| match s {
            Statement::CreateTable(c) => Some(c),
            _ => None,
        })
        .ok_or_else(|| "No CREATE TABLE statement found".to_string())?;

    let mut constraints = Vec::new();

    // Column-level PRIMARY KEY / UNIQUE fold up into `constraints`.
    let columns: Vec<Column> = create
        .columns
        .iter()
        .map(|c| column_from_def(c, dialect, &mut constraints))
        .collect();

    // Table-level constraints.
    for tc in &create.constraints {
        match tc {
            TableConstraint::PrimaryKey { name, columns: cols, .. } => {
                constraints.push(Constraint::PrimaryKey {
                    id: ConstraintId::new(),
                    name: name.as_ref().map(|n| n.value.clone()),
                    columns: cols.iter().map(|c| c.value.clone()).collect(),
                });
            }
            TableConstraint::Unique { name, columns: cols, .. } => {
                constraints.push(Constraint::Unique {
                    id: ConstraintId::new(),
                    name: name.as_ref().map(|n| n.value.clone()),
                    columns: cols.iter().map(|c| c.value.clone()).collect(),
                });
            }
            TableConstraint::ForeignKey {
                name,
                columns,
                foreign_table,
                referred_columns,
                on_delete,
                on_update,
                ..
            } => {
                constraints.push(Constraint::ForeignKey {
                    id: ConstraintId::new(),
                    name: name.as_ref().map(|n| n.value.clone()),
                    columns: columns.iter().map(|c| c.value.clone()).collect(),
                    referenced_table: object_name_last(foreign_table),
                    referenced_columns: referred_columns.iter().map(|c| c.value.clone()).collect(),
                    on_delete: on_delete.as_ref().map(map_referential_action),
                    on_update: on_update.as_ref().map(map_referential_action),
                });
            }
            _ => {}
        }
    }

    Ok(ParsedTable {
        name: object_name_last(&create.name),
        columns,
        constraints,
    })
}

/// Parse the first `ALTER TABLE … ADD CONSTRAINT … FOREIGN KEY …` in `sql`.
pub fn parse_fk_sql(sql: &str, dialect: DialectName) -> Result<ParsedFk, String> {
    let d = dialect_for(dialect);
    let stmts = Parser::parse_sql(d.as_ref(), sql).map_err(|e| format!("{e}"))?;
    for s in stmts {
        if let Statement::AlterTable { name: table_obj, operations, .. } = s {
            for op in operations {
                if let sqlast::AlterTableOperation::AddConstraint(TableConstraint::ForeignKey {
                    name,
                    columns,
                    foreign_table,
                    referred_columns,
                    on_delete,
                    on_update,
                    ..
                }) = op
                {
                    return Ok(ParsedFk {
                        table: object_name_last(&table_obj),
                        name: name.as_ref().map(|n| n.value.clone()),
                        columns: columns.iter().map(|c| c.value.clone()).collect(),
                        referenced_table: object_name_last(&foreign_table),
                        referenced_columns: referred_columns.iter().map(|c| c.value.clone()).collect(),
                        on_delete: on_delete.as_ref().map(map_referential_action),
                        on_update: on_update.as_ref().map(map_referential_action),
                    });
                }
            }
        }
    }
    Err("No ALTER TABLE … ADD CONSTRAINT … FOREIGN KEY statement found".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{DialectName, MySqlType, PostgresType};
    use crate::model::{Column, Constraint, ConstraintId, Table, TableId};

    fn tbl(name: &str, cols: Vec<Column>, cons: Vec<Constraint>) -> Table {
        Table {
            id: TableId::new(),
            name: name.into(),
            columns: cols,
            constraints: cons,
            indexes: vec![],
            comment: None,
        }
    }

    #[test]
    fn mysql_table_round_trips() {
        let table = tbl(
            "users",
            vec![
                Column {
                    id: ColumnId::new(),
                    name: "id".into(),
                    ty: DialectType::MySql(MySqlType::Int { unsigned: false, display_width: Some(11) }),
                    nullable: false,
                    default: None,
                    auto_increment: true,
                    comment: None,
                },
                Column {
                    id: ColumnId::new(),
                    name: "name".into(),
                    ty: DialectType::MySql(MySqlType::Varchar { length: Some(255) }),
                    nullable: true,
                    default: None,
                    auto_increment: false,
                    comment: None,
                },
            ],
            vec![Constraint::PrimaryKey {
                id: ConstraintId::new(),
                name: None,
                columns: vec!["id".into()],
            }],
        );
        let sql = emit_table_sql(&table, DialectName::MySql);
        let parsed = parse_table_sql(&sql, DialectName::MySql).expect("parse");
        assert_eq!(parsed.name, "users");
        assert_eq!(parsed.columns.len(), 2);
        assert_eq!(parsed.columns[0].name, "id");
        assert!(!parsed.columns[0].nullable);
        assert!(parsed.columns[0].auto_increment);
        assert_eq!(parsed.columns[1].name, "name");
        assert!(parsed.columns[1].nullable);
        // PRIMARY KEY round-trips.
        assert!(parsed.constraints.iter().any(|c| matches!(
            c,
            Constraint::PrimaryKey { columns, .. } if columns == &["id".to_string()]
        )));
    }

    #[test]
    fn mysql_fk_edge_round_trips() {
        let posts = tbl(
            "posts",
            vec![Column {
                id: ColumnId::new(),
                name: "user_id".into(),
                ty: DialectType::MySql(MySqlType::Int { unsigned: false, display_width: None }),
                nullable: false,
                default: None,
                auto_increment: false,
                comment: None,
            }],
            vec![Constraint::ForeignKey {
                id: ConstraintId::new(),
                name: Some("posts_user_id_fkey".into()),
                columns: vec!["user_id".into()],
                referenced_table: "users".into(),
                referenced_columns: vec!["id".into()],
                on_delete: Some(ReferentialAction::Cascade),
                on_update: None,
            }],
        );
        let fk = posts.constraints.first().unwrap().clone();
        let sql = emit_fk_sql(&posts, &fk, DialectName::MySql);
        let parsed = parse_fk_sql(&sql, DialectName::MySql).expect("parse fk");
        assert_eq!(parsed.table, "posts");
        assert_eq!(parsed.name.as_deref(), Some("posts_user_id_fkey"));
        assert_eq!(parsed.columns, vec!["user_id".to_string()]);
        assert_eq!(parsed.referenced_table, "users");
        assert_eq!(parsed.referenced_columns, vec!["id".to_string()]);
        assert_eq!(parsed.on_delete, Some(ReferentialAction::Cascade));
    }

    #[test]
    fn postgres_table_round_trips() {
        let table = tbl(
            "accounts",
            vec![Column {
                id: ColumnId::new(),
                name: "id".into(),
                ty: DialectType::Postgres(PostgresType::Serial),
                nullable: false,
                default: None,
                auto_increment: false,
                comment: None,
            }],
            vec![],
        );
        let sql = emit_table_sql(&table, DialectName::Postgres);
        let parsed = parse_table_sql(&sql, DialectName::Postgres).expect("parse");
        assert_eq!(parsed.columns[0].ty, DialectType::Postgres(PostgresType::Serial));
    }
}
