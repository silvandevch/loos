//! Install transaction journal: record each filesystem mutation *before* it happens so a crash
//! or failure can be rolled back by replaying the inverse in reverse order.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A journaled operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// Directory created (undo: remove if empty).
    CreateDir(String),
    /// New file created (undo: delete).
    CreateFile(String),
    /// Existing file replaced; original saved at `backup` (undo: restore).
    Replace {
        /// Target.
        path: String,
        /// Backup location.
        backup: String,
    },
    /// Registry/state key written by the package (undo: delete key).
    SetKey(String),
}

/// Undo action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Undo {
    /// Remove directory if empty.
    RemoveDirIfEmpty(String),
    /// Delete file.
    DeleteFile(String),
    /// Move `backup` over `path`.
    Restore {
        /// Target.
        path: String,
        /// Backup.
        backup: String,
    },
    /// Delete key.
    DeleteKey(String),
}

/// An open transaction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Journal {
    ops: Vec<Op>,
    committed: bool,
}

impl Journal {
    /// New empty journal.
    pub fn new() -> Self {
        Self::default()
    }
    /// Record an op (call before performing it).
    pub fn log(&mut self, op: Op) {
        debug_assert!(!self.committed);
        self.ops.push(op);
    }
    /// Mark success; the rollback plan becomes empty and backups may be deleted.
    pub fn commit(&mut self) -> Vec<String> {
        self.committed = true;
        self.ops.iter().filter_map(|o| if let Op::Replace { backup, .. } = o { Some(backup.clone()) } else { None }).collect()
    }
    /// Inverse operations, newest first. Empty after commit.
    pub fn rollback_plan(&self) -> Vec<Undo> {
        if self.committed {
            return Vec::new();
        }
        self.ops
            .iter()
            .rev()
            .map(|o| match o {
                Op::CreateDir(p) => Undo::RemoveDirIfEmpty(p.clone()),
                Op::CreateFile(p) => Undo::DeleteFile(p.clone()),
                Op::Replace { path, backup } => Undo::Restore { path: path.clone(), backup: backup.clone() },
                Op::SetKey(k) => Undo::DeleteKey(k.clone()),
            })
            .collect()
    }

    /// Serialize for crash recovery (one op per line; fields are tab separated, tabs/newlines in
    /// names are rejected by [`Journal::to_text`]'s caller via [`Journal::valid`]).
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for o in &self.ops {
            match o {
                Op::CreateDir(p) => s.push_str(&alloc::format!("D\t{p}\n")),
                Op::CreateFile(p) => s.push_str(&alloc::format!("F\t{p}\n")),
                Op::Replace { path, backup } => s.push_str(&alloc::format!("R\t{path}\t{backup}\n")),
                Op::SetKey(k) => s.push_str(&alloc::format!("K\t{k}\n")),
            }
        }
        if self.committed {
            s.push_str("C\n");
        }
        s
    }

    /// True if no recorded name contains a tab or newline.
    pub fn valid(&self) -> bool {
        let ok = |s: &str| !s.contains(['\t', '\n']);
        self.ops.iter().all(|o| match o {
            Op::CreateDir(p) | Op::CreateFile(p) | Op::SetKey(p) => ok(p),
            Op::Replace { path, backup } => ok(path) && ok(backup),
        })
    }

    /// Parse; a truncated final line (crash mid-write) is dropped.
    pub fn from_text(t: &str) -> Option<Journal> {
        let mut j = Journal::new();
        let complete = t.ends_with('\n') || t.is_empty();
        let mut lines: Vec<&str> = t.split('\n').collect();
        lines.pop(); // text after the last newline (empty if complete)
        let _ = complete;
        for l in lines {
            let f: Vec<&str> = l.split('\t').collect();
            match f.as_slice() {
                ["D", p] => j.ops.push(Op::CreateDir(p.to_string())),
                ["F", p] => j.ops.push(Op::CreateFile(p.to_string())),
                ["R", p, b] => j.ops.push(Op::Replace { path: p.to_string(), backup: b.to_string() }),
                ["K", k] => j.ops.push(Op::SetKey(k.to_string())),
                ["C"] => j.committed = true,
                _ => return None,
            }
        }
        Some(j)
    }

    /// Needs rollback on next boot? (has ops, never committed.)
    pub fn needs_recovery(&self) -> bool {
        !self.committed && !self.ops.is_empty()
    }
}
