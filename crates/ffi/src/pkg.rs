//! dpkg, winget, msix, msi and pkgstate.

use crate::{bytes, cstr, put_str};
use std::ffi::c_char;

/// Debian version comparison: -1, 0, 1; 2 if either string is not a valid version.
#[no_mangle]
pub unsafe extern "C" fn loos_deb_version_cmp(a: *const c_char, b: *const c_char) -> i32 {
    // SAFETY: see crate contract.
    unsafe {
        use loos_dpkg::version::Version;
        match (cstr(a).and_then(Version::parse), cstr(b).and_then(Version::parse)) {
            (Some(x), Some(y)) => x.cmp(&y) as i32,
            _ => 2,
        }
    }
}

/// `dpkg --compare-versions` ordering for any two strings (valid or not): -1, 0, 1.
#[no_mangle]
pub unsafe extern "C" fn loos_deb_version_compare(a: *const c_char, b: *const c_char) -> i32 {
    // SAFETY: see crate contract.
    unsafe { loos_dpkg::version::compare_lenient(cstr(a).unwrap_or(""), cstr(b).unwrap_or("")) as i32 }
}

/// Plans `want` against a `Packages`-style index and a status db (both control text). Output is one line per
/// package to install, dependencies first: `name version\n`. Returns text length, or negative:
/// -1 bad args/parse, -2 package not found, -3 unsatisfiable dependency, -4 conflict.
#[no_mangle]
pub unsafe extern "C" fn loos_dpkg_plan(avail: *const c_char, installed: *const c_char, want: *const c_char, out: *mut u8, cap: usize) -> i64 {
    use loos_dpkg::{control, resolver::*};
    // SAFETY: see crate contract.
    unsafe {
        let (Some(a), Some(i), Some(w)) = (cstr(avail), cstr(installed), cstr(want)) else { return -1 };
        let (Ok(a), Ok(i)) = (control::parse(a), control::parse(i)) else { return -1 };
        let mut u = Universe::default();
        for s in &a {
            if let Some(p) = Pkg::from_stanza(s) {
                u.available.entry(p.name.clone()).or_default().push(p);
            }
        }
        for s in &i {
            if let Some(p) = Pkg::from_stanza(s) {
                u.installed.insert(p.name.clone(), p);
            }
        }
        match u.plan_install(w) {
            Ok(p) => {
                let mut t = String::new();
                for (n, v) in &p.install {
                    t.push_str(&format!("{} {}{}{}\n", n, if v.epoch != 0 { format!("{}:", v.epoch) } else { String::new() }, v.upstream, if v.revision.is_empty() { String::new() } else { format!("-{}", v.revision) }));
                }
                put_str(&t, out, cap) as i64
            }
            Err(Error::NotFound(_)) => -2,
            Err(Error::Unsatisfiable { .. }) => -3,
            Err(Error::Conflict(..)) => -4,
        }
    }
}

/// Picks the installer from a winget manifest for `arch` (0 x86, 1 x64, 2 arm, 3 arm64) and writes its URL,
/// SHA-256 and type. Returns 0, or -1 parse/validation failure, -2 no matching installer.
#[no_mangle]
pub unsafe extern "C" fn loos_winget_select(manifest: *const c_char, arch: u32, scope: *const c_char, url: *mut u8, url_cap: usize, sha256_hex: *mut u8, kind: *mut u8, kind_cap: usize) -> i32 {
    use loos_winget::manifest::{Arch, Manifest};
    // SAFETY: see crate contract.
    unsafe {
        let Some(m) = cstr(manifest).and_then(|t| Manifest::parse(t).ok()) else { return -1 };
        let host = match arch {
            0 => Arch::X86,
            1 => Arch::X64,
            2 => Arch::Arm,
            _ => Arch::Arm64,
        };
        let Some(i) = m.select_installer(host, cstr(scope).filter(|s| !s.is_empty())) else { return -2 };
        put_str(&i.url, url, url_cap);
        put_str(&i.sha256, sha256_hex, 65);
        put_str(&i.kind, kind, kind_cap);
        0
    }
}

#[repr(C)]
pub struct MsixInfo {
    name: [u8; 64],
    version: [u8; 24],
    arch: [u8; 12],
    display_name: [u8; 64],
    full_trust: u8,
    signed_: u8,
    napps: u32,
    nfiles: u32,
}

/// Opens an .msix/.appx image; 0 ok, -1 zip error, -2 xml error, -3 no manifest, -4 invalid manifest.
#[no_mangle]
pub unsafe extern "C" fn loos_msix_inspect(data: *const u8, len: usize, out: *mut MsixInfo) -> i32 {
    use loos_msix::{Error, Package};
    // SAFETY: see crate contract.
    unsafe {
        match Package::open(bytes(data, len)) {
            Ok(p) => {
                let o = &mut *out;
                put_str(&p.manifest.identity.name, o.name.as_mut_ptr(), 64);
                put_str(&p.manifest.identity.version, o.version.as_mut_ptr(), 24);
                put_str(&p.manifest.identity.arch, o.arch.as_mut_ptr(), 12);
                put_str(&p.manifest.display_name, o.display_name.as_mut_ptr(), 64);
                o.full_trust = p.manifest.full_trust() as u8;
                o.signed_ = p.signed as u8;
                o.napps = p.manifest.applications.len() as u32;
                o.nfiles = p.payload().len() as u32;
                0
            }
            Err(Error::Zip(_)) => -1,
            Err(Error::Xml(_)) => -2,
            Err(Error::NoManifest) => -3,
            Err(Error::Manifest(_)) => -4,
        }
    }
}

#[repr(C)]
pub struct MsiInfo {
    product: [u8; 96],
    version: [u8; 24],
    product_code: [u8; 48],
    manufacturer: [u8; 64],
    nfiles: u32,
    nfeatures: u32,
    ncustom_actions: u32,
    nrisky_actions: u32,
    total_bytes: u64,
}

/// Reads an .msi and builds its install plan summary. 0 ok, -1 not a valid MSI, -2 plan error.
#[no_mangle]
pub unsafe extern "C" fn loos_msi_inspect(data: *const u8, len: usize, out: *mut MsiInfo) -> i32 {
    use loos_msi::{plan::Plan, Database};
    // SAFETY: see crate contract.
    unsafe {
        let Ok(db) = Database::open(bytes(data, len)) else { return -1 };
        let Ok(p) = Plan::build(&db, &Default::default()) else { return -2 };
        let o = &mut *out;
        put_str(&p.name, o.product.as_mut_ptr(), 96);
        put_str(&p.version, o.version.as_mut_ptr(), 24);
        put_str(&p.product_code, o.product_code.as_mut_ptr(), 48);
        put_str(&p.manufacturer, o.manufacturer.as_mut_ptr(), 64);
        o.nfiles = p.files.len() as u32;
        o.nfeatures = p.features.len() as u32;
        o.ncustom_actions = p.custom_actions.len() as u32;
        o.nrisky_actions = p.risky_actions().len() as u32;
        o.total_bytes = p.files.iter().map(|f| f.size).sum();
        0
    }
}

/// Install path and size of file `idx` of an MSI plan. 0 ok, -1 error / out of range.
#[no_mangle]
pub unsafe extern "C" fn loos_msi_file(data: *const u8, len: usize, idx: u32, path: *mut u8, cap: usize, size: *mut u64) -> i32 {
    use loos_msi::{plan::Plan, Database};
    // SAFETY: see crate contract.
    unsafe {
        let Ok(db) = Database::open(bytes(data, len)) else { return -1 };
        let Ok(p) = Plan::build(&db, &Default::default()) else { return -1 };
        match p.files.get(idx as usize) {
            Some(f) => {
                put_str(&f.path, path, cap);
                *size = f.size;
                0
            }
            None => -1,
        }
    }
}

/// Download trust gate. Lists are comma separated. Returns 0 allowed; 1 not https; 2 bad URL; 3 host denied; 4 no/invalid
/// hash; 5 publisher denied. `publisher` may be NULL (unsigned).
#[no_mangle]
pub unsafe extern "C" fn loos_trust_check(hosts: *const c_char, publishers: *const c_char, allow_unsigned: bool, url: *const c_char, sha256_hex: *const c_char, publisher: *const c_char) -> i32 {
    use loos_pkgstate::trust::{Candidate, Denial, Policy};
    // SAFETY: see crate contract.
    unsafe {
        let split = |p: *const c_char| -> Vec<String> { cstr(p).unwrap_or("").split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).map(String::from).collect() };
        let pol = Policy { hosts: split(hosts), publishers: split(publishers), allow_unsigned };
        let (Some(u), Some(h)) = (cstr(url), cstr(sha256_hex)) else { return 2 };
        match pol.check(&Candidate { url: u, sha256: h, signed_publisher: cstr(publisher) }) {
            Ok(()) => 0,
            Err(Denial::NotHttps) => 1,
            Err(Denial::BadUrl) => 2,
            Err(Denial::HostDenied(_)) => 3,
            Err(Denial::NoHash) => 4,
            Err(Denial::PublisherDenied) => 5,
        }
    }
}

/// True when `data` hashes to the 64-hex `expected`.
#[no_mangle]
pub unsafe extern "C" fn loos_sha256_verify(data: *const u8, len: usize, expected_hex: *const c_char) -> bool {
    // SAFETY: see crate contract.
    unsafe { cstr(expected_hex).is_some_and(|h| loos_pkgstate::sha256::verify(bytes(data, len), h)) }
}

/// File-ownership database (opaque).
pub struct OwnDb(loos_pkgstate::ownership::Db);

#[no_mangle]
pub extern "C" fn loos_ownership_new() -> *mut OwnDb {
    Box::into_raw(Box::new(OwnDb(Default::default())))
}
#[no_mangle]
pub unsafe extern "C" fn loos_ownership_free(d: *mut OwnDb) {
    if !d.is_null() {
        // SAFETY: allocated by loos_ownership_new.
        drop(unsafe { Box::from_raw(d) });
    }
}
/// Records an install. `paths` is NUL-separated, double-NUL terminated; a leading '*' marks a shared file.
/// Returns 0, -1 bad args, -2 path conflict (file owned by another package).
#[no_mangle]
pub unsafe extern "C" fn loos_ownership_install(d: *mut OwnDb, pkg: *const c_char, version: *const c_char, paths: *const c_char) -> i32 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(d), Some(p), Some(v)) = (d.as_mut(), cstr(pkg), cstr(version)) else { return -1 };
        let mut files: Vec<(String, bool)> = Vec::new();
        let mut q = paths;
        while !q.is_null() && *q != 0 {
            let Some(s) = cstr(q) else { return -1 };
            files.push(match s.strip_prefix('*') {
                Some(r) => (r.to_string(), true),
                None => (s.to_string(), false),
            });
            q = q.add(s.len() + 1);
        }
        let refs: Vec<(&str, bool)> = files.iter().map(|(s, b)| (s.as_str(), *b)).collect();
        match d.0.install(p, v, &refs) {
            Ok(_) => 0,
            Err(loos_pkgstate::ownership::Error::Conflict { .. }) => -2,
            Err(_) => -1,
        }
    }
}
/// Owner list of a path, comma separated; returns text length.
#[no_mangle]
pub unsafe extern "C" fn loos_ownership_owners(d: *const OwnDb, path: *const c_char, out: *mut u8, cap: usize) -> i64 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(d), Some(p)) = (d.as_ref(), cstr(path)) else { return -1 };
        put_str(&d.0.owners(p).join(","), out, cap) as i64
    }
}
/// Files that removing `pkg` would delete (newline separated, deepest first). Returns text length or -1.
#[no_mangle]
pub unsafe extern "C" fn loos_ownership_plan_remove(d: *const OwnDb, pkg: *const c_char, out: *mut u8, cap: usize) -> i64 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(d), Some(p)) = (d.as_ref(), cstr(pkg)) else { return -1 };
        match d.0.plan_remove(p) {
            Ok(plan) => put_str(&plan.delete.join("\n"), out, cap) as i64,
            Err(_) => -1,
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_ownership_remove(d: *mut OwnDb, pkg: *const c_char) -> i32 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(d), Some(p)) = (d.as_mut(), cstr(pkg)) else { return -1 };
        if d.0.remove(p).is_ok() { 0 } else { -1 }
    }
}
