//! File ownership database.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Normalize a path for comparison: forward slashes, no `.`/empty parts, lower-case (Windows
/// semantics), `..` rejected.
pub fn norm(p: &str) -> Option<String> {
    let mut out: Vec<String> = Vec::new();
    for c in p.split(['/', '\\']) {
        match c {
            "" | "." => {}
            ".." => return None,
            _ => out.push(c.to_ascii_lowercase()),
        }
    }
    (!out.is_empty()).then(|| out.join("/"))
}

/// Installed package record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// Version.
    pub version: String,
    /// Normalized files owned.
    pub files: BTreeSet<String>,
    /// Files this package may share with others (e.g. runtime DLLs).
    pub shared: BTreeSet<String>,
}

/// Database errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Path escapes the root or is empty.
    BadPath(String),
    /// A non-shared file already belongs to another package.
    Conflict {
        /// Path.
        path: String,
        /// Existing owner.
        owner: String,
    },
    /// No such package.
    Unknown(String),
    /// Corrupt serialized form (line number).
    Parse(usize),
}

/// What removing a package does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemovePlan {
    /// Delete these (no other owner), deepest first so directories empty out.
    pub delete: Vec<String>,
    /// Keep these (still owned by someone else).
    pub keep: Vec<(String, Vec<String>)>,
}

/// The database.
#[derive(Default, Clone, Debug)]
pub struct Db {
    pkgs: BTreeMap<String, Record>,
}

impl Db {
    /// Owners of `path`.
    pub fn owners(&self, path: &str) -> Vec<&str> {
        let Some(n) = norm(path) else { return Vec::new() };
        self.pkgs.iter().filter(|(_, r)| r.files.contains(&n)).map(|(k, _)| k.as_str()).collect()
    }

    /// Installed version.
    pub fn version(&self, pkg: &str) -> Option<&str> {
        self.pkgs.get(pkg).map(|r| r.version.as_str())
    }

    /// Installed package names.
    pub fn packages(&self) -> impl Iterator<Item = &str> {
        self.pkgs.keys().map(|s| s.as_str())
    }

    /// Check an install for conflicts without changing anything.
    pub fn check_install(&self, pkg: &str, files: &[(&str, bool)]) -> Result<(), Error> {
        for (f, shared) in files {
            let n = norm(f).ok_or_else(|| Error::BadPath(f.to_string()))?;
            for (o, r) in &self.pkgs {
                if o != pkg && r.files.contains(&n) && !(*shared && r.shared.contains(&n)) {
                    return Err(Error::Conflict { path: n, owner: o.clone() });
                }
            }
        }
        Ok(())
    }

    /// Record an install/upgrade. `files` are `(path, shared)`. Replaces any previous record of
    /// `pkg` (upgrade) and returns the files the old version owned that the new one doesn't
    /// (candidates for deletion, minus those other packages own).
    pub fn install(&mut self, pkg: &str, version: &str, files: &[(&str, bool)]) -> Result<Vec<String>, Error> {
        self.check_install(pkg, files)?;
        let mut rec = Record { version: version.to_string(), files: BTreeSet::new(), shared: BTreeSet::new() };
        for (f, s) in files {
            let n = norm(f).unwrap();
            if *s {
                rec.shared.insert(n.clone());
            }
            rec.files.insert(n);
        }
        let old = self.pkgs.insert(pkg.to_string(), rec.clone());
        Ok(old
            .map(|o| o.files.into_iter().filter(|f| !rec.files.contains(f) && !self.pkgs.iter().any(|(k, r)| k != pkg && r.files.contains(f))).collect())
            .unwrap_or_default())
    }

    /// Plan (without applying) the removal of `pkg`.
    pub fn plan_remove(&self, pkg: &str) -> Result<RemovePlan, Error> {
        let r = self.pkgs.get(pkg).ok_or_else(|| Error::Unknown(pkg.to_string()))?;
        let mut plan = RemovePlan::default();
        for f in &r.files {
            let others: Vec<String> = self.pkgs.iter().filter(|(k, o)| *k != pkg && o.files.contains(f)).map(|(k, _)| k.clone()).collect();
            if others.is_empty() {
                plan.delete.push(f.clone());
            } else {
                plan.keep.push((f.clone(), others));
            }
        }
        plan.delete.sort_by(|a, b| b.matches('/').count().cmp(&a.matches('/').count()).then(a.cmp(b)));
        Ok(plan)
    }

    /// Apply a removal.
    pub fn remove(&mut self, pkg: &str) -> Result<RemovePlan, Error> {
        let p = self.plan_remove(pkg)?;
        self.pkgs.remove(pkg);
        Ok(p)
    }

    /// Repair report: files each package owns that are not present on disk.
    pub fn missing(&self, exists: &dyn Fn(&str) -> bool) -> BTreeMap<String, Vec<String>> {
        let mut out = BTreeMap::new();
        for (k, r) in &self.pkgs {
            let m: Vec<String> = r.files.iter().filter(|f| !exists(f)).cloned().collect();
            if !m.is_empty() {
                out.insert(k.clone(), m);
            }
        }
        out
    }

    /// Serialize: `P name version` then `F path` / `S path` lines.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for (k, r) in &self.pkgs {
            s.push_str(&alloc::format!("P {} {}\n", k, r.version));
            for f in &r.files {
                s.push_str(if r.shared.contains(f) { "S " } else { "F " });
                s.push_str(f);
                s.push('\n');
            }
        }
        s
    }

    /// Parse [`Db::to_text`] output.
    pub fn from_text(t: &str) -> Result<Db, Error> {
        let mut db = Db::default();
        let mut cur: Option<String> = None;
        for (i, l) in t.lines().enumerate() {
            let bad = Error::Parse(i + 1);
            if let Some(r) = l.strip_prefix("P ") {
                let (n, v) = r.split_once(' ').ok_or(bad.clone())?;
                db.pkgs.insert(n.to_string(), Record { version: v.to_string(), files: BTreeSet::new(), shared: BTreeSet::new() });
                cur = Some(n.to_string());
            } else if let (Some(f), Some(c)) = (l.strip_prefix("F ").map(|f| (f, false)).or_else(|| l.strip_prefix("S ").map(|f| (f, true))), &cur) {
                let (path, shared) = f;
                if norm(path).as_deref() != Some(path) {
                    return Err(bad);
                }
                let r = db.pkgs.get_mut(c).unwrap();
                r.files.insert(path.to_string());
                if shared {
                    r.shared.insert(path.to_string());
                }
            } else if !l.is_empty() {
                return Err(bad);
            }
        }
        Ok(db)
    }
}
