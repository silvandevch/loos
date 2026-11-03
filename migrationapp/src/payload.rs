//! The LoOS installer payload is compiled into the app (zstd-compressed), then
//! decompressed and hash-verified on use. The app never downloads anything.

use crate::model::Firmware;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const UEFI_ZST: &[u8] = include_bytes!("../assets/loos-uefi-installer.img.zst");
const BIOS_ZST: &[u8] = include_bytes!("../assets/loos-bios-installer.img.zst");
const MANIFEST: &str = include_str!("../assets/payload.json");

#[derive(Debug, Deserialize, Clone)]
pub struct ImageInfo {
    pub raw_bytes: u64,
    pub raw_sha256: String,
    pub compressed_sha256: String,
    pub compression: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PayloadManifest {
    #[allow(dead_code)]
    pub name: String,
    pub built_utc: String,
    #[allow(dead_code)]
    pub source: String,
    pub uefi: ImageInfo,
    pub bios: ImageInfo,
}

#[derive(Debug, thiserror::Error)]
pub enum PayloadError {
    #[error("internal payload manifest is invalid: {0}")]
    Manifest(String),
    #[error("embedded payload failed its integrity check")]
    Integrity,
    #[error("payload decompression failed: {0}")]
    Decompress(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("operation cancelled")]
    Cancelled,
    #[error("payload images are not valid FAT32/GPT images: {0}")]
    Image(String),
    #[error("the embedded image is larger than the safety limit")]
    TooLarge,
}

pub struct Payload {
    pub manifest: PayloadManifest,
}

#[derive(Clone, Debug)]
pub struct PayloadImage {
    pub firmware: Firmware,
    pub compressed: &'static [u8],
    pub raw_bytes: u64,
    pub raw_sha256_hex: String,
}

impl Payload {
    pub fn embedded() -> Result<Self, PayloadError> {
        let manifest: PayloadManifest =
            serde_json::from_str(MANIFEST).map_err(|e| PayloadError::Manifest(e.to_string()))?;
        Ok(Self { manifest })
    }

    pub fn image(&self, firmware: Firmware) -> Result<PayloadImage, PayloadError> {
        let (compressed, info) = match firmware {
            Firmware::Uefi => (UEFI_ZST, &self.manifest.uefi),
            Firmware::Bios => (BIOS_ZST, &self.manifest.bios),
        };
        if info.compression != "zstd" {
            return Err(PayloadError::Manifest("unsupported compression".into()));
        }
        // The compressed bytes are part of the binary and cannot change, but a
        // corrupted build should fail closed rather than write garbage.
        let got = sha256_hex(compressed);
        if !got.eq_ignore_ascii_case(&info.compressed_sha256) {
            return Err(PayloadError::Integrity);
        }
        Ok(PayloadImage {
            firmware,
            compressed,
            raw_bytes: info.raw_bytes,
            raw_sha256_hex: info.raw_sha256.clone(),
        })
    }

    pub fn all_images(&self) -> Result<Vec<PayloadImage>, PayloadError> {
        Ok(vec![self.image(Firmware::Uefi)?, self.image(Firmware::Bios)?])
    }
}

impl PayloadImage {
    /// Streams the decompressed image to `writer`, hashing as it goes and
    /// checking the expected raw size. Cancellation is checked every chunk.
    pub fn write_to<W: Write>(
        &self,
        writer: &mut W,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<(), PayloadError> {
        let mut dec = zstd::stream::read::Decoder::new(self.compressed)
            .map_err(|e| PayloadError::Decompress(e.to_string()))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 4 * 1024 * 1024];
        let mut written = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(PayloadError::Cancelled);
            }
            let n = dec.read(&mut buf).map_err(|e| PayloadError::Decompress(e.to_string()))?;
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            written += n as u64;
            if written > self.raw_bytes.saturating_add(1024 * 1024) {
                return Err(PayloadError::TooLarge);
            }
            progress(written, self.raw_bytes);
        }
        if written != self.raw_bytes {
            return Err(PayloadError::Decompress(format!(
                "expected {} bytes, got {}",
                self.raw_bytes, written
            )));
        }
        let got = hex::encode(hasher.finalize());
        if !got.eq_ignore_ascii_case(&self.raw_sha256_hex) {
            return Err(PayloadError::Integrity);
        }
        Ok(())
    }

    pub fn decompress_to_memory(&self, cancel: &AtomicBool) -> Result<Vec<u8>, PayloadError> {
        let mut out = Vec::with_capacity(self.raw_bytes.min(256 * 1024 * 1024) as usize);
        self.write_to(&mut out, cancel, |_, _| {})?;
        Ok(out)
    }

    /// Reads just the files needed for local UEFI setup from the embedded ESP.
    pub fn extract_uefi_loader(&self, cancel: &AtomicBool) -> Result<BootFiles, PayloadError> {
        let raw = self.decompress_to_memory(cancel)?;
        let parts = gpt_partitions(&raw).map_err(PayloadError::Image)?;
        let esp = parts
            .into_iter()
            .find(|p| p.is_esp)
            .ok_or_else(|| PayloadError::Image("no EFI System Partition in the image".into()))?;
        let fat = Fat32::new(&raw, esp.start_byte(), esp.size_bytes()).map_err(PayloadError::Image)?;
        let efi = fat
            .read_path("EFI/BOOT/BOOTX64.EFI")
            .map_err(|e| PayloadError::Image(format!("boot loader: {e}")))?
            .ok_or_else(|| PayloadError::Image("BOOTX64.EFI missing from image".into()))?;
        let kernel = fat
            .read_path("LOOS/KERNEL.ELF")
            .map_err(|e| PayloadError::Image(format!("kernel: {e}")))?
            .ok_or_else(|| PayloadError::Image("KERNEL.ELF missing from image".into()))?;
        Ok(BootFiles {
            loader: efi,
            kernel,
        })
    }
}

#[derive(Debug, Clone)]
pub struct BootFiles {
    pub loader: Vec<u8>,
    pub kernel: Vec<u8>,
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

pub fn sha256_file(path: &Path, cancel: &AtomicBool, mut progress: impl FnMut(u64)) -> Result<String, PayloadError> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 4 * 1024 * 1024];
    let mut total = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(PayloadError::Cancelled);
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        total += n as u64;
        progress(total.min(len));
    }
    Ok(hex::encode(h.finalize()))
}

#[derive(Debug, Clone, Copy)]
struct GptPart {
    start: u64,
    len: u64,
    is_esp: bool,
}

impl GptPart {
    fn start_byte(self) -> u64 {
        self.start * 512
    }
    fn size_bytes(self) -> u64 {
        self.len * 512
    }
}

fn gpt_partitions(img: &[u8]) -> Result<Vec<GptPart>, String> {
    if img.len() < 1024 || &img[512..520] != b"EFI PART" {
        return Err("not a GPT disk".into());
    }
    let entry_lba = le64(img, 512 + 72);
    let count = le32(img, 512 + 80) as usize;
    let esize = le32(img, 512 + 84) as usize;
    if esize < 128 || count > 256 {
        return Err("unsupported GPT entry layout".into());
    }
    let table_off = (entry_lba as usize).checked_mul(512).ok_or("GPT table overflow")?;
    if table_off + count * esize > img.len() {
        return Err("truncated GPT table".into());
    }
    let esp_type = [
        0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
    ];
    let mut out = Vec::new();
    for i in 0..count {
        let e = &img[table_off + i * esize..table_off + i * esize + 128];
        if e[..16].iter().all(|b| *b == 0) {
            continue;
        }
        let first = le64(e, 32);
        let last = le64(e, 40);
        if last >= first {
            out.push(GptPart {
                start: first,
                len: last - first + 1,
                is_esp: e[..16] == esp_type,
            });
        }
    }
    Ok(out)
}

fn le16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}
fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}
fn le64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        b[off],
        b[off + 1],
        b[off + 2],
        b[off + 3],
        b[off + 4],
        b[off + 5],
        b[off + 6],
        b[off + 7],
    ])
}

struct Fat32<'a> {
    img: &'a [u8],
    base: usize,
    bps: usize,
    spc: usize,
    reserved: usize,
    root_cluster: u32,
    first_data_sector: usize,
    total_clusters: usize,
}

impl<'a> Fat32<'a> {
    fn new(img: &'a [u8], base: u64, size: u64) -> Result<Self, String> {
        let base = base as usize;
        let size = size as usize;
        if base + 512 > img.len() || size < 512 {
            return Err("ESP is outside the image".into());
        }
        let b = &img[base..base + size];
        let bps = le16(b, 11) as usize;
        let spc = b[13] as usize;
        let reserved = le16(b, 14) as usize;
        let num_fats = b[16] as usize;
        let fat16 = le16(b, 22) as usize;
        let total16 = le16(b, 19) as usize;
        let total32 = le32(b, 32) as usize;
        let total = if total16 != 0 { total16 } else { total32 };
        let fat_sectors = if fat16 != 0 { fat16 } else { le32(b, 36) as usize };
        let root_cluster = le32(b, 44);
        if bps != 512 || !(1..=128).contains(&spc) || reserved == 0 || !(1..=2).contains(&num_fats) || fat_sectors == 0 || total == 0 {
            return Err("unsupported FAT32 layout".into());
        }
        let first_data_sector = reserved + num_fats * fat_sectors;
        let total_clusters = total.saturating_sub(first_data_sector) / spc;
        if total_clusters == 0 || first_data_sector >= total {
            return Err("empty FAT32 data area".into());
        }
        Ok(Self {
            img,
            base,
            bps,
            spc,
            reserved,
            root_cluster,
            first_data_sector,
            total_clusters,
        })
    }

    fn cluster_size(&self) -> usize {
        self.bps * self.spc
    }

    fn cluster_off(&self, cluster: u32) -> Result<usize, String> {
        if cluster < 2 || (cluster as usize) >= self.total_clusters + 2 {
            return Err("cluster out of range".into());
        }
        let sector = self.first_data_sector + (cluster as usize - 2) * self.spc;
        let off = self.base + sector * self.bps;
        if off + self.cluster_size() > self.img.len() {
            return Err("cluster outside image".into());
        }
        Ok(off)
    }

    fn next_cluster(&self, cluster: u32) -> Result<u32, String> {
        let fat_start = self.base + self.reserved * self.bps;
        let off = fat_start + cluster as usize * 4;
        if off + 4 > self.img.len() {
            return Err("FAT outside image".into());
        }
        Ok(le32(self.img, off) & 0x0fff_ffff)
    }

    fn read_chain(&self, start: u32, max_bytes: usize) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let mut c = start;
        let mut seen = 0usize;
        while c >= 2 && c < 0x0fff_fff8 {
            if seen > self.total_clusters + 2 {
                return Err("FAT loop".into());
            }
            let off = self.cluster_off(c)?;
            out.extend_from_slice(&self.img[off..off + self.cluster_size()]);
            if out.len() >= max_bytes {
                out.truncate(max_bytes);
                break;
            }
            c = self.next_cluster(c)?;
            seen += 1;
        }
        Ok(out)
    }

    fn find_in_dir(&self, dir_cluster: u32, want: &str) -> Result<Option<(u8, u32, u64)>, String> {
        let data = self.read_chain(dir_cluster, 16 * 1024 * 1024)?;
        let mut lfn = String::new();
        for ent in data.chunks_exact(32) {
            if ent[0] == 0 {
                break;
            }
            if ent[0] == 0xe5 {
                lfn.clear();
                continue;
            }
            let attr = ent[11];
            if attr == 0x0f {
                let seq = ent[0] & 0x1f;
                if seq == 0 {
                    lfn.clear();
                }
                let mut part: Vec<u16> = Vec::new();
                for offs in [1usize, 3, 5, 7, 9] {
                    let u = u16::from_le_bytes([ent[offs], ent[offs + 1]]);
                    if u != 0 && u != 0xffff {
                        part.push(u);
                    }
                }
                for offs in [14usize, 16, 18, 20, 22, 24] {
                    let u = u16::from_le_bytes([ent[offs], ent[offs + 1]]);
                    if u != 0 && u != 0xffff {
                        part.push(u);
                    }
                }
                let s = String::from_utf16_lossy(&part);
                lfn = format!("{s}{lfn}");
                continue;
            }
            if attr & 0x08 != 0 {
                lfn.clear();
                continue;
            }
            let base = String::from_utf8_lossy(&ent[0..8]).trim_end().to_string();
            let ext = String::from_utf8_lossy(&ent[8..11]).trim_end().to_string();
            let mut short = if ext.is_empty() { base } else { format!("{base}.{ext}") };
            if short.starts_with('.') {
                short = short.trim_start_matches('.').to_string();
            }
            let long = std::mem::take(&mut lfn);
            let name = if !long.is_empty() { long } else { short };
            if name.eq_ignore_ascii_case(want) {
                let cluster = ((le16(ent, 20) as u32) << 16) | le16(ent, 26) as u32;
                let size = le32(ent, 28) as u64;
                return Ok(Some((attr, cluster, size)));
            }
        }
        Ok(None)
    }

    fn read_path(&self, path: &str) -> Result<Option<Vec<u8>>, String> {
        let mut cluster = self.root_cluster;
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return Ok(None);
        }
        for (i, part) in parts.iter().enumerate() {
            let Some((attr, next, size)) = self.find_in_dir(cluster, part)? else {
                return Ok(None);
            };
            let last = i + 1 == parts.len();
            if last {
                if attr & 0x10 != 0 {
                    return Ok(None);
                }
                if size > 64 * 1024 * 1024 {
                    return Err("boot file is implausibly large".into());
                }
                return Ok(Some(self.read_chain(next, size as usize)?));
            }
            if attr & 0x10 == 0 {
                return Ok(None);
            }
            cluster = next;
        }
        Ok(None)
    }
}

/// Extracts the raw UEFI image to a sibling file so it can be inspected or
/// archived. Used by `--extract-payload` for power users.
pub fn extract_payload_to(path: &Path, firmware: Firmware) -> Result<PathBuf, PayloadError> {
    let payload = Payload::embedded()?;
    let image = payload.image(firmware)?;
    let cancel = AtomicBool::new(false);
    let mut f = File::create(path)?;
    image.write_to(&mut f, &cancel, |_, _| {})?;
    f.sync_all()?;
    Ok(path.to_path_buf())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_images_verify_and_uefi_loader_is_extractable() {
        let p = Payload::embedded().expect("manifest");
        let cancel = AtomicBool::new(false);
        for img in p.all_images().unwrap() {
            let mut sink = std::io::sink();
            img.write_to(&mut sink, &cancel, |_, _| {}).expect("decompress + hash");
        }
        let uefi = p.image(Firmware::Uefi).unwrap();
        let files = uefi.extract_uefi_loader(&cancel).expect("extract");
        assert!(files.loader.starts_with(b"MZ"), "loader is not a PE");
        assert!(files.kernel.starts_with(b"\x7fELF"), "kernel is not an ELF");
        assert!(files.loader.len() > 4096);
        assert!(files.kernel.len() > 1_000_000);
    }
}
