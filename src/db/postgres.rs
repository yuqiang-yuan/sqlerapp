//! PostgreSQL data types.
//!
//! Stored verbatim; version differences are not modeled.

/// A PostgreSQL column data type.
#[derive(Clone, Debug, PartialEq)]
pub enum PostgresType {
    // —— Integer ——
    /// `SMALLINT` / `INT2`.
    SmallInt,
    /// `INTEGER` / `INT` / `INT4`.
    Integer,
    /// `BIGINT` / `INT8`.
    BigInt,
    /// `SERIAL` (auto-increment `INTEGER`). Stored verbatim even though
    /// PG 10+ recommends `GENERATED ... AS IDENTITY` — no version rewriting.
    Serial,
    /// `BIGSERIAL` (auto-increment `BIGINT`).
    BigSerial,

    // —— Decimal / fixed-point ——
    /// `DECIMAL(p,s)` / `NUMERIC(p,s)`.
    Numeric { precision: Option<u32>, scale: Option<u32> },
    /// `MONEY`.
    Money,

    // —— Approximate ——
    /// `REAL` / `FLOAT4`.
    Real,
    /// `DOUBLE PRECISION` / `FLOAT8`.
    DoublePrecision,

    // —— Character ——
    /// `CHAR(n)` / `CHARACTER(n)`.
    Char { length: Option<u32> },
    /// `VARCHAR(n)` / `CHARACTER VARYING(n)`. `None` = unbounded.
    Varchar { length: Option<u32> },
    /// `TEXT`.
    Text,

    // —— Binary ——
    /// `BYTEA`.
    Bytea,

    // —— Date & Time ——
    /// `DATE`.
    Date,
    /// `TIME(p)` / `TIME WITHOUT TIME ZONE`.
    Time { precision: Option<u32>, with_tz: bool },
    /// `TIMESTAMP(p)` / `TIMESTAMP WITHOUT TIME ZONE`.
    Timestamp { precision: Option<u32>, with_tz: bool },
    /// `INTERVAL [fields [(p)]]`. `fields` stored verbatim when present
    /// (e.g. "DAY TO SECOND").
    Interval { fields: Option<String> },

    // —— Boolean ——
    /// `BOOLEAN` / `BOOL`.
    Boolean,

    // —— UUID / JSON ——
    /// `UUID`.
    Uuid,
    /// `JSON`.
    Json,
    /// `JSONB`.
    Jsonb,

    // —— Bit strings ——
    /// `BIT(n)` / `BIT VARYING(n)` (when `varying` true).
    Bit { length: Option<u32>, varying: bool },

    // —— Network address (PG-specific) ——
    /// `CIDR`.
    Cidr,
    /// `INET`.
    Inet,
    /// `MACADDR`.
    MacAddr,
    /// `MACADDR8`.
    MacAddr8,

    // —— Geometric (PG-specific) ——
    /// `POINT`, `LINE`, `LSEG`, `BOX`, `PATH`, `POLYGON`, `CIRCLE`.
    Geometric { kind: GeometricKind },

    // —— Enumerated / composite —
    /// `ENUM` type. PG requires these be declared via `CREATE TYPE ... AS
    /// ENUM`; the column references the type by name. Whether `values` is
    /// stored here or resolved via the schema's type registry is TBD.
    Enum { values: Vec<String> },

    // —— Arrays ——
    /// `T[]` — array of an element type. Nested arrays (`T[][]`) are
    /// `Array { inner: Array { ... } }`. The element is boxed because the
    /// enum is recursive.
    Array { inner: Box<PostgresType> },

    /// A syntactically valid type the tool doesn't recognize; stored verbatim
    /// and round-tripped losslessly (e.g. `citext`, a composite/domain type,
    /// or `tsrange`).
    Other { text: String },
}

/// Sub-kind of PG geometric type.
#[derive(Clone, Debug, PartialEq)]
pub enum GeometricKind {
    Point,
    Line,
    Lseg,
    Box,
    Path,
    Polygon,
    Circle,
}
