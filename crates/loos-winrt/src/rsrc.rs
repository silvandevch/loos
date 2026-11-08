//! PE resources (`FindResource`/`LoadResource`/`LoadString`) read from a mapped module, and `DLGTEMPLATE(EX)` parsing.

use alloc::string::String;
use alloc::vec::Vec;
use loos_lin::mem::UserMem;
use loos_win32k::DlgItem;

/// A resource type or name: numeric id or string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResId {
    /// `MAKEINTRESOURCE(n)`.
    Id(u16),
    /// Named resource (compared case-insensitively).
    Name(String),
}

/// Standard resource types.
pub const RT_DIALOG: u16 = 5;
/// `RT_STRING`.
pub const RT_STRING: u16 = 6;

impl ResId {
    /// Decodes an `LPCWSTR` argument (integer atoms are below 0x10000; `#123` also names an id).
    pub fn from_arg(mem: &mut UserMem, p: u64) -> Option<ResId> {
        if p < 0x1_0000 {
            return Some(ResId::Id(p as u16));
        }
        let s = crate::k32_wide(mem, p)?;
        match s.strip_prefix('#').and_then(|n| n.parse::<u16>().ok()) {
            Some(n) => Some(ResId::Id(n)),
            None => Some(ResId::Name(s)),
        }
    }
}

fn u16_at(m: &mut UserMem, a: u64) -> Option<u16> {
    let mut b = [0u8; 2];
    m.read(a, &mut b).ok()?;
    Some(u16::from_le_bytes(b))
}

/// Finds a resource in the module at `base`; returns `(rva, size)` of its data (first language).
pub fn find(mem: &mut UserMem, base: u64, ty: &ResId, name: &ResId) -> Option<(u32, u32)> {
    let nt = u64::from(mem.read_u32(base + 0x3c).ok()?);
    if u16_at(mem, base)? != 0x5a4d || mem.read_u32(base + nt).ok()? != 0x4550 {
        return None;
    }
    let dd = base + nt + 24 + 112 + 16; // DataDirectory[IMAGE_DIRECTORY_ENTRY_RESOURCE]
    let (rva, size) = (u64::from(mem.read_u32(dd).ok()?), mem.read_u32(dd + 4).ok()?);
    if rva == 0 || size == 0 {
        return None;
    }
    let root = base + rva;
    // Directory at `dir`: entries are (name-or-id, offset); returns the sub-entry offset matching `want`.
    let pick = |mem: &mut UserMem, dir: u64, want: Option<&ResId>| -> Option<u32> {
        let named = u64::from(u16_at(mem, dir + 12)?);
        let ids = u64::from(u16_at(mem, dir + 14)?);
        for i in 0..named + ids {
            let e = dir + 16 + 8 * i;
            let (key, off) = (mem.read_u32(e).ok()?, mem.read_u32(e + 4).ok()?);
            let hit = match want {
                None => true,
                Some(ResId::Id(n)) => key & 0x8000_0000 == 0 && key == u32::from(*n),
                Some(ResId::Name(s)) => {
                    key & 0x8000_0000 != 0 && {
                        let at = root + u64::from(key & 0x7fff_ffff);
                        let len = u64::from(u16_at(mem, at)?);
                        let mut u = Vec::new();
                        for k in 0..len {
                            u.push(u16_at(mem, at + 2 + 2 * k)?);
                        }
                        String::from_utf16_lossy(&u).eq_ignore_ascii_case(s)
                    }
                }
            };
            if hit {
                return Some(off);
            }
        }
        None
    };
    let o1 = pick(mem, root, Some(ty))?;
    if o1 & 0x8000_0000 == 0 {
        return None;
    }
    let o2 = pick(mem, root + u64::from(o1 & 0x7fff_ffff), Some(name))?;
    if o2 & 0x8000_0000 == 0 {
        return None;
    }
    let o3 = pick(mem, root + u64::from(o2 & 0x7fff_ffff), None)?;
    let de = root + u64::from(o3 & 0x7fff_ffff);
    Some((mem.read_u32(de).ok()?, mem.read_u32(de + 4).ok()?))
}

/// `LoadString`: string `id` from the module's `RT_STRING` blocks.
pub fn load_string(mem: &mut UserMem, base: u64, id: u16) -> Option<String> {
    let (rva, size) = find(mem, base, &ResId::Id(RT_STRING), &ResId::Id(id / 16 + 1))?;
    let data = mem.read_vec(base + u64::from(rva), size as usize).ok()?;
    let mut at = 0usize;
    for i in 0..16u16 {
        let len = u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?) as usize;
        at += 2;
        if i == id % 16 {
            let u: Vec<u16> = data.get(at..at + 2 * len)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            return Some(String::from_utf16_lossy(&u));
        }
        at += 2 * len;
    }
    None
}

/// A parsed dialog template, sizes already converted from dialog units to pixels.
pub struct Dialog {
    /// Caption.
    pub title: String,
    /// Window style.
    pub style: u32,
    /// Extended style.
    pub exstyle: u32,
    /// Top-left (None = centred).
    pub pos: Option<(i32, i32)>,
    /// Client size in pixels.
    pub client: (i32, i32),
    /// Controls.
    pub items: Vec<DlgItem>,
}

/// Dialog base units (system font): one dialog unit = `BASE_X/4` by `BASE_Y/8` pixels.
pub const BASE_X: i32 = 6;
/// See [`BASE_X`].
pub const BASE_Y: i32 = 13;

struct Rd<'a> {
    b: &'a [u8],
    at: usize,
}

impl Rd<'_> {
    fn u16(&mut self) -> Option<u16> {
        let v = u16::from_le_bytes(self.b.get(self.at..self.at + 2)?.try_into().ok()?);
        self.at += 2;
        Some(v)
    }
    fn i16(&mut self) -> Option<i32> {
        self.u16().map(|v| i32::from(v as i16))
    }
    fn u32(&mut self) -> Option<u32> {
        let v = u32::from_le_bytes(self.b.get(self.at..self.at + 4)?.try_into().ok()?);
        self.at += 4;
        Some(v)
    }
    fn align4(&mut self) {
        self.at = (self.at + 3) & !3;
    }
    /// A string (NUL-terminated UTF-16) or `0xFFFF, ordinal`; returns `Ok(string)` / `Err(ordinal)`.
    fn name(&mut self) -> Option<Result<String, u16>> {
        let first = self.u16()?;
        if first == 0 {
            return Some(Ok(String::new()));
        }
        if first == 0xffff {
            return Some(Err(self.u16()?));
        }
        let mut u = alloc::vec![first];
        loop {
            let c = self.u16()?;
            if c == 0 {
                return Some(Ok(String::from_utf16_lossy(&u)));
            }
            u.push(c);
        }
    }
}

fn class_name(c: Result<String, u16>) -> String {
    match c {
        Ok(s) => s,
        Err(0x80) => "Button".into(),
        Err(0x81) => "Edit".into(),
        Err(0x82) => "Static".into(),
        Err(0x83) => "ListBox".into(),
        Err(0x84) => "ScrollBar".into(),
        Err(0x85) => "ComboBox".into(),
        Err(_) => "Static".into(),
    }
}

/// Parses a `DLGTEMPLATE` or `DLGTEMPLATEEX`.
pub fn parse_dialog(b: &[u8]) -> Option<Dialog> {
    let mut r = Rd { b, at: 0 };
    let ex = b.len() >= 4 && b[0..2] == [1, 0] && b[2..4] == [0xff, 0xff];
    let (style, exstyle);
    if ex {
        r.at = 4;
        r.u32()?; // helpID
        exstyle = r.u32()?;
        style = r.u32()?;
    } else {
        style = r.u32()?;
        exstyle = r.u32()?;
    }
    let count = r.u16()?;
    let (x, y, cx, cy) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?);
    r.name()?; // menu
    r.name()?; // window class (default dialog class)
    let title = r.name()?.unwrap_or_default();
    if style & 0x40 != 0 || (ex && style & 0x48 != 0) {
        r.u16()?; // point size
        if ex {
            r.u16()?; // weight
            r.u16()?; // italic + charset
        }
        r.name()?; // typeface
    }
    let px = |v: i32| v * BASE_X / 4;
    let py = |v: i32| v * BASE_Y / 8;
    let mut items = Vec::new();
    for _ in 0..count {
        r.align4();
        let (istyle, iex, id);
        let (ix, iy, icx, icy);
        if ex {
            r.u32()?;
            iex = r.u32()?;
            istyle = r.u32()?;
            (ix, iy, icx, icy) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?);
            id = r.u32()? as i32;
        } else {
            istyle = r.u32()?;
            iex = r.u32()?;
            (ix, iy, icx, icy) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?);
            id = i32::from(r.u16()?);
        }
        let class = class_name(r.name()?);
        let text = match r.name()? {
            Ok(s) => s,
            Err(_) => String::new(), // ordinal titles (icons/bitmaps) are not rendered
        };
        let extra = r.u16()? as usize;
        r.at += extra;
        items.push(DlgItem { class, text, style: istyle, exstyle: iex, x: px(ix), y: py(iy), cx: px(icx), cy: py(icy), id });
    }
    Some(Dialog { title, style, exstyle, pos: if x == 0 && y == 0 { None } else { Some((px(x), py(y))) }, client: (px(cx), py(cy)), items })
}
