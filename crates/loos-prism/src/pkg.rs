//! macOS installer packages: opening a `.pkg` / `.mpkg` installs it (like Installer.app, without scripts) and
//! returns the app it put into /Applications, which Prism then starts.
//!
//! Formats: flat packages (xar archive: zlib-compressed XML table of contents, then the heap) with one or more
//! components (`<name>.pkg/Payload` + `PackageInfo` with `install-location`), and old bundle packages
//! (`Contents/Archive.pax.gz`, `.mpkg` bundles with `Contents/Packages/*.pkg`). Payloads are gzip'd cpio (odc or
//! newc). Apple's own pbzx (xz) payloads and pre/postinstall scripts are not supported and say so.

use alloc::{format, string::String, vec, vec::Vec};
use loos_lin::fs::{normalize, Kind, LinFs};

const ENOEXEC: i64 = 8;

/// Is `path` an installer package (flat file or bundle folder)?
pub fn is_package(fs: &mut dyn LinFs, path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if !(lower.ends_with(".pkg") || lower.ends_with(".mpkg")) {
        return false;
    }
    match fs.lstat(path) {
        Ok(st) if st.kind == Kind::Dir => true,
        Ok(_) => {
            let mut m = [0u8; 4];
            matches!(fs.read(path, 0, &mut m), Ok(4)) && &m == b"xar!"
        }
        Err(_) => false,
    }
}

fn read_all(fs: &mut dyn LinFs, path: &str) -> Result<Vec<u8>, i64> {
    let st = fs.lstat(path)?;
    let mut b = vec![0u8; st.size as usize];
    let n = fs.read(path, 0, &mut b)?;
    b.truncate(n);
    Ok(b)
}

fn be16(b: &[u8], o: usize) -> u64 {
    u16::from_be_bytes([b[o], b[o + 1]]) as u64
}
fn be64(b: &[u8], o: usize) -> u64 {
    u64::from_be_bytes(b[o..o + 8].try_into().unwrap())
}

/// zlib (RFC 1950) stream.
pub(crate) fn inflate_zlib(d: &[u8]) -> Result<Vec<u8>, String> {
    miniz_oxide::inflate::decompress_to_vec_zlib(d).map_err(|e| format!("zlib: {e:?}"))
}

/// gzip (RFC 1952) member: header, raw deflate.
pub(crate) fn gunzip(d: &[u8]) -> Result<Vec<u8>, String> {
    if d.len() < 18 || d[0] != 0x1f || d[1] != 0x8b || d[2] != 8 {
        return Err("not gzip".into());
    }
    let flg = d[3];
    let mut o = 10;
    if flg & 4 != 0 {
        o += 2 + (d[o] as usize | (d[o + 1] as usize) << 8);
    }
    for bit in [8u8, 16] {
        if flg & bit != 0 {
            while o < d.len() && d[o] != 0 {
                o += 1;
            }
            o += 1;
        }
    }
    if flg & 2 != 0 {
        o += 2;
    }
    miniz_oxide::inflate::decompress_to_vec(d.get(o..).ok_or("gzip: truncated")?).map_err(|e| format!("gzip: {e:?}"))
}

/// One xar member.
struct XarFile {
    path: String,
    offset: u64,
    length: u64,
    encoding: String,
}

/// The value of attribute `name` in tag text `tag` (`<encoding style="...">`).
fn attr(tag: &str, name: &str) -> Option<String> {
    let i = tag.find(&format!("{name}=\""))? + name.len() + 2;
    Some(tag[i..].split('"').next()?.into())
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Members of a xar archive (files only), with heap-relative data ranges.
fn xar_files(toc: &str) -> Vec<XarFile> {
    struct Open {
        name: String,
        offset: u64,
        length: u64,
        encoding: String,
        is_file: bool,
    }
    let mut out = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    let mut rest = toc;
    // Text of the innermost element, and its name.
    let mut cur = String::new();
    while let Some(lt) = rest.find('<') {
        let text = &rest[..lt];
        let Some(gt) = rest[lt..].find('>') else { break };
        let tag = &rest[lt + 1..lt + gt];
        rest = &rest[lt + gt + 1..];
        if let Some(close) = tag.strip_prefix('/') {
            let v = xml_unescape(text.trim());
            if let Some(f) = stack.last_mut() {
                match close {
                    "name" if cur == "name" => f.name = v,
                    "offset" if cur == "offset" => f.offset = v.parse().unwrap_or(0),
                    "length" if cur == "length" => f.length = v.parse().unwrap_or(0),
                    "type" if cur == "type" => f.is_file = v == "file",
                    "file" => {
                        let f = stack.pop().unwrap();
                        let dir: Vec<&str> = stack.iter().map(|o| o.name.as_str()).collect();
                        if f.is_file {
                            let mut path = dir.join("/");
                            if !path.is_empty() {
                                path.push('/');
                            }
                            path += &f.name;
                            out.push(XarFile { path, offset: f.offset, length: f.length, encoding: f.encoding });
                        }
                    }
                    _ => {}
                }
            }
            cur.clear();
            continue;
        }
        let name = tag.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        if name == "file" && !tag.ends_with('/') {
            stack.push(Open { name: String::new(), offset: 0, length: 0, encoding: "application/octet-stream".into(), is_file: false });
        } else if name == "encoding" {
            if let (Some(f), Some(s)) = (stack.last_mut(), attr(tag, "style")) {
                f.encoding = s;
            }
        }
        cur = name.into();
    }
    out
}

/// Data of member `f` (decoded per its xar encoding).
fn xar_data(heap: &[u8], f: &XarFile) -> Result<Vec<u8>, String> {
    let raw = heap.get(f.offset as usize..(f.offset + f.length) as usize).ok_or("xar: data outside the archive")?;
    match f.encoding.as_str() {
        "application/octet-stream" => Ok(raw.to_vec()),
        // xar's "gzip" is a zlib stream
        "application/x-gzip" | "application/zlib" => inflate_zlib(raw),
        e => Err(format!("xar: {e} data is not supported")),
    }
}

/// A cpio archive (odc `070707` or newc `070701`) unpacked below `dest`. Returns the paths written (relative).
fn cpio_extract(fs: &mut dyn LinFs, cpio: &[u8], dest: &str) -> Result<Vec<String>, String> {
    let octal = |b: &[u8]| u64::from_str_radix(core::str::from_utf8(b).unwrap_or("x").trim(), 8).map_err(|_| String::from("cpio: bad header"));
    let hex = |b: &[u8]| u64::from_str_radix(core::str::from_utf8(b).unwrap_or("x"), 16).map_err(|_| String::from("cpio: bad header"));
    let mut o = 0usize;
    let mut written = Vec::new();
    while o + 6 <= cpio.len() {
        let magic = &cpio[o..o + 6];
        let (mode, namesize, filesize, hdr, align) = match magic {
            b"070707" if o + 76 <= cpio.len() => {
                let h = &cpio[o..o + 76];
                (octal(&h[18..24])?, octal(&h[59..65])? as usize, octal(&h[65..76])? as usize, 76, 1)
            }
            b"070701" | b"070702" if o + 110 <= cpio.len() => {
                let h = &cpio[o..o + 110];
                (hex(&h[14..22])?, hex(&h[94..102])? as usize, hex(&h[54..62])? as usize, 110, 4)
            }
            _ => return Err("cpio: unknown format".into()),
        };
        let pad = |n: usize| (n + align - 1) / align * align;
        let name_at = o + hdr;
        let name = cpio.get(name_at..name_at + namesize.saturating_sub(1)).ok_or("cpio: truncated")?;
        let name = String::from_utf8_lossy(name).into_owned();
        let data_at = pad(name_at + namesize);
        let data = cpio.get(data_at..data_at + filesize).ok_or("cpio: truncated")?;
        o = pad(data_at + filesize);
        if name == "TRAILER!!!" {
            break;
        }
        let rel = name.trim_start_matches("./").trim_start_matches('/');
        if rel.is_empty() || rel == "." || rel.split('/').any(|c| c == "..") {
            continue; // the root itself, or an escape attempt
        }
        let path = normalize(dest, rel);
        mkdirs(fs, parent(&path));
        let perm = (mode & 0o7777) as u32;
        match mode & 0o170000 {
            0o040000 => {
                if fs.lstat(&path).is_err() {
                    fs.mkdir(&path, perm | 0o700).map_err(|e| format!("mkdir {path}: {e}"))?;
                }
            }
            0o120000 => {
                let _ = fs.remove(&path, false);
                fs.symlink(&String::from_utf8_lossy(data), &path).map_err(|e| format!("symlink {path}: {e}"))?;
            }
            0o100000 => {
                if fs.lstat(&path).is_ok() {
                    let _ = fs.remove(&path, false);
                }
                fs.create(&path, perm).map_err(|e| format!("create {path}: {e}"))?;
                fs.write(&path, 0, data).map_err(|e| format!("write {path}: {e}"))?;
                let _ = fs.chmod(&path, perm);
            }
            _ => continue, // devices, fifos: not in app packages
        }
        written.push(rel.into());
    }
    Ok(written)
}

fn parent(p: &str) -> &str {
    match p.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &p[..i],
    }
}

pub(crate) fn mkdirs(fs: &mut dyn LinFs, p: &str) {
    if p == "/" || fs.lstat(p).is_ok() {
        return;
    }
    mkdirs(fs, parent(p));
    let _ = fs.mkdir(p, 0o755);
}

/// Decompress a Payload / Archive.pax.gz and unpack it below `dest`.
fn payload(fs: &mut dyn LinFs, data: &[u8], dest: &str) -> Result<Vec<String>, String> {
    if data.starts_with(b"pbzx") {
        return Err("this package uses Apple's pbzx (xz) compression, which Prism cannot unpack yet".into());
    }
    let cpio = if data.starts_with(&[0x1f, 0x8b]) { gunzip(data)? } else { data.to_vec() };
    cpio_extract(fs, &cpio, dest)
}

/// The first `.app` folder among `written` (relative to `dest`), as an absolute path.
fn app_in(written: &[String], dest: &str) -> Option<String> {
    let mut best: Option<String> = None;
    for w in written {
        let mut acc = String::new();
        for c in w.split('/') {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc += c;
            if c.to_ascii_lowercase().ends_with(".app") {
                if best.as_ref().map_or(true, |b| acc.len() < b.len()) {
                    best = Some(acc.clone());
                }
                break;
            }
        }
    }
    best.map(|b| normalize(dest, &b))
}

/// Install the package at `path`. Returns the installed app (to start), or `None` when it installed no app.
pub fn install(fs: &mut dyn LinFs, path: &str, diag: &mut String) -> Result<Option<String>, i64> {
    let st = fs.lstat(path)?;
    let r = if st.kind == Kind::Dir { install_bundle(fs, path) } else { install_flat(fs, path) };
    match r {
        Ok((files, app)) => {
            *diag += &format!("prism: installed {path}: {files} items{}\n", app.as_ref().map_or(String::new(), |a| format!(", app {a}")));
            Ok(app)
        }
        Err(m) => {
            *diag += &format!("prism: cannot install {path}: {m}\n");
            Err(ENOEXEC)
        }
    }
}

fn install_flat(fs: &mut dyn LinFs, path: &str) -> Result<(usize, Option<String>), String> {
    let b = read_all(fs, path).map_err(|e| format!("read: {e}"))?;
    if b.len() < 28 || &b[..4] != b"xar!" {
        return Err("not a xar archive".into());
    }
    let (hsize, toc_len) = (be16(&b, 4) as usize, be64(&b, 8) as usize);
    let toc = b.get(hsize..hsize + toc_len).ok_or("xar: truncated table of contents")?;
    let toc = String::from_utf8_lossy(&inflate_zlib(toc)?).into_owned();
    let heap = &b[hsize + toc_len..];
    let files = xar_files(&toc);
    let (mut count, mut app) = (0usize, None);
    // Each component: "<comp>/Payload" (or a top-level "Payload" in a component package) with its PackageInfo.
    for p in files.iter().filter(|f| f.path == "Payload" || f.path.ends_with("/Payload")) {
        let comp = p.path.strip_suffix("Payload").unwrap_or("");
        let dest = files
            .iter()
            .find(|f| f.path == format!("{comp}PackageInfo"))
            .and_then(|f| xar_data(heap, f).ok())
            .and_then(|x| attr(&String::from_utf8_lossy(&x), "install-location"))
            .unwrap_or_else(|| "/".into());
        let dest = normalize("/", &dest);
        mkdirs(fs, &dest);
        let w = payload(fs, &xar_data(heap, p)?, &dest)?;
        count += w.len();
        if app.is_none() {
            app = app_in(&w, &dest);
        }
    }
    if count == 0 {
        return Err("no payload in the package".into());
    }
    Ok((count, app))
}

fn install_bundle(fs: &mut dyn LinFs, path: &str) -> Result<(usize, Option<String>), String> {
    // .mpkg bundle: its packages in order
    if let Ok(list) = fs.list(&format!("{path}/Contents/Packages")) {
        let (mut count, mut app) = (0, None);
        for e in list {
            let sub = format!("{path}/Contents/Packages/{}", e.name);
            if is_package(fs, &sub) {
                let (n, a) = if e.kind == Kind::Dir { install_bundle(fs, &sub)? } else { install_flat(fs, &sub)? };
                count += n;
                app = app.or(a);
            }
        }
        return if count == 0 { Err("no packages inside".into()) } else { Ok((count, app)) };
    }
    let archive = format!("{path}/Contents/Archive.pax.gz");
    let data = read_all(fs, &archive).map_err(|_| String::from("no Contents/Archive.pax.gz"))?;
    let info = read_all(fs, &format!("{path}/Contents/Info.plist")).unwrap_or_default();
    let info = String::from_utf8_lossy(&info);
    let dest = info
        .find("IFPkgFlagDefaultLocation")
        .and_then(|i| info[i..].split("<string>").nth(1)?.split("</string>").next().map(String::from))
        .unwrap_or_else(|| "/Applications".into());
    let dest = normalize("/", &dest);
    mkdirs(fs, &dest);
    let w = payload(fs, &data, &dest)?;
    Ok((w.len(), app_in(&w, &dest)))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use loos_lin::fs::MemFs;

    /// An odc cpio archive of `(path, mode, data)`.
    pub fn cpio(entries: &[(&str, u32, &[u8])]) -> Vec<u8> {
        let mut o = Vec::new();
        let mut add = |name: &str, mode: u32, data: &[u8]| {
            let h = format!("070707{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:011o}{:06o}{:011o}", 0, 1, mode, 0, 0, 1, 0, 0, name.len() + 1, data.len());
            o.extend_from_slice(h.as_bytes());
            o.extend_from_slice(name.as_bytes());
            o.push(0);
            o.extend_from_slice(data);
        };
        for (n, m, d) in entries {
            add(n, *m, d);
        }
        add("TRAILER!!!", 0, b"");
        o
    }

    pub fn gzip(d: &[u8]) -> Vec<u8> {
        let mut g = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
        g.extend(miniz_oxide::deflate::compress_to_vec(d, 6));
        g.extend_from_slice(&[0; 8]); // crc, size (not checked)
        g
    }

    /// A flat package with one component installing `payload` to `location`.
    pub fn flat_pkg(location: &str, payload: &[u8]) -> Vec<u8> {
        let info = format!("<?xml version=\"1.0\"?><pkg-info identifier=\"org.test\" install-location=\"{location}\"/>");
        let heap: Vec<u8> = [payload, info.as_bytes()].concat();
        let toc = format!(
            "<?xml version=\"1.0\"?><xar><toc><file id=\"1\"><name>app.pkg</name><type>directory</type>\
             <file id=\"2\"><data><length>{pl}</length><offset>0</offset><size>{pl}</size><encoding style=\"application/octet-stream\"/></data>\
             <name>Payload</name><type>file</type></file>\
             <file id=\"3\"><data><length>{il}</length><offset>{pl}</offset><size>{il}</size><encoding style=\"application/octet-stream\"/></data>\
             <name>PackageInfo</name><type>file</type></file></file></toc></xar>",
            pl = payload.len(),
            il = info.len()
        );
        let z = miniz_oxide::deflate::compress_to_vec_zlib(toc.as_bytes(), 6);
        let mut b = b"xar!".to_vec();
        b.extend_from_slice(&28u16.to_be_bytes());
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&(z.len() as u64).to_be_bytes());
        b.extend_from_slice(&(toc.len() as u64).to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend(z);
        b.extend(heap);
        b
    }

    #[test]
    fn flat_package_installs_its_app_into_applications() {
        let payload = gzip(&cpio(&[
            (".", 0o40755, b""),
            ("./Demo.app", 0o40755, b""),
            ("./Demo.app/Contents", 0o40755, b""),
            ("./Demo.app/Contents/MacOS", 0o40755, b""),
            ("./Demo.app/Contents/MacOS/Demo", 0o100755, b"\xcf\xfa\xed\xfe"),
            ("./Demo.app/Contents/Current", 0o120755, b"MacOS"),
            ("../escape", 0o100644, b"no"),
        ]));
        let mut fs = MemFs::new();
        fs.add_file("/home/Downloads/Demo.pkg", &flat_pkg("/Applications", &payload), 0o644);
        assert!(is_package(&mut fs, "/home/Downloads/Demo.pkg"));
        let mut diag = String::new();
        let app = install(&mut fs, "/home/Downloads/Demo.pkg", &mut diag).unwrap();
        assert_eq!(app.as_deref(), Some("/Applications/Demo.app"), "{diag}");
        assert_eq!(fs.file("/Applications/Demo.app/Contents/MacOS/Demo"), Some(&b"\xcf\xfa\xed\xfe"[..]));
        assert_eq!(fs.lstat("/Applications/Demo.app/Contents/MacOS/Demo").unwrap().mode & 0o777, 0o755);
        assert_eq!(fs.readlink("/Applications/Demo.app/Contents/Current").unwrap(), "MacOS");
        assert!(fs.lstat("/escape").is_err() && fs.lstat("/Applications/../escape").is_err(), "no writes outside the destination");
    }

    #[test]
    fn bundle_package_and_unsupported_compression_say_what_happened() {
        let mut fs = MemFs::new();
        fs.add_file("/p/Old.pkg/Contents/Archive.pax.gz", &gzip(&cpio(&[("./Old.app/Contents/MacOS/Old", 0o100755, b"x")])), 0o644);
        fs.add_file("/p/Old.pkg/Contents/Info.plist", b"<key>IFPkgFlagDefaultLocation</key><string>/Applications</string>", 0o644);
        let mut diag = String::new();
        assert_eq!(install(&mut fs, "/p/Old.pkg", &mut diag).unwrap().as_deref(), Some("/Applications/Old.app"));
        fs.add_file("/p/New.pkg", &flat_pkg("/", b"pbzx\0\0\0\0"), 0o644);
        assert_eq!(install(&mut fs, "/p/New.pkg", &mut diag), Err(ENOEXEC));
        assert!(diag.contains("pbzx"), "{diag}");
    }
}
