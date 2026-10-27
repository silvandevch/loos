use loos_pkgstate::journal::*;
use loos_pkgstate::ownership::*;
use loos_pkgstate::sha256::*;
use loos_pkgstate::trust::*;

#[test]
fn sha256_vectors() {
    assert_eq!(hex(&digest(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(hex(&digest(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    let long = vec![b'a'; 1_000_000];
    assert_eq!(hex(&digest(&long)), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    let mut h = Sha256::new();
    for c in long.chunks(777) {
        h.update(c);
    }
    assert_eq!(h.finish(), digest(&long));
    assert!(verify(b"abc", "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"));
    assert!(!verify(b"abd", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad") && !verify(b"abc", "ba78"));
}

const H: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn trust_policy() {
    let mut p = Policy::default();
    p.allow_host("dl.example.com").allow_host("*.cdn.test");
    p.publishers.push("CN=Contoso".into());
    let c = |url, sig| Candidate { url, sha256: H, signed_publisher: sig };
    assert_eq!(p.check(&c("https://dl.example.com/a.msix", Some("cn=contoso"))), Ok(()));
    assert_eq!(p.check(&c("https://x.cdn.test:443/a", Some("CN=Contoso"))), Ok(()));
    assert_eq!(p.check(&c("http://dl.example.com/a", Some("CN=Contoso"))), Err(Denial::NotHttps));
    assert!(matches!(p.check(&c("https://cdn.test/a", Some("CN=Contoso"))), Err(Denial::HostDenied(_)))); // wildcard needs a subdomain
    assert!(matches!(p.check(&c("https://evildl.example.com/a", Some("CN=Contoso"))), Err(Denial::HostDenied(_))));
    assert!(matches!(p.check(&c("https://dl.example.com.evil.io/a", Some("CN=Contoso"))), Err(Denial::HostDenied(_))));
    assert_eq!(p.check(&c("https://dl.example.com@evil.io/a", None)), Err(Denial::BadUrl));
    assert_eq!(p.check(&c("https://dl.example.com:8443/a", None)), Err(Denial::BadUrl));
    assert_eq!(p.check(&c("https://dl.example.com/a", Some("CN=Other"))), Err(Denial::PublisherDenied));
    assert_eq!(p.check(&c("https://dl.example.com/a", None)), Err(Denial::PublisherDenied));
    p.allow_unsigned = true;
    assert_eq!(p.check(&c("https://dl.example.com/a", None)), Ok(()));
    let nohash = Candidate { url: "https://dl.example.com/a", sha256: "zz", signed_publisher: None };
    assert_eq!(p.check(&nohash), Err(Denial::NoHash));
    assert_eq!(Policy::default().check(&c("https://a.b/c", None)), Err(Denial::HostDenied("a.b".into())));
}

#[test]
fn ownership_install_remove_upgrade() {
    let mut db = Db::default();
    db.install("app", "1", &[("C:\\Prog\\App\\app.exe", false), ("C:/Prog/App/lib/a.dll", false), ("sys/vc.dll", true)]).unwrap();
    assert_eq!(db.owners("c:/prog/app/APP.EXE"), ["app"]);
    // shared file may be co-owned only when both declare it shared
    db.install("other", "1", &[("sys/VC.dll", true)]).unwrap();
    assert!(matches!(db.install("bad", "1", &[("sys/vc.dll", false)]), Err(Error::Conflict { .. })));
    assert!(matches!(db.install("bad", "1", &[("c:/prog/app/app.exe", true)]), Err(Error::Conflict { .. })));
    assert!(matches!(db.install("bad", "1", &[("../etc", false)]), Err(Error::BadPath(_))));
    let p = db.plan_remove("app").unwrap();
    assert_eq!(p.delete, ["c:/prog/app/lib/a.dll", "c:/prog/app/app.exe"]); // deepest first
    assert_eq!(p.keep, [("sys/vc.dll".to_string(), vec!["other".to_string()])]);
    // upgrade drops a file the new version no longer ships
    let stale = db.install("app", "2", &[("c:/prog/app/app.exe", false), ("sys/vc.dll", true)]).unwrap();
    assert_eq!(stale, ["c:/prog/app/lib/a.dll"]);
    assert_eq!(db.version("app"), Some("2"));
    let miss = db.missing(&|f| f != "c:/prog/app/app.exe");
    assert_eq!(miss["app"], ["c:/prog/app/app.exe"]);
    assert!(!miss.contains_key("other") || miss["other"] != ["x"]);
    db.remove("app").unwrap();
    assert!(db.remove("app").is_err());
    // serialization
    let t = db.to_text();
    assert_eq!(Db::from_text(&t).unwrap().to_text(), t);
    assert_eq!(Db::from_text("P a 1\nF Bad/Path\n").err(), Some(Error::Parse(2)));
    assert_eq!(Db::from_text("junk\n").err(), Some(Error::Parse(1)));
}

#[test]
fn journal_rollback_and_recovery() {
    let mut j = Journal::new();
    j.log(Op::CreateDir("/a".into()));
    j.log(Op::CreateFile("/a/f".into()));
    j.log(Op::Replace { path: "/b".into(), backup: "/b.bak".into() });
    j.log(Op::SetKey("HKLM/x".into()));
    assert!(j.valid() && j.needs_recovery());
    let r = j.rollback_plan();
    assert_eq!(r[0], Undo::DeleteKey("HKLM/x".into()));
    assert_eq!(r[1], Undo::Restore { path: "/b".into(), backup: "/b.bak".into() });
    assert_eq!(r[3], Undo::RemoveDirIfEmpty("/a".into()));
    // crash mid-line: partial last line dropped, earlier ops preserved
    let t = j.to_text();
    let cut = &t[..t.len() - 3];
    let rec = Journal::from_text(cut).unwrap();
    assert_eq!(rec.rollback_plan().len(), 3);
    assert_eq!(Journal::from_text(&t).unwrap(), j);
    assert!(Journal::from_text("X\tfoo\n").is_none());
    let baks = j.commit();
    assert_eq!(baks, ["/b.bak"]);
    assert!(j.rollback_plan().is_empty() && !j.needs_recovery());
    assert!(Journal::from_text(&j.to_text()).unwrap().rollback_plan().is_empty());
}
