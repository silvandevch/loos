//! `AppxManifest.xml` model.

use crate::xml;
use crate::Error;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// `<Identity>`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    /// Package name.
    pub name: String,
    /// Publisher distinguished name.
    pub publisher: String,
    /// `a.b.c.d`.
    pub version: String,
    /// `x86`, `x64`, `arm`, `arm64`, `neutral` (or empty).
    pub arch: String,
}

/// `<Application>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Application {
    /// Application id.
    pub id: String,
    /// Executable path in the package.
    pub executable: Option<String>,
    /// Entry point.
    pub entry_point: Option<String>,
}

/// `<TargetDeviceFamily>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceFamily {
    /// e.g. `Windows.Desktop`.
    pub name: String,
    /// Minimum version.
    pub min: String,
    /// Maximum tested version.
    pub max_tested: String,
}

/// Parsed manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    /// Identity.
    pub identity: Identity,
    /// Display name.
    pub display_name: String,
    /// Publisher display name.
    pub publisher_display: String,
    /// Device families.
    pub families: Vec<DeviceFamily>,
    /// Package dependencies `(name, min_version)`.
    pub dependencies: Vec<(String, String)>,
    /// Applications.
    pub applications: Vec<Application>,
    /// Capability names (`internetClient`, `runFullTrust`, ...).
    pub capabilities: Vec<String>,
}

fn valid_version(v: &str) -> bool {
    let p: Vec<&str> = v.split('.').collect();
    p.len() == 4 && p.iter().all(|x| x.parse::<u16>().is_ok())
}

impl Manifest {
    /// Parse and validate.
    pub fn parse(text: &str) -> Result<Manifest, Error> {
        let root = xml::parse(text)?;
        if root.local() != "Package" {
            return Err(Error::Manifest("root is not <Package>"));
        }
        let id = root.kid("Identity").ok_or(Error::Manifest("missing Identity"))?;
        let identity = Identity {
            name: id.attr("Name").ok_or(Error::Manifest("Identity.Name"))?.to_string(),
            publisher: id.attr("Publisher").ok_or(Error::Manifest("Identity.Publisher"))?.to_string(),
            version: id.attr("Version").ok_or(Error::Manifest("Identity.Version"))?.to_string(),
            arch: id.attr("ProcessorArchitecture").unwrap_or("").to_ascii_lowercase(),
        };
        if !valid_version(&identity.version) {
            return Err(Error::Manifest("Identity.Version must be a.b.c.d"));
        }
        if identity.name.is_empty() || identity.name.len() > 50 || !identity.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
            return Err(Error::Manifest("Identity.Name invalid"));
        }
        if !matches!(identity.arch.as_str(), "" | "x86" | "x64" | "arm" | "arm64" | "neutral") {
            return Err(Error::Manifest("unknown ProcessorArchitecture"));
        }
        let props = root.kid("Properties");
        let mut m = Manifest {
            display_name: props.and_then(|p| p.kid("DisplayName")).map(|n| n.text.trim().to_string()).unwrap_or_default(),
            publisher_display: props.and_then(|p| p.kid("PublisherDisplayName")).map(|n| n.text.trim().to_string()).unwrap_or_default(),
            identity,
            ..Default::default()
        };
        if let Some(d) = root.kid("Dependencies") {
            for f in d.kids("TargetDeviceFamily") {
                m.families.push(DeviceFamily {
                    name: f.attr("Name").unwrap_or("").to_string(),
                    min: f.attr("MinVersion").unwrap_or("").to_string(),
                    max_tested: f.attr("MaxVersionTested").unwrap_or("").to_string(),
                });
            }
            for p in d.kids("PackageDependency") {
                m.dependencies.push((p.attr("Name").unwrap_or("").to_string(), p.attr("MinVersion").unwrap_or("").to_string()));
            }
        }
        if let Some(a) = root.kid("Applications") {
            for app in a.kids("Application") {
                m.applications.push(Application {
                    id: app.attr("Id").ok_or(Error::Manifest("Application.Id"))?.to_string(),
                    executable: app.attr("Executable").map(|s| s.replace('\\', "/")),
                    entry_point: app.attr("EntryPoint").map(|s| s.to_string()),
                });
            }
        }
        if let Some(c) = root.kid("Capabilities") {
            m.capabilities = c.children.iter().filter_map(|k| k.attr("Name").map(|s| s.to_string())).collect();
        }
        Ok(m)
    }

    /// Does the package need full trust (unrestricted Win32)?
    pub fn full_trust(&self) -> bool {
        self.capabilities.iter().any(|c| c == "runFullTrust")
    }

    /// Does it target the desktop family?
    pub fn runs_on_desktop(&self) -> bool {
        self.families.is_empty() || self.families.iter().any(|f| f.name == "Windows.Desktop" || f.name == "Windows.Universal")
    }
}
