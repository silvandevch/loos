//! X.509 (DER) parsing, RSA/SHA-256 signature verification, chain building and the root store.
//!
//! Supported: RSA PKCS#1 v1.5 with SHA-256 signatures, basicConstraints, SAN dNSName.
//! ECDSA-signed certificates parse but fail verification with [`Error::Unsupported`].

use crate::bignum::Uint;
use crate::hash::{ct_eq, sha256};
use crate::{Error, Result};
use alloc::string::String;
use alloc::vec::Vec;

/// One DER TLV: (tag, content, rest).
pub(crate) fn tlv(b: &[u8]) -> Result<(u8, &[u8], &[u8])> {
    let tag = *b.first().ok_or(Error::Parse)?;
    let l0 = *b.get(1).ok_or(Error::Parse)? as usize;
    let (len, hdr) = if l0 < 0x80 {
        (l0, 2)
    } else {
        let n = l0 & 0x7f;
        if n == 0 || n > 4 || b.len() < 2 + n {
            return Err(Error::Parse);
        }
        (b[2..2 + n].iter().fold(0usize, |a, &x| a << 8 | x as usize), 2 + n)
    };
    let end = hdr.checked_add(len).ok_or(Error::Parse)?;
    if b.len() < end {
        return Err(Error::Parse);
    }
    Ok((tag, &b[hdr..end], &b[end..]))
}

/// Like [`tlv`] but also returns the full encoding (header included).
fn tlv_raw(b: &[u8]) -> Result<(u8, &[u8], &[u8], &[u8])> {
    let (t, c, rest) = tlv(b)?;
    Ok((t, c, &b[..b.len() - rest.len()], rest))
}

fn expect(b: &[u8], tag: u8) -> Result<(&[u8], &[u8])> {
    let (t, c, r) = tlv(b)?;
    if t != tag {
        return Err(Error::Parse);
    }
    Ok((c, r))
}

const OID_SHA256_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b];
const OID_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
const OID_CN: &[u8] = &[0x55, 0x04, 0x03];
const OID_BC: &[u8] = &[0x55, 0x1d, 0x13];
const OID_SAN: &[u8] = &[0x55, 0x1d, 0x11];
const DIGEST_PREFIX: [u8; 19] = [0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00, 0x04, 0x20];

/// A parsed certificate.
#[derive(Clone, Debug)]
pub struct Cert {
    /// Full DER.
    pub der: Vec<u8>,
    tbs: Vec<u8>,
    /// Raw DER of the issuer Name.
    pub issuer: Vec<u8>,
    /// Raw DER of the subject Name.
    pub subject: Vec<u8>,
    /// Validity start, Unix seconds.
    pub not_before: i64,
    /// Validity end, Unix seconds.
    pub not_after: i64,
    n: Vec<u8>,
    e: Vec<u8>,
    sig_is_rsa_sha256: bool,
    sig: Vec<u8>,
    /// basicConstraints cA.
    pub is_ca: bool,
    /// basicConstraints pathLenConstraint.
    pub path_len: Option<u32>,
    /// SAN dNSName entries.
    pub dns_names: Vec<String>,
    /// Subject common name.
    pub cn: Option<String>,
}

fn parse_time(tag: u8, c: &[u8]) -> Result<i64> {
    let s = core::str::from_utf8(c).map_err(|_| Error::Parse)?;
    let (yl, rest) = match tag {
        0x17 => (2, s),
        0x18 => (4, s),
        _ => return Err(Error::Parse),
    };
    if rest.len() < yl + 10 || !rest.ends_with('Z') {
        return Err(Error::Parse);
    }
    let num = |a: usize, l: usize| rest[a..a + l].parse::<i64>().map_err(|_| Error::Parse);
    let mut y = num(0, yl)?;
    if yl == 2 {
        y += if y >= 50 { 1900 } else { 2000 };
    }
    let o = yl;
    let (mo, d, h, mi, se) = (num(o, 2)?, num(o + 2, 2)?, num(o + 4, 2)?, num(o + 6, 2)?, num(o + 8, 2)?);
    // days from civil (Howard Hinnant)
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Ok(days * 86400 + h * 3600 + mi * 60 + se)
}

impl Cert {
    /// Parse a DER certificate.
    pub fn parse(der: &[u8]) -> Result<Cert> {
        let (body, rest) = expect(der, 0x30)?;
        if !rest.is_empty() {
            return Err(Error::Parse);
        }
        let (_, tbs_c, tbs_raw, r) = tlv_raw(body)?;
        let (alg, r) = expect(r, 0x30)?;
        let (sigbits, _) = expect(r, 0x03)?;
        let sig = sigbits.get(1..).ok_or(Error::Parse)?.to_vec();
        let (aoid, _) = expect(alg, 0x06)?;

        let mut t = tbs_c;
        if t.first() == Some(&0xa0) {
            t = tlv(t)?.2;
        }
        t = expect(t, 0x02)?.1; // serial
        t = expect(t, 0x30)?.1; // inner sigAlg
        let (_, _, issuer, t) = tlv_raw(t)?;
        let (val, t) = expect(t, 0x30)?;
        let (nbt, nbc, v2) = tlv(val)?;
        let (nat, nac, _) = tlv(v2)?;
        let (_, _, subject, t) = tlv_raw(t)?;
        let (spki, mut t) = expect(t, 0x30)?;
        let (ka, kr) = expect(spki, 0x30)?;
        if expect(ka, 0x06)?.0 != OID_RSA {
            return Err(Error::Unsupported);
        }
        let (kb, _) = expect(kr, 0x03)?;
        let (keyseq, _) = expect(kb.get(1..).ok_or(Error::Parse)?, 0x30)?;
        let (n, ke) = expect(keyseq, 0x02)?;
        let (e, _) = expect(ke, 0x02)?;
        let strip = |v: &[u8]| v.iter().skip_while(|&&b| b == 0).copied().collect::<Vec<u8>>();

        let (mut is_ca, mut path_len, mut dns_names) = (false, None, Vec::new());
        while !t.is_empty() {
            let (tag, c, r) = tlv(t)?;
            t = r;
            if tag != 0xa3 {
                continue;
            }
            let (mut exts, _) = expect(c, 0x30)?;
            while !exts.is_empty() {
                let (ext, r) = expect(exts, 0x30)?;
                exts = r;
                let (oid, mut e) = expect(ext, 0x06)?;
                if e.first() == Some(&0x01) {
                    e = tlv(e)?.2; // critical
                }
                let (val, _) = expect(e, 0x04)?;
                if oid == OID_BC {
                    let (mut s, _) = expect(val, 0x30)?;
                    if s.first() == Some(&0x01) {
                        let (b, r) = expect(s, 0x01)?;
                        is_ca = b.first().is_some_and(|&x| x != 0);
                        s = r;
                    }
                    if s.first() == Some(&0x02) {
                        let (p, _) = expect(s, 0x02)?;
                        path_len = Some(p.iter().fold(0u32, |a, &x| a << 8 | x as u32));
                    }
                } else if oid == OID_SAN {
                    let (mut s, _) = expect(val, 0x30)?;
                    while !s.is_empty() {
                        let (gt, gc, r) = tlv(s)?;
                        s = r;
                        if gt == 0x82 {
                            dns_names.push(String::from_utf8(gc.to_vec()).map_err(|_| Error::Parse)?);
                        }
                    }
                }
            }
        }

        // Subject CN
        let mut cn = None;
        let (mut rdns, _) = expect(&subject, 0x30)?;
        while !rdns.is_empty() {
            let (set, r) = expect(rdns, 0x31)?;
            rdns = r;
            let (atv, _) = expect(set, 0x30)?;
            let (oid, v) = expect(atv, 0x06)?;
            if oid == OID_CN {
                let (_, s, _) = tlv(v)?;
                cn = core::str::from_utf8(s).ok().map(String::from);
            }
        }

        Ok(Cert {
            der: der.to_vec(),
            tbs: tbs_raw.to_vec(),
            issuer: issuer.to_vec(),
            subject: subject.to_vec(),
            not_before: parse_time(nbt, nbc)?,
            not_after: parse_time(nat, nac)?,
            n: strip(n),
            e: strip(e),
            sig_is_rsa_sha256: aoid == OID_SHA256_RSA,
            sig,
            is_ca,
            path_len,
            dns_names,
            cn,
        })
    }

    /// SHA-256 fingerprint of the DER.
    pub fn fingerprint(&self) -> [u8; 32] {
        sha256(&self.der)
    }

    /// Verify that `issuer_cert`'s key signed this certificate.
    pub fn verify_signed_by(&self, issuer_cert: &Cert) -> Result<()> {
        if !self.sig_is_rsa_sha256 {
            return Err(Error::Unsupported);
        }
        rsa_pkcs1_sha256_verify(&issuer_cert.n, &issuer_cert.e, &self.tbs, &self.sig)
    }

    /// Does this certificate cover `host` (SAN dNSName with `*.` wildcard, else CN)?
    pub fn matches_host(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        let m = |pat: &str| {
            let pat = pat.to_ascii_lowercase();
            match pat.strip_prefix("*.") {
                Some(suffix) => host.split_once('.').is_some_and(|(label, rest)| !label.is_empty() && rest == suffix),
                None => pat == host,
            }
        };
        if self.dns_names.is_empty() {
            self.cn.as_deref().is_some_and(m)
        } else {
            self.dns_names.iter().any(|n| m(n))
        }
    }
}

/// RSASSA-PKCS1-v1_5 verify with SHA-256 over `msg`. `n`/`e` big-endian without leading zeros.
pub fn rsa_pkcs1_sha256_verify(n: &[u8], e: &[u8], msg: &[u8], sig: &[u8]) -> Result<()> {
    let k = n.len();
    if k < 64 || sig.len() != k {
        return Err(Error::BadSignature);
    }
    let nn = Uint::from_be(n);
    let s = Uint::from_be(sig);
    let m = s.modpow(&Uint::from_be(e), &nn).to_be(k);
    let mut expected = alloc::vec![0xffu8; k];
    expected[0] = 0;
    expected[1] = 1;
    let t = k - 32 - DIGEST_PREFIX.len();
    expected[t - 1] = 0;
    expected[t..t + DIGEST_PREFIX.len()].copy_from_slice(&DIGEST_PREFIX);
    expected[k - 32..].copy_from_slice(&sha256(msg));
    if ct_eq(&m, &expected) {
        Ok(())
    } else {
        Err(Error::BadSignature)
    }
}


fn mgf1(seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 32);
    let mut c = 0u32;
    while out.len() < len {
        let mut h = crate::hash::Sha256::new();
        h.update(seed);
        h.update(&c.to_be_bytes());
        out.extend_from_slice(&h.finish());
        c += 1;
    }
    out.truncate(len);
    out
}

/// RSASSA-PSS (SHA-256, MGF1-SHA-256, 32-byte salt) verify — `rsa_pss_rsae_sha256` in TLS 1.3.
pub fn rsa_pss_sha256_verify(n: &[u8], e: &[u8], msg: &[u8], sig: &[u8]) -> Result<()> {
    let k = n.len();
    if k < 64 || sig.len() != k {
        return Err(Error::BadSignature);
    }
    let mod_bits = k * 8 - n[0].leading_zeros() as usize;
    let em_bits = mod_bits - 1;
    let em_len = (em_bits + 7) / 8;
    let m = Uint::from_be(sig).modpow(&Uint::from_be(e), &Uint::from_be(n)).to_be(k);
    let em = &m[k - em_len..];
    if m[..k - em_len].iter().any(|&b| b != 0) || em[em_len - 1] != 0xbc || em_len < 32 + 32 + 2 {
        return Err(Error::BadSignature);
    }
    let (masked, rest) = em.split_at(em_len - 33);
    let h = &rest[..32];
    let top_mask = 0xffu8 >> (8 * em_len - em_bits);
    if masked[0] & !top_mask != 0 {
        return Err(Error::BadSignature);
    }
    let mut db: Vec<u8> = masked.iter().zip(mgf1(h, masked.len())).map(|(a, b)| a ^ b).collect();
    db[0] &= top_mask;
    let ps = db.len() - 33;
    if db[..ps].iter().any(|&b| b != 0) || db[ps] != 1 {
        return Err(Error::BadSignature);
    }
    let mut mp = alloc::vec![0u8; 8];
    mp.extend_from_slice(&sha256(msg));
    mp.extend_from_slice(&db[ps + 1..]);
    if ct_eq(&sha256(&mp), h) {
        Ok(())
    } else {
        Err(Error::BadSignature)
    }
}

impl Cert {
    /// Verify an RSASSA-PKCS1-v1_5/SHA-256 signature made by this certificate's key.
    pub fn verify_pkcs1(&self, msg: &[u8], sig: &[u8]) -> Result<()> {
        rsa_pkcs1_sha256_verify(&self.n, &self.e, msg, sig)
    }

    /// RSA public key `(n, e)`, big-endian.
    pub fn rsa_key(&self) -> (&[u8], &[u8]) {
        (&self.n, &self.e)
    }

    /// Verify an RSA-PSS/SHA-256 signature made by this certificate's key (TLS CertificateVerify).
    pub fn verify_pss(&self, msg: &[u8], sig: &[u8]) -> Result<()> {
        rsa_pss_sha256_verify(&self.n, &self.e, msg, sig)
    }
}

/// Trusted roots plus a local distrust list (by fingerprint). Nothing is fetched from the network.
#[derive(Default)]
pub struct RootStore {
    roots: Vec<Cert>,
    distrusted: Vec<[u8; 32]>,
}

impl RootStore {
    /// Empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a trusted root (self-issued CA certificate).
    pub fn add_root(&mut self, der: &[u8]) -> Result<()> {
        let c = Cert::parse(der)?;
        if !c.is_ca || c.issuer != c.subject {
            return Err(Error::Invalid);
        }
        c.verify_signed_by(&c)?; // roots must be validly self-signed
        if !self.roots.iter().any(|r| r.der == c.der) {
            self.roots.push(c);
        }
        Ok(())
    }

    /// Permanently distrust a certificate by fingerprint (also removes a matching root).
    pub fn distrust(&mut self, fingerprint: [u8; 32]) {
        self.roots.retain(|r| r.fingerprint() != fingerprint);
        self.distrusted.push(fingerprint);
    }

    /// Number of trusted roots.
    pub fn len(&self) -> usize {
        self.roots.len()
    }

    /// True when no roots are installed.
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// Validate `chain` (leaf first) at Unix time `now`, optionally for `host`.
    pub fn verify_chain(&self, chain: &[&[u8]], host: Option<&str>, now: i64) -> Result<()> {
        if chain.is_empty() || chain.len() > 8 {
            return Err(Error::Untrusted);
        }
        let certs: Vec<Cert> = chain.iter().map(|d| Cert::parse(d)).collect::<Result<_>>()?;
        if let Some(h) = host {
            if !certs[0].matches_host(h) {
                return Err(Error::Untrusted);
            }
        }
        for (i, c) in certs.iter().enumerate() {
            if self.distrusted.contains(&c.fingerprint()) || now < c.not_before || now > c.not_after {
                return Err(Error::Untrusted);
            }
            if i > 0 {
                // intermediates must be CAs and respect pathLen (number of CAs below them)
                if !c.is_ca || c.path_len.is_some_and(|p| (i as u32 - 1) > p) {
                    return Err(Error::Untrusted);
                }
            }
        }
        for w in certs.windows(2) {
            if w[0].issuer != w[1].subject {
                return Err(Error::Untrusted);
            }
            w[0].verify_signed_by(&w[1])?;
        }
        let top = certs.last().unwrap();
        // Anchor: the top cert is a root itself, or is issued by one.
        for r in &self.roots {
            if r.der == top.der {
                return Ok(());
            }
            if r.subject == top.issuer && now >= r.not_before && now <= r.not_after && top.verify_signed_by(r).is_ok() {
                if r.path_len.is_some_and(|p| (certs.len() as u32 - 1) > p + 1) {
                    continue;
                }
                return Ok(());
            }
        }
        Err(Error::Untrusted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hex;
    use crate::testcerts::*;

    const NOW: i64 = 1_800_000_000; // 2027-01

    fn store() -> RootStore {
        let mut s = RootStore::new();
        s.add_root(&hex(ROOT)).unwrap();
        s
    }

    #[test]
    fn parses_fields() {
        let l = Cert::parse(&hex(LEAF)).unwrap();
        assert_eq!(l.cn.as_deref(), Some("fallback.local"));
        assert_eq!(l.dns_names, ["example.test", "*.wild.test"]);
        assert!(!l.is_ca);
        let i = Cert::parse(&hex(INT)).unwrap();
        assert!(i.is_ca);
        assert_eq!(i.path_len, Some(0));
        assert!(l.not_before < NOW && NOW < l.not_after);
        assert!(Cert::parse(&[1, 2, 3]).is_err());
    }

    #[test]
    fn host_matching() {
        let l = Cert::parse(&hex(LEAF)).unwrap();
        assert!(l.matches_host("EXAMPLE.test") && l.matches_host("a.wild.test"));
        assert!(!l.matches_host("a.b.wild.test") && !l.matches_host("wild.test") && !l.matches_host("fallback.local"));
    }

    #[test]
    fn chain_validates() {
        let (l, i, r) = (hex(LEAF), hex(INT), hex(ROOT));
        let s = store();
        s.verify_chain(&[&l, &i], Some("example.test"), NOW).unwrap();
        s.verify_chain(&[&l, &i, &r], None, NOW).unwrap();
        assert_eq!(s.verify_chain(&[&l, &i], Some("evil.test"), NOW), Err(Error::Untrusted));
        assert_eq!(s.verify_chain(&[&l], None, NOW), Err(Error::Untrusted)); // missing intermediate
        assert_eq!(s.verify_chain(&[&i, &l], None, NOW), Err(Error::Untrusted));
        assert_eq!(s.verify_chain(&[&l, &i], None, 1_000_000_000), Err(Error::Untrusted)); // not yet valid
        assert_eq!(RootStore::new().verify_chain(&[&l, &i], None, NOW), Err(Error::Untrusted));
    }

    #[test]
    fn tampered_signature_and_distrust() {
        let (mut l, i) = (hex(LEAF), hex(INT));
        let n = l.len();
        l[n - 5] ^= 0x40;
        assert_eq!(store().verify_chain(&[&l, &i], None, NOW), Err(Error::BadSignature));
        let mut s = store();
        let fp = Cert::parse(&i).unwrap().fingerprint();
        s.distrust(fp);
        assert_eq!(s.verify_chain(&[&hex(LEAF), &i], None, NOW), Err(Error::Untrusted));
        let rfp = Cert::parse(&hex(ROOT)).unwrap().fingerprint();
        s.distrust(rfp);
        assert!(s.is_empty());
    }

    #[test]
    fn add_root_rejects_non_roots() {
        let mut s = RootStore::new();
        assert!(s.add_root(&hex(LEAF)).is_err());
        assert!(s.add_root(&hex(INT)).is_err());
    }

    #[test]
    fn time_parse() {
        assert_eq!(parse_time(0x17, b"700101000000Z").unwrap(), 0);
        assert_eq!(parse_time(0x18, b"20000301000000Z").unwrap(), 951_868_800);
    }
}
