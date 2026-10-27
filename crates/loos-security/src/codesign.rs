//! Code signing, driver-signature enforcement and the measured/secure-boot chain.
//!
//! Signed image = `image || blob || len:u32 || "LOOSSIG1"`. The blob carries the SHA-256 of the
//! image, a timestamp, the signer's certificate chain and an RSA PKCS#1 v1.5 signature over
//! `digest || timestamp`.

use crate::bignum::Uint;
use crate::hash::{ct_eq, sha256};
use crate::x509::{Cert, RootStore};
use crate::{Error, Result};
use alloc::string::String;
use alloc::vec::Vec;

const MAGIC: &[u8; 8] = b"LOOSSIG1";
const DIGEST_PREFIX: [u8; 19] = [0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00, 0x04, 0x20];

/// RSASSA-PKCS1-v1_5/SHA-256 signing (build-host tool; private exponent `d`).
pub fn rsa_sign(n: &[u8], d: &[u8], msg: &[u8]) -> Vec<u8> {
    let k = n.len();
    let mut em = alloc::vec![0xffu8; k];
    em[0] = 0;
    em[1] = 1;
    let t = k - 32 - DIGEST_PREFIX.len();
    em[t - 1] = 0;
    em[t..t + DIGEST_PREFIX.len()].copy_from_slice(&DIGEST_PREFIX);
    em[k - 32..].copy_from_slice(&sha256(msg));
    Uint::from_be(&em).modpow(&Uint::from_be(d), &Uint::from_be(n)).to_be(k)
}

/// Sign `image` with the key `(n, d)` whose certificate chain (leaf first) is `chain`.
pub fn sign_image(image: &[u8], chain: &[&[u8]], n: &[u8], d: &[u8], time: u64) -> Vec<u8> {
    let digest = sha256(image);
    let mut msg = digest.to_vec();
    msg.extend_from_slice(&time.to_be_bytes());
    let sig = rsa_sign(n, d, &msg);
    let mut blob = digest.to_vec();
    blob.extend_from_slice(&time.to_be_bytes());
    blob.push(chain.len() as u8);
    for c in chain {
        blob.extend_from_slice(&(c.len() as u32).to_be_bytes());
        blob.extend_from_slice(c);
    }
    blob.extend_from_slice(&(sig.len() as u16).to_be_bytes());
    blob.extend_from_slice(&sig);
    let mut out = image.to_vec();
    out.extend_from_slice(&blob);
    out.extend_from_slice(&(blob.len() as u32).to_be_bytes());
    out.extend_from_slice(MAGIC);
    out
}

/// A parsed signature trailer.
pub struct Parsed<'a> {
    /// The unsigned image bytes.
    pub image: &'a [u8],
    digest: [u8; 32],
    time: u64,
    chain: Vec<&'a [u8]>,
    sig: &'a [u8],
}

/// Split a signed image into image and signature.
pub fn parse(signed: &[u8]) -> Result<Parsed<'_>> {
    let n = signed.len();
    if n < 12 || &signed[n - 8..] != MAGIC {
        return Err(Error::Parse);
    }
    let blen = u32::from_be_bytes(signed[n - 12..n - 8].try_into().unwrap()) as usize;
    let bstart = (n - 12).checked_sub(blen).ok_or(Error::Parse)?;
    let (image, blob) = (&signed[..bstart], &signed[bstart..n - 12]);
    if blob.len() < 41 {
        return Err(Error::Parse);
    }
    let digest: [u8; 32] = blob[..32].try_into().unwrap();
    let time = u64::from_be_bytes(blob[32..40].try_into().unwrap());
    let cnt = blob[40] as usize;
    let mut p = 41;
    let mut chain = Vec::new();
    for _ in 0..cnt {
        let l = u32::from_be_bytes(blob.get(p..p + 4).ok_or(Error::Parse)?.try_into().unwrap()) as usize;
        chain.push(blob.get(p + 4..p + 4 + l).ok_or(Error::Parse)?);
        p += 4 + l;
    }
    let sl = u16::from_be_bytes(blob.get(p..p + 2).ok_or(Error::Parse)?.try_into().unwrap()) as usize;
    let sig = blob.get(p + 2..p + 2 + sl).ok_or(Error::Parse)?;
    if p + 2 + sl != blob.len() || chain.is_empty() {
        return Err(Error::Parse);
    }
    Ok(Parsed { image, digest, time, chain, sig })
}

/// Local revocation lists (the `dbx`): image digests and certificate fingerprints. Never fetched remotely.
#[derive(Default, Clone)]
pub struct Revocations {
    /// Revoked image digests.
    pub images: Vec<[u8; 32]>,
    /// Revoked certificate fingerprints.
    pub certs: Vec<[u8; 32]>,
}

/// Successful verification result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// Signer common name.
    pub publisher: String,
    /// Image SHA-256.
    pub digest: [u8; 32],
    /// Signing timestamp.
    pub signed_at: u64,
    /// Verified only against the test-signing roots.
    pub test_signed: bool,
}

/// Driver enforcement mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Only production-trusted signatures load.
    Enforce,
    /// Development: test roots also accepted; the UI must show a watermark.
    TestSigning,
}

/// Signature policy.
pub struct Policy {
    /// Production trust anchors.
    pub trust: RootStore,
    /// Development trust anchors (consulted only in [`Mode::TestSigning`]).
    pub test_trust: RootStore,
    /// Revocations.
    pub revoked: Revocations,
    /// Enforcement mode.
    pub mode: Mode,
}

impl Policy {
    /// Enforcing policy with the given production roots.
    pub fn new(trust: RootStore) -> Self {
        Policy { trust, test_trust: RootStore::new(), revoked: Revocations::default(), mode: Mode::Enforce }
    }

    /// Verify a signed image at Unix time `now`.
    pub fn verify(&self, signed: &[u8], now: i64) -> Result<Verdict> {
        let p = parse(signed)?;
        let digest = sha256(p.image);
        if !ct_eq(&digest, &p.digest) {
            return Err(Error::BadSignature); // image modified after signing
        }
        if self.revoked.images.contains(&digest) {
            return Err(Error::Untrusted);
        }
        let leaf = Cert::parse(p.chain[0])?;
        if leaf.is_ca || p.chain.iter().any(|c| self.revoked.certs.contains(&sha256(c))) {
            return Err(Error::Untrusted);
        }
        let mut msg = digest.to_vec();
        msg.extend_from_slice(&p.time.to_be_bytes());
        leaf.verify_pkcs1(&msg, p.sig)?;
        let test_signed = match self.trust.verify_chain(&p.chain, None, now) {
            Ok(()) => false,
            Err(e) if self.mode == Mode::TestSigning && e == Error::Untrusted => {
                self.test_trust.verify_chain(&p.chain, None, now)?;
                true
            }
            Err(e) => return Err(e),
        };
        Ok(Verdict { publisher: leaf.cn.unwrap_or_default(), digest, signed_at: p.time, test_signed })
    }

    /// Gate for loading a kernel-mode driver.
    pub fn authorize_driver(&self, signed: &[u8], now: i64) -> Result<Verdict> {
        self.verify(signed, now)
    }
}

/// Boot stages measured into PCR-like registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Boot loader.
    Loader = 0,
    /// Kernel image.
    Kernel = 1,
    /// Boot-start drivers.
    Driver = 2,
}

/// Measured secure-boot chain. Every stage must verify before it is extended into its PCR.
pub struct BootChain {
    policy: Policy,
    pcrs: [[u8; 32]; 3],
    /// `(stage, image digest, verified)` log of everything measured.
    pub log: Vec<(Stage, [u8; 32], bool)>,
    failed: bool,
}

impl BootChain {
    /// Start a chain under `policy` (the platform `db`/`dbx`).
    pub fn new(policy: Policy) -> Self {
        BootChain { policy, pcrs: [[0; 32]; 3], log: Vec::new(), failed: false }
    }

    /// Verify and measure a stage. On failure the PCR is poisoned and the chain stays failed.
    pub fn load(&mut self, stage: Stage, signed: &[u8], now: i64) -> Result<Verdict> {
        let digest = parse(signed).map(|p| sha256(p.image)).unwrap_or([0; 32]);
        let r = if self.failed { Err(Error::Untrusted) } else { self.policy.verify(signed, now) };
        let ok = r.is_ok() && !self.failed;
        let m = if ok { digest } else { [0xff; 32] };
        let pcr = &mut self.pcrs[stage as usize];
        let mut buf = pcr.to_vec();
        buf.extend_from_slice(&m);
        *pcr = sha256(&buf);
        self.log.push((stage, digest, ok));
        if !ok {
            self.failed = true;
        }
        r
    }

    /// Whether every stage so far verified.
    pub fn is_trusted(&self) -> bool {
        !self.failed
    }

    /// Composite measurement for sealing disk keys (see `fde::Secret::Measurement`).
    pub fn composite(&self) -> [u8; 32] {
        sha256(&self.pcrs.concat())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hex;
    use crate::testcerts::*;

    const NOW: i64 = 1_800_000_000;

    fn keys() -> (Vec<u8>, Vec<u8>) {
        let leaf = Cert::parse(&hex(LEAF)).unwrap();
        (leaf.rsa_key().0.to_vec(), hex(LEAF_D))
    }

    fn roots() -> RootStore {
        let mut s = RootStore::new();
        s.add_root(&hex(ROOT)).unwrap();
        s
    }

    fn signed(image: &[u8]) -> Vec<u8> {
        let (n, d) = keys();
        sign_image(image, &[&hex(LEAF), &hex(INT)], &n, &d, 1_700_000_000)
    }

    #[test]
    fn sign_and_verify() {
        let p = Policy::new(roots());
        let s = signed(b"MZ-driver-image");
        let v = p.verify(&s, NOW).unwrap();
        assert_eq!(v.publisher, "fallback.local");
        assert_eq!(v.digest, sha256(b"MZ-driver-image"));
        assert!(!v.test_signed);
        p.authorize_driver(&s, NOW).unwrap();
    }

    #[test]
    fn tamper_and_garbage() {
        let p = Policy::new(roots());
        let mut s = signed(b"MZ-driver-image");
        s[3] ^= 1;
        assert_eq!(p.verify(&s, NOW), Err(Error::BadSignature));
        let mut s = signed(b"img");
        let n = s.len();
        s[n - 40] ^= 1; // inside the signature
        assert!(p.verify(&s, NOW).is_err());
        assert_eq!(p.verify(b"not signed at all", NOW), Err(Error::Parse));
        assert_eq!(p.verify(&b"\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0"[..], NOW), Err(Error::Parse));
    }

    #[test]
    fn untrusted_revoked_and_modes() {
        let s = signed(b"drv");
        let p = Policy::new(RootStore::new());
        assert_eq!(p.verify(&s, NOW), Err(Error::Untrusted));
        let mut p = Policy::new(RootStore::new());
        p.mode = Mode::TestSigning;
        assert_eq!(p.verify(&s, NOW), Err(Error::Untrusted)); // no test root either
        p.test_trust = roots();
        assert!(p.verify(&s, NOW).unwrap().test_signed);
        p.mode = Mode::Enforce;
        assert_eq!(p.verify(&s, NOW), Err(Error::Untrusted)); // test roots ignored when enforcing
        let mut p = Policy::new(roots());
        p.revoked.images.push(sha256(b"drv"));
        assert_eq!(p.verify(&s, NOW), Err(Error::Untrusted));
        let mut p = Policy::new(roots());
        p.revoked.certs.push(sha256(&hex(INT)));
        assert_eq!(p.verify(&s, NOW), Err(Error::Untrusted));
    }

    #[test]
    fn boot_chain_measures_and_poisons() {
        let mut bc = BootChain::new(Policy::new(roots()));
        let empty = bc.composite();
        bc.load(Stage::Loader, &signed(b"loader"), NOW).unwrap();
        bc.load(Stage::Kernel, &signed(b"kernel"), NOW).unwrap();
        let good = bc.composite();
        assert_ne!(good, empty);
        assert!(bc.is_trusted() && bc.log.len() == 2);

        // same images -> same measurement (usable for sealing)
        let mut bc2 = BootChain::new(Policy::new(roots()));
        bc2.load(Stage::Loader, &signed(b"loader"), NOW).unwrap();
        bc2.load(Stage::Kernel, &signed(b"kernel"), NOW).unwrap();
        assert_eq!(bc2.composite(), good);

        // a swapped kernel fails and poisons everything after it
        let mut bc3 = BootChain::new(Policy::new(roots()));
        bc3.load(Stage::Loader, &signed(b"loader"), NOW).unwrap();
        assert!(bc3.load(Stage::Kernel, b"evil kernel", NOW).is_err());
        assert!(bc3.load(Stage::Driver, &signed(b"drv"), NOW).is_err());
        assert!(!bc3.is_trusted());
        assert_ne!(bc3.composite(), good);
        assert_eq!(bc3.log.iter().filter(|l| l.2).count(), 1);
    }
}
