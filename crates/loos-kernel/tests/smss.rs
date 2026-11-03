use loos_kernel::smss::*;
use loos_kernel::ProcessId;

#[derive(Default)]
struct L {
    next: ProcessId,
    launched: Vec<(String, u32, ProcessId)>,
    terminated: Vec<ProcessId>,
    ran: Vec<String>,
    fail: Option<&'static str>,
    envs: Vec<Vec<(String, String)>>,
}

impl Launcher for L {
    fn launch(&mut self, image: &str, _c: &str, session: u32, env: &[(String, String)]) -> Result<ProcessId, String> {
        if self.fail == Some(image) {
            return Err("no such image".into());
        }
        self.next += 1;
        let pid = 100 + self.next;
        self.launched.push((image.into(), session, pid));
        self.envs.push(env.to_vec());
        Ok(pid)
    }
    fn run_to_completion(&mut self, image: &str, _c: &str) -> Result<u32, String> {
        self.ran.push(image.into());
        Ok(0)
    }
    fn terminate(&mut self, pid: ProcessId) {
        self.terminated.push(pid);
    }
}

fn pid_of(l: &L, img: &str, s: u32) -> ProcessId {
    l.launched.iter().find(|(i, ss, _)| i == img && *ss == s).unwrap().2
}

#[test]
fn boot_order_and_session0() {
    let (mut l, mut s) = (L::default(), Smss::new(SmConfig::default()));
    s.boot(&mut l).unwrap();
    assert_eq!(l.ran, ["autocheck"]);
    let order: Vec<_> = l.launched.iter().map(|x| x.0.as_str()).collect();
    assert_eq!(order, ["csrss.exe", "wininit.exe", "services.exe", "lsass.exe"]);
    assert!(s.booted());
    assert_eq!(s.session(0).unwrap().state, SessionState::Active);
    assert_eq!(s.known["kernel32"], "\\SystemRoot\\System32\\kernel32.dll");
    assert!(s.boot(&mut l).is_err(), "boot only once");
}

#[test]
fn interactive_session_lifecycle() {
    let (mut l, mut s) = (L::default(), Smss::new(SmConfig::default()));
    assert_eq!(s.create_session(&mut l), Err(SmError::BadState), "before boot");
    s.boot(&mut l).unwrap();
    let id = s.create_session(&mut l).unwrap();
    assert_eq!(id, 1);
    assert_eq!(s.session(1).unwrap().objects_dir, "\\Sessions\\1\\BaseNamedObjects");
    assert!(l.launched.iter().any(|x| x.0 == "winlogon.exe" && x.1 == 1));
    assert!(l.envs.last().unwrap().iter().any(|(k, v)| k == "SESSIONNAME" && v == "Console1"));
    assert_eq!(s.create_session(&mut l).unwrap(), 2);
    s.terminate_session(1, &mut l).unwrap();
    assert_eq!(s.session(1).unwrap().state, SessionState::Ended);
    assert_eq!(l.terminated.len(), 2, "csrss + winlogon of session 1");
    assert_eq!(s.create_session(&mut l).unwrap(), 1, "ended id is reused");
    assert_eq!(s.terminate_session(0, &mut l), Err(SmError::NoSession));
}

#[test]
fn critical_death_policy() {
    let (mut l, mut s) = (L::default(), Smss::new(SmConfig::default()));
    s.boot(&mut l).unwrap();
    s.create_session(&mut l).unwrap();
    let wl = pid_of(&l, "winlogon.exe", 1);
    assert_eq!(s.on_exit(wl, 0, &mut l), Outcome::SessionEnded(1));
    assert_eq!(s.session(1).unwrap().state, SessionState::Ended);
    let lsass = pid_of(&l, "lsass.exe", 0);
    assert_eq!(s.on_exit(lsass, 0xc0000005, &mut l), Outcome::Bugcheck { code: CRITICAL_PROCESS_DIED, pid: lsass, status: 0xc0000005 });
    assert_eq!(s.on_exit(9999, 0, &mut l), Outcome::Ignored);
}

#[test]
fn noncritical_exit_and_api() {
    let (mut l, mut s) = (L::default(), Smss::new(SmConfig::default()));
    s.boot(&mut l).unwrap();
    assert_eq!(s.handle(SmRequest::CreateSession, &mut l), SmReply::Created(1));
    assert_eq!(s.handle(SmRequest::LoadSubsystem("Nope".into()), &mut l), SmReply::Error(SmError::NoSubsystem));
    let n = l.launched.len();
    assert_eq!(s.handle(SmRequest::LoadSubsystem("windows".into()), &mut l), SmReply::Ok);
    assert_eq!(l.launched.len(), n + 2, "csrss started again in both active sessions");
    let extra = l.launched.last().unwrap().2;
    assert_eq!(s.on_exit(extra, 1, &mut l), Outcome::Reaped);
    match s.handle(SmRequest::QuerySessions, &mut l) {
        SmReply::Sessions(v) => assert_eq!(v, [(0, SessionState::Active), (1, SessionState::Active)]),
        r => panic!("{r:?}"),
    }
    assert_eq!(s.handle(SmRequest::TerminateSession(7), &mut l), SmReply::Error(SmError::NoSession));
}

#[test]
fn required_subsystem_failure_and_logon_unwind() {
    let (mut l, mut s) = (L { fail: Some("csrss.exe"), ..L::default() }, Smss::new(SmConfig::default()));
    assert!(matches!(s.boot(&mut l), Err(SmError::SubsystemFailed(_))));
    let (mut l, mut s) = (L::default(), Smss::new(SmConfig::default()));
    s.boot(&mut l).unwrap();
    l.fail = Some("winlogon.exe");
    assert!(matches!(s.create_session(&mut l), Err(SmError::Launch(_))));
    assert_eq!(l.terminated.len(), 1, "csrss of the failed session is unwound");
}

#[test]
fn session_limit() {
    let (mut l, mut s) = (L::default(), Smss::new(SmConfig::default()));
    s.boot(&mut l).unwrap();
    for _ in 1..MAX_SESSIONS {
        s.create_session(&mut l).unwrap();
    }
    assert_eq!(s.create_session(&mut l), Err(SmError::TooManySessions));
}
