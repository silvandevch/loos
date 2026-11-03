//! App/settings inventory (C `inventory.c`): local items only, no web results.
#![allow(missing_docs)]

use crate::ffi;

pub const SHELL_MAX_APPS: usize = 128;
pub const ITEM_APP: i32 = 0;
pub const ITEM_FILE: i32 = 1;
pub const ITEM_SETTING: i32 = 2;

/// C `shell_item`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShellItem {
    pub name: [u8; 48],
    pub target: [u8; 128],
    pub keywords: [u8; 96],
    pub color: u32,
    pub kind: i32,
}

impl ShellItem {
    pub const EMPTY: Self = Self { name: [0; 48], target: [0; 128], keywords: [0; 96], color: 0, kind: 0 };
}

/// C `shell_inventory`.
#[repr(C)]
pub struct ShellInventory {
    pub item: [ShellItem; SHELL_MAX_APPS],
    pub n: i32,
}

fn put(dst: &mut [u8], src: &[u8]) {
    dst.fill(0);
    let n = src.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&src[..n]);
}

pub fn init(inv: &mut ShellInventory) {
    inv.item.fill(ShellItem::EMPTY);
    inv.n = 0;
}

pub fn add(inv: &mut ShellInventory, name: &[u8], target: &[u8], keywords: &[u8], kind: i32) -> bool {
    if inv.n as usize >= SHELL_MAX_APPS || name.is_empty() {
        return false;
    }
    let it = &mut inv.item[inv.n as usize];
    *it = ShellItem::EMPTY;
    put(&mut it.name, name);
    put(&mut it.target, target);
    put(&mut it.keywords, keywords);
    it.kind = kind;
    inv.n += 1;
    true
}

fn trim(s: &[u8]) -> &[u8] {
    let mut a = 0;
    while a < s.len() && (s[a] == b' ' || s[a] == b'\t') {
        a += 1;
    }
    let mut e = s.len();
    while e > a && (s[e - 1] == b' ' || s[e - 1] == b'\t' || s[e - 1] == b'\r') {
        e -= 1;
    }
    &s[a..e]
}

pub fn parse(inv: &mut ShellInventory, text: &[u8]) -> i32 {
    let mut added = 0;
    let mut off = 0usize;
    while off < text.len() && text[off] != 0 {
        let rest = &text[off..];
        let n = rest.iter().position(|&b| b == b'\n' || b == 0).unwrap_or(rest.len());
        let line = trim(&rest[..n]);
        off += n;
        if off < text.len() && text[off] == b'\n' {
            off += 1;
        }
        if line.is_empty() || line[0] == b'#' {
            continue;
        }
        // kind|name|target[|keywords]
        let mut fields: [&[u8]; 4] = [&[]; 4];
        let mut k = 0;
        let mut start = 0;
        for (i, &b) in line.iter().enumerate() {
            if b == b'|' && k < 3 {
                fields[k] = &line[start..i];
                k += 1;
                start = i + 1;
            }
        }
        fields[k] = &line[start..];
        k += 1;
        if k < 3 {
            continue;
        }
        let kind = match trim(fields[0]) {
            b"app" => ITEM_APP,
            b"file" => ITEM_FILE,
            b"setting" => ITEM_SETTING,
            _ => continue,
        };
        let kw = if k > 3 { trim(fields[3]) } else { &[][..] };
        if add(inv, trim(fields[1]), trim(fields[2]), kw, kind) {
            added += 1;
        }
    }
    added
}

pub const DEFAULTS: &[u8] = b"# built-in LoOS apps and settings pages (no web results, ever)\n\
app|LoOS Calculator|calc.exe|math arithmetic scientific\n\
app|LoCmd|loos-cmd.exe|terminal shell console prompt cmd\n\
app|LoOS Notepad|loos-notepad.exe|text editor notes plugins\n\
app|LoOS Task Manager|loos-taskmgr.exe|processes monitor cpu memory kill\n\
app|LoOS Browser|loos-browser.exe|web browser internet html url firefox tabs history bookmarks\n\
setting|Privacy|settings:privacy|telemetry permissions camera microphone location\n\
setting|Network firewall|settings:firewall|connections block allow sockets\n\
setting|App permissions|settings:permissions|files notifications camera mic\n\
setting|Updates|settings:updates|mirror offline opt-in\n\
setting|Appearance|settings:appearance|theme dark light accent scale dpi wallpaper\n\
setting|Compatibility|settings:compat|windows version 7 10 11 shim\n\
setting|Accounts|settings:accounts|users sign-in password\n\
setting|Accessibility|settings:accessibility|contrast narrator keyboard magnifier motion\n\
setting|Display|settings:display|resolution monitors scale dpi\n\
setting|Looks|settings:looks|theme preset style dock tiler classic dark light layout desktop\n\
setting|Behavior|settings:behavior|tiling gap focus follows mouse night light shortcuts keys\n";

pub fn defaults(inv: &mut ShellInventory) {
    parse(inv, DEFAULTS);
}

#[no_mangle]
pub unsafe extern "C" fn inventory_init(inv: *mut ShellInventory) {
    if let Some(inv) = unsafe { inv.as_mut() } {
        init(inv);
    }
}

#[no_mangle]
pub unsafe extern "C" fn inventory_add(
    inv: *mut ShellInventory,
    name: *const core::ffi::c_char,
    target: *const core::ffi::c_char,
    keywords: *const core::ffi::c_char,
    kind: i32,
) -> bool {
    match unsafe { inv.as_mut() } {
        Some(inv) => add(inv, unsafe { ffi::c_bytes(name) }, unsafe { ffi::c_bytes(target) }, unsafe { ffi::c_bytes(keywords) }, kind),
        None => false,
    }
}

#[no_mangle]
pub unsafe extern "C" fn inventory_parse(inv: *mut ShellInventory, text: *const core::ffi::c_char) -> i32 {
    match unsafe { inv.as_mut() } {
        Some(inv) => parse(inv, unsafe { ffi::c_bytes(text) }),
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn inventory_defaults(inv: *mut ShellInventory) {
    if let Some(inv) = unsafe { inv.as_mut() } {
        defaults(inv);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::SHELL_MAX_WINDOWS;
    use alloc::boxed::Box;
    use core::mem::{offset_of, size_of};

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<ShellItem>(), 280);
        assert_eq!(offset_of!(ShellItem, target), 48);
        assert_eq!(offset_of!(ShellItem, keywords), 176);
        assert_eq!(offset_of!(ShellItem, color), 272);
        assert_eq!(offset_of!(ShellItem, kind), 276);
        assert_eq!(size_of::<ShellInventory>(), 35844);
        assert_eq!(offset_of!(ShellInventory, n), 35840);
    }

    #[test]
    fn parses_and_skips_bad_lines() {
        let inv = Box::leak(Box::new(unsafe { core::mem::zeroed::<ShellInventory>() }));
        init(inv);
        assert_eq!(parse(inv, b"# c\napp|Calc|calc.exe|math\nfile|Doc|/doc.txt\nbad|x\nsetting|Privacy|settings:privacy\n"), 3);
        assert_eq!(inv.n, 3);
        assert_eq!(&inv.item[0].name[..4], b"Calc");
        assert_eq!(inv.item[0].kind, ITEM_APP);
        assert_eq!(inv.item[1].kind, ITEM_FILE);
        assert_eq!(inv.item[2].kind, ITEM_SETTING);
    }

    #[test]
    fn defaults_include_the_builtin_apps() {
        let inv = Box::leak(Box::new(unsafe { core::mem::zeroed::<ShellInventory>() }));
        init(inv);
        defaults(inv);
        assert_eq!(inv.n, 16);
        let apps = inv.item[..16].iter().filter(|i| i.kind == ITEM_APP).count();
        assert_eq!(apps, 5);
        assert_eq!(&inv.item[4].name[..12], b"LoOS Browser");
    }

    #[test]
    fn add_respects_the_capacity() {
        let inv = Box::leak(Box::new(unsafe { core::mem::zeroed::<ShellInventory>() }));
        init(inv);
        for i in 0..SHELL_MAX_APPS {
            assert!(add(inv, b"x", b"x", b"", ITEM_APP), "{i}");
        }
        assert!(!add(inv, b"x", b"x", b"", ITEM_APP));
        let _ = SHELL_MAX_WINDOWS;
    }
}
