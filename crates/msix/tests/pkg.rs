use loos_msix::manifest::Manifest;
use loos_msix::zip::{self, Error as ZE};
use loos_msix::{xml, Error, Package};

fn le16(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}
fn le32(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

/// Build a zip: (name, data, deflate?)
fn make_zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cd = Vec::new();
    for (name, data, defl) in files {
        let (method, body) = if *defl { (8u16, miniz_oxide::deflate::compress_to_vec(data, 6)) } else { (0, data.to_vec()) };
        let off = out.len() as u32;
        let crc = zip::crc32(data);
        out.extend([0x50, 0x4b, 3, 4]);
        out.extend(le16(20));
        out.extend(le16(0));
        out.extend(le16(method));
        out.extend([0; 4]);
        out.extend(le32(crc));
        out.extend(le32(body.len() as u32));
        out.extend(le32(data.len() as u32));
        out.extend(le16(name.len() as u16));
        out.extend(le16(0));
        out.extend(name.as_bytes());
        out.extend(&body);
        cd.extend([0x50, 0x4b, 1, 2]);
        cd.extend(le16(20));
        cd.extend(le16(20));
        cd.extend(le16(0));
        cd.extend(le16(method));
        cd.extend([0; 4]);
        cd.extend(le32(crc));
        cd.extend(le32(body.len() as u32));
        cd.extend(le32(data.len() as u32));
        cd.extend(le16(name.len() as u16));
        cd.extend([0; 12]);
        cd.extend(le32(off));
        cd.extend(name.as_bytes());
    }
    let cd_off = out.len() as u32;
    out.extend(&cd);
    out.extend([0x50, 0x4b, 5, 6, 0, 0, 0, 0]);
    out.extend(le16(files.len() as u16));
    out.extend(le16(files.len() as u16));
    out.extend(le32(cd.len() as u32));
    out.extend(le32(cd_off));
    out.extend(le16(0));
    out
}

const MANIFEST: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<!-- c --><Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10" xmlns:rescap="urn:x">
  <Identity Name="Contoso.App" Publisher="CN=Contoso &amp; Co, O=C" Version="1.2.3.4" ProcessorArchitecture="X64"/>
  <Properties><DisplayName>Con&#x74;oso</DisplayName><PublisherDisplayName><![CDATA[Contoso <Inc>]]></PublisherDisplayName></Properties>
  <Dependencies><TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.17763.0" MaxVersionTested="10.0.22000.0"/>
    <PackageDependency Name="Microsoft.VCLibs.140.00" MinVersion="14.0.0.0"/></Dependencies>
  <Applications><Application Id="App" Executable="bin\app.exe" EntryPoint="Windows.FullTrustApplication"/></Applications>
  <Capabilities><Capability Name="internetClient"/><rescap:Capability Name="runFullTrust"/></Capabilities>
</Package>"#;

#[test]
fn xml_parser() {
    let n = xml::parse("<a x='1' y=\"&lt;\"><b>t&amp;</b><!--c--><c/></a>").unwrap();
    assert_eq!((n.attr("y"), n.kid("b").unwrap().text.as_str(), n.children.len()), (Some("<"), "t&", 2));
    for bad in ["<a>", "<a></b>", "<a x=1/>", "<!DOCTYPE a><a/>", "<a/><b/>", "<a x='1' x='2'/>", "<a>&nope;</a>"] {
        assert!(xml::parse(bad).is_err(), "{bad}");
    }
    let deep = "<a>".repeat(100) + &"</a>".repeat(100);
    assert!(xml::parse(&deep).is_err());
}

#[test]
fn manifest_parse() {
    let m = Manifest::parse(MANIFEST).unwrap();
    assert_eq!((m.identity.name.as_str(), m.identity.arch.as_str(), m.identity.publisher.as_str()), ("Contoso.App", "x64", "CN=Contoso & Co, O=C"));
    assert_eq!((m.display_name.as_str(), m.publisher_display.as_str()), ("Contoso", "Contoso <Inc>"));
    assert_eq!(m.applications[0].executable.as_deref(), Some("bin/app.exe"));
    assert!(m.full_trust() && m.runs_on_desktop());
    assert_eq!(m.dependencies[0].0, "Microsoft.VCLibs.140.00");
    assert!(Manifest::parse(&MANIFEST.replace("1.2.3.4", "1.2")).is_err());
    assert!(Manifest::parse(&MANIFEST.replace("Contoso.App", "bad name")).is_err());
    assert!(Manifest::parse("<Foo/>").is_err());
}

#[test]
fn package_open_and_payload() {
    let big = vec![b'A'; 100_000];
    let z = make_zip(&[
        ("AppxManifest.xml", MANIFEST.as_bytes(), true),
        ("[Content_Types].xml", b"<Types/>", false),
        ("AppxSignature.p7x", b"sig", false),
        ("AppxMetadata/AppxBlockMap.xml", b"<B/>", false),
        ("bin/app.exe", &big, true),
        ("assets/", b"", false),
    ]);
    let p = Package::open(&z).unwrap();
    assert!(p.signed);
    let names: Vec<_> = p.payload().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["AppxManifest.xml", "bin/app.exe"]);
    assert_eq!(p.archive.read("BIN/APP.EXE").unwrap(), big);
    assert_eq!(p.install_dir(), "Contoso.App_1.2.3.4_x64__CN_Contoso___Co__O_C");
    // executable listed in the manifest but absent
    let z2 = make_zip(&[("AppxManifest.xml", MANIFEST.as_bytes(), false)]);
    assert!(matches!(Package::open(&z2), Err(Error::Manifest(_))));
    let z3 = make_zip(&[("x", b"1", false)]);
    assert!(matches!(Package::open(&z3), Err(Error::NoManifest)));
}

#[test]
fn zip_defenses() {
    for bad in ["../evil", "/abs", "a/../../b", "c:/x", "a\\b"] {
        assert_eq!(zip::Archive::open(&make_zip(&[(bad, b"x", false)])).err(), Some(ZE::UnsafePath), "{bad}");
    }
    assert!(zip::safe_name("a/b.txt") && !zip::safe_name(""));
    // corrupt data: flip a byte in the stored body -> CRC mismatch
    let mut z = make_zip(&[("f", b"hello world", false)]);
    z[31] ^= 1;
    let a = zip::Archive::open(&z).unwrap();
    assert_eq!(a.read("f").err(), Some(ZE::BadData));
    // declared size smaller than real inflate output -> rejected by the size limit
    let mut z = make_zip(&[("f", &vec![b'z'; 5000], true)]);
    let cd = z.windows(4).rposition(|w| w == [0x50, 0x4b, 1, 2]).unwrap();
    z[cd + 24..cd + 28].copy_from_slice(&10u32.to_le_bytes());
    assert_eq!(zip::Archive::open(&z).unwrap().read("f").err(), Some(ZE::BadData));
    assert_eq!(zip::Archive::open(b"not a zip at all, no eocd here...").err(), Some(ZE::NoEocd));
    assert_eq!(zip::Archive::open(&z[..z.len() - 5]).err(), Some(ZE::NoEocd));
}
