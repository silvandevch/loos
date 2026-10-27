//! winget manifest model (schema 1.x singleton or version+installer+locale files).

use crate::yaml::{self, Value};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cmp::Ordering;

/// CPU architecture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    /// 32-bit x86.
    X86,
    /// x86-64.
    X64,
    /// 32-bit ARM.
    Arm,
    /// 64-bit ARM.
    Arm64,
    /// Architecture independent.
    Neutral,
}

impl Arch {
    fn parse(s: &str) -> Option<Arch> {
        Some(match s.to_ascii_lowercase().as_str() {
            "x86" => Arch::X86,
            "x64" => Arch::X64,
            "arm" => Arch::Arm,
            "arm64" => Arch::Arm64,
            "neutral" => Arch::Neutral,
            _ => return None,
        })
    }
}

/// One installer entry with manifest-level defaults applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installer {
    /// Architecture.
    pub arch: Arch,
    /// `msi`, `msix`, `exe`, `inno`, `nullsoft`, `wix`, `burn`, `zip`, `portable`, ...
    pub kind: String,
    /// Download URL.
    pub url: String,
    /// Lower-case hex SHA-256 (64 chars) the download must match.
    pub sha256: String,
    /// `user` / `machine` or empty.
    pub scope: String,
    /// Silent-install switches, if given.
    pub silent: Option<String>,
    /// Product code (MSI/uninstall key).
    pub product_code: Option<String>,
}

/// Manifest parse/validation error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// YAML syntax.
    Yaml(yaml::Error),
    /// Required field missing or malformed.
    Field(&'static str),
}

impl From<yaml::Error> for Error {
    fn from(e: yaml::Error) -> Self {
        Error::Yaml(e)
    }
}

/// A package version manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// `Publisher.Package`.
    pub id: String,
    /// Version string.
    pub version: String,
    /// Display name.
    pub name: String,
    /// Publisher.
    pub publisher: String,
    /// Moniker (short name).
    pub moniker: Option<String>,
    /// Tags.
    pub tags: Vec<String>,
    /// Installers.
    pub installers: Vec<Installer>,
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(|s| s.to_string())
}

fn valid_sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Manifest {
    /// Parse a singleton (or already-merged) manifest.
    pub fn parse(src: &str) -> Result<Manifest, Error> {
        Self::from_value(&yaml::parse(src)?)
    }

    /// Merge the multi-file form: version + installer + default-locale documents.
    pub fn parse_multi(version: &str, installer: &str, locale: &str) -> Result<Manifest, Error> {
        let mut all = Vec::new();
        for d in [version, installer, locale] {
            if let Value::Map(m) = yaml::parse(d)? {
                for (k, v) in m {
                    if !all.iter().any(|(n, _): &(String, Value)| *n == k) {
                        all.push((k, v));
                    }
                }
            } else {
                return Err(Error::Field("manifest must be a mapping"));
            }
        }
        Self::from_value(&Value::Map(all))
    }

    fn from_value(v: &Value) -> Result<Manifest, Error> {
        let id = text(v, "PackageIdentifier").ok_or(Error::Field("PackageIdentifier"))?;
        let version = text(v, "PackageVersion").ok_or(Error::Field("PackageVersion"))?;
        if !id.contains('.') || id.contains(char::is_whitespace) {
            return Err(Error::Field("PackageIdentifier"));
        }
        let ins = v.get("Installers").and_then(|x| x.as_seq()).ok_or(Error::Field("Installers"))?;
        let mut installers = Vec::new();
        for i in ins {
            let get = |k: &str| text(i, k).or_else(|| text(v, k));
            let arch = Arch::parse(&get("Architecture").ok_or(Error::Field("Architecture"))?).ok_or(Error::Field("Architecture"))?;
            let sha = get("InstallerSha256").ok_or(Error::Field("InstallerSha256"))?.to_ascii_lowercase();
            if !valid_sha(&sha) {
                return Err(Error::Field("InstallerSha256"));
            }
            let url = get("InstallerUrl").ok_or(Error::Field("InstallerUrl"))?;
            if !url.starts_with("https://") {
                return Err(Error::Field("InstallerUrl must be https"));
            }
            let sw = i.get("InstallerSwitches").or_else(|| v.get("InstallerSwitches"));
            installers.push(Installer {
                arch,
                kind: get("InstallerType").ok_or(Error::Field("InstallerType"))?.to_ascii_lowercase(),
                url,
                sha256: sha,
                scope: get("Scope").unwrap_or_default(),
                silent: sw.and_then(|s| text(s, "Silent")),
                product_code: get("ProductCode"),
            });
        }
        if installers.is_empty() {
            return Err(Error::Field("Installers"));
        }
        Ok(Manifest {
            name: text(v, "PackageName").unwrap_or_else(|| id.clone()),
            publisher: text(v, "Publisher").unwrap_or_else(|| id.split('.').next().unwrap_or("").to_string()),
            moniker: text(v, "Moniker"),
            tags: v.get("Tags").and_then(|t| t.as_seq()).map(|s| s.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default(),
            id,
            version,
            installers,
        })
    }

    /// Best installer for this machine: architecture preference order (x64 host:
    /// x64, neutral, x86), then requested scope, then manifest order.
    pub fn select_installer(&self, host: Arch, scope: Option<&str>) -> Option<&Installer> {
        let pref: &[Arch] = match host {
            Arch::X64 => &[Arch::X64, Arch::Neutral, Arch::X86],
            Arch::X86 => &[Arch::X86, Arch::Neutral],
            Arch::Arm64 => &[Arch::Arm64, Arch::Neutral, Arch::Arm, Arch::X64, Arch::X86],
            Arch::Arm => &[Arch::Arm, Arch::Neutral],
            Arch::Neutral => &[Arch::Neutral],
        };
        for a in pref {
            let cands: Vec<&Installer> = self.installers.iter().filter(|i| i.arch == *a).collect();
            if let Some(s) = scope {
                if let Some(c) = cands.iter().find(|i| i.scope == s) {
                    return Some(c);
                }
            }
            if let Some(c) = cands.first() {
                return Some(c);
            }
        }
        None
    }
}

/// Compare winget versions: dotted parts, numeric when both numeric, trailing zeros ignored.
pub fn cmp_version(a: &str, b: &str) -> Ordering {
    let pa: Vec<&str> = a.split(['.', '-', '+']).collect();
    let pb: Vec<&str> = b.split(['.', '-', '+']).collect();
    for i in 0..pa.len().max(pb.len()) {
        let (x, y) = (pa.get(i).copied().unwrap_or("0"), pb.get(i).copied().unwrap_or("0"));
        let o = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            _ => x.cmp(y),
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}
