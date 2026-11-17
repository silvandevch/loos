//! DirectWrite over real TrueType files, called through the guest-visible vtables like a program would.

use loos_winrt::ttf::Ttf;
use loos_winrt::{Process, Ret};

const SVC: &[u8] = include_bytes!("../../../tests/win/svc.exe");
const FONTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../branding/assets/fonts/");

fn font(name: &str) -> Vec<u8> {
    std::fs::read(format!("{FONTS}{name}")).unwrap()
}

#[test]
fn truetype_parser_reads_metrics_names_cmap_and_outlines() {
    let t = Ttf::parse(font("LiberationSans-Regular.ttf")).expect("parses");
    assert_eq!(t.upm, 2048);
    assert_eq!(t.family(), "Liberation Sans");
    assert_eq!(t.face(), "Regular");
    assert_eq!((t.weight, t.italic), (400, false));
    let a = t.glyph_index('A' as u32);
    assert_ne!(a, 0);
    assert_eq!(t.glyph_index(0x10FFFF), 0, "unmapped code point is .notdef");
    assert_eq!(t.hmetric(a).0, 1366, "Arial-compatible advance of 'A'");
    let o = t.outline(a);
    assert!(!o.ends.is_empty() && o.xy.len() == 2 * o.on.len() && *o.ends.last().unwrap() as usize == o.on.len());
    let (x0, y0, x1, y1) = t.bbox(a).unwrap();
    assert!(x1 > x0 && y1 > y0);
    // A composite glyph (Aacute) flattens to its components' contours.
    let aa = t.glyph_index(0xC1);
    assert!(t.outline(aa).ends.len() > o.ends.len());
    assert!(Ttf::parse(b"not a font".to_vec()).is_none());
    let b = Ttf::parse(font("Inter-Bold.ttf")).unwrap();
    assert_eq!((b.family(), b.weight), ("Inter".to_string(), 700));
}

/// A process with the fonts on its volume and a scratch stack for arguments beyond the fourth.
fn proc() -> (Process, u64) {
    let mut p = Process::create(7, 0, "dw.exe", "", SVC).unwrap();
    for f in ["Inter-Regular.ttf", "Inter-Bold.ttf", "Inter-Italic.ttf", "LiberationSans-Regular.ttf", "LiberationSerif-Regular.ttf"] {
        p.add_file(&format!("C:\\Windows\\Fonts\\{f}"), &font(f));
    }
    let stack = p.mem.map_anywhere(0x10000, 3, &[], "stk").unwrap() + 0x8000;
    p.cur_rsp = stack;
    (p, stack)
}

/// Calls method `slot` of `this` through its vtable: `a` = rdx, r8, r9; `stack` = arguments 5, 6, ...
fn call(p: &mut Process, this: u64, slot: u64, a: [u64; 3], stack: &[u64]) -> Ret {
    let vt = p.mem.read_u64(this).unwrap();
    let stub = p.mem.read_u64(vt + slot * 8).unwrap();
    let nr = p.mem.read_u32(stub + 4).unwrap();
    for (k, v) in stack.iter().enumerate() {
        p.mem.write(p.cur_rsp + 0x28 + 8 * k as u64, &v.to_le_bytes()).unwrap();
    }
    p.syscall(nr, [this, a[0], a[1], a[2]], 0)
}

fn out(p: &mut Process) -> u64 {
    p.mem.map_anywhere(4096, 3, &[], "out").unwrap()
}

fn wide(p: &mut Process, s: &str) -> u64 {
    let b: Vec<u8> = s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    p.mem.map_anywhere(4096, 3, &b, "w").unwrap()
}

fn set_xmm(p: &mut Process, i: usize, f: f32) {
    p.xmm[i][0..4].copy_from_slice(&f.to_bits().to_le_bytes());
}

const IID_FACTORY: [u8; 16] = [0x5a, 0xee, 0x59, 0xb8, 0x38, 0xd8, 0x5b, 0x4b, 0xa2, 0xe8, 0x1a, 0xdc, 0x7d, 0x93, 0xdb, 0x48];
const IID_FACTORY6: [u8; 16] = [0x80, 0x4d, 0x74, 0xf3, 0xf7, 0x21, 0xeb, 0x42, 0xb3, 0x5d, 0x99, 0x5b, 0xc7, 0x2f, 0xc2, 0x23];

fn factory(p: &mut Process) -> u64 {
    let o = out(p);
    let nr = loos_winrt::api::lookup("DWriteCreateFactory").unwrap();
    p.mem.write(o + 64, &IID_FACTORY6).unwrap();
    assert_eq!(p.syscall(nr, [0, o + 64, o, 0], 0), Ret::Value(0x8000_4002), "newer factories are declined");
    p.mem.write(o + 64, &IID_FACTORY).unwrap();
    assert_eq!(p.syscall(nr, [0, o + 64, o, 0], 0), Ret::Value(0));
    p.mem.read_u64(o).unwrap()
}

#[test]
fn system_collection_lists_families_and_substitutes_windows_names() {
    let (mut p, _) = proc();
    let f = factory(&mut p);
    let o = out(&mut p);
    assert_eq!(call(&mut p, f, 3, [o, 0, 0], &[]), Ret::Value(0), "GetSystemFontCollection");
    let coll = p.mem.read_u64(o).unwrap();
    assert_eq!(call(&mut p, coll, 3, [0; 3], &[]), Ret::Value(3), "Inter, Liberation Sans, Liberation Serif");
    // Family 0 is Inter (name order) with three fonts; its name comes through IDWriteLocalizedStrings.
    call(&mut p, coll, 4, [0, o, 0], &[]);
    let fam = p.mem.read_u64(o).unwrap();
    assert_eq!(call(&mut p, fam, 4, [0; 3], &[]), Ret::Value(3));
    call(&mut p, fam, 6, [o, 0, 0], &[]);
    let names = p.mem.read_u64(o).unwrap();
    let buf = out(&mut p);
    assert_eq!(call(&mut p, names, 8, [0, buf, 64], &[]), Ret::Value(0));
    let s: Vec<u16> = p.mem.read_vec(buf, 10).unwrap().chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    assert_eq!(String::from_utf16_lossy(&s), "Inter");
    // "Segoe UI" and "Arial" resolve to the look-alike families; unknown names do not.
    for (name, want) in [("Segoe UI", 0u32), ("arial", 1), ("Times New Roman", 2)] {
        let w = wide(&mut p, name);
        call(&mut p, coll, 5, [w, o, o + 8], &[]);
        assert_eq!((p.mem.read_u32(o).unwrap(), p.mem.read_u32(o + 8).unwrap()), (want, 1), "{name}");
    }
    let w = wide(&mut p, "Wingdings 9");
    call(&mut p, coll, 5, [w, o, o + 8], &[]);
    assert_eq!(p.mem.read_u32(o + 8).unwrap(), 0);
    // Matching: bold upright picks Inter Bold; italic picks Inter Italic.
    call(&mut p, fam, 7, [700, 5, 0], &[o]);
    let bold = p.mem.read_u64(o).unwrap();
    assert_eq!(call(&mut p, bold, 4, [0; 3], &[]), Ret::Value(700));
    call(&mut p, fam, 7, [400, 5, 2], &[o]);
    let it = p.mem.read_u64(o).unwrap();
    assert_eq!(call(&mut p, it, 6, [0; 3], &[]), Ret::Value(2));
    // Reference counting: Release to zero retires the object.
    assert_eq!(call(&mut p, it, 2, [0; 3], &[]), Ret::Value(0));
}

#[test]
fn font_faces_map_characters_and_serve_tables_and_metrics() {
    let (mut p, _) = proc();
    let f = factory(&mut p);
    let o = out(&mut p);
    // GDI interop: a LOGFONTW for "Arial" bold becomes Liberation Sans (only Regular on the volume).
    call(&mut p, f, 17, [o, 0, 0], &[]);
    let gdi = p.mem.read_u64(o).unwrap();
    let mut lf = [0u8; 92];
    lf[16..20].copy_from_slice(&700i32.to_le_bytes());
    for (k, c) in "Arial".encode_utf16().enumerate() {
        lf[28 + 2 * k..30 + 2 * k].copy_from_slice(&c.to_le_bytes());
    }
    let lfp = p.mem.map_anywhere(4096, 3, &lf, "lf").unwrap();
    assert_eq!(call(&mut p, gdi, 3, [lfp, o, 0], &[]), Ret::Value(0));
    let font = p.mem.read_u64(o).unwrap();
    call(&mut p, font, 13, [o, 0, 0], &[]);
    let face = p.mem.read_u64(o).unwrap();
    // Code points to glyph indices, then design metrics.
    let cps = p.mem.map_anywhere(4096, 3, &[0x41, 0, 0, 0, 0x20, 0, 0, 0], "cp").unwrap();
    assert_eq!(call(&mut p, face, 11, [cps, 2, cps + 64], &[]), Ret::Value(0));
    let ga = (p.mem.read_u32(cps + 64).unwrap() & 0xffff) as u16;
    assert_ne!(ga, 0);
    assert_eq!(call(&mut p, face, 10, [cps + 64, 1, cps + 128], &[0]), Ret::Value(0));
    assert_eq!(p.mem.read_u32(cps + 132).unwrap(), 1366, "advance width of 'A'");
    call(&mut p, face, 8, [cps + 256, 0, 0], &[]);
    assert_eq!(p.mem.read_u32(cps + 256).unwrap() & 0xffff, 2048, "designUnitsPerEm");
    // TryGetFontTable('OS/2') points into a guest copy of the file.
    let tag = u32::from_le_bytes(*b"OS/2");
    assert_eq!(call(&mut p, face, 12, [u64::from(tag), o, o + 8], &[o + 16, o + 24]), Ret::Value(0));
    assert_eq!(p.mem.read_u32(o + 24).unwrap(), 1, "exists");
    let (data, len) = (p.mem.read_u64(o).unwrap(), p.mem.read_u32(o + 8).unwrap());
    assert!(len >= 78);
    assert_eq!(p.mem.read_vec(data + 4, 2).unwrap(), [1, 144], "usWeightClass 400 (big endian)");
}

#[test]
fn glyph_run_analysis_rasterises_into_alpha_textures() {
    let (mut p, _) = proc();
    let f = factory(&mut p);
    let o = out(&mut p);
    call(&mut p, f, 3, [o, 0, 0], &[]);
    let coll = p.mem.read_u64(o).unwrap();
    call(&mut p, coll, 4, [1, o, 0], &[]);
    let fam = p.mem.read_u64(o).unwrap();
    call(&mut p, fam, 5, [0, o, 0], &[]);
    let font = p.mem.read_u64(o).unwrap();
    call(&mut p, font, 13, [o, 0, 0], &[]);
    let face = p.mem.read_u64(o).unwrap();
    // DWRITE_GLYPH_RUN for "H" at 32 DIPs.
    let cps = p.mem.map_anywhere(4096, 3, &[0x48, 0, 0, 0], "cp").unwrap();
    call(&mut p, face, 11, [cps, 1, cps + 16], &[]);
    let mut run = [0u8; 48];
    run[0..8].copy_from_slice(&face.to_le_bytes());
    run[8..12].copy_from_slice(&32f32.to_bits().to_le_bytes());
    run[12..16].copy_from_slice(&1u32.to_le_bytes());
    run[16..24].copy_from_slice(&(cps + 16).to_le_bytes());
    let runp = p.mem.map_anywhere(4096, 3, &run, "run").unwrap();
    set_xmm(&mut p, 2, 1.0); // pixelsPerDip
    for (mode, ty, bpp) in [(5u64, 1u64, 3usize), (1, 0, 1)] {
        // CreateGlyphRunAnalysis(run, ppd, transform, mode, NATURAL measuring, x=10, y=40, out)
        assert_eq!(call(&mut p, f, 23, [runp, 0, 0], &[mode, 0, u64::from(10f32.to_bits()), u64::from(40f32.to_bits()), o]), Ret::Value(0));
        let an = p.mem.read_u64(o).unwrap();
        let r = out(&mut p);
        call(&mut p, an, 3, [1 - ty, r, 0], &[]);
        assert_eq!(p.mem.read_vec(r, 16).unwrap(), [0; 16], "the other texture type is empty");
        call(&mut p, an, 3, [ty, r, 0], &[]);
        let rd = |p: &mut Process, k: u64| p.mem.read_u32(r + 4 * k).unwrap() as i32;
        let (l, t, rr, b) = (rd(&mut p, 0), rd(&mut p, 1), rd(&mut p, 2), rd(&mut p, 3));
        assert!(l >= 10 && l <= 14 && b <= 41 && b >= 39 && t < b - 18 && rr - l > 15, "H box {l},{t},{rr},{b}");
        let (w, h) = ((rr - l) as usize, (b - t) as usize);
        let buf = p.mem.map_anywhere(65536, 3, &[], "tex").unwrap();
        assert_eq!(call(&mut p, an, 4, [ty, r, buf], &[2]), Ret::Value(0x8007_007A), "buffer too small");
        assert_eq!(call(&mut p, an, 4, [ty, r, buf], &[(w * h * bpp) as u64]), Ret::Value(0));
        let px = p.mem.read_vec(buf, w * h * bpp).unwrap();
        let full = px.iter().filter(|&&c| c == 255).count();
        assert!(full > w * h * bpp / 8, "stems are solid: {full} of {}", w * h * bpp);
        // The middle of the crossbar row is covered, the gap above it between the stems is empty.
        let at = |x: usize, y: usize| px[(y * w + x) * bpp];
        assert_eq!(at(w / 2, h / 4), 0, "counter above the bar");
        assert!(at(1, h / 2) > 0 && at(w - 2, h / 2) > 0, "stems");
        if bpp == 1 {
            assert!(px.iter().all(|&c| c == 0 || c == 255), "aliased is bi-level");
        }
    }
    // Rendering parameters return FLOATs in xmm0.
    call(&mut p, f, 10, [o, 0, 0], &[]);
    let rp = p.mem.read_u64(o).unwrap();
    assert_eq!(call(&mut p, rp, 3, [0; 3], &[]), Ret::Float(1.8f32.to_bits()));
}

#[test]
fn dwrite_resolves_through_load_library_like_qt() {
    let (mut p, _) = proc();
    let h = p.load_library("C:\\Windows\\system32\\DWrite.dll").expect("built-in dwrite.dll");
    let f = p.proc_address(h, Some("DWriteCreateFactory"), 0);
    assert_ne!(f, 0);
    assert_eq!(p.mem.read_u32(f + 4).unwrap(), loos_winrt::api::lookup("DWriteCreateFactory").unwrap());
}
