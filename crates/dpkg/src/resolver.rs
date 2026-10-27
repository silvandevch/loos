//! Install planning: dependency closure, alternatives, Provides, Conflicts, Breaks.

use crate::control::Stanza;
use crate::deps::{self, Alt, Atom};
use crate::version::Version;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A candidate package from an index or the status db.
#[derive(Clone, Debug)]
pub struct Pkg {
    /// Name.
    pub name: String,
    /// Version.
    pub version: Version,
    /// `Depends` + `Pre-Depends`.
    pub depends: Vec<Alt>,
    /// `Conflicts` + `Breaks`.
    pub conflicts: Vec<Atom>,
    /// Virtual names provided (with optional exact version).
    pub provides: Vec<(String, Option<Version>)>,
    /// Installed size, KiB.
    pub size_kib: u64,
}

impl Pkg {
    /// Build from a stanza; `None` if Package/Version are missing or unparsable.
    pub fn from_stanza(s: &Stanza) -> Option<Pkg> {
        let mut depends = Vec::new();
        for f in ["Pre-Depends", "Depends"] {
            if let Some(v) = s.get(f) {
                depends.extend(deps::parse(v)?);
            }
        }
        let mut conflicts = Vec::new();
        for f in ["Conflicts", "Breaks"] {
            if let Some(v) = s.get(f) {
                for alt in deps::parse(v)? {
                    conflicts.extend(alt);
                }
            }
        }
        let mut provides = Vec::new();
        if let Some(v) = s.get("Provides") {
            for alt in deps::parse(v)? {
                for a in alt {
                    provides.push((a.name, a.ver.map(|(_, v)| v)));
                }
            }
        }
        Some(Pkg {
            name: s.get("Package")?.to_string(),
            version: Version::parse(s.get("Version")?)?,
            depends,
            conflicts,
            provides,
            size_kib: s.get("Installed-Size").and_then(|x| x.trim().parse().ok()).unwrap_or(0),
        })
    }

    fn satisfies(&self, a: &Atom) -> bool {
        if self.name == a.name && a.accepts(&self.version) {
            return true;
        }
        // Provided names satisfy only unversioned atoms, or exact-version provides that match.
        self.provides.iter().any(|(n, v)| {
            *n == a.name
                && match (&a.ver, v) {
                    (None, _) => true,
                    (Some(_), Some(v)) => a.accepts(v),
                    (Some(_), None) => false,
                }
        })
    }
}

/// Planning failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Requested package is not in the index.
    NotFound(String),
    /// No candidate satisfies this dependency of `by`.
    Unsatisfiable {
        /// Package with the dependency.
        by: String,
        /// Rendered dependency alternatives.
        dep: String,
    },
    /// Two packages in the plan conflict.
    Conflict(String, String),
}

/// Result of planning.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Packages to unpack/configure in dependency order (dependencies first).
    pub install: Vec<(String, Version)>,
    /// Packages being upgraded (replaced) by the plan.
    pub upgrade: Vec<String>,
}

/// Repository index + installed set.
#[derive(Default)]
pub struct Universe {
    /// name -> candidates (any order).
    pub available: BTreeMap<String, Vec<Pkg>>,
    /// name -> installed package.
    pub installed: BTreeMap<String, Pkg>,
}

impl Universe {
    fn candidates<'a>(&'a self, a: &'a Atom) -> impl Iterator<Item = &'a Pkg> + 'a {
        self.available.values().flatten().filter(move |p| p.satisfies(a))
    }

    fn render(alt: &Alt) -> String {
        alt.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(" | ")
    }

    /// Plan installing `want` (newest acceptable version), reusing installed packages that
    /// already satisfy dependencies. Deterministic: alternatives are tried in order, preferring
    /// installed, then newest, then name.
    pub fn plan_install(&self, want: &str) -> Result<Plan, Error> {
        let mut chosen: BTreeMap<String, &Pkg> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        let mut visiting = BTreeSet::new();
        let top = self.available.get(want).and_then(|v| v.iter().max_by(|a, b| a.version.cmp(&b.version))).ok_or_else(|| Error::NotFound(want.to_string()))?;
        self.visit(top, &mut chosen, &mut order, &mut visiting)?;
        // Conflicts among chosen + installed-but-kept packages.
        let final_set = |name: &str| -> Option<&Pkg> { chosen.get(name).copied().or_else(|| self.installed.get(name)) };
        let names: BTreeSet<&str> = chosen.keys().map(|s| s.as_str()).chain(self.installed.keys().map(|s| s.as_str())).collect();
        for n in &names {
            let p = final_set(n).unwrap();
            for c in &p.conflicts {
                for m in &names {
                    if m == n {
                        continue;
                    }
                    if final_set(m).unwrap().satisfies(c) && (chosen.contains_key(*n) || chosen.contains_key(*m)) {
                        return Err(Error::Conflict(n.to_string(), m.to_string()));
                    }
                }
            }
        }
        let mut plan = Plan::default();
        for n in order {
            let p = chosen[&n];
            if self.installed.get(&n).is_some_and(|i| i.version == p.version) {
                continue;
            }
            if self.installed.contains_key(&n) {
                plan.upgrade.push(n.clone());
            }
            plan.install.push((n, p.version.clone()));
        }
        Ok(plan)
    }

    fn visit<'a>(&'a self, p: &'a Pkg, chosen: &mut BTreeMap<String, &'a Pkg>, order: &mut Vec<String>, visiting: &mut BTreeSet<String>) -> Result<(), Error> {
        if chosen.contains_key(&p.name) || visiting.contains(&p.name) {
            return Ok(()); // already planned, or a dependency cycle (dpkg configures those together)
        }
        visiting.insert(p.name.clone());
        for alt in &p.depends {
            // Already satisfied by something chosen or installed (and not being replaced)?
            if alt.iter().any(|a| chosen.values().any(|c| c.satisfies(a)) || (self.installed.values().any(|i| i.satisfies(a) && !chosen.contains_key(&i.name)))) {
                continue;
            }
            let mut done = false;
            for a in alt {
                let best = self.candidates(a).max_by(|x, y| x.name.eq(&a.name).cmp(&y.name.eq(&a.name)).then(x.version.cmp(&y.version)).then(y.name.cmp(&x.name)));
                if let Some(c) = best {
                    self.visit(c, chosen, order, visiting)?;
                    done = true;
                    break;
                }
            }
            if !done {
                return Err(Error::Unsatisfiable { by: p.name.clone(), dep: Self::render(alt) });
            }
        }
        visiting.remove(&p.name);
        chosen.insert(p.name.clone(), p);
        order.push(p.name.clone());
        Ok(())
    }

    /// Installed packages that (transitively) depend on `name` and would break if it is removed.
    pub fn reverse_deps(&self, name: &str) -> Vec<String> {
        let mut gone: BTreeSet<String> = BTreeSet::new();
        gone.insert(name.to_string());
        loop {
            let mut grew = false;
            for (n, p) in &self.installed {
                if gone.contains(n) {
                    continue;
                }
                let broken = p.depends.iter().any(|alt| {
                    !alt.iter().any(|a| self.installed.iter().any(|(m, q)| !gone.contains(m) && q.satisfies(a)))
                });
                if broken {
                    gone.insert(n.clone());
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        gone.remove(name);
        gone.into_iter().collect()
    }
}
