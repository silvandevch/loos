//! macOS disk images: opening a `.dmg` does what people do by hand — drag the app inside to /Applications —
//! and returns that app for Prism to start.
//!
//! UDIF images (`koly` trailer, `blkx` table in the XML plist, chunks raw / zero / zlib) and plain raw images,
//! holding an HFS+ (or HFSX) volume. Files are read through the first eight extents of their data fork (fresh
//! disk images are not fragmented). bzip2 / LZFSE / LZMA chunks and APFS volumes are not supported and say so.

use crate::pkg::{inflate_zlib, mkdirs};
use alloc::{collections::BTreeMap, format, string::String, vec, vec::Vec};
use loos_lin::fs::{normalize, LinFs};

const ENOEXEC: i64 = 8;

/// Is `path` a disk image (by name; the contents are checked when it opens)?
pub fn is_disk_image(fs: &mut dyn LinFs, path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".dmg") && fs.lstat(path).is_ok_and(|s| s.kind == loos_lin::fs::Kind::File)
}

fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}
fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes(b[o..o + 4].try_into().unwrap())
}
fn be64(b: &[u8], o: usize) -> u64 {
    u64::from_be_bytes(b[o..o + 8].try_into().unwrap())
}

fn base64(s: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0);
    for v in s.bytes().filter_map(val) {
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// The partitions of a UDIF image, decompressed: `(name, bytes)`.
fn udif_partitions(img: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let k = img.len().checked_sub(512).ok_or("too small")?;
    let t = &img[k..];
    let data_fork = be64(t, 24) as usize;
    let (xo, xl) = (be64(t, 216) as usize, be64(t, 224) as usize);
    let xml = String::from_utf8_lossy(img.get(xo..xo + xl).ok_or("plist outside the image")?).into_owned();
    let blkx = xml.find("<key>blkx</key>").ok_or("no blkx table")?;
    let mut parts = Vec::new();
    for d in xml[blkx..].split("<dict>").skip(1) {
        let Some(data) = d.split("<key>Data</key>").nth(1).and_then(|x| x.split("<data>").nth(1)).and_then(|x| x.split("</data>").next()) else { continue };
        let name = d.split("<key>Name</key>").nth(1).and_then(|x| x.split("<string>").nth(1)).and_then(|x| x.split("</string>").next()).unwrap_or("").into();
        let m = base64(data);
        if m.len() < 204 || &m[..4] != b"mish" {
            continue;
        }
        let (sectors, data_off, n) = (be64(&m, 16) as usize, be64(&m, 24) as usize, be32(&m, 200) as usize);
        let mut out = vec![0u8; sectors * 512];
        for c in 0..n {
            let e = 204 + c * 40;
            let Some(ch) = m.get(e..e + 40) else { break };
            let (ty, sec, cnt, co, cl) = (be32(ch, 0), be64(ch, 8) as usize * 512, be64(ch, 16) as usize * 512, be64(ch, 24) as usize, be64(ch, 32) as usize);
            let src = data_fork + data_off + co;
            let dst = out.get_mut(sec..sec + cnt).ok_or("chunk outside its partition")?;
            match ty {
                0 | 2 | 0x7fff_fffe | 0xffff_ffff => {}
                1 => dst.copy_from_slice(img.get(src..src + cnt).ok_or("chunk outside the image")?),
                0x8000_0005 => {
                    let z = inflate_zlib(img.get(src..src + cl).ok_or("chunk outside the image")?)?;
                    let n = z.len().min(dst.len());
                    dst[..n].copy_from_slice(&z[..n]);
                }
                0x8000_0006 => return Err("bzip2-compressed image (UDBZ): not supported yet".into()),
                0x8000_0007 => return Err("LZFSE-compressed image (ULFO): not supported yet".into()),
                0x8000_0008 => return Err("LZMA-compressed image (ULMO): not supported yet".into()),
                t => return Err(format!("unknown chunk type {t:#x}")),
            }
        }
        parts.push((name, out));
    }
    Ok(parts)
}

/// One HFS+ catalog entry.
struct Node {
    parent: u32,
    name: String,
    folder: bool,
    mode: u16,
    /// `(logical size, extents)`
    fork: (u64, [(u32, u32); 8]),
    symlink: bool,
}

fn fork(b: &[u8], o: usize) -> (u64, [(u32, u32); 8]) {
    let mut ex = [(0, 0); 8];
    for (i, e) in ex.iter_mut().enumerate() {
        *e = (be32(b, o + 16 + i * 8), be32(b, o + 20 + i * 8));
    }
    (be64(b, o), ex)
}

struct Hfs<'a> {
    v: &'a [u8],
    bs: usize,
}

impl Hfs<'_> {
    fn read_fork(&self, f: &(u64, [(u32, u32); 8])) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(f.0 as usize);
        for &(start, count) in &f.1 {
            if out.len() as u64 >= f.0 || count == 0 {
                break;
            }
            let (a, n) = (start as usize * self.bs, count as usize * self.bs);
            out.extend_from_slice(self.v.get(a..a + n).ok_or("extent outside the volume")?);
        }
        if (out.len() as u64) < f.0 {
            return Err("fragmented file (extents overflow): not supported yet".into());
        }
        out.truncate(f.0 as usize);
        Ok(out)
    }

    /// Every catalog entry by its CNID.
    fn catalog(&self) -> Result<BTreeMap<u32, Node>, String> {
        let cat = self.read_fork(&fork(self.v, 1024 + 0x110))?;
        if cat.len() < 512 {
            return Err("empty catalog".into());
        }
        let node_size = be16(&cat, 32) as usize;
        let mut leaf = be32(&cat, 24) as usize;
        let mut nodes = BTreeMap::new();
        let mut guard = 0;
        while leaf != 0 && guard < 1 << 20 {
            guard += 1;
            let nb = cat.get(leaf * node_size..(leaf + 1) * node_size).ok_or("catalog node outside the tree")?;
            for r in 0..be16(nb, 10) as usize {
                let off = be16(nb, node_size - 2 * (r + 1)) as usize;
                let klen = be16(nb, off) as usize;
                let parent = be32(nb, off + 2);
                let nlen = be16(nb, off + 6) as usize;
                let name: Vec<u16> = (0..nlen).map(|i| be16(nb, off + 8 + 2 * i)).collect();
                let name = String::from_utf16_lossy(&name).replace('/', ":");
                let d = off + 2 + klen;
                match be16(nb, d) {
                    1 => {
                        nodes.insert(be32(nb, d + 8), Node { parent, name, folder: true, mode: 0o40755, fork: (0, [(0, 0); 8]), symlink: false });
                    }
                    2 => {
                        let mode = be16(nb, d + 42);
                        let symlink = &nb[d + 48..d + 56] == b"slnkrhap" || mode & 0o170000 == 0o120000;
                        nodes.insert(be32(nb, d + 8), Node { parent, name, folder: false, mode, fork: fork(nb, d + 88), symlink });
                    }
                    _ => {} // thread records
                }
            }
            leaf = be32(nb, 0) as usize;
        }
        Ok(nodes)
    }
}

/// Copy catalog entry `id` (and below) to `dest`; returns the number of items written.
fn copy_out(h: &Hfs, nodes: &BTreeMap<u32, Node>, id: u32, dest: &str, fs: &mut dyn LinFs) -> Result<usize, String> {
    let n = &nodes[&id];
    if n.folder {
        mkdirs(fs, dest);
        let mut count = 1;
        let kids: Vec<u32> = nodes.iter().filter(|(_, c)| c.parent == id).map(|(k, _)| *k).collect();
        for k in kids {
            count += copy_out(h, nodes, k, &format!("{dest}/{}", nodes[&k].name), fs)?;
        }
        return Ok(count);
    }
    let data = h.read_fork(&n.fork)?;
    let _ = fs.remove(dest, false);
    if n.symlink {
        fs.symlink(&String::from_utf8_lossy(&data), dest).map_err(|e| format!("symlink {dest}: {e}"))?;
    } else {
        let perm = if n.mode & 0o777 == 0 { 0o644 } else { (n.mode & 0o7777) as u32 };
        fs.create(dest, perm).map_err(|e| format!("create {dest}: {e}"))?;
        fs.write(dest, 0, &data).map_err(|e| format!("write {dest}: {e}"))?;
        let _ = fs.chmod(dest, perm);
    }
    Ok(1)
}

/// The volume inside the image (UDIF or raw), as bytes.
fn volume(img: Vec<u8>) -> Result<Vec<u8>, String> {
    let hfs = |b: &[u8]| b.len() > 1026 && matches!(&b[1024..1026], b"H+" | b"HX");
    if img.len() >= 512 && &img[img.len() - 512..img.len() - 508] == b"koly" {
        let parts = udif_partitions(&img)?;
        if let Some((_, p)) = parts.iter().find(|(_, p)| hfs(p)) {
            return Ok(p.clone());
        }
        if parts.iter().any(|(n, p)| n.contains("APFS") || p.get(32..36) == Some(b"NXSB")) {
            return Err("APFS disk image: not supported yet (HFS+ images are)".into());
        }
        return Err("no HFS+ volume in the image".into());
    }
    if hfs(&img) {
        return Ok(img);
    }
    Err("not a disk image Prism can read (UDIF or raw HFS+)".into())
}

/// Open the disk image at `path`: copy its first `.app` into /Applications. Returns that app's path.
pub fn open(fs: &mut dyn LinFs, path: &str, diag: &mut String) -> Result<String, i64> {
    let r = (|| -> Result<(String, usize), String> {
        let st = fs.lstat(path).map_err(|e| format!("stat: {e}"))?;
        let mut img = vec![0u8; st.size as usize];
        let n = fs.read(path, 0, &mut img).map_err(|e| format!("read: {e}"))?;
        img.truncate(n);
        let v = volume(img)?;
        let h = Hfs { v: &v, bs: be32(&v, 1024 + 40) as usize };
        if h.bs < 512 || !h.bs.is_power_of_two() {
            return Err("bad HFS+ block size".into());
        }
        let nodes = h.catalog()?;
        // The root folder is CNID 2; apps sit at its top level (next to an /Applications shortcut).
        let (&id, app) = nodes
            .iter()
            .filter(|(_, n)| n.folder && n.parent == 2 && n.name.to_ascii_lowercase().ends_with(".app"))
            .min_by_key(|(_, n)| n.name.clone())
            .ok_or("no .app at the top of the image")?;
        let dest = normalize("/Applications", &app.name);
        mkdirs(fs, "/Applications");
        let count = copy_out(&h, &nodes, id, &dest, fs)?;
        Ok((dest, count))
    })();
    match r {
        Ok((app, count)) => {
            *diag += &format!("prism: copied {app} from {path} ({count} items)\n");
            Ok(app)
        }
        Err(m) => {
            *diag += &format!("prism: cannot open {path}: {m}\n");
            Err(ENOEXEC)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loos_lin::fs::MemFs;

    const BS: usize = 4096;

    /// A small HFS+ volume: root "Demo" (CNID 2) holding Demo.app/Contents/MacOS/Demo and an Applications link.
    fn hfs_volume() -> Vec<u8> {
        let mut v = vec![0u8; 16 * BS];
        // catalog: blocks 2..4 (two 4 KiB nodes: header + one leaf)
        let ns = BS;
        let cat_at = 2 * BS;
        let file_at = 8 * BS;
        v[file_at..file_at + 4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        v[9 * BS..9 * BS + 13].copy_from_slice(b"/Applications");
        let h = 1024;
        v[h..h + 2].copy_from_slice(b"H+");
        v[h + 40..h + 44].copy_from_slice(&(BS as u32).to_be_bytes());
        let cf = h + 0x110;
        v[cf..cf + 8].copy_from_slice(&((2 * ns) as u64).to_be_bytes());
        v[cf + 16..cf + 20].copy_from_slice(&2u32.to_be_bytes());
        v[cf + 20..cf + 24].copy_from_slice(&2u32.to_be_bytes());
        // header node: first leaf = 1, node size
        v[cat_at + 24..cat_at + 28].copy_from_slice(&1u32.to_be_bytes());
        v[cat_at + 32..cat_at + 34].copy_from_slice(&(ns as u16).to_be_bytes());
        // leaf node records
        let leaf = cat_at + ns;
        let mut recs: Vec<Vec<u8>> = Vec::new();
        let key = |parent: u32, name: &str| {
            let u: Vec<u16> = name.encode_utf16().collect();
            let mut k = Vec::new();
            k.extend_from_slice(&((6 + 2 * u.len()) as u16).to_be_bytes());
            k.extend_from_slice(&parent.to_be_bytes());
            k.extend_from_slice(&(u.len() as u16).to_be_bytes());
            for c in u {
                k.extend_from_slice(&c.to_be_bytes());
            }
            k
        };
        let folder = |parent: u32, name: &str, id: u32| {
            let mut r = key(parent, name);
            let mut d = vec![0u8; 88];
            d[1] = 1;
            d[8..12].copy_from_slice(&id.to_be_bytes());
            r.extend(d);
            r
        };
        let file = |parent: u32, name: &str, id: u32, mode: u16, ty: &[u8; 8], block: u32, size: u64| {
            let mut r = key(parent, name);
            let mut d = vec![0u8; 248];
            d[1] = 2;
            d[8..12].copy_from_slice(&id.to_be_bytes());
            d[42..44].copy_from_slice(&mode.to_be_bytes());
            d[48..56].copy_from_slice(ty);
            d[88..96].copy_from_slice(&size.to_be_bytes());
            d[104..108].copy_from_slice(&block.to_be_bytes());
            d[108..112].copy_from_slice(&1u32.to_be_bytes());
            r.extend(d);
            r
        };
        recs.push(folder(1, "Demo", 2));
        recs.push(folder(2, "Demo.app", 16));
        recs.push(folder(16, "Contents", 17));
        recs.push(folder(17, "MacOS", 18));
        recs.push(file(18, "Demo", 19, 0o100755, b"APPL????", 8, 4));
        recs.push(file(2, "Applications", 20, 0o120755, b"slnkrhap", 9, 13));
        let mut off = 14;
        for (i, r) in recs.iter().enumerate() {
            v[leaf + off..leaf + off + r.len()].copy_from_slice(r);
            let slot = leaf + ns - 2 * (i + 1);
            v[slot..slot + 2].copy_from_slice(&(off as u16).to_be_bytes());
            off += r.len();
        }
        v[leaf + 8] = 0xff; // leaf kind
        v[leaf + 10..leaf + 12].copy_from_slice(&(recs.len() as u16).to_be_bytes());
        v
    }

    /// The volume as a UDIF image with one zlib chunk.
    fn udif(vol: &[u8]) -> Vec<u8> {
        let z = miniz_oxide::deflate::compress_to_vec_zlib(vol, 6);
        let mut mish = vec![0u8; 204 + 2 * 40];
        mish[..4].copy_from_slice(b"mish");
        mish[16..24].copy_from_slice(&((vol.len() / 512) as u64).to_be_bytes());
        mish[200..204].copy_from_slice(&2u32.to_be_bytes());
        let c = 204;
        mish[c..c + 4].copy_from_slice(&0x8000_0005u32.to_be_bytes());
        mish[c + 16..c + 24].copy_from_slice(&((vol.len() / 512) as u64).to_be_bytes());
        mish[c + 32..c + 40].copy_from_slice(&(z.len() as u64).to_be_bytes());
        mish[c + 40..c + 44].copy_from_slice(&0xffff_ffffu32.to_be_bytes());
        const B: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut b64 = String::new();
        for ch in mish.chunks(3) {
            let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                b64.push(if i <= ch.len() { B[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
            }
        }
        let xml = format!("<plist><dict><key>resource-fork</key><dict><key>blkx</key><array><dict><key>Data</key><data>{b64}</data><key>Name</key><string>disk image (Apple_HFS : 1)</string></dict></array></dict></dict></plist>");
        let mut img = z.clone();
        let xo = img.len();
        img.extend_from_slice(xml.as_bytes());
        let mut koly = vec![0u8; 512];
        koly[..4].copy_from_slice(b"koly");
        koly[32..40].copy_from_slice(&(z.len() as u64).to_be_bytes());
        koly[216..224].copy_from_slice(&(xo as u64).to_be_bytes());
        koly[224..232].copy_from_slice(&(xml.len() as u64).to_be_bytes());
        img.extend(koly);
        img
    }

    #[test]
    fn compressed_disk_image_copies_its_app_to_applications() {
        let mut fs = MemFs::new();
        fs.add_file("/home/Downloads/Demo.dmg", &udif(&hfs_volume()), 0o644);
        assert!(is_disk_image(&mut fs, "/home/Downloads/Demo.dmg"));
        let mut diag = String::new();
        assert_eq!(open(&mut fs, "/home/Downloads/Demo.dmg", &mut diag).as_deref(), Ok("/Applications/Demo.app"), "{diag}");
        assert_eq!(fs.file("/Applications/Demo.app/Contents/MacOS/Demo"), Some(&[0xcf, 0xfa, 0xed, 0xfe][..]));
        assert_eq!(fs.lstat("/Applications/Demo.app/Contents/MacOS/Demo").unwrap().mode & 0o777, 0o755);
        assert!(fs.lstat("/Applications/Applications").is_err(), "the image's /Applications shortcut is not copied");
    }

    #[test]
    fn raw_images_open_and_unknown_ones_explain() {
        let mut fs = MemFs::new();
        fs.add_file("/r.dmg", &hfs_volume(), 0o644);
        let mut diag = String::new();
        assert_eq!(open(&mut fs, "/r.dmg", &mut diag).as_deref(), Ok("/Applications/Demo.app"));
        fs.add_file("/bad.dmg", b"not an image at all", 0o644);
        assert_eq!(open(&mut fs, "/bad.dmg", &mut diag), Err(ENOEXEC));
        assert!(diag.contains("cannot open /bad.dmg"), "{diag}");
    }
}
