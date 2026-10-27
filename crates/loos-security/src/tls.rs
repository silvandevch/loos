//! TLS 1.3 client (sans-IO): `TLS_AES_128_GCM_SHA256`, X25519, `rsa_pss_rsae_sha256`.
//!
//! Server certificates are validated against a [`RootStore`] (chain, validity, hostname). The
//! state machine consumes bytes with [`TlsClient::feed`] and returns bytes to write to the socket;
//! no I/O, no clock and no randomness of its own (the caller supplies them), and nothing is sent
//! anywhere except the peer the caller connected to.

use crate::aes::Gcm;
use crate::hash::{ct_eq, hkdf_expand, hkdf_extract, hmac, sha256};
use crate::x25519::{public_key, x25519};
use crate::x509::{Cert, RootStore};
use crate::{Error, Result};
use alloc::string::String;
use alloc::vec::Vec;

const CT_CCS: u8 = 20;
const CT_ALERT: u8 = 21;
const CT_HANDSHAKE: u8 = 22;
const CT_APP: u8 = 23;
const HS_CLIENT_HELLO: u8 = 1;
const HS_SERVER_HELLO: u8 = 2;
const HS_NEW_SESSION_TICKET: u8 = 4;
const HS_ENCRYPTED_EXTENSIONS: u8 = 8;
const HS_CERTIFICATE: u8 = 11;
const HS_CERT_VERIFY: u8 = 15;
const HS_FINISHED: u8 = 20;
const MAX_RECORD: usize = 16384 + 256;

/// `HKDF-Expand-Label` (RFC 8446 §7.1).
pub fn expand_label(secret: &[u8], label: &str, ctx: &[u8], len: usize) -> Vec<u8> {
    let mut info = Vec::new();
    info.extend_from_slice(&(len as u16).to_be_bytes());
    info.push((6 + label.len()) as u8);
    info.extend_from_slice(b"tls13 ");
    info.extend_from_slice(label.as_bytes());
    info.push(ctx.len() as u8);
    info.extend_from_slice(ctx);
    let mut out = alloc::vec![0u8; len];
    hkdf_expand(secret, &info, &mut out);
    out
}

fn derive_secret(secret: &[u8], label: &str, transcript_hash: &[u8; 32]) -> [u8; 32] {
    expand_label(secret, label, transcript_hash, 32).try_into().unwrap()
}

/// AEAD state for one direction.
struct Dir {
    gcm: Gcm,
    iv: [u8; 12],
    seq: u64,
    secret: [u8; 32],
}

impl Dir {
    fn new(secret: [u8; 32]) -> Self {
        let key = expand_label(&secret, "key", &[], 16);
        let iv: [u8; 12] = expand_label(&secret, "iv", &[], 12).try_into().unwrap();
        Dir { gcm: Gcm::new(&key).unwrap(), iv, seq: 0, secret }
    }

    fn nonce(&self) -> [u8; 12] {
        let mut n = self.iv;
        for (i, b) in self.seq.to_be_bytes().iter().enumerate() {
            n[4 + i] ^= b;
        }
        n
    }

    fn seal(&mut self, inner_type: u8, data: &[u8]) -> Vec<u8> {
        let mut pt = data.to_vec();
        pt.push(inner_type);
        let len = (pt.len() + 16) as u16;
        let hdr = [CT_APP, 3, 3, (len >> 8) as u8, len as u8];
        let tag = self.gcm.seal(&self.nonce(), &hdr, &mut pt);
        self.seq += 1;
        let mut out = hdr.to_vec();
        out.extend_from_slice(&pt);
        out.extend_from_slice(&tag);
        out
    }

    fn open(&mut self, hdr: &[u8; 5], body: &[u8]) -> Result<(u8, Vec<u8>)> {
        if body.len() < 17 {
            return Err(Error::Parse);
        }
        let (ct, tag) = body.split_at(body.len() - 16);
        let mut pt = ct.to_vec();
        self.gcm.open(&self.nonce(), hdr, &mut pt, tag)?;
        self.seq += 1;
        while pt.last() == Some(&0) {
            pt.pop();
        }
        let t = pt.pop().ok_or(Error::Parse)?;
        Ok((t, pt))
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum State {
    WaitServerHello,
    WaitEncryptedExtensions,
    WaitCertificate,
    WaitCertVerify,
    WaitFinished,
    Connected,
    Closed,
}

/// A TLS 1.3 client connection.
pub struct TlsClient<'a> {
    host: String,
    roots: &'a RootStore,
    now: i64,
    eph_priv: [u8; 32],
    random: [u8; 32],
    state: State,
    transcript: Vec<u8>,
    inbuf: Vec<u8>,
    hsbuf: Vec<u8>,
    hs_secret: [u8; 32],
    c_hs: Option<Dir>,
    s_in: Option<Dir>,
    c_out: Option<Dir>,
    leaf: Option<Cert>,
    chain: Vec<Vec<u8>>,
    app_in: Vec<u8>,
}

fn hs_msg(t: u8, body: &[u8]) -> Vec<u8> {
    let mut m = alloc::vec![t, (body.len() >> 16) as u8, (body.len() >> 8) as u8, body.len() as u8];
    m.extend_from_slice(body);
    m
}

fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}

fn ext(out: &mut Vec<u8>, t: u16, body: &[u8]) {
    out.extend_from_slice(&be16(t));
    out.extend_from_slice(&be16(body.len() as u16));
    out.extend_from_slice(body);
}

impl<'a> TlsClient<'a> {
    /// New connection to `host`. `random` and `eph_priv` must come from a CSPRNG; `now` is Unix time.
    pub fn new(host: &str, roots: &'a RootStore, now: i64, random: [u8; 32], eph_priv: [u8; 32]) -> Self {
        TlsClient {
            host: host.into(),
            roots,
            now,
            eph_priv,
            random,
            state: State::WaitServerHello,
            transcript: Vec::new(),
            inbuf: Vec::new(),
            hsbuf: Vec::new(),
            hs_secret: [0; 32],
            c_hs: None,
            s_in: None,
            c_out: None,
            leaf: None,
            chain: Vec::new(),
            app_in: Vec::new(),
        }
    }

    /// Build the ClientHello record to send first.
    pub fn start(&mut self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&[3, 3]);
        b.extend_from_slice(&self.random);
        b.push(0); // legacy_session_id
        b.extend_from_slice(&[0, 2, 0x13, 0x01]);
        b.extend_from_slice(&[1, 0]); // null compression
        let mut e = Vec::new();
        let mut sni = alloc::vec![0u8];
        sni.extend_from_slice(&be16(self.host.len() as u16));
        sni.extend_from_slice(self.host.as_bytes());
        let mut sni_list = be16(sni.len() as u16).to_vec();
        sni_list.extend_from_slice(&sni);
        ext(&mut e, 0, &sni_list);
        ext(&mut e, 43, &[2, 3, 4]);
        ext(&mut e, 10, &[0, 2, 0, 0x1d]);
        ext(&mut e, 13, &[0, 2, 0x08, 0x04]);
        let mut ks = be16(36).to_vec();
        ks.extend_from_slice(&[0, 0x1d, 0, 32]);
        ks.extend_from_slice(&public_key(&self.eph_priv));
        ext(&mut e, 51, &ks);
        b.extend_from_slice(&be16(e.len() as u16));
        b.extend_from_slice(&e);
        let m = hs_msg(HS_CLIENT_HELLO, &b);
        self.transcript.extend_from_slice(&m);
        let mut rec = alloc::vec![CT_HANDSHAKE, 3, 1];
        rec.extend_from_slice(&be16(m.len() as u16));
        rec.extend_from_slice(&m);
        rec
    }

    /// True once the handshake finished and application data may flow.
    pub fn is_established(&self) -> bool {
        self.state == State::Connected
    }

    /// Take decrypted application data received so far.
    pub fn take_app(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.app_in)
    }

    /// Encrypt application data into records (splits at 16 KiB).
    pub fn send_app(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        if self.state != State::Connected {
            return Err(Error::Invalid);
        }
        let d = self.c_out.as_mut().ok_or(Error::Invalid)?;
        Ok(data.chunks(16384).flat_map(|c| d.seal(CT_APP, c)).collect())
    }

    /// Server certificate chain DER as presented (leaf first), once received.
    pub fn peer_chain(&self) -> &[Vec<u8>] {
        &self.chain
    }

    /// Close notify record.
    pub fn close(&mut self) -> Vec<u8> {
        let out = match (&mut self.c_out, self.state) {
            (Some(d), State::Connected) => d.seal(CT_ALERT, &[1, 0]),
            _ => Vec::new(),
        };
        self.state = State::Closed;
        out
    }

    /// Feed bytes from the network; returns bytes to send back. Any error closes the connection.
    pub fn feed(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        if self.state == State::Closed {
            return Err(Error::Invalid);
        }
        self.inbuf.extend_from_slice(data);
        let mut out = Vec::new();
        let r = self.pump(&mut out);
        if r.is_err() {
            self.state = State::Closed;
        }
        r.map(|_| out)
    }

    fn pump(&mut self, out: &mut Vec<u8>) -> Result<()> {
        loop {
            if self.inbuf.len() < 5 {
                return Ok(());
            }
            let hdr: [u8; 5] = self.inbuf[..5].try_into().unwrap();
            let len = u16::from_be_bytes([hdr[3], hdr[4]]) as usize;
            if len > MAX_RECORD {
                return Err(Error::Parse);
            }
            if self.inbuf.len() < 5 + len {
                return Ok(());
            }
            let body: Vec<u8> = self.inbuf[5..5 + len].to_vec();
            self.inbuf.drain(..5 + len);
            match (hdr[0], self.state) {
                (CT_CCS, _) => {}
                (CT_ALERT, _) => return Err(Error::Denied),
                (CT_HANDSHAKE, State::WaitServerHello) => {
                    self.hsbuf.extend_from_slice(&body);
                    self.process_handshake(out)?;
                }
                (CT_APP, s) if s != State::WaitServerHello => {
                    let (t, pt) = self.s_in.as_mut().ok_or(Error::Parse)?.open(&hdr, &body)?;
                    match t {
                        CT_HANDSHAKE => {
                            self.hsbuf.extend_from_slice(&pt);
                            self.process_handshake(out)?;
                        }
                        CT_APP if s == State::Connected => self.app_in.extend_from_slice(&pt),
                        CT_ALERT => {
                            self.state = State::Closed;
                            return if pt.first() == Some(&1) && pt.get(1) == Some(&0) { Ok(()) } else { Err(Error::Denied) };
                        }
                        _ => return Err(Error::Parse),
                    }
                }
                _ => return Err(Error::Parse),
            }
        }
    }

    fn th(&self) -> [u8; 32] {
        sha256(&self.transcript)
    }

    fn process_handshake(&mut self, out: &mut Vec<u8>) -> Result<()> {
        loop {
            if self.hsbuf.len() < 4 {
                return Ok(());
            }
            let len = (self.hsbuf[1] as usize) << 16 | (self.hsbuf[2] as usize) << 8 | self.hsbuf[3] as usize;
            if self.hsbuf.len() < 4 + len {
                return Ok(());
            }
            let msg: Vec<u8> = self.hsbuf.drain(..4 + len).collect();
            self.handle(msg[0], &msg[4..], &msg, out)?;
            if self.state == State::Closed {
                return Ok(());
            }
        }
    }

    fn handle(&mut self, t: u8, body: &[u8], raw: &[u8], out: &mut Vec<u8>) -> Result<()> {
        match (self.state, t) {
            (State::WaitServerHello, HS_SERVER_HELLO) => {
                self.transcript.extend_from_slice(raw);
                let share = parse_server_hello(body)?;
                let shared = x25519(&self.eph_priv, &share);
                if shared == [0u8; 32] {
                    return Err(Error::Denied);
                }
                let early = hkdf_extract(&[], &[0u8; 32]);
                let derived = derive_secret(&early, "derived", &sha256(b""));
                self.hs_secret = hkdf_extract(&derived, &shared);
                let th = self.th();
                self.c_hs = Some(Dir::new(derive_secret(&self.hs_secret, "c hs traffic", &th)));
                self.s_in = Some(Dir::new(derive_secret(&self.hs_secret, "s hs traffic", &th)));
                self.state = State::WaitEncryptedExtensions;
            }
            (State::WaitEncryptedExtensions, HS_ENCRYPTED_EXTENSIONS) => {
                self.transcript.extend_from_slice(raw);
                self.state = State::WaitCertificate;
            }
            (State::WaitCertificate, HS_CERTIFICATE) => {
                self.transcript.extend_from_slice(raw);
                self.chain = parse_certificates(body)?;
                let refs: Vec<&[u8]> = self.chain.iter().map(|c| c.as_slice()).collect();
                self.roots.verify_chain(&refs, Some(&self.host), self.now)?;
                self.leaf = Some(Cert::parse(&self.chain[0])?);
                self.state = State::WaitCertVerify;
            }
            (State::WaitCertVerify, HS_CERT_VERIFY) => {
                if body.len() < 4 || u16::from_be_bytes([body[0], body[1]]) != 0x0804 {
                    return Err(Error::Unsupported);
                }
                let slen = u16::from_be_bytes([body[2], body[3]]) as usize;
                let sig = body.get(4..4 + slen).ok_or(Error::Parse)?;
                let mut content = alloc::vec![0x20u8; 64];
                content.extend_from_slice(b"TLS 1.3, server CertificateVerify\0");
                content.extend_from_slice(&self.th());
                self.leaf.as_ref().ok_or(Error::Invalid)?.verify_pss(&content, sig)?;
                self.transcript.extend_from_slice(raw);
                self.state = State::WaitFinished;
            }
            (State::WaitFinished, HS_FINISHED) => {
                let s_secret = self.s_in.as_ref().ok_or(Error::Invalid)?.secret;
                let fk = expand_label(&s_secret, "finished", &[], 32);
                if !ct_eq(&hmac(&fk, &self.th()), body) {
                    return Err(Error::BadSignature);
                }
                self.transcript.extend_from_slice(raw);
                let th_app = self.th();
                // client Finished under the client handshake key
                let c_secret = self.c_hs.as_ref().ok_or(Error::Invalid)?.secret;
                let cfk = expand_label(&c_secret, "finished", &[], 32);
                let fin = hs_msg(HS_FINISHED, &hmac(&cfk, &th_app));
                out.extend_from_slice(&self.c_hs.as_mut().unwrap().seal(CT_HANDSHAKE, &fin));
                let derived = derive_secret(&self.hs_secret, "derived", &sha256(b""));
                let master = hkdf_extract(&derived, &[0u8; 32]);
                self.c_out = Some(Dir::new(derive_secret(&master, "c ap traffic", &th_app)));
                self.s_in = Some(Dir::new(derive_secret(&master, "s ap traffic", &th_app)));
                self.c_hs = None;
                self.state = State::Connected;
            }
            (State::Connected, HS_NEW_SESSION_TICKET) => {} // tickets are not stored
            _ => return Err(Error::Parse),
        }
        Ok(())
    }
}

fn parse_server_hello(b: &[u8]) -> Result<[u8; 32]> {
    // version(2) random(32) sid_len sid suite(2) comp(1) ext_len exts
    let sid = *b.get(34).ok_or(Error::Parse)? as usize;
    let p = 35 + sid;
    let tail = b.get(p..p + 5).ok_or(Error::Parse)?;
    if tail[..2] != [0x13, 0x01] || tail[2] != 0 {
        return Err(Error::Unsupported);
    }
    if b[2..34] == HELLO_RETRY_RANDOM {
        return Err(Error::Unsupported);
    }
    let elen = u16::from_be_bytes([tail[3], tail[4]]) as usize;
    let mut e = b.get(p + 5..p + 5 + elen).ok_or(Error::Parse)?;
    let (mut share, mut ver) = (None, false);
    while !e.is_empty() {
        let t = u16::from_be_bytes([*e.first().ok_or(Error::Parse)?, *e.get(1).ok_or(Error::Parse)?]);
        let l = u16::from_be_bytes([*e.get(2).ok_or(Error::Parse)?, *e.get(3).ok_or(Error::Parse)?]) as usize;
        let d = e.get(4..4 + l).ok_or(Error::Parse)?;
        e = &e[4 + l..];
        match t {
            43 => ver = d == [3, 4],
            51 if d.len() == 36 && d[..4] == [0, 0x1d, 0, 32] => share = Some(<[u8; 32]>::try_from(&d[4..]).unwrap()),
            _ => {}
        }
    }
    if !ver {
        return Err(Error::Unsupported);
    }
    share.ok_or(Error::Parse)
}

const HELLO_RETRY_RANDOM: [u8; 32] = [
    0xcf, 0x21, 0xad, 0x74, 0xe5, 0x9a, 0x61, 0x11, 0xbe, 0x1d, 0x8c, 0x02, 0x1e, 0x65, 0xb8, 0x91, 0xc2, 0xa2, 0x11, 0x16, 0x7a, 0xbb, 0x8c, 0x5e,
    0x07, 0x9e, 0x09, 0xe2, 0xc8, 0xa8, 0x33, 0x9c,
];

fn parse_certificates(b: &[u8]) -> Result<Vec<Vec<u8>>> {
    let ctx = *b.first().ok_or(Error::Parse)? as usize;
    let l = b.get(1 + ctx..4 + ctx).ok_or(Error::Parse)?;
    let total = (l[0] as usize) << 16 | (l[1] as usize) << 8 | l[2] as usize;
    let mut list = b.get(4 + ctx..4 + ctx + total).ok_or(Error::Parse)?;
    let mut out = Vec::new();
    while !list.is_empty() {
        let n = (*list.first().ok_or(Error::Parse)? as usize) << 16 | (*list.get(1).ok_or(Error::Parse)? as usize) << 8 | *list.get(2).ok_or(Error::Parse)? as usize;
        out.push(list.get(3..3 + n).ok_or(Error::Parse)?.to_vec());
        let el = u16::from_be_bytes([*list.get(3 + n).ok_or(Error::Parse)?, *list.get(4 + n).ok_or(Error::Parse)?]) as usize;
        list = list.get(5 + n + el..).ok_or(Error::Parse)?;
    }
    if out.is_empty() {
        return Err(Error::Parse);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bignum::Uint;
    use crate::hash::hex;
    use crate::testcerts::*;

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn rfc8448_key_schedule_prefix() {
        let early = hkdf_extract(&[], &[0u8; 32]);
        assert_eq!(early.to_vec(), hex("33ad0a1c607ec03b09e6cd9893680ce210adf300aa1f2660e1b22e10f170f92a"));
        assert_eq!(
            derive_secret(&early, "derived", &sha256(b"")).to_vec(),
            hex("6f2615a108c702c5678f54fc9dbab69716c076189c48250cebeac3576c3611ba")
        );
    }

    // ----- a minimal TLS 1.3 server used only to exercise the client end-to-end -----

    fn mgf1(seed: &[u8], len: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut c = 0u32;
        while out.len() < len {
            let mut v = seed.to_vec();
            v.extend_from_slice(&c.to_be_bytes());
            out.extend_from_slice(&sha256(&v));
            c += 1;
        }
        out.truncate(len);
        out
    }

    fn pss_sign(n: &[u8], d: &[u8], msg: &[u8]) -> Vec<u8> {
        let k = n.len();
        let em_bits = k * 8 - n[0].leading_zeros() as usize - 1;
        let em_len = (em_bits + 7) / 8;
        let salt = [0x5au8; 32];
        let mut mp = alloc::vec![0u8; 8];
        mp.extend_from_slice(&sha256(msg));
        mp.extend_from_slice(&salt);
        let h = sha256(&mp);
        let mut db = alloc::vec![0u8; em_len - 33];
        let l = db.len();
        db[l - 33] = 1;
        db[l - 32..].copy_from_slice(&salt);
        let mut masked: Vec<u8> = db.iter().zip(mgf1(&h, db.len())).map(|(a, b)| a ^ b).collect();
        masked[0] &= 0xff >> (8 * em_len - em_bits);
        let mut em = masked;
        em.extend_from_slice(&h);
        em.push(0xbc);
        Uint::from_be(&em).modpow(&Uint::from_be(d), &Uint::from_be(n)).to_be(k)
    }

    struct Server {
        hs: Dir,
        ap_out: Dir,
        c_hs_secret: [u8; 32],
        c_ap_secret: [u8; 32],
        th_app: [u8; 32],
        c_in: Option<Dir>,
        s_hs_dir: Dir,
    }

    /// Returns (bytes for client, server state).
    fn server_flight(ch_record: &[u8], tamper: bool) -> (Vec<u8>, Server) {
        let ch = &ch_record[5..];
        // pull the client's key_share out of the ClientHello
        let pos = ch.windows(4).position(|w| w == [0, 0x1d, 0, 32]).unwrap();
        let cshare: [u8; 32] = ch[pos + 4..pos + 36].try_into().unwrap();
        let sk = [0x77u8; 32];
        let shared = x25519(&sk, &cshare);
        let mut tr = ch.to_vec();
        let mut sh = alloc::vec![3, 3];
        sh.extend_from_slice(&[0x42; 32]);
        sh.push(0);
        sh.extend_from_slice(&[0x13, 0x01, 0]);
        let mut e = Vec::new();
        ext(&mut e, 43, &[3, 4]);
        let mut ks = alloc::vec![0, 0x1d, 0, 32];
        ks.extend_from_slice(&public_key(&sk));
        ext(&mut e, 51, &ks);
        sh.extend_from_slice(&be16(e.len() as u16));
        sh.extend_from_slice(&e);
        let shm = hs_msg(HS_SERVER_HELLO, &sh);
        tr.extend_from_slice(&shm);
        let mut out = alloc::vec![CT_HANDSHAKE, 3, 3];
        out.extend_from_slice(&be16(shm.len() as u16));
        out.extend_from_slice(&shm);

        let early = hkdf_extract(&[], &[0u8; 32]);
        let derived = derive_secret(&early, "derived", &sha256(b""));
        let hs = hkdf_extract(&derived, &shared);
        let th = sha256(&tr);
        let c_hs_secret = derive_secret(&hs, "c hs traffic", &th);
        let s_hs_secret = derive_secret(&hs, "s hs traffic", &th);
        let mut s_hs = Dir::new(s_hs_secret);

        let ee = hs_msg(HS_ENCRYPTED_EXTENSIONS, &[0, 0]);
        let mut cert_body = alloc::vec![0u8];
        let mut list = Vec::new();
        for c in [hex(LEAF), hex(INT)] {
            list.extend_from_slice(&[(c.len() >> 16) as u8, (c.len() >> 8) as u8, c.len() as u8]);
            list.extend_from_slice(&c);
            list.extend_from_slice(&[0, 0]);
        }
        cert_body.extend_from_slice(&[(list.len() >> 16) as u8, (list.len() >> 8) as u8, list.len() as u8]);
        cert_body.extend_from_slice(&list);
        let cert = hs_msg(HS_CERTIFICATE, &cert_body);
        tr.extend_from_slice(&ee);
        tr.extend_from_slice(&cert);

        let leaf = Cert::parse(&hex(LEAF)).unwrap();
        let (n, _) = leaf.rsa_key();
        let mut content = alloc::vec![0x20u8; 64];
        content.extend_from_slice(b"TLS 1.3, server CertificateVerify\0");
        content.extend_from_slice(&sha256(&tr));
        let mut sig = pss_sign(n, &hex(LEAF_D), &content);
        if tamper {
            sig[10] ^= 1;
        }
        let mut cv_body = alloc::vec![0x08, 0x04];
        cv_body.extend_from_slice(&be16(sig.len() as u16));
        cv_body.extend_from_slice(&sig);
        let cv = hs_msg(HS_CERT_VERIFY, &cv_body);
        tr.extend_from_slice(&cv);
        let fk = expand_label(&s_hs_secret, "finished", &[], 32);
        let fin = hs_msg(HS_FINISHED, &hmac(&fk, &sha256(&tr)));
        tr.extend_from_slice(&fin);

        // coalesce EE..Finished into a single encrypted record, plus a stray CCS first
        out.extend_from_slice(&[CT_CCS, 3, 3, 0, 1, 1]);
        let flight: Vec<u8> = [ee, cert, cv, fin].concat();
        out.extend_from_slice(&s_hs.seal(CT_HANDSHAKE, &flight));

        let th_app = sha256(&tr);
        let derived2 = derive_secret(&hs, "derived", &sha256(b""));
        let master = hkdf_extract(&derived2, &[0u8; 32]);
        let c_ap_secret = derive_secret(&master, "c ap traffic", &th_app);
        let ap_out = Dir::new(derive_secret(&master, "s ap traffic", &th_app));
        (out, Server { hs: Dir::new(c_hs_secret), ap_out, c_hs_secret, c_ap_secret, th_app, c_in: None, s_hs_dir: s_hs })
    }

    fn roots() -> RootStore {
        let mut s = RootStore::new();
        s.add_root(&hex(ROOT)).unwrap();
        s
    }

    fn client<'a>(r: &'a RootStore, host: &str) -> TlsClient<'a> {
        TlsClient::new(host, r, NOW, [9; 32], [0x33; 32])
    }

    #[test]
    fn full_handshake_and_data() {
        let r = roots();
        let mut c = client(&r, "example.test");
        let ch = c.start();
        let (flight, mut srv) = server_flight(&ch, false);
        // deliver byte-by-byte-ish in two chunks to exercise buffering
        let mid = flight.len() / 3;
        assert!(c.feed(&flight[..mid]).unwrap().is_empty());
        let reply = c.feed(&flight[mid..]).unwrap();
        assert!(c.is_established());
        // server verifies the client Finished
        let hdr: [u8; 5] = reply[..5].try_into().unwrap();
        let (t, pt) = srv.hs.open(&hdr, &reply[5..]).unwrap();
        assert_eq!(t, CT_HANDSHAKE);
        let cfk = expand_label(&srv.c_hs_secret, "finished", &[], 32);
        assert_eq!(pt, hs_msg(HS_FINISHED, &hmac(&cfk, &srv.th_app)));
        assert_eq!(c.peer_chain().len(), 2);

        // client -> server application data
        let rec = c.send_app(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        let mut cin = Dir::new(srv.c_ap_secret);
        let h: [u8; 5] = rec[..5].try_into().unwrap();
        assert_eq!(cin.open(&h, &rec[5..]).unwrap(), (CT_APP, b"GET / HTTP/1.1\r\n\r\n".to_vec()));
        srv.c_in = Some(cin);
        // server -> client, plus a NewSessionTicket first
        let tk = srv.ap_out.seal(CT_HANDSHAKE, &hs_msg(HS_NEW_SESSION_TICKET, &[0; 20]));
        c.feed(&tk).unwrap();
        let resp = srv.ap_out.seal(CT_APP, b"HTTP/1.1 200 OK");
        c.feed(&resp).unwrap();
        assert_eq!(c.take_app(), b"HTTP/1.1 200 OK");
        // tampered record is rejected and closes
        let mut bad = srv.ap_out.seal(CT_APP, b"x");
        let l = bad.len();
        bad[l - 1] ^= 1;
        assert_eq!(c.feed(&bad), Err(Error::BadSignature));
        assert!(c.send_app(b"y").is_err());
        let _ = &srv.s_hs_dir;
    }

    #[test]
    fn rejects_wrong_host_bad_cv_and_untrusted_root() {
        let r = roots();
        let mut c = client(&r, "evil.test");
        let ch = c.start();
        let (flight, _) = server_flight(&ch, false);
        assert_eq!(c.feed(&flight), Err(Error::Untrusted));

        let mut c = client(&r, "example.test");
        let ch = c.start();
        let (flight, _) = server_flight(&ch, true);
        assert_eq!(c.feed(&flight), Err(Error::BadSignature));

        let empty = RootStore::new();
        let mut c = client(&empty, "example.test");
        let ch = c.start();
        let (flight, _) = server_flight(&ch, false);
        assert_eq!(c.feed(&flight), Err(Error::Untrusted));
    }

    #[test]
    fn alert_and_garbage() {
        let r = roots();
        let mut c = client(&r, "example.test");
        c.start();
        assert_eq!(c.feed(&[CT_ALERT, 3, 3, 0, 2, 2, 40]), Err(Error::Denied));
        let mut c = client(&r, "example.test");
        c.start();
        assert_eq!(c.feed(&[CT_APP, 3, 3, 0xff, 0xff]), Err(Error::Parse));
        let mut c = client(&r, "example.test");
        assert!(c.send_app(b"x").is_err());
    }
}
