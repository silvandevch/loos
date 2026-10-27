//! Extended attributes, POSIX ACLs and file capabilities for btrfs.
//!
//! * on-disk `XATTR_ITEM` codec (collision-aware, same record layout as `DIR_ITEM`),
//! * namespace / size rules,
//! * `system.posix_acl_{access,default}` binary format, validation, mode equivalence, `chmod`
//!   update, default-ACL inheritance at create time and the POSIX permission check,
//! * `security.capability` (`vfs_cap_data` v1/v2/v3).
use crate::consts::*;
use crate::error::{Error, Result};
use crate::fs::{name_hash, parse_dir_items};
use crate::volume::{le16, le32, put16, put32, Key};
use alloc::vec::Vec;

/// Longest xattr name.
pub const XATTR_NAME_MAX: usize = 255;
/// VFS-level value size limit.
pub const XATTR_SIZE_MAX: usize = 65536;
/// `system.posix_acl_access`.
pub const ACL_ACCESS: &[u8] = b"system.posix_acl_access";
/// `system.posix_acl_default`.
pub const ACL_DEFAULT: &[u8] = b"system.posix_acl_default";
/// `security.capability`.
pub const CAPABILITY: &[u8] = b"security.capability";

/// Space limit of one xattr value in a leaf (`BTRFS_MAX_XATTR_SIZE`).
pub fn max_xattr_value(nodesize: u32, name_len: usize) -> usize {
    (nodesize as usize).saturating_sub(HEADER_SIZE + ITEM_SIZE + 30 + name_len)
}

/// Key of the `XATTR_ITEM` for `name` of inode `ino`.
pub fn xattr_key(ino: u64, name: &[u8]) -> Key {
    Key::new(ino, XATTR_ITEM_KEY, name_hash(name))
}

/// Checks the namespace prefix and length rules the kernel applies.
pub fn validate_name(name: &[u8]) -> Result<()> {
    if name.is_empty() || name.len() > XATTR_NAME_MAX {
        return Err(Error::Invalid("xattr name length"));
    }
    const PREFIXES: [&[u8]; 5] = [b"user.", b"trusted.", b"security.", b"system.", b"btrfs."];
    match PREFIXES.iter().find(|p| name.starts_with(p)) {
        Some(p) if name.len() > p.len() => Ok(()),
        _ => Err(Error::Invalid("xattr namespace")),
    }
}

/// Serializes one dir-item-style xattr record.
pub fn build_xattr_item(name: &[u8], value: &[u8]) -> Vec<u8> {
    let mut v = alloc::vec![0u8; 30 + name.len() + value.len()];
    // location key is all zero for xattrs
    put16(&mut v, 25, value.len() as u16);
    put16(&mut v, 27, name.len() as u16);
    v[29] = FT_XATTR;
    v[30..30 + name.len()].copy_from_slice(name);
    v[30 + name.len()..].copy_from_slice(value);
    v
}

/// Adds or replaces `name` inside the (possibly colliding) item payload; returns the new payload.
pub fn xattr_item_set(existing: Option<&[u8]>, name: &[u8], value: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    if let Some(e) = existing {
        for (_, _, n, v) in parse_dir_items(e)? {
            if n != name {
                out.extend_from_slice(&build_xattr_item(&n, &v));
            }
        }
    }
    out.extend_from_slice(&build_xattr_item(name, value));
    Ok(out)
}

/// Removes `name`; `Ok(None)` means the item is now empty and must be deleted.
pub fn xattr_item_remove(existing: &[u8], name: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut out = Vec::new();
    let mut found = false;
    for (_, _, n, v) in parse_dir_items(existing)? {
        if n == name {
            found = true;
        } else {
            out.extend_from_slice(&build_xattr_item(&n, &v));
        }
    }
    if !found {
        return Err(Error::NotFound);
    }
    Ok(if out.is_empty() { None } else { Some(out) })
}

/// `listxattr` buffer: names separated by NUL.
pub fn list_buffer(names: &[Vec<u8>]) -> Vec<u8> {
    let mut b = Vec::new();
    for n in names {
        b.extend_from_slice(n);
        b.push(0);
    }
    b
}

// ---------------------------------------------------------------------------------------------
// POSIX ACLs

/// ACL tag values.
pub mod tag {
    /// Owner.
    pub const USER_OBJ: u16 = 0x01;
    /// Named user.
    pub const USER: u16 = 0x02;
    /// Owning group.
    pub const GROUP_OBJ: u16 = 0x04;
    /// Named group.
    pub const GROUP: u16 = 0x08;
    /// Mask.
    pub const MASK: u16 = 0x10;
    /// Others.
    pub const OTHER: u16 = 0x20;
}
const ACL_VERSION: u32 = 2;
const UNDEFINED_ID: u32 = 0xffff_ffff;

/// One ACL entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AclEntry {
    /// `tag::*`.
    pub tag: u16,
    /// rwx bits (4 = r, 2 = w, 1 = x).
    pub perm: u16,
    /// Uid/gid for named entries, `0xFFFFFFFF` otherwise.
    pub id: u32,
}

/// A POSIX ACL.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Acl {
    /// Entries in canonical order.
    pub entries: Vec<AclEntry>,
}

impl Acl {
    /// Decodes the `system.posix_acl_*` xattr value.
    pub fn parse(b: &[u8]) -> Result<Acl> {
        if b.len() < 4 || (b.len() - 4) % 8 != 0 || le32(b, 0) != ACL_VERSION {
            return Err(Error::Invalid("posix acl header"));
        }
        let n = (b.len() - 4) / 8;
        let mut entries = Vec::with_capacity(n);
        for i in 0..n {
            let o = 4 + i * 8;
            entries.push(AclEntry { tag: le16(b, o), perm: le16(b, o + 2), id: le32(b, o + 4) });
        }
        let acl = Acl { entries };
        acl.validate()?;
        Ok(acl)
    }

    /// Encodes to the xattr value.
    pub fn encode(&self) -> Vec<u8> {
        let mut v = alloc::vec![0u8; 4 + self.entries.len() * 8];
        put32(&mut v, 0, ACL_VERSION);
        for (i, e) in self.entries.iter().enumerate() {
            let o = 4 + i * 8;
            put16(&mut v, o, e.tag);
            put16(&mut v, o + 2, e.perm);
            put32(&mut v, o + 4, e.id);
        }
        v
    }

    /// Mirrors `posix_acl_valid`: ordering, uniqueness, mask requirement, permission bits.
    pub fn validate(&self) -> Result<()> {
        let bad = Error::Invalid("posix acl");
        let mut state = 0u8; // 0: want USER_OBJ, 1: USER*, 2: GROUP_OBJ, 3: GROUP*, 4: MASK, 5: OTHER, 6: done
        let mut last_id: Option<u32> = None;
        let mut named = false;
        let mut have_mask = false;
        for e in &self.entries {
            if e.perm & !7 != 0 {
                return Err(bad);
            }
            match e.tag {
                tag::USER_OBJ => {
                    if state != 0 {
                        return Err(bad);
                    }
                    state = 1;
                }
                tag::USER => {
                    if state != 1 || e.id == UNDEFINED_ID || last_id.map_or(false, |l| e.id <= l) {
                        return Err(bad);
                    }
                    last_id = Some(e.id);
                    named = true;
                }
                tag::GROUP_OBJ => {
                    if state > 1 {
                        return Err(bad);
                    }
                    state = 3;
                    last_id = None;
                }
                tag::GROUP => {
                    if state != 3 || e.id == UNDEFINED_ID || last_id.map_or(false, |l| e.id <= l) {
                        return Err(bad);
                    }
                    last_id = Some(e.id);
                    named = true;
                }
                tag::MASK => {
                    if state != 3 || have_mask {
                        return Err(bad);
                    }
                    have_mask = true;
                    state = 4;
                }
                tag::OTHER => {
                    if state < 3 || state >= 5 {
                        return Err(bad);
                    }
                    state = 5;
                }
                _ => return Err(bad),
            }
        }
        if state != 5 || (named && !have_mask) {
            return Err(bad);
        }
        Ok(())
    }

    /// The three-entry ACL equivalent to a plain mode.
    pub fn from_mode(mode: u32) -> Acl {
        Acl {
            entries: alloc::vec![
                AclEntry { tag: tag::USER_OBJ, perm: ((mode >> 6) & 7) as u16, id: UNDEFINED_ID },
                AclEntry { tag: tag::GROUP_OBJ, perm: ((mode >> 3) & 7) as u16, id: UNDEFINED_ID },
                AclEntry { tag: tag::OTHER, perm: (mode & 7) as u16, id: UNDEFINED_ID },
            ],
        }
    }

    /// If the ACL carries nothing beyond the mode bits, returns those bits (`posix_acl_equiv_mode`).
    pub fn equiv_mode(&self) -> Option<u32> {
        let mut mode = 0u32;
        for e in &self.entries {
            match e.tag {
                tag::USER_OBJ => mode |= (e.perm as u32) << 6,
                tag::GROUP_OBJ => mode |= (e.perm as u32) << 3,
                tag::OTHER => mode |= e.perm as u32,
                tag::MASK => mode = (mode & !0o070) | ((e.perm as u32) << 3),
                _ => return None,
            }
        }
        Some(mode)
    }

    /// Mode bits reflecting this ACL (group bits come from the mask when present).
    pub fn to_mode(&self) -> u32 {
        let mut mode = 0u32;
        let mut mask = None;
        let mut group_obj = 0u32;
        for e in &self.entries {
            match e.tag {
                tag::USER_OBJ => mode |= (e.perm as u32) << 6,
                tag::GROUP_OBJ => group_obj = e.perm as u32,
                tag::OTHER => mode |= e.perm as u32,
                tag::MASK => mask = Some(e.perm as u32),
                _ => {}
            }
        }
        mode | (mask.unwrap_or(group_obj) << 3)
    }

    /// `chmod` on a file with this ACL: owner/other bits replace USER_OBJ/OTHER, the group class
    /// bits go to MASK (or GROUP_OBJ when there is no mask).
    pub fn chmod(&mut self, mode: u32) {
        let has_mask = self.entries.iter().any(|e| e.tag == tag::MASK);
        for e in &mut self.entries {
            match e.tag {
                tag::USER_OBJ => e.perm = ((mode >> 6) & 7) as u16,
                tag::OTHER => e.perm = (mode & 7) as u16,
                tag::MASK => e.perm = ((mode >> 3) & 7) as u16,
                tag::GROUP_OBJ if !has_mask => e.perm = ((mode >> 3) & 7) as u16,
                _ => {}
            }
        }
    }

    /// Applies the requested creation mode to an inherited default ACL
    /// (`__posix_acl_create`). Returns the access ACL to store (`None` = equivalent to the mode)
    /// and the final mode permission bits.
    pub fn inherit_on_create(default: &Acl, requested_mode: u32) -> (Option<Acl>, u32) {
        let mut acl = default.clone();
        let mut mode = requested_mode & 0o7777;
        let has_mask = acl.entries.iter().any(|e| e.tag == tag::MASK);
        let mut group_class = 7u16;
        for e in &mut acl.entries {
            match e.tag {
                tag::USER_OBJ => {
                    e.perm &= ((requested_mode >> 6) & 7) as u16;
                }
                tag::OTHER => {
                    e.perm &= (requested_mode & 7) as u16;
                }
                tag::MASK => {
                    e.perm &= ((requested_mode >> 3) & 7) as u16;
                    group_class = e.perm;
                }
                tag::GROUP_OBJ if !has_mask => {
                    e.perm &= ((requested_mode >> 3) & 7) as u16;
                    group_class = e.perm;
                }
                _ => {}
            }
        }
        let owner = acl.entries.iter().find(|e| e.tag == tag::USER_OBJ).map_or(0, |e| e.perm);
        let other = acl.entries.iter().find(|e| e.tag == tag::OTHER).map_or(0, |e| e.perm);
        mode = (mode & !0o777) | ((owner as u32) << 6) | ((group_class as u32) << 3) | other as u32;
        match acl.equiv_mode() {
            Some(_) => (None, mode),
            None => (Some(acl), mode),
        }
    }

    /// `posix_acl_permission`: may `uid`/`gids` do `want` (rwx bits) on an object owned by
    /// `file_uid`/`file_gid` governed by this ACL?
    pub fn permits(&self, file_uid: u32, file_gid: u32, uid: u32, gids: &[u32], want: u16) -> bool {
        let want = want & 7;
        let in_group = |g: u32| gids.contains(&g);
        let mask = self.entries.iter().find(|e| e.tag == tag::MASK).map_or(7, |e| e.perm);
        let mut group_found = false;
        for e in &self.entries {
            match e.tag {
                tag::USER_OBJ => {
                    if uid == file_uid {
                        return e.perm & want == want;
                    }
                }
                tag::USER => {
                    if e.id == uid {
                        return e.perm & mask & want == want;
                    }
                }
                tag::GROUP_OBJ => {
                    if in_group(file_gid) {
                        group_found = true;
                        if e.perm & mask & want == want {
                            return true;
                        }
                    }
                }
                tag::GROUP => {
                    if in_group(e.id) {
                        group_found = true;
                        if e.perm & mask & want == want {
                            return true;
                        }
                    }
                }
                tag::MASK => {}
                tag::OTHER => {
                    if group_found {
                        return false;
                    }
                    return e.perm & want == want;
                }
                _ => return false,
            }
        }
        false
    }
}

// ---------------------------------------------------------------------------------------------
// File capabilities

/// Linux capability names by bit number.
pub const CAP_NAMES: [&str; 41] = [
    "chown", "dac_override", "dac_read_search", "fowner", "fsetid", "kill", "setgid", "setuid", "setpcap",
    "linux_immutable", "net_bind_service", "net_broadcast", "net_admin", "net_raw", "ipc_lock", "ipc_owner",
    "sys_module", "sys_rawio", "sys_chroot", "sys_ptrace", "sys_pacct", "sys_admin", "sys_boot", "sys_nice",
    "sys_resource", "sys_time", "sys_tty_config", "mknod", "lease", "audit_write", "audit_control", "setfcap",
    "mac_override", "mac_admin", "syslog", "wake_alarm", "block_suspend", "audit_read", "perfmon", "bpf",
    "checkpoint_restore",
];

/// Decoded `security.capability`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileCaps {
    /// Format revision (1, 2 or 3).
    pub revision: u8,
    /// Effective bit set on exec.
    pub effective: bool,
    /// Permitted set.
    pub permitted: u64,
    /// Inheritable set.
    pub inheritable: u64,
    /// Root uid of the user namespace (v3 only).
    pub rootid: u32,
}

impl FileCaps {
    /// Parses the xattr value.
    pub fn parse(b: &[u8]) -> Result<FileCaps> {
        if b.len() < 4 {
            return Err(Error::Invalid("capability xattr size"));
        }
        let magic = le32(b, 0);
        let revision = (magic >> 24) as u8;
        let effective = magic & 1 != 0;
        let (words, expect) = match revision {
            1 => (1usize, 12usize),
            2 => (2, 20),
            3 => (2, 24),
            _ => return Err(Error::Invalid("capability revision")),
        };
        if b.len() != expect {
            return Err(Error::Invalid("capability xattr size"));
        }
        let mut permitted = 0u64;
        let mut inheritable = 0u64;
        for w in 0..words {
            permitted |= (le32(b, 4 + w * 8) as u64) << (32 * w);
            inheritable |= (le32(b, 8 + w * 8) as u64) << (32 * w);
        }
        let rootid = if revision == 3 { le32(b, 20) } else { 0 };
        Ok(FileCaps { revision, effective, permitted, inheritable, rootid })
    }

    /// Encodes back to the xattr value.
    pub fn encode(&self) -> Vec<u8> {
        let (words, len) = match self.revision {
            1 => (1usize, 12usize),
            2 => (2, 20),
            _ => (2, 24),
        };
        let mut v = alloc::vec![0u8; len];
        put32(&mut v, 0, (self.revision as u32) << 24 | self.effective as u32);
        for w in 0..words {
            put32(&mut v, 4 + w * 8, (self.permitted >> (32 * w)) as u32);
            put32(&mut v, 8 + w * 8, (self.inheritable >> (32 * w)) as u32);
        }
        if self.revision == 3 {
            put32(&mut v, 20, self.rootid);
        }
        v
    }

    /// Names of the capabilities set in a bit mask.
    pub fn names(mask: u64) -> Vec<&'static str> {
        CAP_NAMES.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, n)| *n).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acl(e: &[(u16, u16, u32)]) -> Acl {
        Acl { entries: e.iter().map(|&(tag, perm, id)| AclEntry { tag, perm, id }).collect() }
    }

    fn rich() -> Acl {
        acl(&[
            (tag::USER_OBJ, 6, UNDEFINED_ID),
            (tag::USER, 4, 1000),
            (tag::USER, 7, 2000),
            (tag::GROUP_OBJ, 5, UNDEFINED_ID),
            (tag::GROUP, 2, 50),
            (tag::MASK, 6, UNDEFINED_ID),
            (tag::OTHER, 4, UNDEFINED_ID),
        ])
    }

    #[test]
    fn name_rules() {
        assert!(validate_name(b"user.comment").is_ok());
        assert!(validate_name(b"security.capability").is_ok());
        assert!(validate_name(b"btrfs.compression").is_ok());
        assert!(validate_name(b"user.").is_err());
        assert!(validate_name(b"nonamespace").is_err());
        assert!(validate_name(&[b'u'; 300]).is_err());
        assert_eq!(max_xattr_value(16384, 12), 16384 - 101 - 25 - 30 - 12);
    }

    #[test]
    fn item_codec_handles_hash_collisions() {
        let a = build_xattr_item(b"user.a", b"1");
        assert_eq!(parse_dir_items(&a).unwrap()[0].2, b"user.a");
        let both = xattr_item_set(Some(&a), b"user.b", b"22").unwrap();
        let items = parse_dir_items(&both).unwrap();
        assert_eq!(items.len(), 2);
        let replaced = xattr_item_set(Some(&both), b"user.a", b"NEW").unwrap();
        let items = parse_dir_items(&replaced).unwrap();
        assert_eq!(items.iter().find(|i| i.2 == b"user.a").unwrap().3, b"NEW");
        assert_eq!(items.len(), 2);
        let removed = xattr_item_remove(&replaced, b"user.a").unwrap().unwrap();
        assert_eq!(parse_dir_items(&removed).unwrap().len(), 1);
        assert_eq!(xattr_item_remove(&removed, b"user.b").unwrap(), None);
        assert_eq!(xattr_item_remove(&removed, b"user.zzz"), Err(Error::NotFound));
        assert_eq!(xattr_key(257, b"user.a").offset, name_hash(b"user.a"));
        assert_eq!(list_buffer(&[b"a".to_vec(), b"bc".to_vec()]), b"a\0bc\0");
    }

    #[test]
    fn acl_roundtrip_and_validation() {
        let a = rich();
        assert_eq!(Acl::parse(&a.encode()).unwrap(), a);
        // wrong version / length / ordering / missing mask / duplicate id / bad perm
        let mut e = a.encode();
        e[0] = 1;
        assert!(Acl::parse(&e).is_err());
        assert!(Acl::parse(&a.encode()[..10]).is_err());
        assert!(acl(&[(tag::GROUP_OBJ, 4, UNDEFINED_ID), (tag::USER_OBJ, 4, UNDEFINED_ID), (tag::OTHER, 4, UNDEFINED_ID)]).validate().is_err());
        let mut nomask = rich();
        nomask.entries.retain(|e| e.tag != tag::MASK);
        assert!(nomask.validate().is_err());
        let mut dup = rich();
        dup.entries[2].id = 1000;
        assert!(dup.validate().is_err());
        let mut badperm = rich();
        badperm.entries[0].perm = 8;
        assert!(badperm.validate().is_err());
        assert!(Acl::from_mode(0o640).validate().is_ok());
    }

    #[test]
    fn mode_equivalence_and_chmod() {
        let m = Acl::from_mode(0o751);
        assert_eq!(m.equiv_mode(), Some(0o751));
        assert_eq!(rich().equiv_mode(), None);
        // group bits of a rich ACL come from the mask
        assert_eq!(rich().to_mode(), 0o664);
        let mut a = rich();
        a.chmod(0o700);
        assert_eq!(a.entries.iter().find(|e| e.tag == tag::MASK).unwrap().perm, 0);
        assert_eq!(a.entries.iter().find(|e| e.tag == tag::GROUP_OBJ).unwrap().perm, 5, "GROUP_OBJ untouched when a mask exists");
        assert_eq!(a.to_mode(), 0o700);
        let mut plain = Acl::from_mode(0o644);
        plain.chmod(0o600);
        assert_eq!(plain.equiv_mode(), Some(0o600));
    }

    #[test]
    fn default_acl_inheritance_masks_with_requested_mode() {
        let (acl, mode) = Acl::inherit_on_create(&rich(), 0o640);
        let acl = acl.expect("named entries keep the ACL");
        assert_eq!(mode & 0o777, 0o640 & 0o700 | (4 << 3)); // mask 6 & requested group 4 = 4
        assert_eq!(acl.entries.iter().find(|e| e.tag == tag::USER_OBJ).unwrap().perm, 6 & 6);
        assert_eq!(acl.entries.iter().find(|e| e.tag == tag::OTHER).unwrap().perm, 4 & 0);
        // a default ACL that adds nothing collapses to a plain mode
        let (none, m) = Acl::inherit_on_create(&Acl::from_mode(0o775), 0o664);
        assert!(none.is_none());
        assert_eq!(m & 0o777, 0o664);
    }

    #[test]
    fn permission_check_follows_posix_algorithm() {
        let a = rich();
        let (fu, fg) = (500, 500);
        assert!(a.permits(fu, fg, 500, &[], 6), "owner rw");
        assert!(!a.permits(fu, fg, 500, &[], 1), "owner x denied");
        assert!(a.permits(fu, fg, 1000, &[], 4), "named user r");
        assert!(!a.permits(fu, fg, 1000, &[], 2), "named user w denied");
        assert!(a.permits(fu, fg, 2000, &[], 6), "mask 6 trims rwx to rw");
        assert!(!a.permits(fu, fg, 2000, &[], 1), "mask removes x for named user");
        assert!(a.permits(fu, fg, 7, &[500], 4), "owning group r");
        assert!(!a.permits(fu, fg, 7, &[500], 2), "owning group w denied: no group entry grants it");
        assert!(a.permits(fu, fg, 7, &[50], 2), "named group w");
        assert!(!a.permits(fu, fg, 7, &[50], 4), "group matched but lacks r: other is not consulted");
        assert!(a.permits(fu, fg, 9, &[], 4), "other r");
        assert!(!a.permits(fu, fg, 9, &[], 2), "other w denied");
    }

    #[test]
    fn capabilities_roundtrip() {
        let c = FileCaps { revision: 2, effective: true, permitted: (1 << 13) | (1 << 10) | (1 << 40), inheritable: 1 << 21, rootid: 0 };
        let b = c.encode();
        assert_eq!(b.len(), 20);
        assert_eq!(FileCaps::parse(&b).unwrap(), c);
        assert_eq!(FileCaps::names(c.permitted), alloc::vec!["net_bind_service", "net_raw", "checkpoint_restore"]);
        let v3 = FileCaps { revision: 3, effective: false, permitted: 1, inheritable: 0, rootid: 1000 };
        assert_eq!(FileCaps::parse(&v3.encode()).unwrap(), v3);
        let v1 = FileCaps { revision: 1, effective: true, permitted: 0x20, inheritable: 0, rootid: 0 };
        assert_eq!(v1.encode().len(), 12);
        assert_eq!(FileCaps::parse(&v1.encode()).unwrap(), v1);
        assert!(FileCaps::parse(&b[..19]).is_err());
        assert!(FileCaps::parse(&[0, 0, 0, 9, 0, 0, 0, 0]).is_err());
    }
}
