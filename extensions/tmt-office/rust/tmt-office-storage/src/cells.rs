//! Exact SQLite cells: storage class and bytes, never decoded or re-encoded.

use rusqlite::{
    Connection, ToSql,
    types::{ToSqlOutput, ValueRef},
};
use sha2::{Digest, Sha256};

/// One stored value. TEXT keeps its raw bytes, so non-UTF-8 text survives, and
/// REAL keeps its bit pattern, so equality never rounds or merges signed zeros.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Cell {
    Null,
    Integer(i64),
    Real(u64),
    Text(Vec<u8>),
    Blob(Vec<u8>),
}

impl Cell {
    fn read(value: ValueRef<'_>) -> Self {
        match value {
            ValueRef::Null => Self::Null,
            ValueRef::Integer(value) => Self::Integer(value),
            ValueRef::Real(value) => Self::Real(value.to_bits()),
            ValueRef::Text(bytes) => Self::Text(bytes.to_vec()),
            ValueRef::Blob(bytes) => Self::Blob(bytes.to_vec()),
        }
    }

    fn digest(&self, hasher: &mut Sha256) {
        match self {
            Self::Null => hasher.update([0]),
            Self::Integer(value) => {
                hasher.update([1]);
                hasher.update(value.to_be_bytes());
            }
            Self::Real(bits) => {
                hasher.update([2]);
                hasher.update(bits.to_be_bytes());
            }
            Self::Text(bytes) => {
                hasher.update([3]);
                digest_bytes(hasher, bytes);
            }
            Self::Blob(bytes) => {
                hasher.update([4]);
                digest_bytes(hasher, bytes);
            }
        }
    }
}

impl ToSql for Cell {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(match self {
            Self::Null => ValueRef::Null,
            Self::Integer(value) => ValueRef::Integer(*value),
            Self::Real(bits) => ValueRef::Real(f64::from_bits(*bits)),
            Self::Text(bytes) => ValueRef::Text(bytes),
            Self::Blob(bytes) => ValueRef::Blob(bytes),
        }))
    }
}

fn digest_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

/// Column layout and a deterministic row order for one table. Rowid tables
/// carry their rowid explicitly so gaps and ordering survive the copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TableShape {
    pub name: &'static str,
    pub rowid: bool,
    pub columns: Vec<String>,
    key: Vec<String>,
}

impl TableShape {
    pub fn read(connection: &Connection, name: &'static str) -> rusqlite::Result<Self> {
        let sql: String = connection.query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?",
            [name],
            |row| row.get(0),
        )?;
        let rowid = !sql.to_ascii_uppercase().contains("WITHOUT ROWID");
        let mut statement = connection.prepare(&format!("PRAGMA table_info({})", quote(name)))?;
        let mut columns = Vec::new();
        let mut primary = Vec::new();
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let column: String = row.get(1)?;
            let position: i64 = row.get(5)?;
            if position > 0 {
                primary.push((position, column.clone()));
            }
            columns.push(column);
        }
        primary.sort();
        let key = if rowid {
            vec!["rowid".to_owned()]
        } else {
            primary
                .into_iter()
                .map(|(_, column)| quote(&column))
                .collect()
        };
        Ok(Self {
            name,
            rowid,
            columns,
            key,
        })
    }

    fn selected(&self) -> String {
        let mut names: Vec<String> = self.columns.iter().map(|column| quote(column)).collect();
        if self.rowid {
            names.insert(0, "rowid".to_owned());
        }
        names.join(", ")
    }

    fn select_sql(&self) -> String {
        format!(
            "SELECT {} FROM {} ORDER BY {}",
            self.selected(),
            quote(self.name),
            self.key.join(", ")
        )
    }

    fn insert_sql(&self) -> String {
        let width = self.columns.len() + usize::from(self.rowid);
        format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote(self.name),
            self.selected(),
            vec!["?"; width].join(", ")
        )
    }

    fn width(&self) -> usize {
        self.columns.len() + usize::from(self.rowid)
    }

    fn digest_header(&self, hasher: &mut Sha256) {
        digest_bytes(hasher, self.name.as_bytes());
        hasher.update([u8::from(self.rowid)]);
        hasher.update((self.columns.len() as u64).to_be_bytes());
        for column in &self.columns {
            digest_bytes(hasher, column.as_bytes());
        }
    }
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn read_row(row: &rusqlite::Row<'_>, width: usize) -> rusqlite::Result<Vec<Cell>> {
    (0..width)
        .map(|index| row.get_ref(index).map(Cell::read))
        .collect()
}

/// Row totals and the digest of every table in order: supporting evidence
/// for the cell-by-cell comparison, never a substitute for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Manifest {
    pub digest: String,
    pub rows: Vec<(&'static str, u64)>,
}

#[derive(Default)]
pub(crate) struct ManifestBuilder {
    hasher: Sha256,
    rows: Vec<(&'static str, u64)>,
}

impl ManifestBuilder {
    pub fn finish(self) -> Manifest {
        Manifest {
            digest: hex(&self.hasher.finalize()),
            rows: self.rows,
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Digests one table in key order.
pub(crate) fn digest_table(
    connection: &Connection,
    shape: &TableShape,
    manifest: &mut ManifestBuilder,
) -> rusqlite::Result<()> {
    shape.digest_header(&mut manifest.hasher);
    let mut statement = connection.prepare(&shape.select_sql())?;
    let mut rows = statement.query([])?;
    let mut count = 0_u64;
    while let Some(row) = rows.next()? {
        for cell in read_row(row, shape.width())? {
            cell.digest(&mut manifest.hasher);
        }
        count += 1;
    }
    manifest.hasher.update(count.to_be_bytes());
    manifest.rows.push((shape.name, count));
    Ok(())
}

/// Copies one table's raw cells and digests exactly what was inserted.
pub(crate) fn copy_table(
    source: &Connection,
    destination: &Connection,
    shape: &TableShape,
    manifest: &mut ManifestBuilder,
) -> rusqlite::Result<()> {
    shape.digest_header(&mut manifest.hasher);
    let mut select = source.prepare(&shape.select_sql())?;
    let mut insert = destination.prepare(&shape.insert_sql())?;
    let mut rows = select.query([])?;
    let mut count = 0_u64;
    while let Some(row) = rows.next()? {
        let cells = read_row(row, shape.width())?;
        for cell in &cells {
            cell.digest(&mut manifest.hasher);
        }
        insert.execute(rusqlite::params_from_iter(cells.iter()))?;
        count += 1;
    }
    manifest.hasher.update(count.to_be_bytes());
    manifest.rows.push((shape.name, count));
    Ok(())
}

/// The first difference between two databases' copies of one table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Difference {
    pub table: &'static str,
    pub row: u64,
    pub detail: &'static str,
}

/// Walks both tables in key order and compares every typed cell.
pub(crate) fn compare_table(
    source: &Connection,
    copy: &Connection,
    shape: &TableShape,
) -> rusqlite::Result<Option<Difference>> {
    let sql = shape.select_sql();
    let mut left_statement = source.prepare(&sql)?;
    let mut right_statement = copy.prepare(&sql)?;
    let mut left = left_statement.query([])?;
    let mut right = right_statement.query([])?;
    let mut index = 0_u64;
    loop {
        let difference = match (left.next()?, right.next()?) {
            (None, None) => return Ok(None),
            (Some(_), None) => Some("missing from the copy"),
            (None, Some(_)) => Some("unexpected in the copy"),
            (Some(a), Some(b)) => (read_row(a, shape.width())? != read_row(b, shape.width())?)
                .then_some("cell differs"),
        };
        if let Some(detail) = difference {
            return Ok(Some(Difference {
                table: shape.name,
                row: index,
                detail,
            }));
        }
        index += 1;
    }
}
