//! Local accounts: the SAM (users, local groups, password policy) and the LSA (privileges, logon rights, secrets).
//!
//! Identity model follows Windows: a machine SID `S-1-5-21-a-b-c`, users and local groups numbered from RID 1000,
//! built-in aliases under `S-1-5-32`, and well-known SIDs for Everyone, SYSTEM and friends. Passwords are PBKDF2
//! hashes (see [`crate::logon::Account`]); LSA secrets are sealed with AES-GCM under a key derived from the boot key.
//! The whole database serialises to one blob ([`Sam::to_bytes`]) for storage in the SAM registry hive.

use crate::aes::Gcm;
use crate::hash::{hkdf_expand, hkdf_extract};
use crate::logon::{Account, LogonUi, ITERATIONS};
use crate::uac::{privs, Token, SID_ADMINS};
use crate::{Error, Result};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Built-in Users alias.
pub const SID_USERS: &str = "S-1-5-32-545";
/// Built-in Guests alias.
pub const SID_GUESTS: &str = "S-1-5-32-546";
/// Built-in Power Users alias.
pub const SID_POWER_USERS: &str = "S-1-5-32-547";
/// Built-in Backup Operators alias.
pub const SID_BACKUP_OPS: &str = "S-1-5-32-551";
/// Everyone.
pub const SID_EVERYONE: &str = "S-1-1-0";
/// Authenticated Users.
pub const SID_AUTH_USERS: &str = "S-1-5-11";
/// Local SYSTEM.
pub const SID_SYSTEM: &str = "S-1-5-18";
/// RID of the built-in Administrator.
pub const RID_ADMIN: u32 = 500;
/// RID of the built-in Guest.
pub const RID_GUEST: u32 = 501;
const FIRST_RID: u32 = 1000;
const MAGIC: &[u8; 6] = b"LSAM1\0";

/// `SID_NAME_USE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidType {
    /// A user account.
    User = 1,
    /// A global group.
    Group = 2,
    /// A domain.
    Domain = 3,
    /// A local group (alias).
    Alias = 4,
    /// A well-known group.
    WellKnown = 5,
    /// Not resolvable.
    Unknown = 8,
}

/// Account name rules: 1-20 characters, none of `\ / [ ] : | < > + = ; , ? * @ "`, no control characters.
fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.chars().count() <= 20 && !n.trim().is_empty() && !n.chars().any(|c| c.is_control() || "\\/[]:|<>+=;,?*@\"".contains(c))
}

/// Bounds-checked reader over serialised data.
pub(crate) struct Cur<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Cur<'a> {
    pub(crate) fn new(b: &'a [u8]) -> Self {
        Cur { b, at: 0 }
    }
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).ok_or(Error::Parse)?;
        let s = self.b.get(self.at..end).ok_or(Error::Parse)?;
        self.at = end;
        Ok(s)
    }
    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub(crate) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub(crate) fn bytes(&mut self) -> Result<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    pub(crate) fn string(&mut self) -> Result<String> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| Error::Parse)
    }
    fn done(&self) -> bool {
        self.at == self.b.len()
    }
}

fn put(o: &mut Vec<u8>, b: &[u8]) {
    o.extend_from_slice(&(b.len() as u32).to_le_bytes());
    o.extend_from_slice(b);
}

/// Machine-wide password policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Minimum password length in bytes (0 allows blank passwords).
    pub min_len: usize,
    /// Require at least three of: upper, lower, digit, other.
    pub complexity: bool,
    /// Maximum password age in seconds (0 = never expires).
    pub max_age: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { min_len: 8, complexity: false, max_age: 0 }
    }
}

impl Policy {
    fn check(&self, pw: &[u8], user: &str) -> Result<()> {
        if pw.len() < self.min_len {
            return Err(Error::Invalid);
        }
        if self.complexity {
            let s = core::str::from_utf8(pw).map_err(|_| Error::Invalid)?;
            let classes = [s.chars().any(char::is_uppercase), s.chars().any(char::is_lowercase), s.chars().any(|c| c.is_ascii_digit()), s.chars().any(|c| !c.is_alphanumeric())];
            let ul = s.to_lowercase();
            if classes.iter().filter(|c| **c).count() < 3 || (user.len() >= 3 && ul.contains(&user.to_lowercase())) {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
}

/// A local user.
#[derive(Clone)]
pub struct User {
    /// Relative id.
    pub rid: u32,
    /// Logon credentials and state.
    pub account: Account,
    /// Display name.
    pub full_name: String,
    /// Description.
    pub comment: String,
    /// Exempt from [`Policy::max_age`].
    pub never_expires: bool,
    /// Must set a new password at next logon.
    pub must_change: bool,
    /// Creation time (seconds).
    pub created: u64,
    /// Time the password was last set.
    pub pwd_set: u64,
}

/// A local group (alias).
#[derive(Clone)]
pub struct Alias {
    /// Relative id (built-ins use their well-known SID in `sid`).
    pub rid: u32,
    /// Full SID.
    pub sid: String,
    /// Name.
    pub name: String,
    /// Description.
    pub comment: String,
    /// Member SIDs.
    pub members: Vec<String>,
    builtin: bool,
}

/// The security accounts manager.
#[derive(Clone)]
pub struct Sam {
    machine: String,
    domain_sid: String,
    users: Vec<User>,
    aliases: Vec<Alias>,
    next_rid: u32,
    iterations: u32,
    /// Password policy.
    pub policy: Policy,
}

impl Sam {
    /// Creates the database: disabled Administrator and Guest, and the built-in aliases. `domain_sid` is the machine SID
    /// (`S-1-5-21-a-b-c`).
    pub fn new(machine: &str, domain_sid: &str, iterations: u32) -> Sam {
        let mut s = Sam { machine: machine.into(), domain_sid: domain_sid.into(), users: Vec::new(), aliases: Vec::new(), next_rid: FIRST_RID, iterations, policy: Policy::default() };
        for (sid, name, comment) in [
            (SID_ADMINS, "Administrators", "Full control of the computer"),
            (SID_USERS, "Users", "Ordinary users"),
            (SID_GUESTS, "Guests", "Limited access"),
            (SID_POWER_USERS, "Power Users", "Legacy elevated users"),
            (SID_BACKUP_OPS, "Backup Operators", "May back up and restore files"),
        ] {
            let rid = sid.rsplit('-').next().and_then(|r| r.parse().ok()).unwrap_or(0);
            s.aliases.push(Alias { rid, sid: sid.into(), name: name.into(), comment: comment.into(), members: Vec::new(), builtin: true });
        }
        for (rid, name, comment, alias) in [(RID_ADMIN, "Administrator", "Built-in administrator", SID_ADMINS), (RID_GUEST, "Guest", "Built-in guest", SID_GUESTS)] {
            let sid = alloc::format!("{}-{rid}", s.domain_sid);
            let mut account = Account::new(name, &sid, &[], b"", [0; 16], 1);
            account.disabled = true;
            s.users.push(User { rid, account, full_name: String::new(), comment: comment.into(), never_expires: true, must_change: false, created: 0, pwd_set: 0 });
            s.alias_mut(alias).unwrap().members.push(sid);
        }
        s.sync_groups();
        s
    }

    /// Machine SID prefix.
    pub fn domain_sid(&self) -> &str {
        &self.domain_sid
    }

    fn name_taken(&self, n: &str) -> bool {
        self.users.iter().any(|u| u.account.name.eq_ignore_ascii_case(n)) || self.aliases.iter().any(|a| a.name.eq_ignore_ascii_case(n))
    }

    fn alias_mut(&mut self, sid: &str) -> Option<&mut Alias> {
        self.aliases.iter_mut().find(|a| a.sid == sid)
    }

    /// Recomputes every account's group list (used in logon tokens).
    fn sync_groups(&mut self) {
        let groups: Vec<(u32, Vec<String>)> = self.users.iter().map(|u| (u.rid, self.groups_of(u.rid))).collect();
        for (rid, g) in groups {
            if let Some(u) = self.users.iter_mut().find(|u| u.rid == rid) {
                u.account.groups = g;
            }
        }
    }

    /// Creates a user (member of Users). `salt` must come from a CSPRNG.
    pub fn create_user(&mut self, name: &str, password: &[u8], salt: [u8; 16], now: u64) -> Result<u32> {
        if !valid_name(name) {
            return Err(Error::Invalid);
        }
        if self.name_taken(name) {
            return Err(Error::Exists);
        }
        self.policy.check(password, name)?;
        let rid = self.next_rid;
        self.next_rid += 1;
        let sid = alloc::format!("{}-{rid}", self.domain_sid);
        let account = Account::new(name, &sid, &[], password, salt, self.iterations);
        self.users.push(User { rid, account, full_name: String::new(), comment: String::new(), never_expires: false, must_change: false, created: now, pwd_set: now });
        self.alias_mut(SID_USERS).unwrap().members.push(sid);
        self.sync_groups();
        Ok(rid)
    }

    /// User by RID.
    pub fn user(&self, rid: u32) -> Option<&User> {
        self.users.iter().find(|u| u.rid == rid)
    }

    /// Mutable user by RID.
    pub fn user_mut(&mut self, rid: u32) -> Option<&mut User> {
        self.users.iter_mut().find(|u| u.rid == rid)
    }

    /// User by name (case-insensitive).
    pub fn user_by_name(&self, name: &str) -> Option<&User> {
        self.users.iter().find(|u| u.account.name.eq_ignore_ascii_case(name))
    }

    /// All users.
    pub fn users(&self) -> &[User] {
        &self.users
    }

    /// All local groups.
    pub fn aliases(&self) -> &[Alias] {
        &self.aliases
    }

    fn enabled_admins(&self, excluding: Option<u32>) -> usize {
        let admins = self.aliases.iter().find(|a| a.sid == SID_ADMINS).map(|a| a.members.clone()).unwrap_or_default();
        self.users.iter().filter(|u| Some(u.rid) != excluding && !u.account.disabled && admins.contains(&u.account.sid)).count()
    }

    /// Deletes a user. The built-in accounts and the last enabled administrator are protected.
    pub fn delete_user(&mut self, rid: u32) -> Result<()> {
        if rid == RID_ADMIN || rid == RID_GUEST {
            return Err(Error::Denied);
        }
        let u = self.user(rid).ok_or(Error::NotFound)?;
        let sid = u.account.sid.clone();
        if self.is_admin_sid(&sid) && !u.account.disabled && self.enabled_admins(Some(rid)) == 0 {
            return Err(Error::Denied);
        }
        self.users.retain(|u| u.rid != rid);
        for a in &mut self.aliases {
            a.members.retain(|m| *m != sid);
        }
        Ok(())
    }

    fn is_admin_sid(&self, sid: &str) -> bool {
        self.aliases.iter().any(|a| a.sid == SID_ADMINS && a.members.iter().any(|m| m == sid))
    }

    /// Renames a user (not the built-ins).
    pub fn rename_user(&mut self, rid: u32, new: &str) -> Result<()> {
        if rid == RID_ADMIN || rid == RID_GUEST {
            return Err(Error::Denied);
        }
        if !valid_name(new) {
            return Err(Error::Invalid);
        }
        if self.name_taken(new) && !self.user(rid).is_some_and(|u| u.account.name.eq_ignore_ascii_case(new)) {
            return Err(Error::Exists);
        }
        self.user_mut(rid).ok_or(Error::NotFound)?.account.name = new.into();
        Ok(())
    }

    /// Enables or disables a user; the last enabled administrator cannot be disabled.
    pub fn set_disabled(&mut self, rid: u32, disabled: bool) -> Result<()> {
        let sid = self.user(rid).ok_or(Error::NotFound)?.account.sid.clone();
        if disabled && self.is_admin_sid(&sid) && self.enabled_admins(Some(rid)) == 0 {
            return Err(Error::Denied);
        }
        self.user_mut(rid).unwrap().account.disabled = disabled;
        Ok(())
    }

    /// Administrative password reset (no old password needed).
    pub fn set_password(&mut self, rid: u32, password: &[u8], salt: [u8; 16], now: u64) -> Result<()> {
        let name = self.user(rid).ok_or(Error::NotFound)?.account.name.clone();
        self.policy.check(password, &name)?;
        let it = self.iterations;
        let u = self.user_mut(rid).unwrap();
        u.account.set_password(password, salt, it);
        u.pwd_set = now;
        u.must_change = false;
        Ok(())
    }

    /// User-initiated change: the old password must match.
    pub fn change_password(&mut self, rid: u32, old: &[u8], new: &[u8], salt: [u8; 16], now: u64) -> Result<()> {
        if !self.verify_password(rid, old) {
            return Err(Error::BadCredential);
        }
        self.set_password(rid, new, salt, now)
    }

    /// Checks a password without touching lockout state.
    pub fn verify_password(&self, rid: u32, password: &[u8]) -> bool {
        self.user(rid).is_some_and(|u| u.account.check_password(password, self.iterations))
    }

    /// Password authentication with lockout; returns the user's RID. Unknown names cost the same hashing work.
    pub fn authenticate(&mut self, name: &str, password: &[u8], now: u64) -> Result<u32> {
        let it = self.iterations;
        match self.users.iter_mut().find(|u| u.account.name.eq_ignore_ascii_case(name)) {
            Some(u) => u.account.authenticate(password, it, now).map(|()| u.rid),
            None => {
                let _ = Account::new("", "", &[], password, [0; 16], it);
                Err(Error::BadCredential)
            }
        }
    }

    /// Whether the password has expired under the policy at `now`.
    pub fn password_expired(&self, rid: u32, now: u64) -> bool {
        self.user(rid).is_some_and(|u| u.must_change || (!u.never_expires && self.policy.max_age != 0 && now.saturating_sub(u.pwd_set) > self.policy.max_age))
    }

    /// Creates a local group; returns its RID.
    pub fn create_alias(&mut self, name: &str, comment: &str) -> Result<u32> {
        if !valid_name(name) {
            return Err(Error::Invalid);
        }
        if self.name_taken(name) {
            return Err(Error::Exists);
        }
        let rid = self.next_rid;
        self.next_rid += 1;
        let sid = alloc::format!("{}-{rid}", self.domain_sid);
        self.aliases.push(Alias { rid, sid, name: name.into(), comment: comment.into(), members: Vec::new(), builtin: false });
        Ok(rid)
    }

    /// Deletes a user-created group.
    pub fn delete_alias(&mut self, sid: &str) -> Result<()> {
        let a = self.aliases.iter().find(|a| a.sid == sid).ok_or(Error::NotFound)?;
        if a.builtin {
            return Err(Error::Denied);
        }
        self.aliases.retain(|a| a.sid != sid);
        self.sync_groups();
        Ok(())
    }

    /// Adds a user or group SID to a local group.
    pub fn add_member(&mut self, alias_sid: &str, member_sid: &str) -> Result<()> {
        if self.lookup_sid(member_sid).is_none() {
            return Err(Error::NotFound);
        }
        let a = self.alias_mut(alias_sid).ok_or(Error::NotFound)?;
        if a.members.iter().any(|m| m == member_sid) {
            return Err(Error::Exists);
        }
        a.members.push(member_sid.into());
        self.sync_groups();
        Ok(())
    }

    /// Removes a member; the last enabled administrator cannot leave Administrators.
    pub fn remove_member(&mut self, alias_sid: &str, member_sid: &str) -> Result<()> {
        if alias_sid == SID_ADMINS {
            if let Some(u) = self.users.iter().find(|u| u.account.sid == member_sid) {
                if !u.account.disabled && self.enabled_admins(Some(u.rid)) == 0 {
                    return Err(Error::Denied);
                }
            }
        }
        let a = self.alias_mut(alias_sid).ok_or(Error::NotFound)?;
        let n = a.members.len();
        a.members.retain(|m| m != member_sid);
        if a.members.len() == n {
            return Err(Error::NotFound);
        }
        self.sync_groups();
        Ok(())
    }

    /// Group SIDs for a user's token: Everyone, Authenticated Users, and every local group containing the user
    /// (directly or through nested local groups).
    pub fn groups_of(&self, rid: u32) -> Vec<String> {
        let Some(u) = self.user(rid) else { return Vec::new() };
        let mut set: BTreeSet<String> = BTreeSet::new();
        let mut frontier = alloc::vec![u.account.sid.clone()];
        while let Some(sid) = frontier.pop() {
            for a in &self.aliases {
                if a.members.iter().any(|m| *m == sid) && set.insert(a.sid.clone()) {
                    frontier.push(a.sid.clone());
                }
            }
        }
        let mut v: Vec<String> = alloc::vec![SID_EVERYONE.into(), SID_AUTH_USERS.into()];
        v.extend(set);
        v
    }

    /// `LookupAccountName`: accepts `name`, `MACHINE\name`, `BUILTIN\name` and `NT AUTHORITY\name`.
    pub fn lookup_name(&self, name: &str) -> Option<(String, SidType)> {
        let (dom, n) = match name.split_once('\\') {
            Some((d, n)) => (Some(d), n),
            None => (None, name),
        };
        if let Some(d) = dom {
            if !(d.eq_ignore_ascii_case(&self.machine) || d.eq_ignore_ascii_case("BUILTIN") || d.eq_ignore_ascii_case("NT AUTHORITY") || d.is_empty()) {
                return None;
            }
        }
        if let Some((sid, _)) = WELL_KNOWN.iter().find(|(_, w)| w.eq_ignore_ascii_case(n)) {
            return Some(((*sid).into(), SidType::WellKnown));
        }
        if let Some(u) = self.user_by_name(n).filter(|_| dom.is_none_or(|d| d.eq_ignore_ascii_case(&self.machine))) {
            return Some((u.account.sid.clone(), SidType::User));
        }
        self.aliases.iter().find(|a| a.name.eq_ignore_ascii_case(n)).map(|a| (a.sid.clone(), SidType::Alias))
    }

    /// `LookupAccountSid`: `(name, domain, type)`.
    pub fn lookup_sid(&self, sid: &str) -> Option<(String, String, SidType)> {
        if let Some((_, n)) = WELL_KNOWN.iter().find(|(s, _)| *s == sid) {
            let dom = if sid.starts_with("S-1-5-32") { "BUILTIN" } else { "NT AUTHORITY" };
            return Some(((*n).into(), if sid == SID_EVERYONE { String::new() } else { dom.into() }, SidType::WellKnown));
        }
        if let Some(u) = self.users.iter().find(|u| u.account.sid == sid) {
            return Some((u.account.name.clone(), self.machine.clone(), SidType::User));
        }
        self.aliases.iter().find(|a| a.sid == sid).map(|a| (a.name.clone(), if a.builtin { "BUILTIN".to_string() } else { self.machine.clone() }, SidType::Alias))
    }

    /// Hands the accounts to the logon UI (it must be built with the same iteration count).
    pub fn export_to_logon(&self, ui: &mut LogonUi) {
        for u in &self.users {
            ui.add_account(u.account.clone());
        }
    }

    /// Serialises the database.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::from(&MAGIC[..]);
        put(&mut o, self.machine.as_bytes());
        put(&mut o, self.domain_sid.as_bytes());
        o.extend_from_slice(&self.next_rid.to_le_bytes());
        o.extend_from_slice(&self.iterations.to_le_bytes());
        o.extend_from_slice(&(self.policy.min_len as u32).to_le_bytes());
        o.push(self.policy.complexity as u8);
        o.extend_from_slice(&self.policy.max_age.to_le_bytes());
        o.extend_from_slice(&(self.users.len() as u32).to_le_bytes());
        for u in &self.users {
            o.extend_from_slice(&u.rid.to_le_bytes());
            put(&mut o, &u.account.to_record());
            put(&mut o, u.full_name.as_bytes());
            put(&mut o, u.comment.as_bytes());
            o.push(u.never_expires as u8 | (u.must_change as u8) << 1);
            o.extend_from_slice(&u.created.to_le_bytes());
            o.extend_from_slice(&u.pwd_set.to_le_bytes());
        }
        o.extend_from_slice(&(self.aliases.len() as u32).to_le_bytes());
        for a in &self.aliases {
            o.extend_from_slice(&a.rid.to_le_bytes());
            put(&mut o, a.sid.as_bytes());
            put(&mut o, a.name.as_bytes());
            put(&mut o, a.comment.as_bytes());
            o.push(a.builtin as u8);
            o.extend_from_slice(&(a.members.len() as u32).to_le_bytes());
            for m in &a.members {
                put(&mut o, m.as_bytes());
            }
        }
        o
    }

    /// Inverse of [`Sam::to_bytes`]; rejects truncated or inconsistent data.
    pub fn from_bytes(b: &[u8]) -> Result<Sam> {
        let mut c = Cur::new(b);
        if c.take(MAGIC.len())? != MAGIC {
            return Err(Error::Parse);
        }
        let machine = c.string()?;
        let domain_sid = c.string()?;
        let next_rid = c.u32()?;
        let iterations = c.u32()?;
        let min_len = c.u32()? as usize;
        let complexity = c.take(1)?[0] != 0;
        let max_age = c.u64()?;
        let mut s = Sam { machine, domain_sid, users: Vec::new(), aliases: Vec::new(), next_rid, iterations, policy: Policy { min_len, complexity, max_age } };
        for _ in 0..c.u32()?.min(100_000) {
            let rid = c.u32()?;
            let account = Account::from_record(c.bytes()?)?;
            let (full_name, comment) = (c.string()?, c.string()?);
            let flags = c.take(1)?[0];
            let (created, pwd_set) = (c.u64()?, c.u64()?);
            s.users.push(User { rid, account, full_name, comment, never_expires: flags & 1 != 0, must_change: flags & 2 != 0, created, pwd_set });
        }
        for _ in 0..c.u32()?.min(100_000) {
            let rid = c.u32()?;
            let (sid, name, comment) = (c.string()?, c.string()?, c.string()?);
            let builtin = c.take(1)?[0] != 0;
            let mut members = Vec::new();
            for _ in 0..c.u32()?.min(100_000) {
                members.push(c.string()?);
            }
            s.aliases.push(Alias { rid, sid, name, comment, members, builtin });
        }
        if !c.done() || s.next_rid < FIRST_RID {
            return Err(Error::Parse);
        }
        Ok(s)
    }
}

const WELL_KNOWN: &[(&str, &str)] = &[
    (SID_EVERYONE, "Everyone"),
    (SID_AUTH_USERS, "Authenticated Users"),
    (SID_SYSTEM, "SYSTEM"),
    ("S-1-5-19", "LOCAL SERVICE"),
    ("S-1-5-20", "NETWORK SERVICE"),
    ("S-1-5-4", "INTERACTIVE"),
    ("S-1-5-2", "NETWORK"),
];

/// Privilege names and their token bits.
pub const PRIVILEGES: &[(&str, u32)] = &[
    ("SeChangeNotifyPrivilege", privs::CHANGE_NOTIFY),
    ("SeShutdownPrivilege", privs::SHUTDOWN),
    ("SeUndockPrivilege", privs::UNDOCK),
    ("SeIncreaseWorkingSetPrivilege", privs::WORKING_SET),
    ("SeTimeZonePrivilege", privs::TIME_ZONE),
    ("SeLoadDriverPrivilege", privs::LOAD_DRIVER),
    ("SeTakeOwnershipPrivilege", privs::TAKE_OWNERSHIP),
    ("SeDebugPrivilege", privs::DEBUG),
    ("SeBackupPrivilege", privs::BACKUP),
    ("SeSystemtimePrivilege", privs::SYSTEMTIME),
];
const LOGON_RIGHTS: &[&str] = &["SeInteractiveLogonRight", "SeDenyInteractiveLogonRight", "SeNetworkLogonRight", "SeDenyNetworkLogonRight", "SeServiceLogonRight"];

/// The local security authority: account rights and sealed secrets.
#[derive(Clone)]
pub struct Lsa {
    rights: BTreeMap<String, BTreeSet<String>>,
    secrets: BTreeMap<String, ([u8; 12], Vec<u8>, [u8; 16])>,
    key: [u8; 32],
    counter: u64,
}

impl Lsa {
    /// New authority with the default rights assignment; `boot_key` protects the secrets.
    pub fn new(boot_key: &[u8; 32]) -> Lsa {
        let mut key = [0u8; 32];
        hkdf_expand(&hkdf_extract(b"LoOS LSA secrets", boot_key), b"policy-secret-key", &mut key);
        let mut l = Lsa { rights: BTreeMap::new(), secrets: BTreeMap::new(), key, counter: 0 };
        for (sid, names) in [
            (SID_ADMINS, &PRIVILEGES.iter().map(|p| p.0).collect::<Vec<_>>()[..]),
            (SID_USERS, &["SeChangeNotifyPrivilege", "SeShutdownPrivilege", "SeUndockPrivilege", "SeIncreaseWorkingSetPrivilege", "SeTimeZonePrivilege"][..]),
            (SID_BACKUP_OPS, &["SeBackupPrivilege", "SeShutdownPrivilege"][..]),
            (SID_EVERYONE, &["SeChangeNotifyPrivilege"][..]),
        ] {
            for n in names {
                l.grant(sid, n).ok();
            }
        }
        for sid in [SID_ADMINS, SID_USERS, SID_BACKUP_OPS] {
            l.grant(sid, "SeInteractiveLogonRight").ok();
            l.grant(sid, "SeNetworkLogonRight").ok();
        }
        l.grant(SID_GUESTS, "SeDenyNetworkLogonRight").ok();
        l.grant(SID_SYSTEM, "SeServiceLogonRight").ok();
        l
    }

    fn known(right: &str) -> bool {
        PRIVILEGES.iter().any(|p| p.0 == right) || LOGON_RIGHTS.contains(&right)
    }

    /// `LsaAddAccountRights`.
    pub fn grant(&mut self, sid: &str, right: &str) -> Result<()> {
        if !Self::known(right) {
            return Err(Error::Invalid);
        }
        self.rights.entry(sid.into()).or_default().insert(right.into());
        Ok(())
    }

    /// `LsaRemoveAccountRights`.
    pub fn revoke(&mut self, sid: &str, right: &str) -> Result<()> {
        if self.rights.get_mut(sid).is_some_and(|s| s.remove(right)) { Ok(()) } else { Err(Error::NotFound) }
    }

    /// `LsaEnumerateAccountRights`.
    pub fn rights_of(&self, sid: &str) -> Vec<String> {
        self.rights.get(sid).map(|s| s.iter().cloned().collect()).unwrap_or_default()
    }

    fn effective(&self, sids: &[String]) -> BTreeSet<&str> {
        sids.iter().filter_map(|s| self.rights.get(s)).flat_map(|s| s.iter().map(String::as_str)).collect()
    }

    /// Privilege bitmask held by a principal with these SIDs (user SID first, then groups).
    pub fn privilege_mask(&self, sids: &[String]) -> u32 {
        let r = self.effective(sids);
        PRIVILEGES.iter().filter(|p| r.contains(p.0)).fold(0, |m, p| m | p.1)
    }

    /// Whether the principal may log on interactively: some allow right and no deny right.
    pub fn may_logon_interactive(&self, sids: &[String]) -> bool {
        let r = self.effective(sids);
        r.contains("SeInteractiveLogonRight") && !r.contains("SeDenyInteractiveLogonRight")
    }

    /// Builds the logon token for user `rid`: groups from the SAM, privileges from the LSA. Fails when the account is
    /// disabled, unknown, or denied interactive logon.
    pub fn token_for(&self, sam: &Sam, rid: u32, session: u32) -> Result<Token> {
        let u = sam.user(rid).ok_or(Error::NotFound)?;
        if u.account.disabled {
            return Err(Error::Denied);
        }
        let mut sids = alloc::vec![u.account.sid.clone()];
        sids.extend(sam.groups_of(rid));
        if !self.may_logon_interactive(&sids) {
            return Err(Error::Denied);
        }
        let groups: Vec<&str> = sids[1..].iter().map(String::as_str).collect();
        let mut t = Token::for_user(&u.account.sid, &groups, session);
        t.privileges = self.privilege_mask(&sids);
        Ok(t)
    }

    /// `LsaStorePrivateData`: seals `data` under `name`.
    pub fn store_secret(&mut self, name: &str, data: &[u8]) -> Result<()> {
        if name.is_empty() || name.len() > 128 {
            return Err(Error::Invalid);
        }
        self.counter += 1;
        let mut nonce = [0u8; 12];
        nonce[4..].copy_from_slice(&self.counter.to_be_bytes());
        let mut ct = data.to_vec();
        let tag = Gcm::new(&self.key)?.seal(&nonce, name.as_bytes(), &mut ct);
        self.secrets.insert(name.into(), (nonce, ct, tag));
        Ok(())
    }

    /// `LsaRetrievePrivateData`.
    pub fn read_secret(&self, name: &str) -> Result<Vec<u8>> {
        let (nonce, ct, tag) = self.secrets.get(name).ok_or(Error::NotFound)?;
        let mut pt = ct.clone();
        Gcm::new(&self.key)?.open(nonce, name.as_bytes(), &mut pt, tag)?;
        Ok(pt)
    }

    /// Deletes a secret.
    pub fn delete_secret(&mut self, name: &str) -> Result<()> {
        self.secrets.remove(name).map(|_| ()).ok_or(Error::NotFound)
    }

    /// Serialises rights and sealed secrets (the boot key is not included).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::from(&b"LLSA1\0"[..]);
        o.extend_from_slice(&self.counter.to_le_bytes());
        o.extend_from_slice(&(self.rights.len() as u32).to_le_bytes());
        for (sid, set) in &self.rights {
            put(&mut o, sid.as_bytes());
            o.extend_from_slice(&(set.len() as u32).to_le_bytes());
            for r in set {
                put(&mut o, r.as_bytes());
            }
        }
        o.extend_from_slice(&(self.secrets.len() as u32).to_le_bytes());
        for (n, (nonce, ct, tag)) in &self.secrets {
            put(&mut o, n.as_bytes());
            o.extend_from_slice(nonce);
            put(&mut o, ct);
            o.extend_from_slice(tag);
        }
        o
    }

    /// Inverse of [`Lsa::to_bytes`].
    pub fn from_bytes(boot_key: &[u8; 32], b: &[u8]) -> Result<Lsa> {
        let mut l = Lsa::new(boot_key);
        l.rights.clear();
        let mut c = Cur::new(b);
        if c.take(6)? != b"LLSA1\0" {
            return Err(Error::Parse);
        }
        l.counter = c.u64()?;
        for _ in 0..c.u32()?.min(100_000) {
            let sid = c.string()?;
            let mut set = BTreeSet::new();
            for _ in 0..c.u32()?.min(1000) {
                let r = c.string()?;
                if !Self::known(&r) {
                    return Err(Error::Parse);
                }
                set.insert(r);
            }
            l.rights.insert(sid, set);
        }
        for _ in 0..c.u32()?.min(100_000) {
            let n = c.string()?;
            let nonce: [u8; 12] = c.take(12)?.try_into().unwrap();
            let ct = c.bytes()?.to_vec();
            let tag: [u8; 16] = c.take(16)?.try_into().unwrap();
            l.secrets.insert(n, (nonce, ct, tag));
        }
        if !c.done() {
            return Err(Error::Parse);
        }
        Ok(l)
    }
}

/// Default PBKDF2 cost for a production database.
pub const DEFAULT_ITERATIONS: u32 = ITERATIONS;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logon::{Credentials, PasswordProvider};

    const SID: &str = "S-1-5-21-1-2-3";
    fn sam() -> Sam {
        Sam::new("LOOS", SID, 2)
    }
    fn salt(n: u8) -> [u8; 16] {
        [n; 16]
    }

    #[test]
    fn users_names_and_policy() {
        let mut s = sam();
        let bob = s.create_user("Bob", b"hunter2hunter2", salt(1), 100).unwrap();
        assert_eq!(bob, 1000);
        assert_eq!(s.create_user("bob", b"hunter2hunter2", salt(2), 100), Err(Error::Exists), "case-insensitive");
        assert_eq!(s.create_user("Users", b"hunter2hunter2", salt(2), 100), Err(Error::Exists), "group name clash");
        assert_eq!(s.create_user("a/b", b"hunter2hunter2", salt(2), 100), Err(Error::Invalid));
        assert_eq!(s.create_user(&"x".repeat(21), b"hunter2hunter2", salt(2), 100), Err(Error::Invalid));
        assert_eq!(s.create_user("Short", b"abc", salt(2), 100), Err(Error::Invalid), "min length 8");
        s.policy.complexity = true;
        assert_eq!(s.create_user("Plain", b"alllowercase", salt(2), 100), Err(Error::Invalid));
        assert_eq!(s.create_user("Carol", b"Carol-Is-Cool1", salt(2), 100), Err(Error::Invalid), "contains the user name");
        assert!(s.create_user("Carol", b"Tr0ub4dor&3x", salt(2), 100).is_ok());
        assert_eq!(s.user(bob).unwrap().account.sid, "S-1-5-21-1-2-3-1000");
    }

    #[test]
    fn authenticate_locks_out() {
        let mut s = sam();
        let bob = s.create_user("Bob", b"hunter2hunter2", salt(1), 0).unwrap();
        assert_eq!(s.authenticate("bob", b"hunter2hunter2", 1), Ok(bob));
        assert_eq!(s.authenticate("nobody", b"x", 1), Err(Error::BadCredential));
        for _ in 0..crate::logon::LOCKOUT_THRESHOLD {
            assert_eq!(s.authenticate("Bob", b"wrong", 10), Err(Error::BadCredential));
        }
        assert_eq!(s.authenticate("Bob", b"hunter2hunter2", 11), Err(Error::LockedOut), "even the right password");
        assert_eq!(s.authenticate("Bob", b"hunter2hunter2", 10 + crate::logon::LOCKOUT_BASE), Ok(bob));
        s.set_disabled(bob, true).unwrap();
        assert_eq!(s.authenticate("Bob", b"hunter2hunter2", 1000), Err(Error::BadCredential));
    }

    #[test]
    fn password_change_reset_and_expiry() {
        let mut s = sam();
        let bob = s.create_user("Bob", b"first-password", salt(1), 0).unwrap();
        assert!(s.verify_password(bob, b"first-password"));
        assert!(!s.verify_password(bob, b"nope-nope-nope"));
        assert_eq!(s.change_password(bob, b"wrong-password", b"second-password", salt(2), 10), Err(Error::BadCredential));
        s.change_password(bob, b"first-password", b"second-password", salt(2), 10).unwrap();
        assert!(!s.verify_password(bob, b"first-password") && s.verify_password(bob, b"second-password"));
        assert_eq!(s.change_password(bob, b"second-password", b"x", salt(3), 20), Err(Error::Invalid), "policy applies to changes");
        s.set_password(bob, b"admin-reset-pw", salt(4), 30).unwrap();
        assert!(s.verify_password(bob, b"admin-reset-pw"));
        s.policy.max_age = 100;
        assert!(!s.password_expired(bob, 100));
        assert!(s.password_expired(bob, 131));
        s.user_mut(bob).unwrap().never_expires = true;
        assert!(!s.password_expired(bob, 10_000));
        s.user_mut(bob).unwrap().must_change = true;
        assert!(s.password_expired(bob, 0));
    }

    #[test]
    fn groups_nesting_and_last_admin_protection() {
        let mut s = sam();
        let bob = s.create_user("Bob", b"hunter2hunter2", salt(1), 0).unwrap();
        let ann = s.create_user("Ann", b"hunter2hunter2", salt(2), 0).unwrap();
        let bob_sid = s.user(bob).unwrap().account.sid.clone();
        assert!(s.groups_of(bob).contains(&SID_USERS.to_string()));
        assert!(!s.groups_of(bob).contains(&SID_ADMINS.to_string()));
        let devs = s.create_alias("Devs", "developers").unwrap();
        let devs_sid = alloc::format!("{SID}-{devs}");
        s.add_member(&devs_sid, &bob_sid).unwrap();
        assert_eq!(s.add_member(&devs_sid, &bob_sid), Err(Error::Exists));
        s.add_member(SID_ADMINS, &devs_sid).unwrap(); // nested: Bob -> Devs -> Administrators
        let g = s.groups_of(bob);
        assert!(g.contains(&devs_sid) && g.contains(&SID_ADMINS.to_string()));
        assert_eq!(s.user(bob).unwrap().account.groups, g, "logon account sees the groups");
        assert!(!s.groups_of(ann).contains(&SID_ADMINS.to_string()));
        assert_eq!(s.add_member(SID_ADMINS, "S-1-5-21-9-9-9-5000"), Err(Error::NotFound));
    }

    #[test]
    fn builtin_and_last_admin_protection() {
        let mut s = sam();
        assert_eq!(s.delete_user(RID_ADMIN), Err(Error::Denied));
        assert_eq!(s.rename_user(RID_GUEST, "Visitor"), Err(Error::Denied));
        assert_eq!(s.delete_alias(SID_ADMINS), Err(Error::Denied));
        let ann = s.create_user("Ann", b"hunter2hunter2", salt(2), 0).unwrap();
        let sid = s.user(ann).unwrap().account.sid.clone();
        s.add_member(SID_ADMINS, &sid).unwrap();
        s.set_disabled(RID_ADMIN, false).unwrap();
        // Two enabled admins: one may go.
        s.set_disabled(RID_ADMIN, true).unwrap();
        // Ann is now the last enabled administrator.
        assert_eq!(s.set_disabled(ann, true), Err(Error::Denied));
        assert_eq!(s.delete_user(ann), Err(Error::Denied));
        assert_eq!(s.remove_member(SID_ADMINS, &sid), Err(Error::Denied));
        let bob = s.create_user("Bob", b"hunter2hunter2", salt(1), 0).unwrap();
        s.add_member(SID_ADMINS, &s.user(bob).unwrap().account.sid.clone()).unwrap();
        assert!(s.delete_user(ann).is_ok(), "another admin exists now");
        assert!(s.user(ann).is_none());
        assert!(!s.aliases().iter().any(|a| a.members.contains(&sid)), "membership removed with the user");
    }

    #[test]
    fn name_and_sid_lookup() {
        let mut s = sam();
        let bob = s.create_user("Bob", b"hunter2hunter2", salt(1), 0).unwrap();
        let sid = s.user(bob).unwrap().account.sid.clone();
        assert_eq!(s.lookup_name("bob"), Some((sid.clone(), SidType::User)));
        assert_eq!(s.lookup_name("LOOS\\Bob"), Some((sid.clone(), SidType::User)));
        assert_eq!(s.lookup_name("OTHER\\Bob"), None);
        assert_eq!(s.lookup_name("BUILTIN\\Administrators"), Some((SID_ADMINS.into(), SidType::Alias)));
        assert_eq!(s.lookup_name("Everyone"), Some((SID_EVERYONE.into(), SidType::WellKnown)));
        assert_eq!(s.lookup_sid(&sid), Some(("Bob".into(), "LOOS".into(), SidType::User)));
        assert_eq!(s.lookup_sid(SID_ADMINS), Some(("Administrators".into(), "BUILTIN".into(), SidType::Alias)));
        assert_eq!(s.lookup_sid(SID_SYSTEM), Some(("SYSTEM".into(), "NT AUTHORITY".into(), SidType::WellKnown)));
        assert_eq!(s.lookup_sid("S-1-5-21-1-2-3-4242"), None);
        assert_eq!(s.lookup_sid(&alloc::format!("{SID}-500")).unwrap().0, "Administrator");
    }

    #[test]
    fn lsa_privileges_tokens_and_secrets() {
        let mut s = sam();
        let mut lsa = Lsa::new(&[7; 32]);
        let bob = s.create_user("Bob", b"hunter2hunter2", salt(1), 0).unwrap();
        let t = lsa.token_for(&s, bob, 3).unwrap();
        assert_eq!(t.privileges & privs::DEBUG, 0);
        assert_ne!(t.privileges & privs::SHUTDOWN, 0);
        assert!(!t.is_admin());
        // Grant a privilege to Bob directly; promote him to Administrators for the full set.
        let bob_sid = s.user(bob).unwrap().account.sid.clone();
        lsa.grant(&bob_sid, "SeDebugPrivilege").unwrap();
        assert_ne!(lsa.token_for(&s, bob, 3).unwrap().privileges & privs::DEBUG, 0);
        assert_eq!(lsa.grant(&bob_sid, "SeNoSuchPrivilege"), Err(Error::Invalid));
        lsa.revoke(&bob_sid, "SeDebugPrivilege").unwrap();
        assert_eq!(lsa.revoke(&bob_sid, "SeDebugPrivilege"), Err(Error::NotFound));
        s.add_member(SID_ADMINS, &bob_sid).unwrap();
        let t = lsa.token_for(&s, bob, 4).unwrap();
        assert!(t.is_admin() && t.privileges == privs::ADMIN_ALL);
        assert_eq!(t.filtered().privileges & privs::DEBUG, 0, "UAC filtering still applies");
        // Deny right and disabled accounts block logon.
        lsa.grant(&bob_sid, "SeDenyInteractiveLogonRight").unwrap();
        assert_eq!(lsa.token_for(&s, bob, 5), Err(Error::Denied));
        assert_eq!(lsa.token_for(&s, RID_GUEST, 5), Err(Error::Denied), "disabled");
        // Secrets: sealed, distinct nonces, tamper-evident, survive a round trip with the same boot key only.
        lsa.store_secret("DefaultPassword", b"s3cret").unwrap();
        lsa.store_secret("Other", b"s3cret").unwrap();
        assert_eq!(lsa.read_secret("DefaultPassword").unwrap(), b"s3cret");
        assert_ne!(lsa.secrets["DefaultPassword"].1, lsa.secrets["Other"].1, "same plaintext, different nonce, different ciphertext");
        assert_ne!(lsa.secrets["DefaultPassword"].1, b"s3cret", "not stored in the clear");
        let blob = lsa.to_bytes();
        let back = Lsa::from_bytes(&[7; 32], &blob).unwrap();
        assert_eq!(back.read_secret("DefaultPassword").unwrap(), b"s3cret");
        assert_eq!(back.rights_of(&bob_sid), lsa.rights_of(&bob_sid));
        let wrong = Lsa::from_bytes(&[8; 32], &blob).unwrap();
        assert!(wrong.read_secret("DefaultPassword").is_err(), "wrong boot key cannot open secrets");
        lsa.secrets.get_mut("Other").unwrap().1[0] ^= 1;
        assert!(lsa.read_secret("Other").is_err(), "tampered ciphertext is rejected");
        assert_eq!(lsa.read_secret("DefaultPassword").unwrap(), b"s3cret", "other secrets unaffected");
        assert!(Lsa::from_bytes(&[7; 32], &blob[..blob.len() - 3]).is_err(), "truncated");
    }

    #[test]
    fn database_round_trip_and_logon_integration() {
        let mut s = sam();
        let bob = s.create_user("Bob", b"hunter2hunter2", salt(1), 5).unwrap();
        s.user_mut(bob).unwrap().full_name = "Bob B".into();
        let devs = s.create_alias("Devs", "d").unwrap();
        s.add_member(&alloc::format!("{SID}-{devs}"), &s.user(bob).unwrap().account.sid.clone()).unwrap();
        s.policy.min_len = 10;
        let blob = s.to_bytes();
        let back = Sam::from_bytes(&blob).unwrap();
        assert_eq!(back.policy, s.policy);
        assert_eq!(back.user(bob).unwrap().full_name, "Bob B");
        assert!(back.verify_password(bob, b"hunter2hunter2"), "hash and salt survive");
        assert_eq!(back.groups_of(bob), s.groups_of(bob));
        assert_eq!(back.to_bytes(), blob, "stable encoding");
        for cut in [0, 5, 20, blob.len() / 2, blob.len() - 1] {
            assert!(Sam::from_bytes(&blob[..cut]).is_err(), "cut at {cut}");
        }
        let mut extra = blob.clone();
        extra.push(0);
        assert!(Sam::from_bytes(&extra).is_err(), "trailing bytes");
        // The accounts drive the real logon UI (same iteration count).
        let mut ui = LogonUi::new(2);
        ui.add_provider(alloc::boxed::Box::new(PasswordProvider));
        back.export_to_logon(&mut ui);
        ui.secure_attention();
        let sess = ui.logon(&Credentials { provider: "password", user: "Bob", secret: b"hunter2hunter2" }, 10).unwrap();
        assert_eq!(sess.token.user, "S-1-5-21-1-2-3-1000");
        assert!(sess.token.groups.contains(&alloc::format!("{SID}-{devs}")));
    }
}
