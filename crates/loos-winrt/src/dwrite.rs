//! DirectWrite (`dwrite.dll`) over the TrueType files in `C:\Windows\Fonts`: the factory, the system font
//! collection (families, fonts, faces, localized names), font tables, glyph metrics, GDI interop and glyph-run
//! rasterisation into alpha textures (the shell's exact-area scanline rasteriser). Enough for toolkits that build
//! their own text stack on DirectWrite (Qt 6's font database and font engine); text layout objects
//! (`IDWriteTextFormat`/`IDWriteTextLayout`) are not provided yet and answer `E_NOTIMPL`.

use crate::com::{guid, ComTable, E_INVALIDARG, E_NOTIMPL, E_NOT_SUFFICIENT_BUFFER, E_POINTER, S_OK};
use crate::ttf::{Outline, Ttf};
use crate::{Process, Ret};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use loos_lin::mem::PROT_READ;

/// `DWRITE_E_NOFONT`.
const DWRITE_E_NOFONT: u64 = 0x8898_5002;

// Interface ids (vtable families) and their slot counts (IUnknown included).
const FACTORY: u16 = 0;
const COLLECTION: u16 = 1;
const FAMILY: u16 = 2;
const LIST: u16 = 3;
const FONT: u16 = 4;
const FACE: u16 = 5;
const STRINGS: u16 = 6;
const ANALYSIS: u16 = 7;
const INTEROP: u16 = 8;
const FILE: u16 = 9;
const PARAMS: u16 = 10;
const LOADER: u16 = 11;
/// Interface ids below this belong to DirectWrite.
pub(crate) const IFACES: u16 = 12;
const SLOT_COUNT: [u32; IFACES as usize] = [24, 7, 9, 6, 14, 18, 9, 6, 8, 6, 8, 7];

const IID_FACTORY: [u8; 16] = guid(0xb859ee5a, 0xd838, 0x4b5b, [0xa2, 0xe8, 0x1a, 0xdc, 0x7d, 0x93, 0xdb, 0x48]);
const IID_COLLECTION: [u8; 16] = guid(0xa84cee02, 0x3eea, 0x4eee, [0xa8, 0x27, 0x87, 0xc1, 0xa0, 0x2a, 0x0f, 0xcc]);
const IID_FAMILY: [u8; 16] = guid(0xda20d8ef, 0x812a, 0x4c43, [0x98, 0x02, 0x62, 0xec, 0x4a, 0xbd, 0x7a, 0xdd]);
const IID_LIST: [u8; 16] = guid(0x1a0d8438, 0x1d97, 0x4ec1, [0xae, 0xf9, 0xa2, 0xfb, 0x86, 0xed, 0x6a, 0xcb]);
const IID_FONT: [u8; 16] = guid(0xacd16696, 0x8c14, 0x4f5d, [0x87, 0x7e, 0xfe, 0x3f, 0xc1, 0xd3, 0x27, 0x37]);
const IID_FACE: [u8; 16] = guid(0x5f49804d, 0x7024, 0x4d43, [0xbf, 0xa9, 0xd2, 0x59, 0x84, 0xf5, 0x38, 0x49]);
const IID_STRINGS: [u8; 16] = guid(0x08256209, 0x099a, 0x4b34, [0xb8, 0x6d, 0xc2, 0x2b, 0x11, 0x0e, 0x77, 0x71]);
const IID_ANALYSIS: [u8; 16] = guid(0x7d97dbf7, 0xe085, 0x42d4, [0x81, 0xe3, 0x6a, 0x88, 0x3b, 0xde, 0xd1, 0x18]);
const IID_INTEROP: [u8; 16] = guid(0x1edd9491, 0x9853, 0x4299, [0x89, 0x8f, 0x64, 0x32, 0x98, 0x3b, 0x6f, 0x3a]);
const IID_FILE: [u8; 16] = guid(0x739d886a, 0xcef5, 0x47dc, [0x87, 0x69, 0x1a, 0x8b, 0x41, 0xbe, 0xbb, 0xb0]);
const IID_PARAMS: [u8; 16] = guid(0x2f0da53a, 0x2add, 0x47cd, [0x82, 0xee, 0xd9, 0xec, 0x34, 0x68, 0x8e, 0x75]);
const IID_FILE_LOADER: [u8; 16] = guid(0x727cad4e, 0xd6af, 0x4c9e, [0x8a, 0x08, 0xd6, 0x95, 0xb1, 0x1c, 0xaa, 0x49]);
const IID_LOCAL_LOADER: [u8; 16] = guid(0xb2d9f3ec, 0xc9fe, 0x4a11, [0xa2, 0xec, 0xd8, 0x62, 0x08, 0xf7, 0xc0, 0xa2]);

/// Families Windows programs ask for by name, answered by metric-compatible or look-alike fonts on the volume
/// (the substitution Windows itself applies through `FontSubstitutes`).
const SUBSTITUTES: &[(&str, &[&str])] = &[
    ("segoe ui", &["inter", "liberation sans"]),
    ("segoe ui variable", &["inter", "liberation sans"]),
    ("ms shell dlg", &["inter", "liberation sans"]),
    ("ms shell dlg 2", &["inter", "liberation sans"]),
    ("tahoma", &["inter", "liberation sans"]),
    ("arial", &["liberation sans"]),
    ("helvetica", &["liberation sans"]),
    ("calibri", &["carlito", "liberation sans"]),
    ("times new roman", &["liberation serif"]),
    ("cambria", &["caladea", "liberation serif"]),
    ("courier new", &["liberation mono"]),
    ("consolas", &["liberation mono"]),
];

struct FontRec {
    ttf: Ttf,
    /// Guest path (`C:\Windows\Fonts\x.ttf`).
    path: String,
    /// Read-only guest copy of the file for `TryGetFontTable` (mapped on first use).
    guest: u64,
    family: usize,
}

struct Family {
    name: String,
    fonts: Vec<usize>,
}

/// A rasterised glyph run.
pub(crate) struct Texture {
    left: i32,
    top: i32,
    w: i32,
    h: i32,
    cov: Vec<u8>,
    aliased: bool,
}

pub(crate) enum Kind {
    Factory,
    Collection,
    Family(usize),
    List(Vec<usize>),
    Font(usize),
    Face(usize),
    Strings(Vec<(String, String)>),
    Analysis(Texture),
    Interop,
    File(usize),
    Params { gamma: f32, contrast: f32, cleartype: f32, geometry: u32, mode: u32 },
    Loader,
}

/// Per-process DirectWrite state.
#[derive(Default)]
pub(crate) struct State {
    fonts: Vec<FontRec>,
    families: Vec<Family>,
    scanned: bool,
    pub(crate) com: ComTable<Kind>,
}

fn tab(p: &mut Process) -> &mut ComTable<Kind> {
    &mut p.dw.com
}

fn wide_z(s: &str) -> Vec<u8> {
    let mut b: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    b.extend_from_slice(&[0, 0]);
    b
}

/// Closeness of a font to a requested weight/stretch/style (lower is better), like DirectWrite's matching.
fn distance(t: &Ttf, weight: u32, stretch: u32, style: u32) -> u32 {
    let style_miss = u32::from(t.italic != (style != 0)) * 10_000;
    let stretch_miss = (i32::from(t.stretch) - stretch as i32).unsigned_abs() * 1000;
    // Heavier is preferred above 400, lighter below (CSS rule): a small bias breaks ties toward that side.
    let w = i32::from(t.weight);
    let wanted = weight as i32;
    let dir = if (wanted >= 400) == (w >= wanted) { 0 } else { 1 };
    style_miss + stretch_miss + (w - wanted).unsigned_abs() * 2 + dir
}

impl Process {
    /// Loads every TrueType file under `C:\Windows\Fonts` once and groups them into families.
    fn dw_scan(&mut self) {
        if self.dw.scanned {
            return;
        }
        self.dw.scanned = true;
        let files: Vec<(String, Vec<u8>)> = self
            .vfs
            .iter()
            .filter(|(k, _)| k.starts_with("/windows/fonts/") && k.ends_with(".ttf"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (k, data) in files {
            if let Some(ttf) = Ttf::parse(data) {
                self.dw_add_font(ttf, alloc::format!("C:{}", k.replace('/', "\\")));
            }
        }
        // Families in name order, fonts by weight then style (stable enumeration like Windows).
        let mut order: Vec<usize> = (0..self.dw.families.len()).collect();
        order.sort_by(|&a, &b| self.dw.families[a].name.to_lowercase().cmp(&self.dw.families[b].name.to_lowercase()));
        let mut remap = alloc::vec![0; order.len()];
        for (new, &old) in order.iter().enumerate() {
            remap[old] = new;
        }
        let mut fams: Vec<Option<Family>> = core::mem::take(&mut self.dw.families).into_iter().map(Some).collect();
        self.dw.families = order.iter().map(|&o| fams[o].take().unwrap()).collect();
        for f in &mut self.dw.fonts {
            f.family = remap[f.family];
        }
        let fonts = &self.dw.fonts;
        for fam in &mut self.dw.families {
            fam.fonts.sort_by_key(|&i| (fonts[i].ttf.weight, fonts[i].ttf.italic));
        }
    }

    fn dw_add_font(&mut self, ttf: Ttf, path: String) -> usize {
        let name = ttf.family();
        let fam = match self.dw.families.iter().position(|f| f.name.eq_ignore_ascii_case(&name)) {
            Some(i) => i,
            None => {
                self.dw.families.push(Family { name, fonts: Vec::new() });
                self.dw.families.len() - 1
            }
        };
        self.dw.fonts.push(FontRec { ttf, path, guest: 0, family: fam });
        let i = self.dw.fonts.len() - 1;
        self.dw.families[fam].fonts.push(i);
        i
    }

    /// Family index for a name, through [`SUBSTITUTES`] when the volume has no such family.
    fn dw_find_family(&mut self, name: &str) -> Option<usize> {
        self.dw_scan();
        let n = name.trim().to_lowercase();
        let find = |p: &Process, n: &str| p.dw.families.iter().position(|f| f.name.to_lowercase() == n);
        if let Some(i) = find(self, &n) {
            return Some(i);
        }
        let subs = SUBSTITUTES.iter().find(|(k, _)| *k == n)?.1;
        subs.iter().find_map(|s| find(self, s))
    }

    fn dw_best(&self, fam: usize, weight: u32, stretch: u32, style: u32) -> Option<usize> {
        self.dw.families.get(fam)?.fonts.iter().copied().min_by_key(|&i| distance(&self.dw.fonts[i].ttf, weight, stretch, style))
    }

    fn dw_new(&mut self, iface: u16, kind: Kind) -> u64 {
        self.com_new(tab, iface, SLOT_COUNT[iface as usize], kind)
    }

    /// Creates an object and stores it in the out-parameter; `E_POINTER` when there is none.
    fn dw_out(&mut self, out: u64, iface: u16, kind: Kind) -> Ret {
        if out == 0 {
            return Ret::Value(E_POINTER);
        }
        let p = self.dw_new(iface, kind);
        self.put_ptr(out, p);
        Ret::Value(S_OK)
    }

    fn dw_kind(&mut self, this: u64) -> Option<(usize, &Kind)> {
        let i = self.com_index(tab, this)?;
        Some((i, &self.dw.com.objs[i].as_ref()?.kind))
    }

    /// Font record behind a font or font-face object.
    fn dw_font_of(&mut self, obj: u64) -> Option<usize> {
        match self.dw_kind(obj)?.1 {
            Kind::Font(f) | Kind::Face(f) | Kind::File(f) => Some(*f),
            _ => None,
        }
    }

    /// Writes a wide string with terminator into a buffer of `cap` characters.
    fn dw_put_string(&mut self, s: &str, buf: u64, cap: u64) -> u64 {
        let b = wide_z(s);
        if (b.len() as u64 / 2) > cap {
            return E_NOT_SUFFICIENT_BUFFER;
        }
        if self.mem.write(buf, &b).is_err() { E_INVALIDARG } else { S_OK }
    }

    fn dw_metrics_bytes(&self, f: usize) -> [u8; 20] {
        let t = &self.dw.fonts[f].ttf;
        let mut b = [0u8; 20];
        let vals: [u16; 10] = [t.upm, t.ascent, t.descent, t.line_gap as u16, t.cap_height, t.x_height, t.underline.0 as u16, t.underline.1, t.strikeout.0 as u16, t.strikeout.1];
        for (i, v) in vals.iter().enumerate() {
            b[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    /// `DWRITE_GLYPH_METRICS` (28 bytes) in design units.
    fn dw_glyph_metrics(&self, f: usize, g: u16) -> [u8; 28] {
        let t = &self.dw.fonts[f].ttf;
        let (adv, lsb) = t.hmetric(g);
        let (x0, y0, x1, y1) = t.bbox(g).unwrap_or((0, 0, 0, 0));
        let asc = i32::from(t.ascent);
        let adv_h = asc + i32::from(t.descent);
        let rsb = i32::from(adv) - i32::from(lsb) - (i32::from(x1) - i32::from(x0));
        let tsb = asc - i32::from(y1);
        let bsb = adv_h - tsb - (i32::from(y1) - i32::from(y0));
        let vals = [i32::from(lsb), i32::from(adv), rsb, tsb, adv_h, bsb, asc];
        let mut b = [0u8; 28];
        for (i, v) in vals.iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    /// Name list for a font: `(locale, string)` entries.
    fn dw_names(&self, f: usize, id: u16) -> Vec<(String, String)> {
        self.dw.fonts[f].ttf.name(id).map(|s| alloc::vec![(String::from("en-us"), s)]).unwrap_or_default()
    }

    /// `DWriteCreateFactory(type, riid, IUnknown**)`.
    pub(crate) fn dwrite_create_factory(&mut self, a: [u64; 4]) -> Ret {
        let iid: [u8; 16] = self.mem.read_vec(a[1], 16).ok().and_then(|v| v.try_into().ok()).unwrap_or([0; 16]);
        if iid != IID_FACTORY && iid != crate::com::IID_IUNKNOWN {
            let g = u32::from_le_bytes([iid[0], iid[1], iid[2], iid[3]]);
            let msg = alloc::format!("[winrt] {}: DWriteCreateFactory: interface {g:08x}-... not provided\n", self.name);
            self.console_write(msg.as_bytes());
            self.put_ptr(a[2], 0);
            return Ret::Value(crate::com::E_NOINTERFACE);
        }
        self.dw_out(a[2], FACTORY, Kind::Factory)
    }

    /// One DirectWrite method call: `iface`/`slot` from the stub, `a` = `[this, rdx, r8, r9]`.
    pub(crate) fn dwrite_call(&mut self, iface: u16, slot: u32, a: [u64; 4], rsp: u64) -> Ret {
        let v = Ret::Value;
        let Some(i) = self.com_index(tab, a[0]) else { return v(E_POINTER) };
        if slot < 3 {
            let accepts: &[[u8; 16]] = match iface {
                FACTORY => &[IID_FACTORY],
                COLLECTION => &[IID_COLLECTION],
                FAMILY => &[IID_FAMILY, IID_LIST],
                LIST => &[IID_LIST],
                FONT => &[IID_FONT],
                FACE => &[IID_FACE],
                STRINGS => &[IID_STRINGS],
                ANALYSIS => &[IID_ANALYSIS],
                INTEROP => &[IID_INTEROP],
                FILE => &[IID_FILE],
                PARAMS => &[IID_PARAMS],
                _ => &[IID_FILE_LOADER, IID_LOCAL_LOADER],
            };
            return self.com_unknown(tab, i, slot, a, accepts);
        }
        let arg = |p: &mut Process, n: u64| p.stack_arg(rsp, n);
        match (iface, slot) {
            // ---- IDWriteFactory
            (FACTORY, 3) | (FACTORY, 17) => {
                // GetSystemFontCollection(out, checkForUpdates) / GetGdiInterop(out)
                self.dw_scan();
                self.dw_out(a[1], if slot == 3 { COLLECTION } else { INTEROP }, if slot == 3 { Kind::Collection } else { Kind::Interop })
            }
            (FACTORY, 7) => {
                // CreateFontFileReference(path, lastWriteTime, out)
                let Some(path) = self.wstr_at(a[1], 520) else { return v(E_INVALIDARG) };
                let key = crate::files::norm(&path);
                self.dw_scan();
                let known = self.dw.fonts.iter().position(|f| crate::files::norm(&f.path) == key);
                let f = match known {
                    Some(f) => f,
                    None => {
                        let Some(ttf) = self.vfs.get(&key).cloned().and_then(Ttf::parse) else { return v(DWRITE_E_NOFONT) };
                        self.dw_add_font(ttf, path)
                    }
                };
                self.dw_out(a[3], FILE, Kind::File(f))
            }
            (FACTORY, 9) => {
                // CreateFontFace(type, nfiles, files, index, simulations, out)
                let (files, out) = (a[3], arg(self, 7));
                let first = self.mem.read_u64(files).unwrap_or(0);
                match self.dw_font_of(first) {
                    Some(f) if a[2] >= 1 => self.dw_out(out, FACE, Kind::Face(f)),
                    _ => v(E_INVALIDARG),
                }
            }
            (FACTORY, 10) | (FACTORY, 11) | (FACTORY, 12) => {
                // CreateRenderingParams(out) / CreateMonitorRenderingParams(monitor, out) /
                // CreateCustomRenderingParams(gamma, contrast, cleartype, geometry, mode, out)
                let (params, out) = match slot {
                    10 => (Kind::Params { gamma: 1.8, contrast: 0.5, cleartype: 0.0, geometry: 0, mode: 5 }, a[1]),
                    11 => (Kind::Params { gamma: 1.8, contrast: 0.5, cleartype: 0.0, geometry: 0, mode: 5 }, a[2]),
                    _ => {
                        let (g, c, t) = (self.f32_arg(rsp, 1), self.f32_arg(rsp, 2), self.f32_arg(rsp, 3));
                        (Kind::Params { gamma: g, contrast: c, cleartype: t, geometry: arg(self, 5) as u32, mode: arg(self, 6) as u32 }, arg(self, 7))
                    }
                };
                self.dw_out(out, PARAMS, params)
            }
            (FACTORY, 5) | (FACTORY, 6) | (FACTORY, 13) | (FACTORY, 14) => v(S_OK), // (un)register loaders
            (FACTORY, 23) => self.dw_glyph_run_analysis(a, rsp),
            // ---- IDWriteFontCollection
            (COLLECTION, 3) => v(self.dw.families.len() as u64),
            (COLLECTION, 4) => {
                if a[1] as usize >= self.dw.families.len() {
                    return v(E_INVALIDARG);
                }
                self.dw_out(a[2], FAMILY, Kind::Family(a[1] as usize))
            }
            (COLLECTION, 5) => {
                // FindFamilyName(name, *index, *exists)
                let name = self.wstr_at(a[1], 256).unwrap_or_default();
                let found = self.dw_find_family(&name);
                let _ = self.mem.write(a[2], &(found.map_or(u32::MAX, |f| f as u32)).to_le_bytes());
                let _ = self.mem.write(a[3], &u32::from(found.is_some()).to_le_bytes());
                v(S_OK)
            }
            (COLLECTION, 6) => match self.dw_font_of(a[1]) {
                Some(f) => self.dw_out(a[2], FONT, Kind::Font(f)),
                None => v(DWRITE_E_NOFONT),
            },
            // ---- IDWriteFontList / IDWriteFontFamily (family adds names and matching)
            (LIST, 3) | (FAMILY, 3) => self.dw_out(a[1], COLLECTION, Kind::Collection),
            (LIST, 4) | (FAMILY, 4) => v(self.dw_list(i).len() as u64),
            (LIST, 5) | (FAMILY, 5) => match self.dw_list(i).get(a[1] as usize).copied() {
                Some(f) => self.dw_out(a[2], FONT, Kind::Font(f)),
                None => v(E_INVALIDARG),
            },
            (FAMILY, 6) => {
                let Some(Kind::Family(fam)) = self.dw.com.objs[i].as_ref().map(|o| &o.kind) else { return v(E_POINTER) };
                let name = self.dw.families[*fam].name.clone();
                self.dw_out(a[1], STRINGS, Kind::Strings(alloc::vec![(String::from("en-us"), name)]))
            }
            (FAMILY, 7) | (FAMILY, 8) => {
                // GetFirstMatchingFont(weight, stretch, style, out) / GetMatchingFonts(..., out)
                let Some(Kind::Family(fam)) = self.dw.com.objs[i].as_ref().map(|o| &o.kind) else { return v(E_POINTER) };
                let fam = *fam;
                let (w, s, st, out) = (a[1] as u32, a[2] as u32, a[3] as u32, arg(self, 5));
                if slot == 7 {
                    return match self.dw_best(fam, w, s, st) {
                        Some(f) => self.dw_out(out, FONT, Kind::Font(f)),
                        None => v(DWRITE_E_NOFONT),
                    };
                }
                let mut list = self.dw.families[fam].fonts.clone();
                let fonts = &self.dw.fonts;
                list.sort_by_key(|&f| distance(&fonts[f].ttf, w, s, st));
                self.dw_out(out, LIST, Kind::List(list))
            }
            // ---- IDWriteFont
            (FONT, 3) => {
                let Some(f) = self.dw_font_of(a[0]) else { return v(E_POINTER) };
                let fam = self.dw.fonts[f].family;
                self.dw_out(a[1], FAMILY, Kind::Family(fam))
            }
            (FONT, 4) | (FONT, 5) | (FONT, 6) | (FONT, 7) | (FONT, 10) => {
                let Some(f) = self.dw_font_of(a[0]) else { return v(0) };
                let t = &self.dw.fonts[f].ttf;
                v(match slot {
                    4 => u64::from(t.weight),
                    5 => u64::from(t.stretch),
                    6 => u64::from(t.italic) * 2, // DWRITE_FONT_STYLE_ITALIC
                    7 => u64::from(t.symbol),
                    _ => 0, // no simulations
                })
            }
            (FONT, 8) => {
                let Some(f) = self.dw_font_of(a[0]) else { return v(E_POINTER) };
                let face = self.dw.fonts[f].ttf.face();
                self.dw_out(a[1], STRINGS, Kind::Strings(alloc::vec![(String::from("en-us"), face)]))
            }
            (FONT, 9) => {
                // GetInformationalStrings(id, out, *exists)
                let Some(f) = self.dw_font_of(a[0]) else { return v(E_POINTER) };
                let name_id = match a[1] as u32 {
                    1 => 0, 2 => 5, 3 => 7, 4 => 8, 5 => 9, 6 => 12, 7 => 10, 8 => 11, 9 => 13, 10 => 14,
                    11 => 1, 12 => 2, 13 => 16, 14 => 17, 15 => 19, 16 => 4, 17 => 6, 18 => 20,
                    _ => u16::MAX,
                };
                let mut names = self.dw_names(f, name_id);
                if names.is_empty() && name_id == 16 {
                    names = self.dw_names(f, 1);
                }
                let exists = !names.is_empty();
                let _ = self.mem.write(a[3], &u32::from(exists).to_le_bytes());
                if !exists {
                    self.put_ptr(a[2], 0);
                    return v(S_OK);
                }
                self.dw_out(a[2], STRINGS, Kind::Strings(names))
            }
            (FONT, 11) | (FACE, 8) | (FACE, 16) => {
                // GetMetrics(DWRITE_FONT_METRICS*) / GetGdiCompatibleMetrics(em, ppd, transform, metrics*)
                let Some(f) = self.dw_font_of(a[0]) else { return v(E_POINTER) };
                let out = if slot == 16 { arg(self, 5) } else { a[1] };
                let m = self.dw_metrics_bytes(f);
                let _ = self.mem.write(out, &m);
                v(S_OK)
            }
            (FONT, 12) => {
                let Some(f) = self.dw_font_of(a[0]) else { return v(E_POINTER) };
                let has = self.dw.fonts[f].ttf.glyph_index(a[1] as u32) != 0;
                let _ = self.mem.write(a[2], &u32::from(has).to_le_bytes());
                v(S_OK)
            }
            (FONT, 13) => match self.dw_font_of(a[0]) {
                Some(f) => self.dw_out(a[1], FACE, Kind::Face(f)),
                None => v(E_POINTER),
            },
            // ---- IDWriteFontFace
            (FACE, 3) => v(1), // DWRITE_FONT_FACE_TYPE_TRUETYPE
            (FACE, 4) => {
                // GetFiles(*count, files)
                let Some(f) = self.dw_font_of(a[0]) else { return v(E_POINTER) };
                let _ = self.mem.write(a[1], &1u32.to_le_bytes());
                if a[2] != 0 {
                    let file = self.dw_new(FILE, Kind::File(f));
                    self.put_ptr(a[2], file);
                }
                v(S_OK)
            }
            (FACE, 5) | (FACE, 6) => v(0),
            (FACE, 7) | (FACE, 9) => {
                let Some(f) = self.dw_font_of(a[0]) else { return v(0) };
                let t = &self.dw.fonts[f].ttf;
                v(if slot == 7 { u64::from(t.symbol) } else { u64::from(t.glyphs) })
            }
            (FACE, 10) | (FACE, 17) => {
                // GetDesignGlyphMetrics(indices, count, metrics*, sideways) /
                // GetGdiCompatibleGlyphMetrics(em, ppd, transform, natural, indices, count, metrics*, sideways)
                let Some(f) = self.dw_font_of(a[0]) else { return v(E_POINTER) };
                let (idx, n, out) = if slot == 10 { (a[1], a[2], a[3]) } else { (arg(self, 6), arg(self, 7), arg(self, 8)) };
                let n = n as u32 as usize;
                let ids = self.mem.read_vec(idx, n * 2).unwrap_or_default();
                let mut b = Vec::with_capacity(n * 28);
                for c in ids.chunks_exact(2) {
                    b.extend_from_slice(&self.dw_glyph_metrics(f, u16::from_le_bytes([c[0], c[1]])));
                }
                v(if self.mem.write(out, &b).is_ok() { S_OK } else { E_INVALIDARG })
            }
            (FACE, 11) => {
                // GetGlyphIndices(codepoints, count, indices*)
                let Some(f) = self.dw_font_of(a[0]) else { return v(E_POINTER) };
                let n = a[2] as u32 as usize;
                let cps = self.mem.read_vec(a[1], n * 4).unwrap_or_default();
                let t = &self.dw.fonts[f].ttf;
                let out: Vec<u8> = cps.chunks_exact(4).flat_map(|c| t.glyph_index(u32::from_le_bytes([c[0], c[1], c[2], c[3]])).to_le_bytes()).collect();
                v(if self.mem.write(a[3], &out).is_ok() { S_OK } else { E_INVALIDARG })
            }
            (FACE, 12) => {
                // TryGetFontTable(tag, const void** data, *size, void** context, BOOL* exists)
                let Some(f) = self.dw_font_of(a[0]) else { return v(E_POINTER) };
                let (ctx, exists) = (arg(self, 5), arg(self, 6));
                let (off, len) = self.dw.fonts[f].ttf.table_le(a[1] as u32).unwrap_or((0, 0));
                let found = len > 0;
                let base = if found { self.dw_guest_file(f) } else { 0 };
                let data = if found && base != 0 { base + off as u64 } else { 0 };
                self.put_ptr(a[2], data);
                let _ = self.mem.write(a[3], &(if data != 0 { len as u32 } else { 0 }).to_le_bytes());
                self.put_ptr(ctx, 0);
                let _ = self.mem.write(exists, &u32::from(data != 0).to_le_bytes());
                v(S_OK)
            }
            (FACE, 13) => v(0), // ReleaseFontTable: tables live as long as the process
            (FACE, 15) => {
                // GetRecommendedRenderingMode(em, ppd, measuring, params, *mode): NATURAL_SYMMETRIC (antialiased).
                let out = arg(self, 6);
                let _ = self.mem.write(out, &5u32.to_le_bytes());
                v(S_OK)
            }
            // ---- IDWriteLocalizedStrings
            (STRINGS, 3) => v(self.dw_strings(i).len() as u64),
            (STRINGS, 4) => {
                // FindLocaleName(locale, *index, *exists): exact match, then the language part, then entry 0.
                let want = self.wstr_at(a[1], 85).unwrap_or_default().to_lowercase();
                let list = self.dw_strings(i);
                let lang = want.split('-').next().unwrap_or("").to_string();
                let hit = list.iter().position(|(l, _)| *l == want).or_else(|| list.iter().position(|(l, _)| l.split('-').next() == Some(lang.as_str())));
                let _ = self.mem.write(a[2], &hit.map_or(u32::MAX, |h| h as u32).to_le_bytes());
                let _ = self.mem.write(a[3], &u32::from(hit.is_some()).to_le_bytes());
                v(S_OK)
            }
            (STRINGS, 5) | (STRINGS, 7) => {
                let Some((l, s)) = self.dw_strings(i).get(a[1] as usize).cloned() else { return v(E_INVALIDARG) };
                let n = if slot == 5 { l.encode_utf16().count() } else { s.encode_utf16().count() };
                let _ = self.mem.write(a[2], &(n as u32).to_le_bytes());
                v(S_OK)
            }
            (STRINGS, 6) | (STRINGS, 8) => {
                let Some((l, s)) = self.dw_strings(i).get(a[1] as usize).cloned() else { return v(E_INVALIDARG) };
                v(self.dw_put_string(if slot == 6 { &l } else { &s }, a[2], a[3] & 0xffff_ffff))
            }
            // ---- IDWriteGlyphRunAnalysis
            (ANALYSIS, 3) | (ANALYSIS, 4) => self.dw_texture(i, slot, a, rsp),
            (ANALYSIS, 5) => {
                // GetAlphaBlendParams(params, *gamma, *contrast, *cleartype)
                let (g, c, t) = match self.dw_kind(a[1]).map(|k| k.1) {
                    Some(Kind::Params { gamma, contrast, cleartype, .. }) => (*gamma, *contrast, *cleartype),
                    _ => (1.8, 0.5, 0.0),
                };
                let _ = self.mem.write(a[2], &g.to_bits().to_le_bytes());
                let _ = self.mem.write(a[3], &c.to_bits().to_le_bytes());
                let out = arg(self, 5);
                let _ = self.mem.write(out, &t.to_bits().to_le_bytes());
                v(S_OK)
            }
            // ---- IDWriteGdiInterop
            (INTEROP, 3) => self.dw_from_logfont(a[1], a[2]),
            (INTEROP, 4) | (INTEROP, 5) => {
                // ConvertFontToLOGFONT(font, LOGFONTW*, *isSystem) / ConvertFontFaceToLOGFONT(face, LOGFONTW*)
                let Some(f) = self.dw_font_of(a[1]) else { return v(E_INVALIDARG) };
                let mut lf = [0u8; 92];
                let t = &self.dw.fonts[f].ttf;
                lf[16..20].copy_from_slice(&u32::from(t.weight).to_le_bytes());
                lf[20] = u8::from(t.italic);
                lf[23] = if t.symbol { 2 } else { 1 }; // SYMBOL_CHARSET / DEFAULT_CHARSET
                let name = self.dw.families[self.dw.fonts[f].family].name.clone();
                for (k, c) in name.encode_utf16().take(31).enumerate() {
                    lf[28 + k * 2..30 + k * 2].copy_from_slice(&c.to_le_bytes());
                }
                let _ = self.mem.write(a[2], &lf);
                if slot == 4 {
                    let _ = self.mem.write(a[3], &1u32.to_le_bytes());
                }
                v(S_OK)
            }
            // ---- IDWriteFontFile
            (FILE, 3) => {
                // GetReferenceKey(const void** key, *size): the path, owned by the file object.
                let Some(f) = self.dw_font_of(a[0]) else { return v(E_POINTER) };
                let key = wide_z(&self.dw.fonts[f].path.clone());
                let p = self.heap_alloc(key.len() as u64);
                let _ = self.mem.write(p, &key);
                self.put_ptr(a[1], p);
                let _ = self.mem.write(a[2], &(key.len() as u32).to_le_bytes());
                v(S_OK)
            }
            (FILE, 4) => self.dw_out(a[1], LOADER, Kind::Loader),
            (FILE, 5) => {
                // Analyze(*supported, *fileType, *faceType, *faces)
                let _ = self.mem.write(a[1], &1u32.to_le_bytes());
                let _ = self.mem.write(a[2], &2u32.to_le_bytes()); // DWRITE_FONT_FILE_TYPE_TRUETYPE
                let _ = self.mem.write(a[3], &1u32.to_le_bytes()); // DWRITE_FONT_FACE_TYPE_TRUETYPE
                let out = arg(self, 5);
                let _ = self.mem.write(out, &1u32.to_le_bytes());
                v(S_OK)
            }
            // ---- IDWriteLocalFontFileLoader: keys are the wide paths GetReferenceKey hands out
            (LOADER, 4) => {
                // GetFilePathLengthFromKey(key, keySize, *length)
                let n = (a[2] as u32 / 2).saturating_sub(1);
                let _ = self.mem.write(a[3], &n.to_le_bytes());
                v(S_OK)
            }
            (LOADER, 5) => {
                // GetFilePathFromKey(key, keySize, buffer, chars)
                let key = self.mem.read_vec(a[1], a[2] as u32 as usize).unwrap_or_default();
                let cap = arg(self, 5);
                if key.len() as u64 / 2 > cap {
                    return v(E_NOT_SUFFICIENT_BUFFER);
                }
                let _ = self.mem.write(a[3], &key);
                v(S_OK)
            }
            (LOADER, 6) => {
                let _ = self.mem.write(a[3], &0u64.to_le_bytes()); // no timestamps on the volume
                v(S_OK)
            }
            // ---- IDWriteRenderingParams: three FLOAT getters, then two enums
            (PARAMS, 3..=7) => match self.dw.com.objs[i].as_ref().map(|o| &o.kind) {
                Some(Kind::Params { gamma, contrast, cleartype, geometry, mode }) => match slot {
                    3 => Ret::Float(gamma.to_bits()),
                    4 => Ret::Float(contrast.to_bits()),
                    5 => Ret::Float(cleartype.to_bits()),
                    6 => v(u64::from(*geometry)),
                    _ => v(u64::from(*mode)),
                },
                _ => v(0),
            },
            _ => v(E_NOTIMPL),
        }
    }

    fn dw_list(&self, i: usize) -> Vec<usize> {
        match self.dw.com.objs[i].as_ref().map(|o| &o.kind) {
            Some(Kind::Family(f)) => self.dw.families[*f].fonts.clone(),
            Some(Kind::List(l)) => l.clone(),
            _ => Vec::new(),
        }
    }

    fn dw_strings(&self, i: usize) -> Vec<(String, String)> {
        match self.dw.com.objs[i].as_ref().map(|o| &o.kind) {
            Some(Kind::Strings(s)) => s.clone(),
            _ => Vec::new(),
        }
    }

    /// Guest address of the font file (read-only copy, mapped once).
    fn dw_guest_file(&mut self, f: usize) -> u64 {
        if self.dw.fonts[f].guest == 0 {
            let data = self.dw.fonts[f].ttf.data.clone();
            let len = (data.len() as u64 + 0xfff) & !0xfff;
            self.dw.fonts[f].guest = self.mem.map_anywhere(len, PROT_READ, &data, "font").unwrap_or(0);
        }
        self.dw.fonts[f].guest
    }

    /// `CreateFontFromLOGFONT(LOGFONTW*, IDWriteFont**)`: family by face name (with substitutes), then the closest
    /// weight and italic.
    fn dw_from_logfont(&mut self, lf: u64, out: u64) -> Ret {
        let Ok(b) = self.mem.read_vec(lf, 92) else { return Ret::Value(E_INVALIDARG) };
        let units: Vec<u16> = b[28..92].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
        let name = String::from_utf16_lossy(&units);
        let weight = match i32::from_le_bytes(b[16..20].try_into().unwrap()) {
            0 => 400,
            w => w.clamp(1, 999) as u32,
        };
        let Some(fam) = self.dw_find_family(if name.is_empty() { "Segoe UI" } else { &name }) else {
            self.put_ptr(out, 0);
            return Ret::Value(DWRITE_E_NOFONT);
        };
        match self.dw_best(fam, weight, 5, u32::from(b[20] != 0) * 2) {
            Some(f) => self.dw_out(out, FONT, Kind::Font(f)),
            None => Ret::Value(DWRITE_E_NOFONT),
        }
    }

    /// `CreateGlyphRunAnalysis(run, pixelsPerDip, transform, renderingMode, measuringMode, originX, originY, out)`:
    /// rasterises the run now; the analysis object keeps the coverage.
    fn dw_glyph_run_analysis(&mut self, a: [u64; 4], rsp: u64) -> Ret {
        let run = a[1];
        let ppd = self.f32_arg(rsp, 2);
        let transform = a[3];
        let mode = self.stack_arg(rsp, 5) as u32;
        let (ox, oy) = (self.f32_arg(rsp, 6), self.f32_arg(rsp, 7));
        let out = self.stack_arg(rsp, 9);
        let Ok(r) = self.mem.read_vec(run, 48) else { return Ret::Value(E_INVALIDARG) };
        let q = |o: usize| u64::from_le_bytes(r[o..o + 8].try_into().unwrap());
        let Some(f) = self.dw_font_of(q(0)) else { return Ret::Value(E_INVALIDARG) };
        let em = f32::from_bits(u32::from_le_bytes(r[8..12].try_into().unwrap()));
        let n = u32::from_le_bytes(r[12..16].try_into().unwrap()).min(4096) as usize;
        let ids: Vec<u16> = self.mem.read_vec(q(16), n * 2).unwrap_or_default().chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let floats = |p: &mut Process, at: u64, k: usize| -> Vec<f32> {
            if at == 0 {
                return Vec::new();
            }
            p.mem.read_vec(at, k * 4).unwrap_or_default().chunks_exact(4).map(|c| f32::from_bits(u32::from_le_bytes([c[0], c[1], c[2], c[3]]))).collect()
        };
        let advances = floats(self, q(24), n);
        let offsets = floats(self, q(32), n * 2);
        let m = if transform != 0 { floats(self, transform, 6) } else { alloc::vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0] };
        let (sx, sy, dx, dy) = (m[0], m[3], m[4], m[5]);
        let t = &self.dw.fonts[f].ttf;
        let upm = f32::from(t.upm.max(1));
        let k = em * ppd * sx / upm; // pixels per font unit (uniform scale assumed; shear is ignored)
        // Glyph origins in pixels.
        let mut pen = 0.0f32;
        let mut glyphs: Vec<(u16, f32, f32)> = Vec::with_capacity(n);
        for (j, &g) in ids.iter().enumerate() {
            let (ao, vo) = (offsets.get(2 * j).copied().unwrap_or(0.0), offsets.get(2 * j + 1).copied().unwrap_or(0.0));
            let gx = (ox + pen + ao) * ppd * sx + dx * ppd;
            let gy = (oy - vo) * ppd * sy + dy * ppd;
            glyphs.push((g, gx, gy));
            pen += advances.get(j).copied().unwrap_or_else(|| f32::from(t.hmetric(g).0) * em / upm);
        }
        let mut boxes: Vec<(i32, i32, i32, i32)> = Vec::with_capacity(n);
        let (mut l, mut tp, mut rt, mut bt) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &(g, gx, gy) in &glyphs {
            let b = t.bbox(g).map(|(x0, y0, x1, y1)| {
                let fl = |v: f32| v as i32 - i32::from(v < 0.0 && v != (v as i32) as f32);
                (fl(gx + f32::from(x0) * k), fl(gy - f32::from(y1) * k), fl(gx + f32::from(x1) * k) + 1, fl(gy - f32::from(y0) * k) + 1)
            });
            let b = b.unwrap_or((0, 0, 0, 0));
            if b.2 > b.0 && b.3 > b.1 {
                (l, tp, rt, bt) = (l.min(b.0), tp.min(b.1), rt.max(b.2), bt.max(b.3));
            }
            boxes.push(b);
        }
        let aliased = mode == 1; // DWRITE_RENDERING_MODE_ALIASED
        let mut tex = Texture { left: 0, top: 0, w: 0, h: 0, cov: Vec::new(), aliased };
        if rt > l && bt > tp && (rt - l) as i64 * (bt - tp) as i64 <= 16 << 20 {
            tex = Texture { left: l, top: tp, w: rt - l, h: bt - tp, cov: alloc::vec![0; ((rt - l) * (bt - tp)) as usize], aliased };
            let kq24 = (k * 16_777_216.0) as i32;
            let mut cell = Vec::new();
            for (&(g, gx, gy), &(x0, y0, x1, y1)) in glyphs.iter().zip(&boxes) {
                let (w, h) = (x1 - x0, y1 - y0);
                if w <= 0 || h <= 0 || w > loos_shell::raster::RASTER_MAX_SIZE || h > loos_shell::raster::RASTER_MAX_SIZE {
                    continue;
                }
                let Outline { xy, on, ends } = t.outline(g);
                cell.clear();
                cell.resize((w * h) as usize, 0);
                let (px, py) = (((gx - x0 as f32) * 256.0) as i32, ((gy - y0 as f32) * 256.0) as i32);
                if !loos_shell::raster::fill(&xy, &on, &ends, kq24, px, py, w, h, &mut cell) {
                    continue;
                }
                for row in 0..h {
                    let dst = ((y0 - tp + row) * tex.w + (x0 - l)) as usize;
                    for col in 0..w as usize {
                        let c = cell[(row * w) as usize + col];
                        let d = &mut tex.cov[dst + col];
                        *d = d.saturating_add(c);
                    }
                }
            }
            if aliased {
                for c in &mut tex.cov {
                    *c = if *c >= 128 { 255 } else { 0 };
                }
            }
        }
        self.dw_out(out, ANALYSIS, Kind::Analysis(tex))
    }

    /// `GetAlphaTextureBounds(type, RECT*)` (slot 3) and `CreateAlphaTexture(type, RECT*, buffer, size)` (slot 4;
    /// the size is the fifth argument, on the stack).
    /// Aliased analyses answer the 1x1 texture type only, antialiased ones the ClearType 3x1 type (grey: the three
    /// subpixels carry the same coverage).
    fn dw_texture(&mut self, i: usize, slot: u32, a: [u64; 4], rsp: u64) -> Ret {
        let Some(Kind::Analysis(t)) = self.dw.com.objs[i].as_ref().map(|o| &o.kind) else { return Ret::Value(E_POINTER) };
        let (left, top, w, h, aliased) = (t.left, t.top, t.w, t.h, t.aliased);
        let ty_aliased = a[1] as u32 == 0; // DWRITE_TEXTURE_ALIASED_1x1
        if ty_aliased != aliased {
            if slot == 3 {
                let _ = self.mem.write(a[2], &[0u8; 16]);
                return Ret::Value(S_OK);
            }
            return Ret::Value(crate::com::E_INVALIDARG);
        }
        if slot == 3 {
            let mut r = [0u8; 16];
            if w > 0 {
                for (k, val) in [left, top, left + w, top + h].iter().enumerate() {
                    r[k * 4..k * 4 + 4].copy_from_slice(&val.to_le_bytes());
                }
            }
            let _ = self.mem.write(a[2], &r);
            return Ret::Value(S_OK);
        }
        let Ok(rb) = self.mem.read_vec(a[2], 16) else { return Ret::Value(E_INVALIDARG) };
        let rd = |o: usize| i32::from_le_bytes(rb[o..o + 4].try_into().unwrap());
        let (rl, rtp, rr, rbm) = (rd(0), rd(4), rd(8), rd(12));
        let bpp: usize = if aliased { 1 } else { 3 };
        let (ow, oh) = ((rr - rl).max(0) as usize, (rbm - rtp).max(0) as usize);
        let need = ow * oh * bpp;
        if (self.stack_arg(rsp, 5) as u32 as usize) < need {
            return Ret::Value(E_NOT_SUFFICIENT_BUFFER);
        }
        let Some(Kind::Analysis(t)) = self.dw.com.objs[i].as_ref().map(|o| &o.kind) else { return Ret::Value(E_POINTER) };
        let mut buf = alloc::vec![0u8; need];
        for y in 0..oh {
            let sy = rtp + y as i32 - top;
            if sy < 0 || sy >= h {
                continue;
            }
            for x in 0..ow {
                let sx = rl + x as i32 - left;
                if sx < 0 || sx >= w {
                    continue;
                }
                let c = t.cov[(sy * w + sx) as usize];
                for k in 0..bpp {
                    buf[(y * ow + x) * bpp + k] = c;
                }
            }
        }
        Ret::Value(if self.mem.write(a[3], &buf).is_ok() { S_OK } else { E_INVALIDARG })
    }
}
