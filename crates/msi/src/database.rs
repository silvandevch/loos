//! MSI table decoding.

use crate::cfb::{self, Cfb};
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Compound file layer.
    Cfb(cfb::Error),
    /// Missing or malformed system stream/table.
    Corrupt(&'static str),
    /// String index out of range.
    BadString,
}

impl From<cfb::Error> for Error {
    fn from(e: cfb::Error) -> Self {
        Error::Cfb(e)
    }
}

/// A cell value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    /// NULL.
    Null,
    /// Integer.
    Int(i32),
    /// String.
    Str(String),
}

impl Cell {
    /// String view ("" for null / ints rendered).
    pub fn s(&self) -> String {
        match self {
            Cell::Str(s) => s.clone(),
            Cell::Int(i) => i.to_string(),
            Cell::Null => String::new(),
        }
    }
    /// Integer view (0 for null/non-numeric).
    pub fn i(&self) -> i32 {
        match self {
            Cell::Int(i) => *i,
            Cell::Str(s) => s.parse().unwrap_or(0),
            Cell::Null => 0,
        }
    }
}

/// Column definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    /// Name.
    pub name: String,
    /// Raw type word.
    pub ty: u16,
}

impl Column {
    fn is_string(&self) -> bool {
        self.ty & 0x0800 != 0
    }
}

/// A table.
#[derive(Clone, Debug, Default)]
pub struct Table {
    /// Columns in order.
    pub cols: Vec<Column>,
    /// Rows.
    pub rows: Vec<Vec<Cell>>,
}

impl Table {
    /// Column index.
    pub fn col(&self, name: &str) -> Option<usize> {
        self.cols.iter().position(|c| c.name == name)
    }
    /// Value in row `r`, column `name`.
    pub fn get(&self, r: usize, name: &str) -> Option<&Cell> {
        self.rows.get(r)?.get(self.col(name)?)
    }
}

const NAMES: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz._";

/// Decode an MSI stream name. Returns `(is_table_stream, name)`.
pub fn decode_name(raw: &[u16]) -> (bool, String) {
    let mut out = String::new();
    let mut table = false;
    for (i, &c) in raw.iter().enumerate() {
        match c {
            0x4840 if i == 0 => table = true,
            0x3800..=0x47FF => {
                let v = (c - 0x3800) as usize;
                out.push(NAMES[v & 63] as char);
                out.push(NAMES[(v >> 6) & 63] as char);
            }
            0x4800..=0x483F => out.push(NAMES[(c - 0x4800) as usize] as char),
            _ => out.push(char::from_u32(c as u32).unwrap_or('\u{FFFD}')),
        }
    }
    (table, out)
}

/// Encode a table name to the stream form (used by tests/writers).
pub fn encode_name(name: &str, table: bool) -> Vec<u16> {
    let idx = |c: char| NAMES.iter().position(|&n| n as char == c);
    let cs: Vec<char> = name.chars().collect();
    let mut out = Vec::new();
    if table {
        out.push(0x4840);
    }
    let mut i = 0;
    while i < cs.len() {
        match (idx(cs[i]), cs.get(i + 1).and_then(|&c| idx(c))) {
            (Some(a), Some(b)) => {
                out.push(0x3800 + (a | (b << 6)) as u16);
                i += 2;
            }
            (Some(a), None) => {
                out.push(0x4800 + a as u16);
                i += 1;
            }
            _ => {
                out.push(cs[i] as u16);
                i += 1;
            }
        }
    }
    out
}

/// Whole database.
#[derive(Default, Debug)]
pub struct Database {
    /// Tables by name.
    pub tables: BTreeMap<String, Table>,
    /// Streams that are not tables (cabinets `#name`, icons, binaries) by decoded name.
    pub blobs: BTreeMap<String, Vec<u8>>,
}

struct Pool {
    strings: Vec<String>,
    wide: bool,
}

impl Pool {
    fn get(&self, i: usize) -> Result<String, Error> {
        if i == 0 {
            return Ok(String::new());
        }
        self.strings.get(i - 1).cloned().ok_or(Error::BadString)
    }
}

fn pool(data: &[u8], pool_raw: &[u8]) -> Result<Pool, Error> {
    if pool_raw.len() < 4 {
        return Err(Error::Corrupt("_StringPool"));
    }
    let hdr = u32::from_le_bytes(pool_raw[..4].try_into().unwrap());
    let wide = hdr & 0x8000_0000 != 0;
    let mut strings = Vec::new();
    let mut pos = 0usize;
    let mut p = 4;
    while p + 4 <= pool_raw.len() {
        let len = u16::from_le_bytes([pool_raw[p], pool_raw[p + 1]]) as usize;
        let refc = u16::from_le_bytes([pool_raw[p + 2], pool_raw[p + 3]]);
        p += 4;
        if len == 0 && refc != 0 {
            return Err(Error::Corrupt("long strings unsupported"));
        }
        let b = data.get(pos..pos + len).ok_or(Error::Corrupt("_StringData"))?;
        pos += len;
        strings.push(match core::str::from_utf8(b) {
            Ok(s) => s.to_string(),
            Err(_) => b.iter().map(|&c| c as char).collect(), // codepage fallback: Latin-1
        });
    }
    Ok(Pool { strings, wide })
}

fn read_table(raw: &[u8], cols: &[Column], pool: &Pool) -> Result<Vec<Vec<Cell>>, Error> {
    let sw = if pool.wide { 3 } else { 2 };
    let widths: Vec<usize> = cols.iter().map(|c| if c.is_string() { sw } else if c.ty & 0xFF == 4 { 4 } else { 2 }).collect();
    let rowlen: usize = widths.iter().sum();
    if rowlen == 0 || raw.len() % rowlen != 0 {
        return Err(Error::Corrupt("table size"));
    }
    let n = raw.len() / rowlen;
    let mut rows = alloc::vec![Vec::with_capacity(cols.len()); n];
    let mut off = 0;
    for (c, &w) in cols.iter().zip(&widths) {
        for r in 0..n {
            let b = &raw[off + r * w..off + r * w + w];
            let cell = if c.is_string() {
                let i = if w == 3 { b[0] as usize | (b[1] as usize) << 8 | (b[2] as usize) << 16 } else { u16::from_le_bytes([b[0], b[1]]) as usize };
                if i == 0 { Cell::Null } else { Cell::Str(pool.get(i)?) }
            } else if w == 4 {
                let v = u32::from_le_bytes(b.try_into().unwrap());
                if v == 0 { Cell::Null } else { Cell::Int((v ^ 0x8000_0000) as i32) }
            } else {
                let v = u16::from_le_bytes([b[0], b[1]]);
                if v == 0 { Cell::Null } else { Cell::Int(((v ^ 0x8000) as i16) as i32) }
            };
            rows[r].push(cell);
        }
        off += n * w;
    }
    Ok(rows)
}

impl Database {
    /// Parse an `.msi` image.
    pub fn open(image: &[u8]) -> Result<Database, Error> {
        let cfb = Cfb::open(image)?;
        let mut by_name: BTreeMap<String, (bool, usize)> = BTreeMap::new();
        for i in cfb.streams() {
            let (t, n) = decode_name(&cfb.entries[i].raw_name);
            by_name.insert(n, (t, i));
        }
        let get = |n: &str| -> Result<Vec<u8>, Error> {
            let (_, i) = by_name.get(n).ok_or(Error::Corrupt("missing system stream"))?;
            Ok(cfb.read_entry(*i)?)
        };
        let pool = pool(&get("_StringData")?, &get("_StringPool")?)?;
        // System tables have fixed layouts.
        let sw_ty = |s: bool| Column { name: String::new(), ty: if s { 0x0D02 } else { 0x1502 } };
        let tables_cols = [Column { name: "Name".into(), ..sw_ty(true) }];
        let names: Vec<String> = match by_name.get("_Tables") {
            Some(_) => read_table(&get("_Tables")?, &tables_cols, &pool)?.into_iter().map(|r| r[0].s()).collect(),
            None => Vec::new(),
        };
        let col_defs = [
            Column { name: "Table".into(), ..sw_ty(true) },
            Column { name: "Number".into(), ..sw_ty(false) },
            Column { name: "Name".into(), ..sw_ty(true) },
            Column { name: "Type".into(), ..sw_ty(false) },
        ];
        let colrows = read_table(&get("_Columns")?, &col_defs, &pool)?;
        let mut db = Database::default();
        for name in names {
            let mut defs: Vec<(i32, Column)> = colrows
                .iter()
                .filter(|r| r[0].s() == name)
                .map(|r| (r[1].i(), Column { name: r[2].s(), ty: r[3].i() as u16 }))
                .collect();
            defs.sort_by_key(|d| d.0);
            let cols: Vec<Column> = defs.into_iter().map(|d| d.1).collect();
            let rows = match by_name.get(&name) {
                Some((true, i)) => read_table(&cfb.read_entry(*i)?, &cols, &pool)?,
                _ => Vec::new(), // empty table has no stream
            };
            db.tables.insert(name, Table { cols, rows });
        }
        for (n, (t, i)) in by_name {
            if !t && !n.starts_with('_') && !n.starts_with('\u{5}') {
                db.blobs.insert(n, cfb.read_entry(i)?);
            }
        }
        Ok(db)
    }

    /// Table by name.
    pub fn table(&self, n: &str) -> Option<&Table> {
        self.tables.get(n)
    }

    /// `Property` table value.
    pub fn property(&self, name: &str) -> Option<String> {
        let t = self.table("Property")?;
        (0..t.rows.len()).find(|&r| t.get(r, "Property").map(|c| c.s()) == Some(name.to_string())).and_then(|r| t.get(r, "Value")).map(|c| c.s())
    }
}
