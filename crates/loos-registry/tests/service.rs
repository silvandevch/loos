use loos_registry::service::*;
use loos_registry::{Error, ValueType};

fn open(st: MemStorage) -> Registry<MemStorage> {
    Registry::open(st, STANDARD_HIVES).unwrap()
}

#[test]
fn roots_route_to_hives_and_persist_across_reopen() {
    let mut r = open(MemStorage::default());
    r.set_string("HKLM\\SOFTWARE\\LoOS\\Shell", "Theme", "dark").unwrap();
    r.set_dword("HKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Control", "Boot", 7).unwrap();
    r.set_string("HKCU\\Software\\App", "Name", "Zażółć").unwrap();
    r.set_string("HKCR\\.txt", "", "txtfile").unwrap();
    assert_eq!(r.flush().unwrap(), 5, "all standard hives are new, so all are written");
    assert_eq!(r.flush().unwrap(), 0, "clean after flush");
    let storage = r.into_storage();
    let r2 = open(storage);
    assert_eq!(r2.get_string("hklm\\software\\loos\\SHELL", "theme").unwrap(), "dark");
    assert_eq!(r2.get_dword("HKLM\\SYSTEM\\CurrentControlSet\\Control", "Boot").unwrap(), 7);
    assert_eq!(r2.get_string("HKU\\.DEFAULT\\Software\\App", "Name").unwrap(), "Zażółć");
    assert_eq!(r2.get_string("HKLM\\SOFTWARE\\Classes\\.txt", "").unwrap(), "txtfile");
}

#[test]
fn delete_enum_and_errors() {
    let mut r = open(MemStorage::default());
    r.set_dword("HKLM\\SOFTWARE\\A\\B", "x", 1).unwrap();
    r.set_dword("HKLM\\SOFTWARE\\A\\C", "y", 2).unwrap();
    let mut k = r.enum_keys("HKLM\\SOFTWARE\\A").unwrap();
    k.sort();
    assert_eq!(k, ["B", "C"]);
    assert!(r.enum_keys("HKLM").unwrap().iter().any(|n| n == "SYSTEM"));
    assert_eq!(r.enum_values("HKLM\\SOFTWARE\\A\\B").unwrap().len(), 1);
    r.delete_value("HKLM\\SOFTWARE\\A\\B", "x").unwrap();
    assert_eq!(r.get_value("HKLM\\SOFTWARE\\A\\B", "x").unwrap_err(), Error::NotFound);
    r.delete_key("HKLM\\SOFTWARE\\A").unwrap();
    assert!(!r.key_exists("HKLM\\SOFTWARE\\A\\C"));
    assert_eq!(r.delete_key("HKLM\\SOFTWARE\\A").unwrap_err(), Error::NotFound);
    assert_eq!(r.set_dword("BOGUS\\x", "v", 1).unwrap_err(), Error::InvalidName);
    assert_eq!(r.get_string("HKLM\\SYSTEM", "none").unwrap_err(), Error::NotFound);
    r.set_dword("HKLM\\SYSTEM", "d", 5).unwrap();
    assert_eq!(r.get_string("HKLM\\SYSTEM", "d").unwrap_err(), Error::Unsupported);
    let _ = ValueType::Dword;
}

#[test]
fn corrupt_hive_is_an_error_not_silently_replaced() {
    let mut st = MemStorage::default();
    st.files.insert("config/SYSTEM".into(), b"garbage not a hive".to_vec());
    assert!(Registry::open(st, STANDARD_HIVES).is_err());
}

#[test]
fn user_hive_load_and_hkcu_switch() {
    let mut r = open(MemStorage::default());
    r.load_hive("USER\\S-1-5-21-1001", "users/silvan/NTUSER.DAT").unwrap();
    r.current_user = "S-1-5-21-1001".into();
    r.set_string("HKCU\\Control Panel\\Desktop", "Wallpaper", "w.png").unwrap();
    assert_eq!(r.get_string("HKU\\S-1-5-21-1001\\Control Panel\\Desktop", "Wallpaper").unwrap(), "w.png");
    assert_eq!(r.load_hive("user\\s-1-5-21-1001", "x").unwrap_err(), Error::Exists);
    r.unload_hive("USER\\S-1-5-21-1001").unwrap();
    assert!(r.storage().files.contains_key("users/silvan/NTUSER.DAT"));
    assert!(r.get_string("HKCU\\Control Panel\\Desktop", "Wallpaper").is_err());
}

#[cfg(feature = "vfs")]
mod on_vfs {
    use super::*;
    use loos_registry::vfs_store::VfsStorage;
    use loos_vfs::{FileSystem, MountManager, VfsError, VolumeId};
    use loos_fs::{Entry, FileKind, FsError};
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Ram(BTreeMap<String, Vec<u8>>);
    fn ent(name: &str, size: u64) -> Entry {
        Entry { name: name.into(), kind: FileKind::File, size, mtime: 0, mode: 0o644, uid: 0, gid: 0, target: None, id: 1 }
    }
    impl FileSystem for Ram {
        fn stat(&mut self, p: &str) -> Result<Entry, VfsError> {
            self.0.get(p).map(|d| ent(p, d.len() as u64)).ok_or(VfsError::Fs(FsError::NotFound))
        }
        fn read_dir(&mut self, _: &str) -> Result<Vec<Entry>, VfsError> {
            Ok(vec![])
        }
        fn read(&mut self, p: &str, off: u64, buf: &mut [u8]) -> Result<usize, VfsError> {
            let d = self.0.get(p).ok_or(VfsError::Fs(FsError::NotFound))?;
            let o = (off as usize).min(d.len());
            let n = buf.len().min(d.len() - o);
            buf[..n].copy_from_slice(&d[o..o + n]);
            Ok(n)
        }
        fn write(&mut self, p: &str, off: u64, data: &[u8]) -> Result<(), VfsError> {
            let d = self.0.get_mut(p).ok_or(VfsError::Fs(FsError::NotFound))?;
            let end = off as usize + data.len();
            if d.len() < end {
                d.resize(end, 0);
            }
            d[off as usize..end].copy_from_slice(data);
            Ok(())
        }
        fn create_file(&mut self, p: &str) -> Result<(), VfsError> {
            self.0.insert(p.into(), vec![]);
            Ok(())
        }
        fn rename(&mut self, a: &str, b: &str) -> Result<(), VfsError> {
            let d = self.0.remove(a).ok_or(VfsError::Fs(FsError::NotFound))?;
            self.0.insert(b.into(), d);
            Ok(())
        }
    }

    #[test]
    fn registry_persists_through_vfs_with_atomic_replace() {
        let mut vfs = MountManager::new();
        let id = VolumeId::new([7; 16]);
        vfs.register_volume(id, Ram::default()).unwrap();
        vfs.mount(id, "/System", &[], false).unwrap();
        {
            let mut r = Registry::open(VfsStorage::new(&mut vfs, "/System"), STANDARD_HIVES).unwrap();
            r.set_string("HKLM\\SOFTWARE\\LoOS", "Version", "0.1").unwrap();
            r.flush().unwrap();
            // grow then shrink: shrinking must not leave a stale tail
            for i in 0..200 {
                r.set_dword("HKLM\\SOFTWARE\\Big", &format!("v{i}"), i).unwrap();
            }
            r.flush().unwrap();
            r.delete_key("HKLM\\SOFTWARE\\Big").unwrap();
            r.flush().unwrap();
        }
        assert!(vfs.stat("/System/config/SOFTWARE.new").is_err(), "tmp renamed away");
        let r = Registry::open(VfsStorage::new(&mut vfs, "/System"), STANDARD_HIVES).unwrap();
        assert_eq!(r.get_string("HKLM\\SOFTWARE\\LoOS", "Version").unwrap(), "0.1");
        assert!(!r.key_exists("HKLM\\SOFTWARE\\Big"));
    }
}
