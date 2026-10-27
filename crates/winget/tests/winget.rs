use loos_winget::index::Index;
use loos_winget::manifest::*;
use loos_winget::yaml::{self, Value};
use std::cmp::Ordering::*;

const SHA: &str = "A1B2C3D4E5F60718293A4B5C6D7E8F90A1B2C3D4E5F60718293A4B5C6D7E8F90";

fn manifest(ver: &str) -> String {
    format!(
        "# comment\n---\nPackageIdentifier: Contoso.Tool\nPackageVersion: {ver}\nPackageName: Contoso Tool\nPublisher: Contoso\nMoniker: tool\nTags:\n- cli\n- dev\nInstallerType: exe\nInstallerSwitches:\n  Silent: /S\nInstallers:\n- Architecture: x86\n  InstallerUrl: https://e.com/x86.exe\n  InstallerSha256: {SHA}\n- Architecture: x64\n  InstallerUrl: \"https://e.com/x64.exe\"\n  InstallerSha256: {SHA}\n  Scope: machine\n- Architecture: x64\n  InstallerUrl: https://e.com/x64u.exe\n  InstallerSha256: {SHA}\n  Scope: user\n  InstallerType: msi\n"
    )
}

#[test]
fn yaml_basics() {
    let v = yaml::parse("a: 1\nb:\n  - x\n  - 'it''s'\n  - \"q\\n\"\nc: [1, two]\nd: ~\ne: |\n  line1\n    line2\n\n  line3\nf: >-\n  folded\n  text\ng:\n- k: v\n  m: n\n- z\n").unwrap();
    assert_eq!(v.get("a").unwrap().as_str(), Some("1"));
    let b = v.get("b").unwrap().as_seq().unwrap();
    assert_eq!((b[1].as_str(), b[2].as_str()), (Some("it's"), Some("q\n")));
    assert_eq!(v.get("c").unwrap().as_seq().unwrap().len(), 2);
    assert_eq!(v.get("d"), Some(&Value::Null));
    assert_eq!(v.get("e").unwrap().as_str(), Some("line1\n  line2\n\nline3\n"));
    assert_eq!(v.get("f").unwrap().as_str(), Some("folded text"));
    let g = v.get("g").unwrap().as_seq().unwrap();
    assert_eq!(g[0].get("m").unwrap().as_str(), Some("n"));
    assert_eq!(g[1].as_str(), Some("z"));
}

#[test]
fn yaml_rejects() {
    assert!(yaml::parse("a: 1\na: 2\n").is_err());
    assert!(yaml::parse("a: &x 1\n").is_err());
    assert!(yaml::parse("a: 1\n---\nb: 2\n").is_err());
    assert!(yaml::parse("a: {b: 1}\n").is_err());
    assert!(yaml::parse("a:\n\tb: 1\n").is_err());
    assert!(yaml::parse("a: 1\n  b: 2\n").is_err());
    assert_eq!(yaml::parse("a: \"x # not comment\" # real\n").unwrap().get("a").unwrap().as_str(), Some("x # not comment"));
}

#[test]
fn manifest_and_installer_selection() {
    let m = Manifest::parse(&manifest("1.2.0")).unwrap();
    assert_eq!((m.id.as_str(), m.moniker.as_deref(), m.tags.len()), ("Contoso.Tool", Some("tool"), 2));
    assert_eq!(m.installers[0].sha256, SHA.to_lowercase());
    assert_eq!(m.installers[0].silent.as_deref(), Some("/S"));
    assert_eq!(m.installers[1].kind, "exe");
    assert_eq!(m.select_installer(Arch::X64, None).unwrap().url, "https://e.com/x64.exe");
    assert_eq!(m.select_installer(Arch::X64, Some("user")).unwrap().kind, "msi");
    assert_eq!(m.select_installer(Arch::X86, Some("user")).unwrap().url, "https://e.com/x86.exe");
    assert!(m.select_installer(Arch::Arm, None).is_none());
}

#[test]
fn manifest_validation() {
    let bad_hash = manifest("1").replace(SHA, "abc");
    assert_eq!(Manifest::parse(&bad_hash), Err(Error::Field("InstallerSha256")));
    let http = manifest("1").replace("https://e.com/x86", "http://e.com/x86");
    assert!(Manifest::parse(&http).is_err());
    assert!(Manifest::parse("PackageVersion: 1\n").is_err());
    let multi = Manifest::parse_multi(
        "PackageIdentifier: A.B\nPackageVersion: 3\n",
        &format!("Installers:\n- Architecture: neutral\n  InstallerType: msix\n  InstallerUrl: https://x/y.msix\n  InstallerSha256: {SHA}\n"),
        "PackageName: AB\nPublisher: A\n",
    )
    .unwrap();
    assert_eq!((multi.name.as_str(), multi.installers.len()), ("AB", 1));
}

#[test]
fn versions_and_index() {
    assert_eq!(cmp_version("1.10", "1.9"), Greater);
    assert_eq!(cmp_version("1.0", "1"), Equal);
    assert_eq!(cmp_version("2.0.0-beta", "2.0.0"), Greater); // string "beta" > "0": acceptable ordering for unknown suffixes
    let mut ix = Index::default();
    for v in ["1.2.0", "1.10.0", "1.9.0"] {
        ix.add(Manifest::parse(&manifest(v)).unwrap());
    }
    ix.add(Manifest::parse(&manifest("1.9.0")).unwrap()); // dedup
    assert_eq!(ix.latest("contoso.tool").unwrap().version, "1.10.0");
    assert!(ix.version("Contoso.Tool", "1.9.0").is_some());
    assert_eq!(ix.search("tool").len(), 1);
    assert_eq!(ix.search("cli").len(), 1);
    assert!(ix.search("nothing").is_empty());
    assert_eq!(ix.upgrades(&[("Contoso.Tool".into(), "1.2.0".into()), ("Other.App".into(), "1".into())]), vec![("Contoso.Tool".to_string(), "1.10.0".to_string())]);
    assert!(ix.upgrades(&[("Contoso.Tool".into(), "1.10".into())]).is_empty());
}
