use loos_msi::database::{decode_name, encode_name, Cell, Database};
use loos_msi::plan::{CaKind, Plan};
use std::collections::BTreeMap;

// ---------------------------------------------------------------- CFB writer (test fixture)
const FREE: u32 = 0xFFFF_FFFF;
const END: u32 = 0xFFFF_FFFE;

fn cfb(streams: &[(Vec<u16>, Vec<u8>)]) -> Vec<u8> {
    let (small, big): (Vec<_>, Vec<_>) = streams.iter().enumerate().partition(|(_, (_, d))| d.len() < 4096);
    // mini stream
    let mut mini = Vec::new();
    let mut minifat: Vec<u32> = Vec::new();
    let mut start_of = BTreeMap::new();
    for (i, (_, d)) in &small {
        let n = d.len().div_ceil(64).max(1);
        start_of.insert(*i, minifat.len() as u32);
        for k in 0..n {
            minifat.push(if k + 1 == n { END } else { (minifat.len() + 1) as u32 });
        }
        mini.extend_from_slice(d);
        mini.resize(minifat.len() * 64, 0);
    }
    let nd = (streams.len() + 1).div_ceil(4);
    let nm = minifat.len().div_ceil(128);
    let nms = mini.len().div_ceil(512);
    let mut fat = vec![FREE; 128];
    fat[0] = 0xFFFF_FFFD;
    let mut next = 1usize;
    let mut alloc = |n: usize, fat: &mut Vec<u32>| -> u32 {
        let s = next;
        for k in 0..n {
            fat[s + k] = if k + 1 == n { END } else { (s + k + 1) as u32 };
        }
        next += n;
        s as u32
    };
    let dir_start = alloc(nd, &mut fat);
    let mf_start = if nm > 0 { alloc(nm, &mut fat) } else { END };
    let ms_start = if nms > 0 { alloc(nms, &mut fat) } else { END };
    let mut big_start = BTreeMap::new();
    for (i, (_, d)) in &big {
        big_start.insert(*i, alloc(d.len().div_ceil(512), &mut fat));
    }
    assert!(next <= 128, "fixture too large");
    let mut img = vec![0u8; 512 * (next + 1)];
    img[..8].copy_from_slice(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
    img[24..26].copy_from_slice(&0x3Eu16.to_le_bytes());
    img[26..28].copy_from_slice(&3u16.to_le_bytes());
    img[28..30].copy_from_slice(&0xFFFEu16.to_le_bytes());
    img[30..32].copy_from_slice(&9u16.to_le_bytes());
    img[32..34].copy_from_slice(&6u16.to_le_bytes());
    img[44..48].copy_from_slice(&1u32.to_le_bytes());
    img[48..52].copy_from_slice(&dir_start.to_le_bytes());
    img[56..60].copy_from_slice(&4096u32.to_le_bytes());
    img[60..64].copy_from_slice(&mf_start.to_le_bytes());
    img[64..68].copy_from_slice(&(nm as u32).to_le_bytes());
    img[68..72].copy_from_slice(&END.to_le_bytes());
    for i in 0..109 {
        img[76 + i * 4..80 + i * 4].copy_from_slice(&(if i == 0 { 0 } else { FREE }).to_le_bytes());
    }
    let put = |img: &mut Vec<u8>, sec: u32, data: &[u8]| {
        let o = 512 * (sec as usize + 1);
        img[o..o + data.len()].copy_from_slice(data);
    };
    let fat_bytes: Vec<u8> = fat.iter().flat_map(|v| v.to_le_bytes()).collect();
    put(&mut img, 0, &fat_bytes);
    // directory
    let mut dir = vec![0u8; nd * 512];
    let mut entry = |idx: usize, name: &[u16], kind: u8, start: u32, size: u64, right: u32, child: u32| {
        let e = &mut dir[idx * 128..(idx + 1) * 128];
        for (k, u) in name.iter().enumerate() {
            e[k * 2..k * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }
        e[64..66].copy_from_slice(&(((name.len() + 1) * 2) as u16).to_le_bytes());
        e[66] = kind;
        e[67] = 1;
        e[68..72].copy_from_slice(&FREE.to_le_bytes());
        e[72..76].copy_from_slice(&right.to_le_bytes());
        e[76..80].copy_from_slice(&child.to_le_bytes());
        e[116..120].copy_from_slice(&start.to_le_bytes());
        e[120..128].copy_from_slice(&size.to_le_bytes());
    };
    entry(0, &"Root Entry".encode_utf16().collect::<Vec<_>>(), 5, ms_start, mini.len() as u64, FREE, 1);
    for (i, (n, d)) in streams.iter().enumerate() {
        let start = start_of.get(&i).or(big_start.get(&i)).copied().unwrap();
        entry(i + 1, n, 2, start, d.len() as u64, if i + 1 < streams.len() { (i + 2) as u32 } else { FREE }, FREE);
    }
    put(&mut img, dir_start, &dir);
    if nm > 0 {
        let mut mf: Vec<u8> = minifat.iter().flat_map(|v| v.to_le_bytes()).collect();
        mf.resize(nm * 512, 0xFF);
        put(&mut img, mf_start, &mf);
    }
    if nms > 0 {
        put(&mut img, ms_start, &mini);
    }
    for (i, (_, d)) in &big {
        put(&mut img, big_start[i], d);
    }
    img
}

// ---------------------------------------------------------------- MSI writer
const S: u16 = 0x0900; // string
const I2: u16 = 0x0102;
const I4: u16 = 0x0104;

fn s(v: &str) -> Cell {
    if v.is_empty() { Cell::Null } else { Cell::Str(v.into()) }
}
fn n(v: i32) -> Cell {
    Cell::Int(v)
}

struct Pool(Vec<String>);
impl Pool {
    fn id(&mut self, v: &str) -> u16 {
        if let Some(p) = self.0.iter().position(|x| x == v) {
            return p as u16 + 1;
        }
        self.0.push(v.to_string());
        self.0.len() as u16
    }
}

fn enc(cols: &[(&str, u16)], rows: &[Vec<Cell>], pool: &mut Pool) -> Vec<u8> {
    let mut out = Vec::new();
    for (c, (_, ty)) in cols.iter().enumerate() {
        for r in rows {
            match (&r[c], *ty) {
                (Cell::Null, t) if t & 0x800 != 0 => out.extend(0u16.to_le_bytes()),
                (Cell::Null, I4) => out.extend(0u32.to_le_bytes()),
                (Cell::Null, _) => out.extend(0u16.to_le_bytes()),
                (Cell::Str(x), _) => out.extend(pool.id(x).to_le_bytes()),
                (Cell::Int(v), I4) => out.extend(((*v as u32) ^ 0x8000_0000).to_le_bytes()),
                (Cell::Int(v), _) => out.extend((((*v as i16) as u16) ^ 0x8000).to_le_bytes()),
            }
        }
    }
    out
}

type T<'a> = (&'a str, Vec<(&'a str, u16)>, Vec<Vec<Cell>>);

fn build_msi(tables: &[T], cab: Option<&[u8]>) -> Vec<u8> {
    let mut pool = Pool(vec![]);
    let mut streams: Vec<(Vec<u16>, Vec<u8>)> = vec![];
    let mut col_rows = vec![];
    for (name, cols, rows) in tables {
        streams.push((encode_name(name, true), enc(cols, rows, &mut pool)));
        for (i, (cn, ty)) in cols.iter().enumerate() {
            col_rows.push(vec![s(name), n(i as i32 + 1), s(cn), n(*ty as i32)]);
        }
    }
    let tn: Vec<Vec<Cell>> = tables.iter().map(|t| vec![s(t.0)]).collect();
    streams.push((encode_name("_Tables", true), enc(&[("Name", S)], &tn, &mut pool)));
    streams.push((encode_name("_Columns", true), enc(&[("Table", S), ("Number", I2), ("Name", S), ("Type", I2)], &col_rows, &mut pool)));
    let mut sp = 0u32.to_le_bytes().to_vec(); // codepage 0, narrow indices
    let mut sd = Vec::new();
    for x in &pool.0 {
        sp.extend((x.len() as u16).to_le_bytes());
        sp.extend(1u16.to_le_bytes());
        sd.extend(x.as_bytes());
    }
    streams.push((encode_name("_StringPool", true), sp));
    streams.push((encode_name("_StringData", true), sd));
    if let Some(c) = cab {
        streams.push((encode_name("#cab1.cab", false), c.to_vec()));
    }
    cfb(&streams)
}

fn sample() -> Vec<u8> {
    let tables: Vec<T> = vec![
        ("Directory", vec![("Directory", S), ("Directory_Parent", S), ("DefaultDir", S)], vec![
            vec![s("TARGETDIR"), s(""), s("SourceDir")],
            vec![s("ProgramFilesFolder"), s("TARGETDIR"), s(".")],
            vec![s("INSTALLDIR"), s("ProgramFilesFolder"), s("MYAPP~1|My App")],
            vec![s("BIN"), s("INSTALLDIR"), s("bin")],
        ]),
        ("Component", vec![("Component", S), ("ComponentId", S), ("Directory_", S), ("Attributes", I2), ("Condition", S), ("KeyPath", S)], vec![
            vec![s("C_Main"), s("{AAAA}"), s("INSTALLDIR"), n(0), s(""), s("F_Exe")],
            vec![s("C_Lib"), s("{BBBB}"), s("BIN"), n(0), s(""), s("F_Dll")],
        ]),
        ("File", vec![("File", S), ("Component_", S), ("FileName", S), ("FileSize", I4), ("Version", S), ("Language", S), ("Attributes", I2), ("Sequence", I2)], vec![
            vec![s("F_Dll"), s("C_Lib"), s("LIB~1.DLL|helper.dll"), n(3000), s(""), s(""), n(0), n(2)],
            vec![s("F_Exe"), s("C_Main"), s("APP~1.EXE|app.exe"), n(70000), s("1.0.0.0"), s(""), n(0), n(1)],
        ]),
        ("Feature", vec![("Feature", S), ("Feature_Parent", S), ("Title", S), ("Level", I2)], vec![
            vec![s("Main"), s(""), s("Everything"), n(1)],
            vec![s("Extra"), s("Main"), s("Extras"), n(3)],
        ]),
        ("FeatureComponents", vec![("Feature_", S), ("Component_", S)], vec![vec![s("Main"), s("C_Main")], vec![s("Extra"), s("C_Lib")]]),
        ("CustomAction", vec![("Action", S), ("Type", I2), ("Source", S), ("Target", S)], vec![
            vec![s("RunSetup"), n(0x0C02), s("F_Exe"), s("/configure")],
            vec![s("CallDll"), n(1), s("Bin1"), s("Entry")],
            vec![s("SetDir"), n(51), s("INSTALLDIR"), s("[ProgramFilesFolder]X")],
            vec![s("Vb"), n(6), s("Bin2"), s("MsgBox 1")],
        ]),
        ("Media", vec![("DiskId", I2), ("LastSequence", I2), ("DiskPrompt", S), ("Cabinet", S), ("VolumeLabel", S), ("Source", S)], vec![vec![n(1), n(2), s(""), s("#cab1.cab"), s(""), s("")]]),
        ("Property", vec![("Property", S), ("Value", S)], vec![
            vec![s("ProductName"), s("My App")],
            vec![s("ProductVersion"), s("1.2.3")],
            vec![s("ProductCode"), s("{11111111-2222-3333-4444-555555555555}")],
            vec![s("Manufacturer"), s("Contoso")],
        ]),
        ("AdminExecuteSequence", vec![("Action", S), ("Condition", S), ("Sequence", I2)], vec![]), // empty table: no data stream emitted
    ];
    let cabdata = vec![7u8; 6000]; // > 4096 → regular sectors
    build_msi(&tables, Some(&cabdata))
}

#[test]
fn name_codec_roundtrip() {
    for nm in ["File", "Property", "_StringPool", "FeatureComponents", "A", "AB-C"] {
        let e = encode_name(nm, true);
        assert_eq!(decode_name(&e), (true, nm.to_string()), "{nm}");
    }
}

#[test]
fn reads_database() {
    let db = Database::open(&sample()).unwrap();
    assert_eq!(db.tables.len(), 9);
    assert_eq!(db.property("ProductVersion").as_deref(), Some("1.2.3"));
    let f = db.table("File").unwrap();
    assert_eq!(f.rows.len(), 2);
    assert_eq!(f.get(1, "FileSize"), Some(&Cell::Int(70000)));
    assert_eq!(f.get(0, "Language"), Some(&Cell::Null));
    assert!(db.table("AdminExecuteSequence").unwrap().rows.is_empty());
    assert_eq!(db.blobs["#cab1.cab"].len(), 6000);
}

#[test]
fn plan_files_features_and_audit() {
    let db = Database::open(&sample()).unwrap();
    let mut props = BTreeMap::new();
    let p = Plan::build(&db, &props).unwrap();
    assert_eq!((p.name.as_str(), p.version.as_str(), p.manufacturer.as_str()), ("My App", "1.2.3", "Contoso"));
    let paths: Vec<_> = p.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["ProgramFilesFolder/My App/app.exe", "ProgramFilesFolder/My App/bin/helper.dll"]); // sequence order
    assert_eq!(p.cabinets, ["#cab1.cab"]);
    // INSTALLDIR override replaces the whole prefix
    props.insert("INSTALLDIR".into(), "D:/Apps/Mine/".into());
    let p2 = Plan::build(&db, &props).unwrap();
    assert_eq!(p2.files[1].path, "D:/Apps/Mine/bin/helper.dll");
    // features / size by level
    assert_eq!(p.features.iter().find(|f| f.name == "Extra").unwrap().parent.as_deref(), Some("Main"));
    assert_eq!(p.size_for_level(1), 70000);
    assert_eq!(p.size_for_level(3), 73000);
    // custom action audit
    let kinds: Vec<_> = p.custom_actions.iter().map(|c| c.kind).collect();
    assert_eq!(kinds, [CaKind::Exe, CaKind::Dll, CaKind::SetValue, CaKind::Script]);
    assert_eq!(p.risky_actions().len(), 3);
    assert!(p.custom_actions[0].elevated && !p.custom_actions[1].elevated);
}

#[test]
fn plan_rejects_traversal_and_cycles() {
    let mk = |dirs: Vec<Vec<Cell>>, fname: &str| {
        let tables: Vec<T> = vec![
            ("Directory", vec![("Directory", S), ("Directory_Parent", S), ("DefaultDir", S)], dirs),
            ("Component", vec![("Component", S), ("ComponentId", S), ("Directory_", S), ("Attributes", I2), ("Condition", S), ("KeyPath", S)], vec![vec![s("C"), s("{1}"), s("D"), n(0), s(""), s("")]]),
            ("File", vec![("File", S), ("Component_", S), ("FileName", S), ("FileSize", I4), ("Version", S), ("Language", S), ("Attributes", I2), ("Sequence", I2)], vec![vec![s("F"), s("C"), s(fname), n(1), s(""), s(""), n(0), n(1)]]),
        ];
        Database::open(&build_msi(&tables, None)).unwrap()
    };
    let ok = vec![vec![s("TARGETDIR"), s(""), s("SourceDir")], vec![s("D"), s("TARGETDIR"), s("d")]];
    assert!(Plan::build(&mk(ok.clone(), "a.txt"), &BTreeMap::new()).is_ok());
    assert!(Plan::build(&mk(ok.clone(), "..|.."), &BTreeMap::new()).is_err());
    let bad_dir = vec![vec![s("TARGETDIR"), s(""), s("SourceDir")], vec![s("D"), s("TARGETDIR"), s("..")]];
    assert!(Plan::build(&mk(bad_dir, "a"), &BTreeMap::new()).is_err());
    let cycle = vec![vec![s("D"), s("E"), s("x")], vec![s("E"), s("D"), s("y")]];
    assert!(Plan::build(&mk(cycle, "a"), &BTreeMap::new()).is_err());
}

#[test]
fn corrupt_images_fail_cleanly() {
    let good = sample();
    assert!(Database::open(&good[..100]).is_err());
    assert!(Database::open(b"not an msi, definitely not an msi!!!").is_err());
    // zero out the FAT sector -> chains break
    let mut bad = good.clone();
    bad[512..1024].fill(0);
    assert!(Database::open(&bad).is_err());
    // flip bits throughout: must never panic
    for i in (512..good.len()).step_by(37) {
        let mut b = good.clone();
        b[i] ^= 0xA5;
        let _ = Database::open(&b);
    }
}

#[test]
fn subst_works() {
    let mut p = BTreeMap::new();
    p.insert("A".to_string(), "x".to_string());
    assert_eq!(loos_msi::plan::subst("[A]/[B]/[\\[]/[unterminated", &p), "x//[/[unterminated");
}
