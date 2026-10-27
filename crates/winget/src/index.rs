//! Local repository index: search and version lookup.

use crate::manifest::{cmp_version, Manifest};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Searchable collection of manifests.
#[derive(Default)]
pub struct Index {
    entries: Vec<Manifest>,
}

impl Index {
    /// Add a manifest (replaces an identical id+version).
    pub fn add(&mut self, m: Manifest) {
        self.entries.retain(|e| !(e.id == m.id && e.version == m.version));
        self.entries.push(m);
    }

    /// Latest version of `id` (case-insensitive id).
    pub fn latest(&self, id: &str) -> Option<&Manifest> {
        self.entries.iter().filter(|e| e.id.eq_ignore_ascii_case(id)).max_by(|a, b| cmp_version(&a.version, &b.version))
    }

    /// Exact version.
    pub fn version(&self, id: &str, ver: &str) -> Option<&Manifest> {
        self.entries.iter().find(|e| e.id.eq_ignore_ascii_case(id) && e.version == ver)
    }

    /// Search id/name/moniker/tags; one result per package (its latest version), best first.
    pub fn search(&self, query: &str) -> Vec<&Manifest> {
        let q = query.to_ascii_lowercase();
        let mut ids: Vec<String> = Vec::new();
        for e in &self.entries {
            if !ids.iter().any(|i| i.eq_ignore_ascii_case(&e.id)) {
                ids.push(e.id.clone());
            }
        }
        let mut hits: Vec<(u32, &Manifest)> = Vec::new();
        for id in ids {
            let m = self.latest(&id).unwrap();
            let (lid, name) = (m.id.to_ascii_lowercase(), m.name.to_ascii_lowercase());
            let mon = m.moniker.as_deref().map(|s| s.to_ascii_lowercase());
            let score = if lid == q || mon.as_deref() == Some(&q) {
                100
            } else if name == q {
                90
            } else if name.starts_with(&q) || lid.starts_with(&q) {
                70
            } else if name.contains(&q) || lid.contains(&q) {
                50
            } else if m.tags.iter().any(|t| t.to_ascii_lowercase() == q) {
                30
            } else {
                0
            };
            if score > 0 {
                hits.push((score, m));
            }
        }
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
        hits.into_iter().map(|h| h.1).collect()
    }

    /// Ids with a newer version than `installed` (`(id, installed_version)` pairs).
    pub fn upgrades(&self, installed: &[(String, String)]) -> Vec<(String, String)> {
        installed
            .iter()
            .filter_map(|(id, v)| self.latest(id).filter(|m| cmp_version(&m.version, v).is_gt()).map(|m| (id.clone(), m.version.to_string())))
            .collect()
    }
}
