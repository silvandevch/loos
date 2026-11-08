//! LogonUI model: credential providers, secure-attention-sequence gating, lockout and sessions.

use crate::hash::{ct_eq, pbkdf2};
use crate::uac::Token;
use crate::{Error, Result};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// PBKDF2 iterations for passwords (PINs use the same KDF; their strength comes from the lockout).
pub const ITERATIONS: u32 = 100_000;

/// Which desktop is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Desktop {
    /// Normal user desktop.
    Default,
    /// Isolated Winlogon desktop: only trusted code draws here.
    Secure,
}

/// Credentials typed by the user.
pub struct Credentials<'a> {
    /// Provider id.
    pub provider: &'a str,
    /// Account name.
    pub user: &'a str,
    /// Password or PIN.
    pub secret: &'a [u8],
}

/// A stored local account.
#[derive(Clone)]
pub struct Account {
    /// Name.
    pub name: String,
    /// SID.
    pub sid: String,
    /// Group SIDs.
    pub groups: Vec<String>,
    salt: [u8; 16],
    hash: [u8; 32],
    pin: Option<([u8; 16], [u8; 32])>,
    failures: u32,
    locked_until: u64,
    /// Disabled accounts cannot log on.
    pub disabled: bool,
}

fn derive(secret: &[u8], salt: &[u8; 16], iterations: u32) -> [u8; 32] {
    let mut o = [0u8; 32];
    pbkdf2(secret, salt, iterations, &mut o);
    o
}

/// Lockout threshold: after this many consecutive failures the account locks.
pub const LOCKOUT_THRESHOLD: u32 = 5;
/// First lockout duration (seconds); doubles per additional failure, capped at an hour.
pub const LOCKOUT_BASE: u64 = 30;

impl Account {
    /// Create an account with a password (`salt` from a CSPRNG).
    pub fn new(name: &str, sid: &str, groups: &[&str], password: &[u8], salt: [u8; 16], iterations: u32) -> Account {
        Account {
            name: name.into(),
            sid: sid.into(),
            groups: groups.iter().map(|g| String::from(*g)).collect(),
            salt,
            hash: derive(password, &salt, iterations),
            pin: None,
            failures: 0,
            locked_until: 0,
            disabled: false,
        }
    }

    /// Set a PIN (4-12 digits).
    pub fn set_pin(&mut self, pin: &[u8], salt: [u8; 16], iterations: u32) -> Result<()> {
        if !(4..=12).contains(&pin.len()) || !pin.iter().all(u8::is_ascii_digit) {
            return Err(Error::Invalid);
        }
        self.pin = Some((salt, derive(pin, &salt, iterations)));
        Ok(())
    }
}

impl Account {
    /// Replaces the password (the PIN is kept) and clears the failure counter and lockout.
    pub fn set_password(&mut self, password: &[u8], salt: [u8; 16], iterations: u32) {
        self.salt = salt;
        self.hash = derive(password, &salt, iterations);
        self.failures = 0;
        self.locked_until = 0;
    }

    /// Password check with the same lockout policy as the logon UI: a locked or disabled account always fails (after
    /// the same hashing work), [`LOCKOUT_THRESHOLD`] consecutive failures lock it with doubling delay.
    pub fn authenticate(&mut self, password: &[u8], iterations: u32, now: u64) -> Result<()> {
        let ok = self.check_password(password, iterations);
        if self.disabled {
            return Err(Error::BadCredential);
        }
        if now < self.locked_until {
            return Err(Error::LockedOut);
        }
        if !ok {
            self.failures += 1;
            if self.failures >= LOCKOUT_THRESHOLD {
                let shift = (self.failures - LOCKOUT_THRESHOLD).min(7);
                self.locked_until = now + (LOCKOUT_BASE << shift).min(3600);
            }
            return Err(Error::BadCredential);
        }
        self.failures = 0;
        Ok(())
    }

    /// Clears the failure counter and any lockout.
    pub fn unlock(&mut self) {
        self.failures = 0;
        self.locked_until = 0;
    }

    /// Whether `password` matches (constant-time compare).
    pub fn check_password(&self, password: &[u8], iterations: u32) -> bool {
        ct_eq(&derive(password, &self.salt, iterations), &self.hash)
    }

    /// Serialises the account for the SAM database.
    pub(crate) fn to_record(&self) -> Vec<u8> {
        let mut o = Vec::new();
        let put = |o: &mut Vec<u8>, b: &[u8]| {
            o.extend_from_slice(&(b.len() as u32).to_le_bytes());
            o.extend_from_slice(b);
        };
        put(&mut o, self.name.as_bytes());
        put(&mut o, self.sid.as_bytes());
        o.extend_from_slice(&(self.groups.len() as u32).to_le_bytes());
        for g in &self.groups {
            put(&mut o, g.as_bytes());
        }
        o.extend_from_slice(&self.salt);
        o.extend_from_slice(&self.hash);
        match &self.pin {
            Some((s, h)) => {
                o.push(1);
                o.extend_from_slice(s);
                o.extend_from_slice(h);
            }
            None => o.push(0),
        }
        o.extend_from_slice(&self.failures.to_le_bytes());
        o.extend_from_slice(&self.locked_until.to_le_bytes());
        o.push(self.disabled as u8);
        o
    }

    /// Inverse of [`Account::to_record`].
    pub(crate) fn from_record(b: &[u8]) -> Result<Account> {
        let mut c = crate::sam::Cur::new(b);
        let name = c.string()?;
        let sid = c.string()?;
        let ng = c.u32()? as usize;
        let mut groups = Vec::new();
        for _ in 0..ng.min(4096) {
            groups.push(c.string()?);
        }
        let salt: [u8; 16] = c.take(16)?.try_into().unwrap();
        let hash: [u8; 32] = c.take(32)?.try_into().unwrap();
        let pin = if c.take(1)?[0] == 1 {
            let s: [u8; 16] = c.take(16)?.try_into().unwrap();
            let h: [u8; 32] = c.take(32)?.try_into().unwrap();
            Some((s, h))
        } else {
            None
        };
        let failures = c.u32()?;
        let locked_until = c.u64()?;
        let disabled = c.take(1)?[0] != 0;
        Ok(Account { name, sid, groups, salt, hash, pin, failures, locked_until, disabled })
    }
}

/// A way of proving identity (password, PIN, smart card, ...).
pub trait CredentialProvider {
    /// Stable id.
    fn id(&self) -> &'static str;
    /// Tile label for the UI.
    fn label(&self) -> &'static str;
    /// Check `secret` against `account`. Must not leak which part was wrong.
    fn verify(&self, account: &Account, secret: &[u8], iterations: u32) -> bool;
}

/// Password provider.
pub struct PasswordProvider;
/// PIN provider.
pub struct PinProvider;

impl CredentialProvider for PasswordProvider {
    fn id(&self) -> &'static str {
        "password"
    }
    fn label(&self) -> &'static str {
        "Password"
    }
    fn verify(&self, a: &Account, secret: &[u8], it: u32) -> bool {
        ct_eq(&derive(secret, &a.salt, it), &a.hash)
    }
}

impl CredentialProvider for PinProvider {
    fn id(&self) -> &'static str {
        "pin"
    }
    fn label(&self) -> &'static str {
        "PIN"
    }
    fn verify(&self, a: &Account, secret: &[u8], it: u32) -> bool {
        match &a.pin {
            Some((salt, h)) => ct_eq(&derive(secret, salt, it), h),
            None => {
                let _ = derive(secret, &[0; 16], it); // same work either way
                false
            }
        }
    }
}

/// An interactive logon session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Session id.
    pub id: u32,
    /// User name.
    pub user: String,
    /// Whether the workstation is locked.
    pub locked: bool,
    /// The (full) token; the shell should run under `.filtered()` for administrators.
    pub token: Token,
}

/// LogonUI controller.
pub struct LogonUi {
    accounts: BTreeMap<String, Account>,
    providers: Vec<alloc::boxed::Box<dyn CredentialProvider>>,
    desktop: Desktop,
    sessions: Vec<Session>,
    next_session: u32,
    iterations: u32,
    dummy: Account,
}

impl LogonUi {
    /// New controller with the password and PIN providers. `iterations` is [`ITERATIONS`] in production.
    pub fn new(iterations: u32) -> Self {
        LogonUi {
            accounts: BTreeMap::new(),
            providers: alloc::vec![alloc::boxed::Box::new(PasswordProvider), alloc::boxed::Box::new(PinProvider)],
            desktop: Desktop::Default,
            sessions: Vec::new(),
            next_session: 1,
            iterations,
            dummy: Account::new("", "", &[], b"", [0; 16], iterations),
        }
    }

    /// Register an additional provider (smart card, ...).
    pub fn add_provider(&mut self, p: alloc::boxed::Box<dyn CredentialProvider>) {
        self.providers.push(p);
    }

    /// Provider ids and labels for tile rendering.
    pub fn tiles(&self) -> Vec<(&'static str, &'static str)> {
        self.providers.iter().map(|p| (p.id(), p.label())).collect()
    }

    /// Add or replace an account.
    pub fn add_account(&mut self, a: Account) {
        self.accounts.insert(a.name.to_ascii_lowercase(), a);
    }

    /// Current desktop.
    pub fn desktop(&self) -> Desktop {
        self.desktop
    }

    /// Secure attention sequence (Ctrl+Alt+Del): switch to the secure desktop.
    pub fn secure_attention(&mut self) {
        self.desktop = Desktop::Secure;
    }

    /// Return to the user desktop.
    pub fn dismiss(&mut self) {
        self.desktop = Desktop::Default;
    }

    /// Attempt a logon. Credentials are only accepted on the secure desktop so an app cannot
    /// spoof the prompt on the default one.
    pub fn logon(&mut self, c: &Credentials, now: u64) -> Result<Session> {
        if self.desktop != Desktop::Secure {
            return Err(Error::Denied);
        }
        let it = self.iterations;
        let provider = self.providers.iter().find(|p| p.id() == c.provider).ok_or(Error::Unsupported)?;
        let key = c.user.to_ascii_lowercase();
        let Some(acct) = self.accounts.get_mut(&key) else {
            provider.verify(&self.dummy, c.secret, it); // equalise timing, hide existence
            return Err(Error::BadCredential);
        };
        if acct.disabled || now < acct.locked_until {
            provider.verify(&self.dummy, c.secret, it);
            return Err(if now < acct.locked_until { Error::LockedOut } else { Error::BadCredential });
        }
        if !provider.verify(acct, c.secret, it) {
            acct.failures += 1;
            if acct.failures >= LOCKOUT_THRESHOLD {
                let shift = (acct.failures - LOCKOUT_THRESHOLD).min(7);
                acct.locked_until = now + (LOCKOUT_BASE << shift).min(3600);
            }
            return Err(Error::BadCredential);
        }
        acct.failures = 0;
        let groups: Vec<&str> = acct.groups.iter().map(|g| g.as_str()).collect();
        let id = self.next_session;
        self.next_session += 1;
        let s = Session { id, user: acct.name.clone(), locked: false, token: Token::for_user(&acct.sid, &groups, id) };
        self.sessions.push(s.clone());
        self.desktop = Desktop::Default;
        Ok(s)
    }

    /// Lock a session's workstation.
    pub fn lock(&mut self, id: u32) {
        if let Some(s) = self.sessions.iter_mut().find(|s| s.id == id) {
            s.locked = true;
        }
        self.desktop = Desktop::Secure;
    }

    /// Unlock a session: requires the same user's credentials on the secure desktop.
    pub fn unlock(&mut self, id: u32, c: &Credentials, now: u64) -> Result<()> {
        let user = self.sessions.iter().find(|s| s.id == id).ok_or(Error::Invalid)?.user.clone();
        if !user.eq_ignore_ascii_case(c.user) {
            return Err(Error::Denied); // another user's credentials do not unlock this session
        }
        let s = self.logon(c, now)?;
        self.sessions.retain(|x| x.id != s.id); // logon created a duplicate; keep the original
        self.next_session -= 1;
        if let Some(o) = self.sessions.iter_mut().find(|x| x.id == id) {
            o.locked = false;
        }
        Ok(())
    }

    /// Log a session off.
    pub fn logoff(&mut self, id: u32) {
        self.sessions.retain(|s| s.id != id);
    }

    /// Active sessions.
    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uac::SID_ADMINS;

    fn ui() -> LogonUi {
        let mut u = LogonUi::new(10);
        let mut a = Account::new("Alice", "S-1-5-21-1-1001", &[SID_ADMINS], b"correct horse", [1; 16], 10);
        a.set_pin(b"123456", [2; 16], 10).unwrap();
        u.add_account(a);
        u.add_account(Account::new("bob", "S-1-5-21-1-1002", &[], b"hunter2", [3; 16], 10));
        u
    }
    fn cred<'a>(p: &'a str, u: &'a str, s: &'a [u8]) -> Credentials<'a> {
        Credentials { provider: p, user: u, secret: s }
    }

    #[test]
    fn requires_secure_desktop() {
        let mut u = ui();
        assert_eq!(u.logon(&cred("password", "alice", b"correct horse"), 0).err(), Some(Error::Denied));
        u.secure_attention();
        assert_eq!(u.desktop(), Desktop::Secure);
        let s = u.logon(&cred("password", "ALICE", b"correct horse"), 0).unwrap();
        assert!(s.token.is_admin() && s.token.elevated);
        assert_eq!(u.desktop(), Desktop::Default);
        assert_eq!(u.tiles(), [("password", "Password"), ("pin", "PIN")]);
    }

    #[test]
    fn pin_provider_and_unknowns() {
        let mut u = ui();
        u.secure_attention();
        u.logon(&cred("pin", "alice", b"123456"), 0).unwrap();
        u.secure_attention();
        assert_eq!(u.logon(&cred("pin", "bob", b"123456"), 0).err(), Some(Error::BadCredential)); // bob has no PIN
        assert_eq!(u.logon(&cred("password", "nobody", b"x"), 0).err(), Some(Error::BadCredential));
        assert_eq!(u.logon(&cred("smartcard", "alice", b"x"), 0).err(), Some(Error::Unsupported));
        let mut a = Account::new("c", "s", &[], b"p", [0; 16], 10);
        assert!(a.set_pin(b"12", [0; 16], 10).is_err() && a.set_pin(b"12ab", [0; 16], 10).is_err());
    }

    #[test]
    fn lockout_escalates_and_clears() {
        let mut u = ui();
        u.secure_attention();
        for _ in 0..LOCKOUT_THRESHOLD {
            assert_eq!(u.logon(&cred("password", "bob", b"bad"), 100).err(), Some(Error::BadCredential));
        }
        assert_eq!(u.logon(&cred("password", "bob", b"hunter2"), 101).err(), Some(Error::LockedOut));
        assert_eq!(u.logon(&cred("password", "bob", b"bad"), 100 + LOCKOUT_BASE).err(), Some(Error::BadCredential)); // second strike -> 60s
        assert_eq!(u.logon(&cred("password", "bob", b"hunter2"), 100 + LOCKOUT_BASE + 59).err(), Some(Error::LockedOut));
        u.logon(&cred("password", "bob", b"hunter2"), 100 + LOCKOUT_BASE + 61).unwrap();
        u.secure_attention();
        // failures reset on success
        assert_eq!(u.logon(&cred("password", "bob", b"bad"), 1000).err(), Some(Error::BadCredential));
        u.logon(&cred("password", "bob", b"hunter2"), 1000).unwrap();
    }

    #[test]
    fn lock_unlock_logoff() {
        let mut u = ui();
        u.secure_attention();
        let s = u.logon(&cred("password", "bob", b"hunter2"), 0).unwrap();
        u.lock(s.id);
        assert!(u.sessions()[0].locked && u.desktop() == Desktop::Secure);
        assert_eq!(u.unlock(s.id, &cred("password", "alice", b"correct horse"), 0), Err(Error::Denied));
        assert_eq!(u.unlock(s.id, &cred("password", "bob", b"nope"), 0), Err(Error::BadCredential));
        u.unlock(s.id, &cred("password", "bob", b"hunter2"), 0).unwrap();
        assert_eq!(u.sessions().len(), 1);
        assert!(!u.sessions()[0].locked);
        u.logoff(s.id);
        assert!(u.sessions().is_empty());
        u.accounts.get_mut("bob").unwrap().disabled = true;
        u.secure_attention();
        assert_eq!(u.logon(&cred("password", "bob", b"hunter2"), 0).err(), Some(Error::BadCredential));
    }
}
