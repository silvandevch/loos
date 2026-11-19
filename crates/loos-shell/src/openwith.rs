//! What double-clicking a file does: recognise Windows, Linux and macOS programs by their bytes (not just
//! their names) and build the command line the matching compatibility layer runs. Windows programs run on
//! Wine, Linux programs on the Linux layer, macOS programs, bundles, disk images and installer packages on
//! Prism; all three live in the compat runtime the kernel starts (`loos_compat_run`).
#![allow(missing_docs)]

use alloc::{format, string::String, vec, vec::Vec};

/// How a file opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opener {
    /// A folder (also a `.app` bundle's insides when browsed explicitly): browse it.
    Folder,
    /// Windows program, installer, script or shortcut (`.exe`, `.msi`, `.bat`, `.lnk`, ...): Wine.
    Windows,
    /// Linux program (ELF) or script with a `#!` line: the Linux layer. Needs the exec permission.
    Linux,
    /// Mach-O program or `.app` bundle: Prism.
    Mac,
    /// macOS disk image (`.dmg`): Prism mounts it and opens the app inside.
    MacDisk,
    /// macOS installer package (`.pkg`, `.mpkg`): Prism installs it into /Applications and opens the app.
    MacPackage,
    /// Anything else (documents): no compatibility layer involved.
    Document,
}

fn ext_is(name: &[u8], exts: &[&[u8]]) -> bool {
    let Some(dot) = name.iter().rposition(|&c| c == b'.') else { return false };
    let e = &name[dot + 1..];
    exts.iter().any(|x| x.eq_ignore_ascii_case(e))
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Classify `name` (a file or folder name) from its first bytes `head` (64 are plenty; may be shorter).
pub fn classify(name: &[u8], is_dir: bool, head: &[u8]) -> Opener {
    if is_dir {
        return if ext_is(name, &[b"app"]) { Opener::Mac } else if ext_is(name, &[b"mpkg", b"pkg"]) { Opener::MacPackage } else { Opener::Folder };
    }
    if head.len() >= 4 {
        let m = be32(head);
        match m {
            0x7f45_4c46 => return Opener::Linux,                                      // ELF
            0xfeed_face | 0xfeed_facf | 0xcefa_edfe | 0xcffa_edfe => return Opener::Mac, // Mach-O (either byte order)
            // Universal binary; Java class files share the magic but put a version (>= 45) where the
            // architecture count goes.
            0xcafe_babe if head.len() >= 8 && be32(&head[4..]) < 20 => return Opener::Mac,
            0x7861_7221 => return Opener::MacPackage,                                 // "xar!": flat .pkg/.mpkg
            0xd0cf_11e0 if ext_is(name, &[b"msi", b"msp"]) => return Opener::Windows, // MSI (OLE compound file)
            _ => {}
        }
    }
    if head.starts_with(b"MZ") {
        return Opener::Windows;
    }
    if head.starts_with(b"#!") {
        return Opener::Linux;
    }
    if ext_is(name, &[b"dmg"]) {
        return Opener::MacDisk; // UDIF keeps its signature ("koly") in the last 512 bytes
    }
    if ext_is(name, &[b"bat", b"cmd", b"lnk", b"msi"]) {
        return Opener::Windows;
    }
    Opener::Document
}

/// Running it needs the Unix exec permission (Linux semantics: a downloaded program is not executable
/// until the user says so).
pub fn needs_exec_permission(op: Opener) -> bool {
    op == Opener::Linux
}

/// Is this something a compatibility layer runs?
pub fn is_program(op: Opener) -> bool {
    !matches!(op, Opener::Folder | Opener::Document)
}

/// The compat runtime's command for `path` (the file's path, which is the same inside the runtime): leading
/// `VAR=value` entries are environment, then the program and its arguments (one entry per line in the
/// launch request).
pub fn command(op: Opener, path: &str) -> Vec<String> {
    match op {
        // `start /wait /unix` lets Wine pick by type (exe, msi -> msiexec, bat -> cmd, lnk) and keeps the
        // session open until the program ends.
        Opener::Windows => vec![
            "WINEPREFIX=/home/.wine".into(),
            "USER=user".into(),
            "WINEDEBUG=err+all,fixme-all".into(),
            "VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json".into(),
            "/usr/bin/wine64".into(),
            "start".into(),
            "/wait".into(),
            "/unix".into(),
            path.into(),
        ],
        Opener::Linux | Opener::Mac | Opener::MacDisk | Opener::MacPackage => vec![path.into()],
        Opener::Folder | Opener::Document => Vec::new(),
    }
}

/// Short user-facing description, for the status line ("Starting ... with Wine").
pub fn layer_name(op: Opener) -> &'static str {
    match op {
        Opener::Windows => "Windows (Wine)",
        Opener::Linux => "Linux",
        Opener::Mac => "macOS (Prism)",
        Opener::MacDisk => "macOS disk image (Prism)",
        Opener::MacPackage => "macOS installer (Prism)",
        Opener::Folder | Opener::Document => "",
    }
}

/// The launch request's text: one command entry per line.
pub fn request_text(cmd: &[String]) -> String {
    let mut s = String::new();
    for a in cmd {
        s += &format!("{a}\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_are_recognised_by_their_bytes() {
        assert_eq!(classify(b"setup.exe", false, b"MZ\x90\0"), Opener::Windows);
        assert_eq!(classify(b"renamed.bin", false, b"MZ\x90\0"), Opener::Windows, "content wins over the name");
        assert_eq!(classify(b"tool", false, b"\x7fELF\x02\x01"), Opener::Linux);
        assert_eq!(classify(b"run.sh", false, b"#!/bin/sh\n"), Opener::Linux);
        assert_eq!(classify(b"hello", false, &[0xcf, 0xfa, 0xed, 0xfe]), Opener::Mac);
        assert_eq!(classify(b"uni", false, &[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 2]), Opener::Mac);
        assert_eq!(classify(b"Main.class", false, &[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 52]), Opener::Document, "Java class, not Mach-O");
        assert_eq!(classify(b"App.pkg", false, b"xar!\0\x1c"), Opener::MacPackage);
        assert_eq!(classify(b"App.dmg", false, b"\0\0\0\0"), Opener::MacDisk);
        assert_eq!(classify(b"Setup.MSI", false, &[0xd0, 0xcf, 0x11, 0xe0]), Opener::Windows);
        assert_eq!(classify(b"go.bat", false, b"@echo off"), Opener::Windows);
        assert_eq!(classify(b"notes.txt", false, b"hello"), Opener::Document);
        assert_eq!(classify(b"empty", false, b""), Opener::Document);
        assert_eq!(classify(b"tiny.exe", false, b"MZ"), Opener::Windows, "short heads still count");
    }

    #[test]
    fn folders_and_bundles() {
        assert_eq!(classify(b"Safari.app", true, b""), Opener::Mac);
        assert_eq!(classify(b"Suite.mpkg", true, b""), Opener::MacPackage);
        assert_eq!(classify(b"Documents", true, b""), Opener::Folder);
    }

    #[test]
    fn only_linux_programs_need_the_exec_permission() {
        assert!(needs_exec_permission(Opener::Linux));
        assert!(!needs_exec_permission(Opener::Windows));
        assert!(!needs_exec_permission(Opener::Mac));
    }

    #[test]
    fn commands_per_layer() {
        let w = command(Opener::Windows, "/home/user/Downloads/game.exe");
        assert_eq!(w.iter().position(|a| a == "/usr/bin/wine64"), Some(4));
        assert_eq!(&w[5..], ["start", "/wait", "/unix", "/home/user/Downloads/game.exe"]);
        assert!(w[..4].iter().all(|e| e.contains('=')), "environment first");
        assert_eq!(command(Opener::Linux, "/home/user/a.out"), ["/home/user/a.out"]);
        assert_eq!(command(Opener::Mac, "/Applications/X.app"), ["/Applications/X.app"]);
        assert!(command(Opener::Document, "/x.txt").is_empty());
        assert_eq!(request_text(&command(Opener::Linux, "/a")), "/a\n");
    }
}
