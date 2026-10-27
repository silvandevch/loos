use loos_dpkg::control;
use loos_dpkg::deps::{self, Rel};
use loos_dpkg::resolver::*;
use loos_dpkg::version::Version;
use std::cmp::Ordering::*;

fn v(s: &str) -> Version {
    Version::parse(s).unwrap()
}

#[test]
fn version_order() {
    let cases = [
        ("1.0", "1.0", Equal),
        ("1.0~rc1", "1.0", Less),
        ("1.0", "1.0+b1", Less),
        ("1.2", "1.10", Less),
        ("1:0.1", "9.9", Greater),
        ("1.0-1", "1.0-1ubuntu1", Less),
        ("2.0-0", "2.0", Equal),
        ("1.0a", "1.0", Greater),
        ("1.0~~", "1.0~", Less),
        ("1.0+1", "1.0.1", Less),
        ("007", "7", Equal),
        ("1.0-1~bpo1", "1.0-1", Less),
    ];
    for (a, b, o) in cases {
        assert_eq!(v(a).cmp(&v(b)), o, "{a} vs {b}");
        assert_eq!(v(b).cmp(&v(a)), o.reverse(), "{b} vs {a}");
    }
    assert!(Version::parse("abc").is_none() && Version::parse("x:1").is_none() && Version::parse("").is_none());
}

#[test]
fn control_roundtrip() {
    let t = "# c\nPackage: foo\nVersion: 1.0-1\nDescription: short\n long line\n .\n more\n\n\nPackage: bar\nVersion: 2\n";
    let s = control::parse(t).unwrap();
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].get("description").unwrap(), "short\nlong line\n\nmore");
    let again = control::parse(&s[0].to_text()).unwrap();
    assert_eq!(again[0], s[0]);
    assert_eq!(control::parse("bad line\n").unwrap_err(), 1);
    assert_eq!(control::parse(" cont first\n").unwrap_err(), 1);
}

#[test]
fn dep_parse() {
    let d = deps::parse("libc6 (>= 2.34), a | b:any (<< 3~) [amd64], c <!nocheck>").unwrap();
    assert_eq!(d.len(), 3);
    assert_eq!(d[0][0].ver.as_ref().unwrap().0, Rel::Ge);
    assert_eq!(d[1][1].arch.as_deref(), Some("any"));
    assert_eq!(d[1][1].ver.as_ref().unwrap().0, Rel::Lt);
    assert_eq!(d[2][0].name, "c");
    assert!(d[0][0].accepts(&v("2.35-0ubuntu1")) && !d[0][0].accepts(&v("2.33")));
    assert!(deps::parse("foo (?? 1)").is_none());
}

fn pkgs(text: &str) -> Vec<Pkg> {
    control::parse(text).unwrap().iter().map(|s| Pkg::from_stanza(s).unwrap()).collect()
}

fn universe(avail: &str, inst: &str) -> Universe {
    let mut u = Universe::default();
    for p in pkgs(avail) {
        u.available.entry(p.name.clone()).or_default().push(p);
    }
    for p in pkgs(inst) {
        u.installed.insert(p.name.clone(), p);
    }
    u
}

#[test]
fn plan_orders_deps_and_uses_installed_and_newest() {
    let u = universe(
        "Package: app\nVersion: 1\nDepends: libfoo (>= 2), mta | base\n\nPackage: libfoo\nVersion: 1\n\nPackage: libfoo\nVersion: 3\nDepends: libc6\n\nPackage: libc6\nVersion: 2.35\n\nPackage: postfix\nVersion: 1\nProvides: mta\n\nPackage: base\nVersion: 1\n",
        "Package: libc6\nVersion: 2.35\n",
    );
    let p = u.plan_install("app").unwrap();
    let names: Vec<_> = p.install.iter().map(|x| x.0.as_str()).collect();
    assert_eq!(names, ["libfoo", "postfix", "app"]); // libc6 already installed; mta via Provides beats base? first alt wins
    assert_eq!(p.install[0].1, v("3"));
    assert!(p.upgrade.is_empty());
}

#[test]
fn plan_errors_and_conflicts_and_upgrade() {
    let u = universe("Package: a\nVersion: 1\nDepends: ghost\n\nPackage: x\nVersion: 2\nConflicts: y\n\nPackage: y\nVersion: 1\n", "Package: y\nVersion: 1\n");
    assert!(matches!(u.plan_install("a"), Err(Error::Unsatisfiable { by, .. }) if by == "a"));
    assert_eq!(u.plan_install("zzz"), Err(Error::NotFound("zzz".into())));
    assert!(matches!(u.plan_install("x"), Err(Error::Conflict(..))));
    let u = universe("Package: y\nVersion: 2\n", "Package: y\nVersion: 1\n");
    let p = u.plan_install("y").unwrap();
    assert_eq!(p.upgrade, ["y"]);
    // already current -> nothing to do
    let u = universe("Package: y\nVersion: 1\n", "Package: y\nVersion: 1\n");
    assert!(u.plan_install("y").unwrap().install.is_empty());
}

#[test]
fn cycles_and_reverse_deps() {
    let u = universe("Package: a\nVersion: 1\nDepends: b\n\nPackage: b\nVersion: 1\nDepends: a\n", "");
    assert_eq!(u.plan_install("a").unwrap().install.len(), 2);
    let u = universe("", "Package: base\nVersion: 1\n\nPackage: mid\nVersion: 1\nDepends: base\n\nPackage: top\nVersion: 1\nDepends: mid | other\n\nPackage: solo\nVersion: 1\n");
    assert_eq!(u.reverse_deps("base"), ["mid", "top"]);
    assert!(u.reverse_deps("solo").is_empty());
}
