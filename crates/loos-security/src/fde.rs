//! Full-disk encryption: AES-256-XTS data, a random volume master key (VMK) wrapped by up to
//! eight independent key slots (passphrase, recovery key, boot measurement), integrity-checked header.

use crate::aes::{key_unwrap, key_wrap, Xts};
use crate::hash::{ct_eq, hkdf_extract, hmac, pbkdf2};
use crate::{Error, Result};
use alloc::string::String;
use alloc::vec::Vec;

/// Bytes the header occupies (padded).
pub const HEADER_BYTES: usize = 1024;
const MAGIC: &[u8; 8] = b"LOOSFDE1";
const SLOTS: usize = 8;
const WRAPPED: usize = 72;
const SLOT_BYTES: usize = 1 + 16 + WRAPPED;
const BODY: usize = 8 + 4 + 4 + 8 + 4 + SLOTS * SLOT_BYTES;

/// Slot kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// Unused.
    Empty = 0,
    /// User passphrase (PBKDF2).
    Passphrase = 1,
    /// 48-digit recovery key.
    Recovery = 2,
    /// Sealed to the secure-boot measurement (see `codesign::BootChain::composite`).
    Measurement = 3,
}

/// A secret presented to unlock.
pub enum Secret<'a> {
    /// Passphrase bytes.
    Passphrase(&'a [u8]),
    /// Recovery key string as shown to the user.
    Recovery(&'a str),
    /// Boot measurement composite.
    Measurement(&'a [u8; 32]),
}

#[derive(Clone, Copy)]
struct Slot {
    kind: u8,
    salt: [u8; 16],
    wrapped: [u8; WRAPPED],
}

const EMPTY: Slot = Slot { kind: 0, salt: [0; 16], wrapped: [0; WRAPPED] };

/// Random source: fills the buffer with CSPRNG bytes.
pub type Rng<'a> = &'a mut dyn FnMut(&mut [u8]);

/// An unlocked volume (holds the VMK).
pub struct Volume {
    sector_size: u32,
    sectors: u64,
    iterations: u32,
    slots: [Slot; SLOTS],
    vmk: [u8; 64],
}

impl Drop for Volume {
    fn drop(&mut self) {
        self.vmk = [0; 64]; // best effort; Rust gives no volatile guarantee without unsafe
    }
}

fn kek(kind: u8, secret: &[u8], salt: &[u8; 16], iterations: u32) -> [u8; 32] {
    match kind {
        1 => {
            let mut k = [0u8; 32];
            pbkdf2(secret, salt, iterations, &mut k);
            k
        }
        _ => hkdf_extract(salt, secret), // high-entropy secrets need no stretching
    }
}

/// Encode a 128-bit recovery key as 8 groups of 6 digits, each a multiple of 11 (typo check).
pub fn recovery_string(key: &[u8; 16]) -> String {
    let mut s = String::new();
    for (i, c) in key.chunks(2).enumerate() {
        if i > 0 {
            s.push('-');
        }
        let v = u16::from_le_bytes([c[0], c[1]]) as u32 * 11;
        s.push_str(&alloc::format!("{:06}", v));
    }
    s
}

/// Decode and validate a recovery key string.
pub fn parse_recovery(s: &str) -> Result<[u8; 16]> {
    let mut out = [0u8; 16];
    let mut n = 0;
    for g in s.split('-') {
        let v: u32 = g.parse().map_err(|_| Error::Invalid)?;
        if g.len() != 6 || v % 11 != 0 || v / 11 > 0xffff || n >= 8 {
            return Err(Error::Invalid);
        }
        out[n * 2..n * 2 + 2].copy_from_slice(&((v / 11) as u16).to_le_bytes());
        n += 1;
    }
    if n != 8 {
        return Err(Error::Invalid);
    }
    Ok(out)
}

impl Volume {
    /// Create a new volume: random VMK, one passphrase slot, one recovery slot.
    /// Returns the volume and the recovery key string to show the user once.
    pub fn format(sector_size: u32, sectors: u64, passphrase: &[u8], iterations: u32, rng: Rng) -> Result<(Volume, String)> {
        if sector_size < 512 || !sector_size.is_power_of_two() || sectors == 0 || passphrase.is_empty() || iterations == 0 {
            return Err(Error::Invalid);
        }
        let mut vmk = [0u8; 64];
        rng(&mut vmk);
        if vmk[..32] == vmk[32..] {
            return Err(Error::Invalid); // XTS requires distinct keys
        }
        let mut v = Volume { sector_size, sectors, iterations, slots: [EMPTY; SLOTS], vmk };
        v.add_slot(SlotKind::Passphrase, passphrase, rng)?;
        let mut rk = [0u8; 16];
        rng(&mut rk);
        v.add_slot(SlotKind::Recovery, &rk, rng)?;
        Ok((v, recovery_string(&rk)))
    }

    /// Add a key slot protecting the VMK with `secret`.
    pub fn add_slot(&mut self, kind: SlotKind, secret: &[u8], rng: Rng) -> Result<usize> {
        if kind == SlotKind::Empty {
            return Err(Error::Invalid);
        }
        let i = self.slots.iter().position(|s| s.kind == 0).ok_or(Error::Full)?;
        let mut salt = [0u8; 16];
        rng(&mut salt);
        let k = kek(kind as u8, secret, &salt, self.iterations);
        let w = key_wrap(&k, &self.vmk)?;
        self.slots[i] = Slot { kind: kind as u8, salt, wrapped: w.try_into().map_err(|_| Error::Invalid)? };
        Ok(i)
    }

    /// Remove a slot; the last remaining slot cannot be removed.
    pub fn remove_slot(&mut self, i: usize) -> Result<()> {
        if i >= SLOTS || self.slots[i].kind == 0 || self.slots.iter().filter(|s| s.kind != 0).count() == 1 {
            return Err(Error::Invalid);
        }
        self.slots[i] = EMPTY;
        Ok(())
    }

    /// Replace the passphrase slot(s) with a new passphrase.
    pub fn change_passphrase(&mut self, new: &[u8], rng: Rng) -> Result<()> {
        if new.is_empty() {
            return Err(Error::Invalid);
        }
        let old: Vec<usize> = (0..SLOTS).filter(|&i| self.slots[i].kind == 1).collect();
        self.add_slot(SlotKind::Passphrase, new, rng)?;
        for i in old {
            self.slots[i] = EMPTY;
        }
        Ok(())
    }

    /// Serialize the header (1024 bytes, MAC'd under a key derived from the VMK).
    pub fn header(&self) -> Vec<u8> {
        let mut h = Vec::with_capacity(HEADER_BYTES);
        h.extend_from_slice(MAGIC);
        h.extend_from_slice(&1u32.to_le_bytes());
        h.extend_from_slice(&self.sector_size.to_le_bytes());
        h.extend_from_slice(&self.sectors.to_le_bytes());
        h.extend_from_slice(&self.iterations.to_le_bytes());
        for s in &self.slots {
            h.push(s.kind);
            h.extend_from_slice(&s.salt);
            h.extend_from_slice(&s.wrapped);
        }
        let mac = hmac(&mac_key(&self.vmk), &h);
        h.extend_from_slice(&mac);
        h.resize(HEADER_BYTES, 0);
        h
    }

    /// Unlock with any slot's secret.
    pub fn unlock(header: &[u8], secret: Secret) -> Result<Volume> {
        if header.len() < BODY + 32 || &header[..8] != MAGIC || u32::from_le_bytes(header[8..12].try_into().unwrap()) != 1 {
            return Err(Error::Parse);
        }
        let sector_size = u32::from_le_bytes(header[12..16].try_into().unwrap());
        let sectors = u64::from_le_bytes(header[16..24].try_into().unwrap());
        let iterations = u32::from_le_bytes(header[24..28].try_into().unwrap());
        if sector_size < 512 || !sector_size.is_power_of_two() || iterations == 0 {
            return Err(Error::Parse);
        }
        let mut slots = [EMPTY; SLOTS];
        for (i, s) in slots.iter_mut().enumerate() {
            let o = 28 + i * SLOT_BYTES;
            s.kind = header[o];
            s.salt.copy_from_slice(&header[o + 1..o + 17]);
            s.wrapped.copy_from_slice(&header[o + 17..o + SLOT_BYTES]);
        }
        let (want, bytes): (u8, Vec<u8>) = match secret {
            Secret::Passphrase(p) => (1, p.to_vec()),
            Secret::Recovery(r) => (2, parse_recovery(r)?.to_vec()),
            Secret::Measurement(m) => (3, m.to_vec()),
        };
        let mut vmk = None;
        for s in slots.iter().filter(|s| s.kind == want) {
            if let Ok(k) = key_unwrap(&kek(want, &bytes, &s.salt, iterations), &s.wrapped) {
                vmk = k.try_into().ok();
                break;
            }
        }
        let vmk: [u8; 64] = vmk.ok_or(Error::BadCredential)?;
        if !ct_eq(&hmac(&mac_key(&vmk), &header[..BODY]), &header[BODY..BODY + 32]) {
            return Err(Error::BadSignature); // header modified (e.g. slot swap / downgrade)
        }
        Ok(Volume { sector_size, sectors, iterations, slots, vmk })
    }

    /// Data-sector size.
    pub fn sector_size(&self) -> u32 {
        self.sector_size
    }

    /// Number of data sectors.
    pub fn sectors(&self) -> u64 {
        self.sectors
    }

    /// Encrypt one logical sector in place.
    pub fn encrypt_sector(&self, lba: u64, buf: &mut [u8]) -> Result<()> {
        self.check(lba, buf)?;
        Xts::new(&self.vmk)?.encrypt_sector(lba, buf)
    }

    /// Decrypt one logical sector in place.
    pub fn decrypt_sector(&self, lba: u64, buf: &mut [u8]) -> Result<()> {
        self.check(lba, buf)?;
        Xts::new(&self.vmk)?.decrypt_sector(lba, buf)
    }

    fn check(&self, lba: u64, buf: &[u8]) -> Result<()> {
        if lba >= self.sectors || buf.len() != self.sector_size as usize {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

fn mac_key(vmk: &[u8; 64]) -> [u8; 32] {
    hkdf_extract(b"loos-fde-header", vmk)
}

/// Raw sector device.
pub trait BlockDevice {
    /// Read physical sector `lba`.
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<()>;
    /// Write physical sector `lba`.
    fn write(&mut self, lba: u64, buf: &[u8]) -> Result<()>;
}

/// An unlocked encrypted disk: the header lives at the start, data sectors follow.
pub struct EncryptedDisk<D: BlockDevice> {
    dev: D,
    vol: Volume,
    data_start: u64,
}

impl<D: BlockDevice> EncryptedDisk<D> {
    /// Physical sectors the header takes for a given sector size.
    pub fn header_sectors(sector_size: u32) -> u64 {
        (HEADER_BYTES as u64).div_ceil(sector_size as u64)
    }

    /// Write `vol`'s header to a fresh device and open it.
    pub fn create(mut dev: D, vol: Volume) -> Result<Self> {
        let hs = Self::header_sectors(vol.sector_size);
        let mut h = vol.header();
        h.resize((hs * vol.sector_size as u64) as usize, 0);
        for (i, c) in h.chunks(vol.sector_size as usize).enumerate() {
            dev.write(i as u64, c)?;
        }
        Ok(EncryptedDisk { dev, vol, data_start: hs })
    }

    /// Read the header from `dev` and unlock.
    pub fn open(mut dev: D, sector_size: u32, secret: Secret) -> Result<Self> {
        let hs = Self::header_sectors(sector_size);
        let mut h = alloc::vec![0u8; (hs * sector_size as u64) as usize];
        for (i, c) in h.chunks_mut(sector_size as usize).enumerate() {
            dev.read(i as u64, c)?;
        }
        let vol = Volume::unlock(&h, secret)?;
        if vol.sector_size != sector_size {
            return Err(Error::Parse);
        }
        Ok(EncryptedDisk { dev, vol, data_start: hs })
    }

    /// Decrypting read of logical sector `lba`.
    pub fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<()> {
        self.vol.check(lba, buf)?;
        self.dev.read(self.data_start + lba, buf)?;
        self.vol.decrypt_sector(lba, buf)
    }

    /// Encrypting write of logical sector `lba`.
    pub fn write(&mut self, lba: u64, data: &[u8]) -> Result<()> {
        let mut b = data.to_vec();
        self.vol.encrypt_sector(lba, &mut b)?;
        self.dev.write(self.data_start + lba, &b)
    }

    /// The volume (to manage key slots).
    pub fn volume(&mut self) -> &mut Volume {
        &mut self.vol
    }

    /// Persist the current header (after slot changes).
    pub fn flush_header(&mut self) -> Result<()> {
        let ss = self.vol.sector_size as usize;
        let mut h = self.vol.header();
        h.resize(self.data_start as usize * ss, 0);
        for (i, c) in h.chunks(ss).enumerate() {
            self.dev.write(i as u64, c)?;
        }
        Ok(())
    }

    /// Give back the device.
    pub fn into_inner(self) -> D {
        self.dev
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn rng(seed: u8) -> impl FnMut(&mut [u8]) {
        let mut c = seed as u32;
        move |b: &mut [u8]| {
            for x in b.iter_mut() {
                c = c.wrapping_mul(1664525).wrapping_add(1013904223);
                *x = (c >> 24) as u8;
            }
        }
    }

    struct Ram(Vec<u8>);
    impl BlockDevice for Ram {
        fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<()> {
            let o = lba as usize * 512;
            buf.copy_from_slice(self.0.get(o..o + 512).ok_or(Error::Invalid)?);
            Ok(())
        }
        fn write(&mut self, lba: u64, buf: &[u8]) -> Result<()> {
            let o = lba as usize * 512;
            self.0.get_mut(o..o + 512).ok_or(Error::Invalid)?.copy_from_slice(buf);
            Ok(())
        }
    }

    #[test]
    fn recovery_string_roundtrip() {
        let k = [7u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 255];
        let s = recovery_string(&k);
        assert_eq!(s.len(), 8 * 6 + 7);
        assert_eq!(parse_recovery(&s).unwrap(), k);
        let mut bad = s.clone().into_bytes();
        bad[5] = if bad[5] == b'9' { b'8' } else { bad[5] + 1 };
        assert!(parse_recovery(core::str::from_utf8(&bad).unwrap()).is_err());
        assert!(parse_recovery("000000-000000").is_err());
    }

    #[test]
    fn disk_roundtrip_unlock_variants() {
        let mut r = rng(1);
        let (vol, recovery) = Volume::format(512, 16, b"pass phrase", 10, &mut r).unwrap();
        let mut d = EncryptedDisk::create(Ram(vec![0; 512 * 18]), vol).unwrap();
        let secret_data: Vec<u8> = (0..512).map(|i| (i % 251) as u8).collect();
        d.write(3, &secret_data).unwrap();
        let mut raw = [0u8; 512];
        d.dev.read(d.data_start + 3, &mut raw).unwrap();
        assert_ne!(raw.to_vec(), secret_data); // ciphertext on the medium
        let mut buf = [0u8; 512];
        d.read(3, &mut buf).unwrap();
        assert_eq!(buf.to_vec(), secret_data);
        assert_eq!(d.read(16, &mut buf), Err(Error::Invalid));
        let ram = d.into_inner();

        let mut d = EncryptedDisk::open(Ram(ram.0.clone()), 512, Secret::Passphrase(b"pass phrase")).unwrap();
        d.read(3, &mut buf).unwrap();
        assert_eq!(buf.to_vec(), secret_data);
        assert_eq!(EncryptedDisk::open(Ram(ram.0.clone()), 512, Secret::Passphrase(b"nope")).err(), Some(Error::BadCredential));
        let mut d = EncryptedDisk::open(Ram(ram.0.clone()), 512, Secret::Recovery(&recovery)).unwrap();
        d.read(3, &mut buf).unwrap();
        assert_eq!(buf.to_vec(), secret_data);
        assert!(EncryptedDisk::open(Ram(ram.0), 512, Secret::Measurement(&[0; 32])).is_err());
    }

    #[test]
    fn slot_management_and_measurement_binding() {
        let mut r = rng(9);
        let (mut v, _) = Volume::format(512, 4, b"old", 10, &mut r).unwrap();
        let pcr = [0xabu8; 32];
        v.add_slot(SlotKind::Measurement, &pcr, &mut r).unwrap();
        v.change_passphrase(b"new", &mut r).unwrap();
        let h = v.header();
        assert!(Volume::unlock(&h, Secret::Passphrase(b"old")).is_err());
        Volume::unlock(&h, Secret::Passphrase(b"new")).unwrap();
        Volume::unlock(&h, Secret::Measurement(&pcr)).unwrap();
        assert!(Volume::unlock(&h, Secret::Measurement(&[0xac; 32])).is_err());
        // can't remove the last slot
        for i in 0..SLOTS {
            if v.slots.iter().filter(|s| s.kind != 0).count() > 1 && v.slots[i].kind != 0 {
                v.remove_slot(i).unwrap();
            }
        }
        assert_eq!(v.slots.iter().filter(|s| s.kind != 0).count(), 1);
        let last = v.slots.iter().position(|s| s.kind != 0).unwrap();
        assert_eq!(v.remove_slot(last), Err(Error::Invalid));
        // slots fill up
        let mut r = rng(3);
        let (mut v, _) = Volume::format(512, 4, b"x", 1, &mut r).unwrap();
        for _ in 0..6 {
            v.add_slot(SlotKind::Passphrase, b"y", &mut r).unwrap();
        }
        assert_eq!(v.add_slot(SlotKind::Passphrase, b"y", &mut r), Err(Error::Full));
    }

    #[test]
    fn header_tampering_detected() {
        let mut r = rng(5);
        let (v, _) = Volume::format(512, 8, b"pw", 10, &mut r).unwrap();
        let h = v.header();
        let mut t = h.clone();
        t[16] ^= 1; // sector count
        assert_eq!(Volume::unlock(&t, Secret::Passphrase(b"pw")).err(), Some(Error::BadSignature));
        let mut t = h.clone();
        t[0] ^= 1;
        assert_eq!(Volume::unlock(&t, Secret::Passphrase(b"pw")).err(), Some(Error::Parse));
        assert_eq!(Volume::unlock(&h[..100], Secret::Passphrase(b"pw")).err(), Some(Error::Parse));
        assert!(Volume::format(100, 8, b"pw", 10, &mut r).is_err());
        assert!(Volume::format(512, 8, b"", 10, &mut r).is_err());
    }
}
