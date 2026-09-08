//! MySQL data types.
//!
//! Stored verbatim from what the user writes, including deprecated syntax
//! like `INT(11)` display width (deprecated since 8.0.17 but still accepted
//! by the parser). Version differences are **not** modeled — no warnings,
//! no stripping.

/// A MySQL column data type.
#[derive(Clone, Debug, PartialEq)]
pub enum MySqlType {
    // —— Integer ——
    /// `TINYINT`.
    TinyInt { unsigned: bool, display_width: Option<u32> },
    /// `SMALLINT`.
    SmallInt { unsigned: bool, display_width: Option<u32> },
    /// `INT` / `INTEGER`. `display_width` (the `(11)` in `INT(11)`) is
    /// deprecated since 8.0.17 but stored verbatim.
    Int { unsigned: bool, display_width: Option<u32> },
    /// `MEDIUMINT` (MySQL-only).
    MediumInt { unsigned: bool, display_width: Option<u32> },
    /// `BIGINT`.
    BigInt { unsigned: bool, display_width: Option<u32> },

    // —— Decimal / fixed-point ——
    /// `DECIMAL(p,s)` / `NUMERIC(p,s)` (synonyms).
    Decimal { precision: Option<u32>, scale: Option<u32> },

    // —— Approximate ——
    /// `FLOAT`. `(M,D)` precision is deprecated since 8.0.17; stored verbatim.
    Float { precision: Option<u32>, scale: Option<u32> },
    /// `DOUBLE` / `DOUBLE PRECISION` / `REAL`. `(M,D)` deprecated; stored verbatim.
    Double { precision: Option<u32>, scale: Option<u32> },

    // —— Boolean ——
    /// `BOOL` / `BOOLEAN` (aliases for `TINYINT(1)`).
    Boolean,

    // —— Character ——
    /// `CHAR(n)`.
    Char { length: Option<u32> },
    /// `VARCHAR(n)`.
    Varchar { length: Option<u32> },
    /// `TINYTEXT` / `TEXT` / `MEDIUMTEXT` / `LONGTEXT` distinguished by size.
    Text { size: TextSize },
    /// `ENUM('a','b')`.
    Enum { values: Vec<String> },
    /// `SET('a','b','c')`.
    Set { values: Vec<String> },

    // —— Binary ——
    /// `BINARY(n)`.
    Binary { length: Option<u32> },
    /// `VARBINARY(n)`.
    VarBinary { length: Option<u32> },
    /// `TINYBLOB` / `BLOB` / `MEDIUMBLOB` / `LONGBLOB` distinguished by size.
    Blob { size: BlobSize },

    // —— Date & Time ——
    /// `DATE`.
    Date,
    /// `TIME(p)`.
    Time { precision: Option<u32> },
    /// `DATETIME(p)` — literal storage, no timezone conversion.
    DateTime { precision: Option<u32> },
    /// `TIMESTAMP(p)` — auto-converted to UTC on storage and retrieval.
    Timestamp { precision: Option<u32> },
    /// `YEAR` — 1-byte year value.
    Year,
    /// `BIT(n)` — bit-field.
    Bit { length: Option<u32> },

    /// A syntactically valid type the tool doesn't recognize; stored verbatim
    /// and round-tripped losslessly (e.g. a future/extension type).
    Other { text: String },
}

/// Size modifier for the `TEXT` family.
#[derive(Clone, Debug, PartialEq)]
pub enum TextSize {
    /// `TINYTEXT`.
    Tiny,
    /// `TEXT`.
    Normal,
    /// `MEDIUMTEXT`.
    Medium,
    /// `LONGTEXT`.
    Long,
}

/// Size modifier for the `BLOB` family (mirrors [`TextSize`]).
#[derive(Clone, Debug, PartialEq)]
pub enum BlobSize {
    /// `TINYBLOB`.
    Tiny,
    /// `BLOB`.
    Normal,
    /// `MEDIUMBLOB`.
    Medium,
    /// `LONGBLOB`.
    Long,
}
