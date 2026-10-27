//! Install planning from a [`Database`]: directory resolution, file list, features, and the
//! custom-action audit.

use crate::database::{Cell, Database};
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// A file to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedFile {
    /// `File` key.
    pub key: String,
    /// Path relative to the root of the install tree, forward slashes (`ProgramFilesFolder/App/x.exe`).
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// Owning component.
    pub component: String,
    /// Media sequence number (position in the cabinet).
    pub sequence: i32,
}

/// What a custom action does, by `Type & 0x3F`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaKind {
    /// Calls a DLL export (types 1, 17, 33, 65 variants).
    Dll,
    /// Runs an EXE (2, 18, 34, 50).
    Exe,
    /// VBScript / JScript (5, 6, 21, 22, 37, 38, 53, 54).
    Script,
    /// Sets a directory (35, 51 family: 19=error, 35=SetDirectory, 51=SetProperty).
    SetValue,
    /// Shows an error / stops install (19).
    Error,
    /// Anything else.
    Other,
}

/// A custom action for audit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomAction {
    /// Action name.
    pub name: String,
    /// Kind.
    pub kind: CaKind,
    /// Raw type.
    pub ty: i32,
    /// Source (binary name / file key / property).
    pub source: String,
    /// Target (entry point / command line / value).
    pub target: String,
    /// Runs deferred in system context (`msidbCustomActionTypeInScript` + no-impersonate).
    pub elevated: bool,
}

/// Feature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feature {
    /// Key.
    pub name: String,
    /// Parent.
    pub parent: Option<String>,
    /// Title.
    pub title: String,
    /// Install level.
    pub level: i32,
    /// Component keys.
    pub components: Vec<String>,
}

/// The plan.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// Product name.
    pub name: String,
    /// Product version.
    pub version: String,
    /// Product code (GUID).
    pub product_code: String,
    /// Manufacturer.
    pub manufacturer: String,
    /// Files in sequence order.
    pub files: Vec<PlannedFile>,
    /// Features.
    pub features: Vec<Feature>,
    /// Custom actions found.
    pub custom_actions: Vec<CustomAction>,
    /// Embedded cabinet stream names (`#cab1.cab` -> `cab1.cab`) and external cabinet names.
    pub cabinets: Vec<String>,
}

/// Planning errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Required table absent.
    MissingTable(&'static str),
    /// Directory table has a cycle or dangling parent.
    BadDirectory(String),
    /// A resolved path escapes the install root.
    Unsafe(String),
}

/// Standard folder properties mapped to root segment names (the caller maps them to real
/// LoOS locations).
pub fn is_standard_folder(p: &str) -> bool {
    matches!(
        p,
        "TARGETDIR" | "ProgramFilesFolder" | "ProgramFiles64Folder" | "CommonFilesFolder" | "CommonFiles64Folder" | "SystemFolder" | "System64Folder" | "WindowsFolder"
            | "AppDataFolder" | "LocalAppDataFolder" | "CommonAppDataFolder" | "DesktopFolder" | "ProgramMenuFolder" | "StartMenuFolder" | "StartupFolder"
            | "TempFolder" | "PersonalFolder" | "FontsFolder" | "WindowsVolume" | "SourceDir"
    )
}

/// Pick the long name from `short|long`, resolving `.` (same as parent) to "".
fn long_name(s: &str) -> &str {
    s.rsplit('|').next().unwrap_or(s)
}

fn safe_segment(s: &str) -> bool {
    s != ".." && !s.contains(['/', '\\', ':', '\0'])
}

/// Substitute `[Property]` references using `props` (unknown -> empty, `[\x]` escapes -> x).
pub fn subst(s: &str, props: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('[') {
        out.push_str(&rest[..i]);
        match rest[i..].find(']') {
            Some(j) => {
                let name = &rest[i + 1..i + j];
                if let Some(esc) = name.strip_prefix('\\') {
                    out.push_str(esc);
                } else if let Some(v) = props.get(name) {
                    out.push_str(v);
                }
                rest = &rest[i + j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

impl Plan {
    /// Build a plan. `props` override/extend the `Property` table (e.g. `INSTALLDIR`).
    pub fn build(db: &Database, props: &BTreeMap<String, String>) -> Result<Plan, Error> {
        let dir_t = db.table("Directory").ok_or(Error::MissingTable("Directory"))?;
        let comp_t = db.table("Component").ok_or(Error::MissingTable("Component"))?;
        let file_t = db.table("File").ok_or(Error::MissingTable("File"))?;

        // Directory key -> path.
        let mut parent: BTreeMap<String, (Option<String>, String)> = BTreeMap::new();
        for r in 0..dir_t.rows.len() {
            let k = dir_t.get(r, "Directory").map(Cell::s).unwrap_or_default();
            let p = dir_t.get(r, "Directory_Parent").map(Cell::s).filter(|s| !s.is_empty());
            let d = dir_t.get(r, "DefaultDir").map(Cell::s).unwrap_or_default();
            // DefaultDir is "target[:source]" or "short|long[:source]"
            let target = d.split(':').next().unwrap_or("");
            parent.insert(k, (p, long_name(target).to_string()));
        }
        let resolve = |key: &str| -> Result<String, Error> {
            let mut parts: Vec<String> = Vec::new();
            let mut cur = key.to_string();
            for _ in 0..64 {
                let Some((p, name)) = parent.get(&cur) else { return Err(Error::BadDirectory(cur)) };
                // A property override or standard folder terminates the chain with its own root.
                if let Some(v) = props.get(&cur) {
                    parts.push(v.trim_end_matches(['/', '\\']).to_string());
                    parts.reverse();
                    return Ok(parts.join("/"));
                }
                if is_standard_folder(&cur) || p.is_none() {
                    parts.push(cur.clone());
                    parts.reverse();
                    return Ok(parts.join("/"));
                }
                if name != "." && !name.is_empty() {
                    if !safe_segment(name) {
                        return Err(Error::Unsafe(name.clone()));
                    }
                    parts.push(subst(name, props));
                }
                cur = p.clone().unwrap();
            }
            Err(Error::BadDirectory(key.to_string()))
        };

        let mut comp_dir: BTreeMap<String, String> = BTreeMap::new();
        for r in 0..comp_t.rows.len() {
            let c = comp_t.get(r, "Component").map(Cell::s).unwrap_or_default();
            let d = comp_t.get(r, "Directory_").map(Cell::s).unwrap_or_default();
            comp_dir.insert(c, resolve(&d)?);
        }
        let mut files = Vec::new();
        for r in 0..file_t.rows.len() {
            let g = |n: &str| file_t.get(r, n).cloned().unwrap_or(Cell::Null);
            let comp = g("Component_").s();
            let name = long_name(&g("FileName").s()).to_string();
            if !safe_segment(&name) || name.is_empty() {
                return Err(Error::Unsafe(name));
            }
            let dir = comp_dir.get(&comp).ok_or(Error::BadDirectory(comp.clone()))?;
            files.push(PlannedFile { key: g("File").s(), path: alloc::format!("{dir}/{name}"), size: g("FileSize").i() as u32 as u64, component: comp, sequence: g("Sequence").i() });
        }
        files.sort_by_key(|f| f.sequence);

        let mut features = Vec::new();
        if let Some(ft) = db.table("Feature") {
            for r in 0..ft.rows.len() {
                let name = ft.get(r, "Feature").map(Cell::s).unwrap_or_default();
                let components = db
                    .table("FeatureComponents")
                    .map(|fc| (0..fc.rows.len()).filter(|&i| fc.get(i, "Feature_").map(Cell::s).as_deref() == Some(&name)).filter_map(|i| fc.get(i, "Component_").map(Cell::s)).collect())
                    .unwrap_or_default();
                features.push(Feature {
                    parent: ft.get(r, "Feature_Parent").map(Cell::s).filter(|s| !s.is_empty()),
                    title: ft.get(r, "Title").map(Cell::s).unwrap_or_default(),
                    level: ft.get(r, "Level").map(Cell::i).unwrap_or(0),
                    components,
                    name,
                });
            }
        }

        let mut cas = Vec::new();
        if let Some(t) = db.table("CustomAction") {
            for r in 0..t.rows.len() {
                let ty = t.get(r, "Type").map(Cell::i).unwrap_or(0);
                let base = ty & 0x3F;
                let kind = match base {
                    1 | 17 | 33 => CaKind::Dll,
                    2 | 18 | 34 | 50 => CaKind::Exe,
                    5 | 6 | 21 | 22 | 37 | 38 | 53 | 54 => CaKind::Script,
                    19 => CaKind::Error,
                    35 | 51 => CaKind::SetValue,
                    _ => CaKind::Other,
                };
                cas.push(CustomAction {
                    name: t.get(r, "Action").map(Cell::s).unwrap_or_default(),
                    kind,
                    ty,
                    source: t.get(r, "Source").map(Cell::s).unwrap_or_default(),
                    target: t.get(r, "Target").map(Cell::s).unwrap_or_default(),
                    elevated: ty & 0x400 != 0 && ty & 0x800 != 0, // InScript + NoImpersonate
                });
            }
        }

        let mut cabinets = Vec::new();
        if let Some(m) = db.table("Media") {
            for r in 0..m.rows.len() {
                if let Some(c) = m.get(r, "Cabinet").map(Cell::s).filter(|s| !s.is_empty()) {
                    cabinets.push(c);
                }
            }
        }

        let p = |n: &str| props.get(n).cloned().or_else(|| db.property(n)).unwrap_or_default();
        Ok(Plan { name: p("ProductName"), version: p("ProductVersion"), product_code: p("ProductCode"), manufacturer: p("Manufacturer"), files, features, custom_actions: cas, cabinets })
    }

    /// Custom actions that execute foreign code (DLL/EXE/script).
    pub fn risky_actions(&self) -> Vec<&CustomAction> {
        self.custom_actions.iter().filter(|c| matches!(c.kind, CaKind::Dll | CaKind::Exe | CaKind::Script)).collect()
    }

    /// Total bytes of files in the selected features (level <= `max_level`).
    pub fn size_for_level(&self, max_level: i32) -> u64 {
        let comps: Vec<&String> = self.features.iter().filter(|f| f.level > 0 && f.level <= max_level).flat_map(|f| f.components.iter()).collect();
        self.files.iter().filter(|f| comps.contains(&&f.component)).map(|f| f.size).sum()
    }
}
