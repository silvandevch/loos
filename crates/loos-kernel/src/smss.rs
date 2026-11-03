//! Session Manager (smss.exe) and subsystem initialisation.
//!
//! Behaviour follows the NT boot sequence: set up `KnownDLLs` and the system environment, run
//! `BootExecute` images to completion, start the required subsystems (Debug, Windows → csrss), bring up
//! session 0 (wininit, services, lsass), and create interactive sessions on demand (csrss + winlogon).
//! A critical process dying in session 0 is a bugcheck (CRITICAL_PROCESS_DIED); in an interactive
//! session it ends only that session. Process creation is delegated through [`Launcher`] so the policy is
//! host-testable and the kernel supplies the real loader.

use crate::ProcessId;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Bugcheck code for a critical process terminating.
pub const CRITICAL_PROCESS_DIED: u32 = 0xef;
/// Maximum concurrent sessions (including session 0).
pub const MAX_SESSIONS: u32 = 16;

/// Errors from the session manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmError {
    /// The launcher could not create the process.
    Launch(String),
    /// Unknown or already-ended session.
    NoSession,
    /// Session table full.
    TooManySessions,
    /// A required subsystem failed to start (fatal at boot).
    SubsystemFailed(String),
    /// Boot not finished yet / already finished.
    BadState,
    /// Subsystem name not configured.
    NoSubsystem,
}

/// Creates and kills processes on behalf of smss.
pub trait Launcher {
    /// Starts `image` with `cmdline` in `session`, with the given environment; returns the process id.
    fn launch(&mut self, image: &str, cmdline: &str, session: u32, env: &[(String, String)]) -> Result<ProcessId, String>;
    /// Runs `image` to completion (BootExecute); returns its exit code.
    fn run_to_completion(&mut self, image: &str, cmdline: &str) -> Result<u32, String>;
    /// Forcibly terminates a process.
    fn terminate(&mut self, pid: ProcessId);
}

/// One configured subsystem (`HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\SubSystems`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subsystem {
    /// Name ("Windows", "Debug", "Posix").
    pub name: String,
    /// Image to run in each session; empty for subsystems with no server process.
    pub image: String,
    /// Command line.
    pub cmdline: String,
    /// Boot fails if a required subsystem cannot start.
    pub required: bool,
}

/// smss configuration (normally read from the registry).
#[derive(Debug, Clone)]
pub struct SmConfig {
    /// Images run before anything else.
    pub boot_execute: Vec<String>,
    /// Subsystems; "Windows" provides csrss.
    pub subsystems: Vec<Subsystem>,
    /// `(name, file)` KnownDLLs.
    pub known_dlls: Vec<(String, String)>,
    /// Directory the KnownDLLs live in.
    pub dll_directory: String,
    /// System environment block.
    pub environment: Vec<(String, String)>,
    /// Processes started in session 0 after csrss, in order.
    pub session0_services: Vec<String>,
    /// Interactive-session shell started after csrss.
    pub logon: String,
}

impl Default for SmConfig {
    fn default() -> Self {
        let s = |a: &str| a.to_string();
        SmConfig {
            boot_execute: alloc::vec![s("autocheck autochk *")],
            subsystems: alloc::vec![
                Subsystem { name: s("Debug"), image: String::new(), cmdline: String::new(), required: true },
                Subsystem { name: s("Windows"), image: s("csrss.exe"), cmdline: s("ObjectDirectory=\\Windows SharedSection=1024,20480,768 ServerDll=basesrv,1 ServerDll=winsrv:UserServerDllInitialization,3"), required: true },
            ],
            known_dlls: alloc::vec![(s("kernel32"), s("kernel32.dll")), (s("ntdll"), s("ntdll.dll")), (s("user32"), s("user32.dll")), (s("gdi32"), s("gdi32.dll")), (s("advapi32"), s("advapi32.dll"))],
            dll_directory: s("\\SystemRoot\\System32"),
            environment: alloc::vec![(s("SystemRoot"), s("C:\\Windows")), (s("windir"), s("C:\\Windows")), (s("ComSpec"), s("C:\\Windows\\System32\\cmd.exe")), (s("OS"), s("LoOS")), (s("PATHEXT"), s(".COM;.EXE;.BAT;.CMD"))],
            session0_services: alloc::vec![s("wininit.exe"), s("services.exe"), s("lsass.exe")],
            logon: s("winlogon.exe"),
        }
    }
}

/// Session lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Processes are being started.
    Starting,
    /// All startup processes running.
    Active,
    /// Terminated.
    Ended,
}

/// One session.
#[derive(Debug, Clone)]
pub struct Session {
    /// Session id (0 = services).
    pub id: u32,
    /// Lifecycle.
    pub state: SessionState,
    /// Object-manager directory for the session's named objects.
    pub objects_dir: String,
    /// csrss for this session.
    pub csrss: Option<ProcessId>,
    /// winlogon (interactive) or wininit (session 0).
    pub logon: Option<ProcessId>,
    /// Every process smss started in the session.
    pub procs: Vec<(ProcessId, String)>,
}

/// Result of reporting a process exit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Not a process smss cares about.
    Ignored,
    /// A non-critical process ended.
    Reaped,
    /// Interactive session torn down (all its processes were terminated).
    SessionEnded(u32),
    /// Critical system process died: the kernel must bugcheck.
    Bugcheck {
        /// Bugcheck code.
        code: u32,
        /// Dead process id.
        pid: ProcessId,
        /// Exit status.
        status: u32,
    },
}

/// Requests accepted on the `\SmApiPort`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmRequest {
    /// Create a new interactive session.
    CreateSession,
    /// Terminate an interactive session.
    TerminateSession(u32),
    /// Start a deferred subsystem in every active session.
    LoadSubsystem(String),
    /// List session ids and states.
    QuerySessions,
}

/// Replies to [`SmRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmReply {
    /// New session id.
    Created(u32),
    /// Done.
    Ok,
    /// Session list.
    Sessions(Vec<(u32, SessionState)>),
    /// Failure.
    Error(SmError),
}

/// The session manager.
pub struct Smss {
    cfg: SmConfig,
    sessions: BTreeMap<u32, Session>,
    booted: bool,
    /// KnownDLLs section directory as created at boot (`name -> path`).
    pub known: BTreeMap<String, String>,
    /// Log lines (what real smss would send to the debugger).
    pub log: Vec<String>,
}

impl Smss {
    /// New session manager; call [`Smss::boot`] next.
    pub fn new(cfg: SmConfig) -> Self {
        Smss { cfg, sessions: BTreeMap::new(), booted: false, known: BTreeMap::new(), log: Vec::new() }
    }

    /// Session by id.
    pub fn session(&self, id: u32) -> Option<&Session> {
        self.sessions.get(&id)
    }

    /// True once session 0 is active.
    pub fn booted(&self) -> bool {
        self.booted
    }

    fn windows_subsystem(&self) -> Option<&Subsystem> {
        self.cfg.subsystems.iter().find(|s| s.name == "Windows" && !s.image.is_empty())
    }

    /// Full boot sequence: KnownDLLs, BootExecute, subsystems, session 0.
    pub fn boot(&mut self, l: &mut dyn Launcher) -> Result<(), SmError> {
        if self.booted || !self.sessions.is_empty() {
            return Err(SmError::BadState);
        }
        for (n, f) in &self.cfg.known_dlls {
            self.known.insert(n.to_ascii_lowercase(), alloc::format!("{}\\{}", self.cfg.dll_directory, f));
        }
        for cmd in self.cfg.boot_execute.clone() {
            let (img, args) = cmd.split_once(' ').unwrap_or((&cmd, ""));
            match l.run_to_completion(img, args) {
                Ok(0) => self.log.push(alloc::format!("BootExecute {img}: ok")),
                Ok(c) => self.log.push(alloc::format!("BootExecute {img}: exit {c:#x}")),
                Err(e) => self.log.push(alloc::format!("BootExecute {img}: {e}")),
            }
        }
        for sub in self.cfg.subsystems.clone().iter().filter(|s| s.image.is_empty()) {
            self.log.push(alloc::format!("Subsystem {} (no server)", sub.name));
        }
        self.start_session(0, l)?;
        self.booted = true;
        Ok(())
    }

    fn start_session(&mut self, id: u32, l: &mut dyn Launcher) -> Result<(), SmError> {
        let mut env = self.cfg.environment.clone();
        env.push(("SESSIONNAME".to_string(), if id == 0 { "Services".to_string() } else { alloc::format!("Console{id}") }));
        let mut s = Session { id, state: SessionState::Starting, objects_dir: alloc::format!("\\Sessions\\{id}\\BaseNamedObjects"), csrss: None, logon: None, procs: Vec::new() };
        if let Some(sub) = self.windows_subsystem().cloned() {
            match l.launch(&sub.image, &alloc::format!("{} SessionId={id}", sub.cmdline), id, &env) {
                Ok(pid) => {
                    s.csrss = Some(pid);
                    s.procs.push((pid, sub.image.clone()));
                }
                Err(e) if sub.required => return Err(SmError::SubsystemFailed(alloc::format!("{}: {e}", sub.name))),
                Err(e) => self.log.push(alloc::format!("Windows subsystem: {e}")),
            }
        }
        let list: Vec<String> = if id == 0 { self.cfg.session0_services.clone() } else { alloc::vec![self.cfg.logon.clone()] };
        for (i, img) in list.iter().enumerate() {
            match l.launch(img, "", id, &env) {
                Ok(pid) => {
                    if i == 0 {
                        s.logon = Some(pid);
                    }
                    s.procs.push((pid, img.clone()));
                }
                Err(e) => {
                    // Unwind: a session without its logon process is unusable.
                    for (p, _) in s.procs.iter().rev() {
                        l.terminate(*p);
                    }
                    return Err(SmError::Launch(alloc::format!("{img}: {e}")));
                }
            }
        }
        s.state = SessionState::Active;
        self.log.push(alloc::format!("Session {id} active ({} processes)", s.procs.len()));
        self.sessions.insert(id, s);
        Ok(())
    }

    /// Creates the next interactive session.
    pub fn create_session(&mut self, l: &mut dyn Launcher) -> Result<u32, SmError> {
        if !self.booted {
            return Err(SmError::BadState);
        }
        let id = (1..MAX_SESSIONS).find(|i| self.sessions.get(i).map_or(true, |s| s.state == SessionState::Ended)).ok_or(SmError::TooManySessions)?;
        self.sessions.remove(&id);
        self.start_session(id, l)?;
        Ok(id)
    }

    /// Terminates an interactive session and everything in it.
    pub fn terminate_session(&mut self, id: u32, l: &mut dyn Launcher) -> Result<(), SmError> {
        if id == 0 {
            return Err(SmError::NoSession);
        }
        let s = self.sessions.get_mut(&id).filter(|s| s.state != SessionState::Ended).ok_or(SmError::NoSession)?;
        for (p, _) in s.procs.drain(..).rev() {
            l.terminate(p);
        }
        s.state = SessionState::Ended;
        s.csrss = None;
        s.logon = None;
        self.log.push(alloc::format!("Session {id} ended"));
        Ok(())
    }

    /// Reports that process `pid` exited with `status`.
    pub fn on_exit(&mut self, pid: ProcessId, status: u32, l: &mut dyn Launcher) -> Outcome {
        let Some((sid, critical)) = self.sessions.values().find_map(|s| {
            s.procs.iter().find(|(p, _)| *p == pid).map(|_| (s.id, Some(pid) == s.csrss || Some(pid) == s.logon || (s.id == 0 && s.procs.iter().any(|(p, n)| *p == pid && matches!(n.as_str(), "services.exe" | "lsass.exe" | "wininit.exe")))))
        }) else {
            return Outcome::Ignored;
        };
        if let Some(s) = self.sessions.get_mut(&sid) {
            s.procs.retain(|(p, _)| *p != pid);
        }
        if !critical {
            return Outcome::Reaped;
        }
        if sid == 0 {
            self.log.push(alloc::format!("Critical process {pid} died in session 0: {status:#x}"));
            return Outcome::Bugcheck { code: CRITICAL_PROCESS_DIED, pid, status };
        }
        let _ = self.terminate_session(sid, l);
        Outcome::SessionEnded(sid)
    }

    /// Handles one SM API request.
    pub fn handle(&mut self, req: SmRequest, l: &mut dyn Launcher) -> SmReply {
        match req {
            SmRequest::CreateSession => self.create_session(l).map_or_else(SmReply::Error, SmReply::Created),
            SmRequest::TerminateSession(id) => self.terminate_session(id, l).map_or_else(SmReply::Error, |()| SmReply::Ok),
            SmRequest::QuerySessions => SmReply::Sessions(self.sessions.values().map(|s| (s.id, s.state)).collect()),
            SmRequest::LoadSubsystem(name) => {
                let Some(sub) = self.cfg.subsystems.iter().find(|s| s.name.eq_ignore_ascii_case(&name)).cloned() else { return SmReply::Error(SmError::NoSubsystem) };
                if sub.image.is_empty() {
                    return SmReply::Ok;
                }
                let env = self.cfg.environment.clone();
                let ids: Vec<u32> = self.sessions.values().filter(|s| s.state == SessionState::Active).map(|s| s.id).collect();
                for id in ids {
                    match l.launch(&sub.image, &sub.cmdline, id, &env) {
                        Ok(pid) => {
                            if let Some(s) = self.sessions.get_mut(&id) {
                                s.procs.push((pid, sub.image.clone()));
                            }
                        }
                        Err(e) => return SmReply::Error(SmError::Launch(e)),
                    }
                }
                SmReply::Ok
            }
        }
    }
}
