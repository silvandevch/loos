//! Registry service: several hives (SYSTEM, SOFTWARE, SAM, SECURITY, .DEFAULT, per-user)
//! mounted into one `HKEY_*` namespace and persisted through a [`HiveStorage`]
//! (backed by btrfs via `loos-vfs`, see [`crate::vfs_store`]).
//!
//! Saves are crash-safe: the new image is written to `<name>.new` and renamed over
//! the old one (a btrfs CoW rename), so a hive file is always either old or new.

use crate::{Error, Hive, Key, Value, ValueType};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Persistent store for raw hive images.
pub trait HiveStorage {
    /// Read the image `file`, or `None` if it does not exist.
    fn load(&mut self, file: &str) -> Result<Option<Vec<u8>>, Error>;
    /// Atomically replace `file` with `data`.
    fn store(&mut self, file: &str, data: &[u8]) -> Result<(), Error>;
}

/// In-memory storage (tests, early boot before btrfs is mounted).
#[derive(Default)]
pub struct MemStorage {
    /// Stored files.
    pub files: alloc::collections::BTreeMap<String, Vec<u8>>,
    /// Number of `store` calls.
    pub writes: usize,
}

impl HiveStorage for MemStorage {
    fn load(&mut self, file: &str) -> Result<Option<Vec<u8>>, Error> {
        Ok(self.files.get(file).cloned())
    }
    fn store(&mut self, file: &str, data: &[u8]) -> Result<(), Error> {
        self.writes += 1;
        self.files.insert(file.to_string(), data.to_vec());
        Ok(())
    }
}

struct Mounted {
    /// Mount point in the unified namespace, upper-case, `\`-separated (e.g. `MACHINE\SYSTEM`).
    mount: String,
    file: String,
    hive: Hive,
    dirty: bool,
}

/// Standard hive layout: `(mount point, file name)`.
pub const STANDARD_HIVES: &[(&str, &str)] = &[
    ("MACHINE\\SYSTEM", "config/SYSTEM"),
    ("MACHINE\\SOFTWARE", "config/SOFTWARE"),
    ("MACHINE\\SAM", "config/SAM"),
    ("MACHINE\\SECURITY", "config/SECURITY"),
    ("USER\\.DEFAULT", "config/DEFAULT"),
];

/// The registry: unified namespace over hives.
pub struct Registry<S: HiveStorage> {
    storage: S,
    hives: Vec<Mounted>,
    /// SID string used for `HKEY_CURRENT_USER`.
    pub current_user: String,
}

fn upper(s: &str) -> String {
    s.chars().map(|c| c.to_ascii_uppercase()).collect()
}

impl<S: HiveStorage> Registry<S> {
    /// Open the registry, loading every hive in `layout` (a missing file yields an empty hive).
    /// A corrupt hive is an error (never silently replaced).
    pub fn open(mut storage: S, layout: &[(&str, &str)]) -> Result<Self, Error> {
        let mut hives = Vec::new();
        for (mount, file) in layout {
            let (hive, dirty) = match storage.load(file)? {
                Some(d) => (Hive::load(&d)?, false),
                None => (Hive::new(), true),
            };
            hives.push(Mounted { mount: upper(mount), file: (*file).to_string(), hive, dirty });
        }
        Ok(Self { storage, hives, current_user: ".DEFAULT".to_string() })
    }

    /// Mount an additional hive (e.g. a user's `NTUSER.DAT`) at `mount`.
    pub fn load_hive(&mut self, mount: &str, file: &str) -> Result<(), Error> {
        let m = upper(mount);
        if self.hives.iter().any(|h| h.mount == m) {
            return Err(Error::Exists);
        }
        let (hive, dirty) = match self.storage.load(file)? {
            Some(d) => (Hive::load(&d)?, false),
            None => (Hive::new(), true),
        };
        self.hives.push(Mounted { mount: m, file: file.to_string(), hive, dirty });
        Ok(())
    }

    /// Flush and unmount a hive.
    pub fn unload_hive(&mut self, mount: &str) -> Result<(), Error> {
        let m = upper(mount);
        let i = self.hives.iter().position(|h| h.mount == m).ok_or(Error::NotFound)?;
        self.flush_one(i)?;
        self.hives.remove(i);
        Ok(())
    }

    /// Expand `HKEY_*` roots and return the namespace path (`MACHINE\...` / `USER\...`).
    pub fn canonical(&self, path: &str) -> Result<String, Error> {
        let norm: String = path.replace('/', "\\");
        let mut parts = norm.split('\\').filter(|p| !p.is_empty());
        let root = parts.next().ok_or(Error::InvalidName)?;
        let rest: Vec<&str> = parts.collect();
        let rest = rest.join("\\");
        let base = match upper(root).as_str() {
            "HKLM" | "HKEY_LOCAL_MACHINE" | "MACHINE" => "MACHINE".to_string(),
            "HKU" | "HKEY_USERS" | "USER" => "USER".to_string(),
            "HKCU" | "HKEY_CURRENT_USER" => alloc::format!("USER\\{}", self.current_user),
            "HKCR" | "HKEY_CLASSES_ROOT" => "MACHINE\\SOFTWARE\\Classes".to_string(),
            "HKCC" | "HKEY_CURRENT_CONFIG" => "MACHINE\\SYSTEM\\CurrentControlSet\\Hardware Profiles\\Current".to_string(),
            _ => return Err(Error::InvalidName),
        };
        Ok(if rest.is_empty() { base } else { alloc::format!("{base}\\{rest}") })
    }

    fn route(&self, path: &str) -> Result<(usize, String), Error> {
        let canon = self.canonical(path)?;
        let up = upper(&canon);
        let mut best: Option<(usize, usize)> = None;
        for (i, h) in self.hives.iter().enumerate() {
            let m = &h.mount;
            let hit = up == *m || (up.starts_with(m.as_str()) && up.as_bytes().get(m.len()) == Some(&b'\\'));
            if hit && best.is_none_or(|(_, l)| m.len() > l) {
                best = Some((i, m.len()));
            }
        }
        let (i, l) = best.ok_or(Error::NotFound)?;
        Ok((i, canon[l..].trim_start_matches('\\').to_string()))
    }

    /// Create a key (and parents).
    pub fn create_key(&mut self, path: &str) -> Result<(), Error> {
        let (i, sub) = self.route(path)?;
        self.hives[i].hive.create_key(&sub)?;
        self.hives[i].dirty = true;
        Ok(())
    }

    /// Does the key exist?
    pub fn key_exists(&self, path: &str) -> bool {
        self.route(path).and_then(|(i, s)| self.hives[i].hive.key(&s).map(|_| ())).is_ok()
    }

    /// Delete a key and its subtree (`RegDeleteTree`). Hive roots cannot be deleted.
    pub fn delete_key(&mut self, path: &str) -> Result<(), Error> {
        let (i, sub) = self.route(path)?;
        let (parent, name) = sub.rsplit_once('\\').map_or(("", sub.as_str()), |(p, n)| (p, n));
        if name.is_empty() {
            return Err(Error::InvalidName);
        }
        let pk = self.hives[i].hive.key_mut(parent)?;
        let idx = pk.children.iter().position(|c| c.name.eq_ignore_ascii_case(name)).ok_or(Error::NotFound)?;
        pk.children.remove(idx);
        self.hives[i].dirty = true;
        Ok(())
    }

    /// Set a value (creates the key if needed).
    pub fn set_value(&mut self, path: &str, name: &str, kind: ValueType, data: &[u8]) -> Result<(), Error> {
        let (i, sub) = self.route(path)?;
        self.hives[i].hive.set_value(&sub, name, kind, data)?;
        self.hives[i].dirty = true;
        Ok(())
    }

    /// Set a `REG_SZ` value.
    pub fn set_string(&mut self, path: &str, name: &str, s: &str) -> Result<(), Error> {
        let mut d: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        d.extend([0, 0]);
        self.set_value(path, name, ValueType::Sz, &d)
    }

    /// Set a `REG_DWORD` value.
    pub fn set_dword(&mut self, path: &str, name: &str, v: u32) -> Result<(), Error> {
        self.set_value(path, name, ValueType::Dword, &v.to_le_bytes())
    }

    /// Get a value.
    pub fn get_value(&self, path: &str, name: &str) -> Result<&Value, Error> {
        let (i, sub) = self.route(path)?;
        self.hives[i].hive.value(&sub, name)
    }

    /// Get a `REG_SZ`/`REG_EXPAND_SZ` value as text.
    pub fn get_string(&self, path: &str, name: &str) -> Result<String, Error> {
        let v = self.get_value(path, name)?;
        if !matches!(v.kind, ValueType::Sz | ValueType::ExpandSz) {
            return Err(Error::Unsupported);
        }
        let u: Vec<u16> = v.data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
        Ok(String::from_utf16_lossy(&u))
    }

    /// Get a `REG_DWORD`.
    pub fn get_dword(&self, path: &str, name: &str) -> Result<u32, Error> {
        let v = self.get_value(path, name)?;
        match (v.kind, v.data.as_slice()) {
            (ValueType::Dword, [a, b, c, d]) => Ok(u32::from_le_bytes([*a, *b, *c, *d])),
            _ => Err(Error::Unsupported),
        }
    }

    /// Delete a value.
    pub fn delete_value(&mut self, path: &str, name: &str) -> Result<(), Error> {
        let (i, sub) = self.route(path)?;
        self.hives[i].hive.delete_value(&sub, name)?;
        self.hives[i].dirty = true;
        Ok(())
    }

    /// Names of child keys (`RegEnumKeyEx`).
    pub fn enum_keys(&self, path: &str) -> Result<Vec<String>, Error> {
        let me = upper(&self.canonical(path)?);
        let mut v: Vec<String> = Vec::new();
        if let Ok((i, sub)) = self.route(path) {
            v.extend(self.hives[i].hive.key(&sub)?.children.iter().map(|c| c.name.clone()));
        }
        // Mounted hives that hang directly below this namespace node (e.g. HKLM -> SYSTEM, SOFTWARE).
        let pre = alloc::format!("{me}\\");
        let mut virt = false;
        for h in &self.hives {
            if let Some(rest) = h.mount.strip_prefix(pre.as_str()) {
                virt = true;
                let n = rest.split('\\').next().unwrap_or("");
                if !n.is_empty() && !v.iter().any(|x| x.eq_ignore_ascii_case(n)) {
                    v.push(n.to_string());
                }
            }
        }
        if v.is_empty() && !virt && self.route(path).is_err() {
            return Err(Error::NotFound);
        }
        Ok(v)
    }

    /// Values of a key (`RegEnumValue`).
    pub fn enum_values(&self, path: &str) -> Result<Vec<Value>, Error> {
        let (i, sub) = self.route(path)?;
        Ok(self.hives[i].hive.key(&sub)?.values.clone())
    }

    fn flush_one(&mut self, i: usize) -> Result<(), Error> {
        if !self.hives[i].dirty {
            return Ok(());
        }
        let img = self.hives[i].hive.save()?;
        let file = self.hives[i].file.clone();
        self.storage.store(&file, &img)?;
        self.hives[i].dirty = false;
        Ok(())
    }

    /// Write every modified hive (`RegFlushKey`). Stops at the first error, leaving the rest dirty.
    pub fn flush(&mut self) -> Result<usize, Error> {
        let mut n = 0;
        for i in 0..self.hives.len() {
            if self.hives[i].dirty {
                self.flush_one(i)?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Consume the registry, returning the storage (does not flush).
    pub fn into_storage(self) -> S {
        self.storage
    }

    /// Storage access (tests).
    pub fn storage(&self) -> &S {
        &self.storage
    }
}
