//! MSIX/APPX packages (task #171): [`zip`] reader with zip-slip and zip-bomb defenses,
//! [`xml`] subset parser, [`manifest`] model. [`Package::open`] ties them together and yields
//! the validated extraction plan. Signature *verification* is #176; here we only report whether
//! a signature part exists.
#![no_std]

extern crate alloc;

pub mod manifest;
pub mod xml;
pub mod zip;

use alloc::string::String;
use alloc::vec::Vec;

/// Errors opening a package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// ZIP structure.
    Zip(zip::Error),
    /// XML syntax.
    Xml(xml::Error),
    /// `AppxManifest.xml` missing.
    NoManifest,
    /// Manifest content invalid.
    Manifest(&'static str),
}

impl From<zip::Error> for Error {
    fn from(e: zip::Error) -> Self {
        Error::Zip(e)
    }
}
impl From<xml::Error> for Error {
    fn from(e: xml::Error) -> Self {
        Error::Xml(e)
    }
}

/// An opened package.
pub struct Package<'a> {
    /// ZIP archive.
    pub archive: zip::Archive<'a>,
    /// Parsed manifest.
    pub manifest: manifest::Manifest,
    /// True if `AppxSignature.p7x` is present (not verified here).
    pub signed: bool,
}

impl<'a> Package<'a> {
    /// Open and validate.
    pub fn open(data: &'a [u8]) -> Result<Package<'a>, Error> {
        let archive = zip::Archive::open(data)?;
        let m = archive.read("AppxManifest.xml").map_err(|e| if e == zip::Error::NotFound { Error::NoManifest } else { Error::Zip(e) })?;
        let text = core::str::from_utf8(&m).map_err(|_| Error::Manifest("manifest not UTF-8"))?;
        let manifest = manifest::Manifest::parse(text)?;
        // every <Application Executable=...> must exist in the package
        for app in &manifest.applications {
            if let Some(exe) = &app.executable {
                if archive.find(exe).is_none() {
                    return Err(Error::Manifest("application executable missing from package"));
                }
            }
        }
        let signed = archive.find("AppxSignature.p7x").is_some();
        Ok(Package { archive, manifest, signed })
    }

    /// Files to extract under the package root (excludes `[Content_Types].xml`, signature and
    /// block map metadata which belong to the store, not the install dir).
    pub fn payload(&self) -> Vec<&zip::Entry> {
        self.archive
            .entries()
            .iter()
            .filter(|e| !e.is_dir() && e.name != "[Content_Types].xml" && e.name != "AppxSignature.p7x" && !e.name.starts_with("AppxMetadata/"))
            .collect()
    }

    /// Install directory name: `Name_Version_Arch__Publisher` (publisher sanitized).
    pub fn install_dir(&self) -> String {
        let i = &self.manifest.identity;
        let pubr: String = i.publisher.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        alloc::format!("{}_{}_{}__{}", i.name, i.version, if i.arch.is_empty() { "neutral" } else { &i.arch }, pubr)
    }
}
