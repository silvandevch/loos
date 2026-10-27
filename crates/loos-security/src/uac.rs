//! Access tokens, split-token elevation and the UAC elevation broker.
//!
//! The broker never hands out a high-integrity token without a one-shot prompt that was answered
//! on the secure desktop. Auto-elevation exceptions are deliberately not implemented.

use crate::{Error, Result};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// Mandatory integrity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Integrity {
    /// Sandboxed content.
    Low = 1,
    /// Standard user.
    Medium = 2,
    /// Elevated administrator.
    High = 3,
    /// Services / kernel.
    System = 4,
}

/// Privilege bits.
pub mod privs {
    /// Bypass traverse checking.
    pub const CHANGE_NOTIFY: u32 = 1;
    /// Shut down the system.
    pub const SHUTDOWN: u32 = 2;
    /// Remove computer from docking station.
    pub const UNDOCK: u32 = 4;
    /// Increase a process working set.
    pub const WORKING_SET: u32 = 8;
    /// Change the time zone.
    pub const TIME_ZONE: u32 = 16;
    /// Load and unload device drivers.
    pub const LOAD_DRIVER: u32 = 32;
    /// Take ownership of objects.
    pub const TAKE_OWNERSHIP: u32 = 64;
    /// Debug programs.
    pub const DEBUG: u32 = 128;
    /// Back up files and directories.
    pub const BACKUP: u32 = 256;
    /// Change the system time.
    pub const SYSTEMTIME: u32 = 512;
    /// Everything an administrator holds.
    pub const ADMIN_ALL: u32 = 0x3ff;
    /// What a filtered (UAC) token keeps.
    pub const STANDARD: u32 = CHANGE_NOTIFY | SHUTDOWN | UNDOCK | WORKING_SET | TIME_ZONE;
}

/// Well-known group SID for local administrators.
pub const SID_ADMINS: &str = "S-1-5-32-544";

/// A security access token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// User SID.
    pub user: String,
    /// Enabled groups.
    pub groups: Vec<String>,
    /// Groups present but only usable to deny access.
    pub deny_only: Vec<String>,
    /// Integrity level.
    pub integrity: Integrity,
    /// Privilege bitmask.
    pub privileges: u32,
    /// Whether this is the elevated half of a split token.
    pub elevated: bool,
    /// Logon session id.
    pub session: u32,
}

impl Token {
    /// Whether the token carries the Administrators group (enabled).
    pub fn is_admin(&self) -> bool {
        self.groups.iter().any(|g| g == SID_ADMINS)
    }

    /// Build a logon token. Administrators receive the full (High) token; use [`Token::filtered`]
    /// for the one the shell runs under.
    pub fn for_user(user: &str, groups: &[&str], session: u32) -> Token {
        let admin = groups.contains(&SID_ADMINS);
        Token {
            user: user.into(),
            groups: groups.iter().map(|g| String::from(*g)).collect(),
            deny_only: Vec::new(),
            integrity: if admin { Integrity::High } else { Integrity::Medium },
            privileges: if admin { privs::ADMIN_ALL } else { privs::STANDARD },
            elevated: admin,
            session,
        }
    }

    /// The filtered Medium-integrity token for an administrator (UAC split token).
    pub fn filtered(&self) -> Token {
        let mut t = self.clone();
        if let Some(i) = t.groups.iter().position(|g| g == SID_ADMINS) {
            t.deny_only.push(t.groups.remove(i));
        }
        t.integrity = Integrity::Medium;
        t.privileges &= privs::STANDARD;
        t.elevated = false;
        t
    }
}

/// Manifest execution level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionLevel {
    /// Run with the caller's token.
    AsInvoker,
    /// Elevate if the caller is an administrator.
    HighestAvailable,
    /// Must run elevated.
    RequireAdministrator,
}

/// UAC slider positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UacLevel {
    /// Prompt for everything that elevates, always on the secure desktop.
    AlwaysNotify,
    /// Default: prompt on the secure desktop.
    Notify,
    /// Prompt on the normal desktop (no dim).
    NotifyNoDim,
    /// Elevation requests from administrators succeed silently (discouraged).
    Never,
}

/// What the caller wants to run.
#[derive(Debug, Clone)]
pub struct ElevationRequest {
    /// Image path.
    pub image: String,
    /// Verified publisher name from code signing, if any.
    pub publisher: Option<String>,
    /// Manifest level.
    pub level: ExecutionLevel,
    /// Requesting token (the filtered token for admins).
    pub requester: Token,
    /// The requester's linked full token, present only for administrators.
    pub linked_full: Option<Token>,
}

/// What the UI must show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Run with the requester's token, no prompt.
    RunAsInvoker,
    /// Refuse (standard user asked for `RequireAdministrator` and policy forbids credential prompts, etc.).
    Deny,
    /// Elevated without prompting (`UacLevel::Never` only).
    ElevateSilently(Token),
    /// Show a Yes/No consent dialog.
    Consent(PromptId),
    /// Ask for administrator credentials.
    Credentials(PromptId),
}

/// One-shot prompt identifier.
pub type PromptId = u64;

/// The user's answer, relayed from the secure desktop.
pub enum Response {
    /// Consent granted.
    Allow,
    /// Consent refused.
    Deny,
    /// Administrator credentials, already verified by a credential provider; carries that account's full token.
    Admin(Token),
}

struct Pending {
    req: ElevationRequest,
    created: u64,
    credentials: bool,
    secure: bool,
}

/// Prompts expire after this many seconds.
pub const PROMPT_TTL: u64 = 120;

/// The elevation broker.
pub struct Broker {
    /// Current policy.
    pub level: UacLevel,
    next: PromptId,
    pending: BTreeMap<PromptId, Pending>,
    /// Audit trail `(time, image, outcome)`; the caller drains this into the event log.
    pub audit: Vec<(u64, String, &'static str)>,
}

impl Broker {
    /// New broker with the given slider position.
    pub fn new(level: UacLevel) -> Self {
        Broker { level, next: 1, pending: BTreeMap::new(), audit: Vec::new() }
    }

    /// Whether prompt `id` must be displayed on the secure desktop.
    pub fn needs_secure_desktop(&self, id: PromptId) -> bool {
        self.pending.get(&id).is_some_and(|p| p.secure)
    }

    /// Decide what to do with a launch request.
    pub fn evaluate(&mut self, req: ElevationRequest, now: u64) -> Decision {
        self.expire(now);
        let wants = match req.level {
            ExecutionLevel::AsInvoker => false,
            ExecutionLevel::RequireAdministrator => true,
            ExecutionLevel::HighestAvailable => req.linked_full.is_some(),
        };
        if !wants || req.requester.elevated {
            return Decision::RunAsInvoker;
        }
        let admin = req.linked_full.is_some();
        if admin && self.level == UacLevel::Never {
            let t = req.linked_full.clone().unwrap();
            self.audit.push((now, req.image, "elevated-silently"));
            return Decision::ElevateSilently(t);
        }
        let secure = self.level != UacLevel::NotifyNoDim;
        let id = self.next;
        self.next += 1;
        self.pending.insert(id, Pending { req, created: now, credentials: !admin, secure });
        if admin {
            Decision::Consent(id)
        } else {
            Decision::Credentials(id)
        }
    }

    fn expire(&mut self, now: u64) {
        let stale: Vec<PromptId> = self.pending.iter().filter(|(_, p)| now.saturating_sub(p.created) > PROMPT_TTL).map(|(k, _)| *k).collect();
        for k in stale {
            if let Some(p) = self.pending.remove(&k) {
                self.audit.push((now, p.req.image, "expired"));
            }
        }
    }

    /// Complete a prompt. Each id is valid once; a response of the wrong kind consumes it and denies.
    pub fn respond(&mut self, id: PromptId, resp: Response, now: u64) -> Result<Token> {
        self.expire(now);
        let p = self.pending.remove(&id).ok_or(Error::Denied)?;
        let image = p.req.image.clone();
        let out = match (resp, p.credentials) {
            (Response::Allow, false) => p.req.linked_full.ok_or(Error::Denied),
            (Response::Admin(t), true) if t.is_admin() && t.integrity == Integrity::High && t.elevated => Ok(t),
            _ => Err(Error::Denied),
        };
        self.audit.push((now, image, if out.is_ok() { "elevated" } else { "denied" }));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admin() -> Token {
        Token::for_user("S-1-5-21-1-1001", &["S-1-1-0", SID_ADMINS], 1)
    }
    fn user() -> Token {
        Token::for_user("S-1-5-21-1-1002", &["S-1-1-0"], 2)
    }
    fn req(level: ExecutionLevel, t: Token, full: Option<Token>) -> ElevationRequest {
        ElevationRequest { image: "C:\\setup.exe".into(), publisher: None, level, requester: t, linked_full: full }
    }

    #[test]
    fn split_token() {
        let full = admin();
        assert!(full.elevated && full.is_admin() && full.integrity == Integrity::High);
        let f = full.filtered();
        assert!(!f.is_admin() && f.deny_only == [SID_ADMINS] && f.integrity == Integrity::Medium);
        assert_eq!(f.privileges, privs::STANDARD);
        assert!(!user().elevated);
    }

    #[test]
    fn admin_consent_flow_is_one_shot() {
        let mut b = Broker::new(UacLevel::Notify);
        let full = admin();
        let d = b.evaluate(req(ExecutionLevel::RequireAdministrator, full.filtered(), Some(full.clone())), 10);
        let Decision::Consent(id) = d else { panic!("{:?}", d) };
        assert!(b.needs_secure_desktop(id));
        assert_eq!(b.respond(id, Response::Allow, 11).unwrap(), full);
        assert_eq!(b.respond(id, Response::Allow, 12), Err(Error::Denied)); // replay
        assert_eq!(b.audit.len(), 1);
        assert_eq!(b.audit[0].2, "elevated");
    }

    #[test]
    fn deny_expiry_and_wrong_response() {
        let mut b = Broker::new(UacLevel::AlwaysNotify);
        let full = admin();
        let r = || req(ExecutionLevel::HighestAvailable, full.filtered(), Some(full.clone()));
        let Decision::Consent(a) = b.evaluate(r(), 0) else { panic!() };
        assert_eq!(b.respond(a, Response::Deny, 1), Err(Error::Denied));
        let Decision::Consent(c) = b.evaluate(r(), 0) else { panic!() };
        assert_eq!(b.respond(c, Response::Allow, PROMPT_TTL + 1), Err(Error::Denied));
        assert_eq!(b.audit.last().unwrap().2, "expired");
        // a standard user cannot answer a consent prompt with a non-admin token
        let Decision::Credentials(id) = b.evaluate(req(ExecutionLevel::RequireAdministrator, user(), None), 5) else { panic!() };
        assert_eq!(b.respond(id, Response::Admin(user()), 6), Err(Error::Denied));
        // ... nor with the filtered token
        let Decision::Credentials(id) = b.evaluate(req(ExecutionLevel::RequireAdministrator, user(), None), 5) else { panic!() };
        assert_eq!(b.respond(id, Response::Admin(admin().filtered()), 6), Err(Error::Denied));
        let Decision::Credentials(id) = b.evaluate(req(ExecutionLevel::RequireAdministrator, user(), None), 5) else { panic!() };
        assert_eq!(b.respond(id, Response::Admin(admin()), 6).unwrap(), admin());
    }

    #[test]
    fn policy_levels_and_invoker() {
        let mut b = Broker::new(UacLevel::NotifyNoDim);
        let full = admin();
        assert_eq!(b.evaluate(req(ExecutionLevel::AsInvoker, full.filtered(), Some(full.clone())), 0), Decision::RunAsInvoker);
        assert_eq!(b.evaluate(req(ExecutionLevel::HighestAvailable, user(), None), 0), Decision::RunAsInvoker);
        assert_eq!(b.evaluate(req(ExecutionLevel::RequireAdministrator, full.clone(), None), 0), Decision::RunAsInvoker); // already elevated
        let Decision::Consent(id) = b.evaluate(req(ExecutionLevel::RequireAdministrator, full.filtered(), Some(full.clone())), 0) else { panic!() };
        assert!(!b.needs_secure_desktop(id));
        b.level = UacLevel::Never;
        assert_eq!(
            b.evaluate(req(ExecutionLevel::RequireAdministrator, full.filtered(), Some(full.clone())), 0),
            Decision::ElevateSilently(full)
        );
    }
}
