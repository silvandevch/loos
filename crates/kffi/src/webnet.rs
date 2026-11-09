//! Native HTTP/1.1 and HTTPS client for the browser: DNS over the stack's UDP, TCP, TLS 1.3 (`loos-security`) against the
//! embedded root store, redirects, `Content-Length` and chunked bodies. Blocking (the UI waits, as it did for the COM2
//! bridge) but bounded by timeouts. Results go back to the browser deflate-compressed, the same contract `web.rs` used.

use super::*;
use alloc::format;
use alloc::string::ToString;
use loos_security::tls::TlsClient;
use loos_security::x509::RootStore;

extern "C" {
    fn time_ms() -> u32;
    fn krandom(out: *mut u8, n: usize);
    fn rtc_unix() -> u64;
    fn net_nic_summary(out: *mut u8, cap: usize) -> usize;
}

const ROOTS: &[u8] = include_bytes!("../../../certs/roots.bin");
const MAX_BODY: usize = 4 * 1024 * 1024;
const MAX_REDIRECTS: usize = 6;
const CONNECT_MS: u64 = 10_000;
const IDLE_MS: u64 = 15_000;
/// RTC clocks on VMs often run on local time rather than UTC; accept certificates that are "not yet valid" by up to this much.
const CLOCK_SLACK_S: i64 = 14 * 3600;

static LAST_ERROR: Global<String> = Global::new();

fn fail<T>(msg: impl Into<String>) -> Result<T, String> {
    Err(msg.into())
}

fn now_ms() -> u64 {
    // SAFETY: plain C accessor.
    u64::from(unsafe { time_ms() })
}

fn pump() {
    loos_net_poll(now_ms());
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    // SAFETY: b is N writable bytes.
    unsafe { krandom(b.as_mut_ptr(), N) };
    b
}

fn roots() -> RootStore {
    let mut store = RootStore::new();
    let mut d = ROOTS;
    while d.len() > 4 {
        let n = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) as usize;
        let Some(der) = d.get(4..4 + n) else { break };
        let _ = store.add_root(der); // roots with key types we cannot verify are skipped
        d = &d[4 + n..];
    }
    store
}

struct Url {
    https: bool,
    host: String,
    port: u16,
    path: String,
}

fn parse_url(u: &str) -> Result<Url, String> {
    let (https, rest) = if let Some(r) = u.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = u.strip_prefix("http://") {
        (false, r)
    } else {
        return fail("only http:// and https:// addresses can be loaded");
    };
    let (auth, path) = match rest.find(['/', '?', '#']) {
        Some(i) if rest.as_bytes()[i] == b'/' => (&rest[..i], rest[i..].to_string()),
        Some(i) => (&rest[..i], format!("/{}", &rest[i..])),
        None => (rest, "/".to_string()),
    };
    let path = path.split('#').next().unwrap_or("/").to_string();
    let auth = auth.rsplit('@').next().unwrap_or(auth);
    let (host, port) = match auth.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => (h, p.parse::<u16>().map_err(|_| "bad port".to_string())?),
        _ => (auth, if https { 443 } else { 80 }),
    };
    if host.is_empty() {
        return fail("the address has no host name");
    }
    Ok(Url { https, host: host.to_string(), port, path })
}

fn resolve(host: &str) -> Result<[u8; 4], String> {
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return Ok(ip.octets());
    }
    let Some(n) = NET.get().as_mut() else { return fail("no network adapter") };
    let dns = n.cfg.dns.to_be_bytes();
    if n.cfg.ip == 0 {
        return fail("the network is not configured (no DHCP lease yet)");
    }
    let id = 0x4c00 | (now_ms() as u16 & 0xff);
    let port = 41000 + (now_ms() % 1000) as u16;
    let udp = n.stack.udp_bind(port).map_err(|_| "cannot open a DNS socket".to_string())?;
    let q = loos_winrt::ws2::dns_query(host, id);
    let t0 = now_ms();
    let mut last_send = None::<u64>;
    let res = loop {
        pump();
        let Some(n) = NET.get().as_mut() else { return fail("the network went away") };
        if last_send.is_none_or(|t| now_ms() - t > 1500) {
            if now_ms() - t0 > 6000 {
                break fail(format!("DNS server {}.{}.{}.{} did not answer", dns[0], dns[1], dns[2], dns[3]));
            }
            let _ = n.stack.udp_send(udp, IpAddr::V4(Ipv4Addr::from(dns)), 53, &q);
            last_send = Some(now_ms());
        }
        let (mut found, mut nx) = (None, false);
        while let Some((_, _, data)) = n.stack.udp_recv(udp) {
            match loos_winrt::ws2::dns_answer(&data, id) {
                Ok(Some(ip)) => found = Some(ip),
                Err(()) => nx = true,
                Ok(None) => {}
            }
        }
        if let Some(ip) = found {
            break Ok(ip);
        }
        if nx {
            break fail(format!("the name {host} does not exist (DNS)"));
        }
    };
    if let Some(n) = NET.get().as_mut() {
        n.stack.udp_close(udp);
    }
    res
}

/// A connected socket, with TLS on top for https.
struct Conn<'a> {
    h: usize,
    tls: Option<TlsClient<'a>>,
}

fn net() -> Result<&'static mut NetState, String> {
    NET.get().as_mut().ok_or_else(|| "no network adapter".to_string())
}

fn tcp_write(h: usize, mut data: &[u8]) -> Result<(), String> {
    let t0 = now_ms();
    while !data.is_empty() {
        let n = net()?.stack.tcp_send(h, data).map_err(|_| "connection closed while sending".to_string())?;
        data = &data[n..];
        if n == 0 {
            if net()?.stack.tcp_failure(h).is_some() {
                return fail("connection reset while sending");
            }
            if now_ms() - t0 > IDLE_MS {
                return fail("timed out sending the request");
            }
        }
        pump();
    }
    Ok(())
}

impl Conn<'_> {
    /// Reads whatever is available; decrypted if TLS. Returns false at end of stream.
    fn read(&mut self, out: &mut Vec<u8>) -> Result<bool, String> {
        pump();
        let n = net()?;
        let mut buf = [0u8; 4096];
        let got = n.stack.tcp_recv(self.h, &mut buf).map_err(|_| "connection lost".to_string())?;
        if got > 0 {
            match &mut self.tls {
                Some(t) => {
                    let reply = t.feed(&buf[..got]).map_err(|e| format!("TLS error: {e:?}"))?;
                    if !reply.is_empty() {
                        tcp_write(self.h, &reply)?;
                    }
                    out.extend_from_slice(&t.take_app());
                }
                None => out.extend_from_slice(&buf[..got]),
            }
            return Ok(true);
        }
        let n = net()?;
        if n.stack.tcp_eof(self.h) {
            return Ok(false);
        }
        if let Some(f) = n.stack.tcp_failure(self.h) {
            return fail(format!("connection failed: {f:?}"));
        }
        Ok(true)
    }

    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        match &mut self.tls {
            Some(t) => {
                let rec = t.send_app(data).map_err(|e| format!("TLS error: {e:?}"))?;
                tcp_write(self.h, &rec)
            }
            None => tcp_write(self.h, data),
        }
    }
}

struct Response {
    status: u16,
    location: Option<String>,
    body: Vec<u8>,
}

fn header<'a>(h: &'a str, name: &str) -> Option<&'a str> {
    h.lines().skip(1).find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then_some(v.trim())
    })
}

/// Decodes a chunked body; `Some` once the terminating chunk has arrived.
fn dechunk(mut d: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = d.windows(2).position(|w| w == b"\r\n")?;
        let size = usize::from_str_radix(core::str::from_utf8(&d[..eol]).ok()?.split(';').next()?.trim(), 16).ok()?;
        d = &d[eol + 2..];
        if size == 0 {
            return Some(out);
        }
        if d.len() < size + 2 {
            return None;
        }
        out.extend_from_slice(&d[..size]);
        d = &d[size + 2..];
    }
}

fn get_once(u: &Url, roots: &RootStore) -> Result<Response, String> {
    let ip = resolve(&u.host)?;
    let h = net()?.stack.tcp_connect(IpAddr::V4(Ipv4Addr::from(ip)), u.port).map_err(|_| "no route to the host".to_string())?;
    let t0 = now_ms();
    loop {
        pump();
        match net()?.stack.tcp_state(h) {
            Some(State::Established) => break,
            Some(State::Closed) | None => return fail(format!("{} refused or dropped the connection", u.host)),
            _ => {}
        }
        if now_ms() - t0 > CONNECT_MS {
            return fail(format!("timed out connecting to {}.{}.{}.{}:{}", ip[0], ip[1], ip[2], ip[3], u.port));
        }
    }
    let now = unsafe { rtc_unix() } as i64 + CLOCK_SLACK_S;
    let mut c = Conn { h, tls: None };
    if u.https {
        let mut t = TlsClient::new(&u.host, roots, now, random::<32>(), random::<32>());
        tcp_write(h, &t.start())?;
        c.tls = Some(t);
        let mut sink = Vec::new();
        let t1 = now_ms();
        while !c.tls.as_ref().is_some_and(|t| t.is_established()) {
            if !c.read(&mut sink)? {
                return fail("the server closed the connection during the TLS handshake");
            }
            if now_ms() - t1 > CONNECT_MS + 8000 {
                return fail("TLS handshake timed out");
            }
        }
    }
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: LoOS-Browser/1.0\r\nAccept: text/html,text/plain,*/*;q=0.8\r\nAccept-Language: en\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
        u.path, u.host
    );
    c.write(req.as_bytes())?;
    let mut raw = Vec::new();
    let mut idle = now_ms();
    loop {
        let before = raw.len();
        let open = c.read(&mut raw)?;
        if raw.len() != before {
            idle = now_ms();
        }
        if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = core::str::from_utf8(&raw[..end]).map_err(|_| "bad response header".to_string())?;
            let body = &raw[end + 4..];
            let complete = if header(head, "transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
                dechunk(body)
            } else if let Some(n) = header(head, "content-length").and_then(|v| v.parse::<usize>().ok()) {
                (body.len() >= n).then(|| body[..n].to_vec())
            } else if !open {
                Some(body.to_vec())
            } else {
                None
            };
            if let Some(body) = complete.or_else(|| (!open).then(|| body.to_vec())) {
                let status = head.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                let location = header(head, "location").map(|s| s.to_string());
                if let Some(t) = c.tls.as_mut() {
                    let bye = t.close();
                    let _ = tcp_write(h, &bye);
                }
                let _ = net()?.stack.tcp_close(h);
                return Ok(Response { status, location, body });
            }
        } else if !open {
            return fail("the server closed the connection without answering");
        }
        if raw.len() > MAX_BODY + 65536 {
            return fail("the page is too large");
        }
        if now_ms() - idle > IDLE_MS {
            return fail("the server stopped sending data");
        }
    }
}

fn absolute(base: &Url, loc: &str) -> String {
    if loc.starts_with("http://") || loc.starts_with("https://") {
        return loc.to_string();
    }
    let scheme = if base.https { "https" } else { "http" };
    let default = if base.https { 443 } else { 80 };
    let port = if base.port == default { String::new() } else { format!(":{}", base.port) };
    if let Some(r) = loc.strip_prefix("//") {
        return format!("{scheme}://{r}");
    }
    if loc.starts_with('/') {
        return format!("{scheme}://{}{port}{loc}", base.host);
    }
    let dir = base.path.rsplit_once('/').map_or("", |(d, _)| d);
    format!("{scheme}://{}{port}{dir}/{loc}", base.host)
}

/// GETs `url`, following redirects. Returns the body and the final URL.
pub(super) fn get(url: &str) -> Result<(Vec<u8>, String), String> {
    let store = roots();
    let mut cur = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let u = parse_url(&cur)?;
        let r = get_once(&u, &store)?;
        match r.status {
            301 | 302 | 303 | 307 | 308 => {
                let Some(l) = r.location else { return fail("redirect without a Location") };
                cur = absolute(&u, &l);
            }
            200..=299 => return Ok((r.body, cur)),
            s => {
                // Error pages are still pages: show what the server sent.
                if r.body.is_empty() {
                    return fail(format!("the server answered HTTP {s}"));
                }
                return Ok((r.body, cur));
            }
        }
    }
    fail("too many redirects")
}

/// Raw deflate using only stored blocks. The browser needs a valid deflate stream, not a small one, and the real compressor
/// needs more stack than the boot kernel has to spare.
fn deflate_stored(d: &[u8]) -> Vec<u8> {
    let mut z = Vec::with_capacity(d.len() + d.len() / 65535 * 5 + 8);
    let mut chunks = d.chunks(65535).peekable();
    if chunks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(c) = chunks.next() {
        z.push(u8::from(chunks.peek().is_none()));
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    z
}

/// C ABI for `loos-shell`'s web layer. Returns 1 when `out` holds a deflated page, 0 when no native network is configured
/// (the caller may fall back to the development bridge), -1 on failure (reason via [`loos_net_last_error`]).
///
/// # Safety
/// `url` is NUL-terminated; `out`/`final_url` are writable for their stated capacities; the length pointers are valid.
#[no_mangle]
pub unsafe extern "C" fn loos_net_fetch(url: *const u8, out: *mut u8, cap: usize, out_len: *mut usize, raw_len: *mut usize, final_url: *mut u8, final_cap: usize) -> i32 {
    let msg = match NET.get().as_ref() {
        None => {
            let mut b = [0u8; 128];
            // SAFETY: b is writable for its length.
            let n = unsafe { net_nic_summary(b.as_mut_ptr(), b.len()) };
            let ids = core::str::from_utf8(&b[..n]).unwrap_or("");
            if ids.is_empty() {
                "No network adapter was found. Add one to the virtual machine, type e1000.".to_string()
            } else {
                format!("None of the network adapters here is supported (found PCI {ids}). LoOS supports Intel e1000 and e1000e (8086:100e, 100f, 10d3).")
            }
        }
        Some(n) if n.cfg.ip == 0 => format!(
            "The e1000 adapter is present but DHCP has not answered (link {}, {} frames sent, {} received). Check that the VM network is NAT or bridged and connected.",
            if n.link { "up" } else { "down" }, n.tx, n.rx
        ),
        Some(_) => String::new(),
    };
    if !msg.is_empty() {
        *LAST_ERROR.get() = Some(msg);
        return 0;
    }
    let mut n = 0;
    // SAFETY: NUL-terminated per contract.
    while unsafe { *url.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: n bytes before the NUL.
    let Ok(u) = core::str::from_utf8(unsafe { core::slice::from_raw_parts(url, n) }) else { return -1 };
    match get(u) {
        Ok((body, fin)) => {
            let body = &body[..body.len().min(MAX_BODY)];
            let z = deflate_stored(body);
            if z.len() > cap {
                *LAST_ERROR.get() = Some("The page is too large to display.".to_string());
                return -1;
            }
            let fb = fin.as_bytes();
            let fl = fb.len().min(final_cap - 1);
            // SAFETY: capacities checked above; pointers valid per contract.
            unsafe {
                core::ptr::copy_nonoverlapping(z.as_ptr(), out, z.len());
                *out_len = z.len();
                *raw_len = body.len().max(1);
                core::ptr::copy_nonoverlapping(fb.as_ptr(), final_url, fl);
                *final_url.add(fl) = 0;
            }
            1
        }
        Err(e) => {
            *LAST_ERROR.get() = Some(e);
            -1
        }
    }
}

/// Copies the reason of the last failed [`loos_net_fetch`] (NUL-terminated, truncated to `cap`) and returns its length.
///
/// # Safety
/// `out` must be writable for `cap` bytes.
#[no_mangle]
pub unsafe extern "C" fn loos_net_last_error(out: *mut u8, cap: usize) -> usize {
    let s = LAST_ERROR.get().as_deref().unwrap_or("");
    let n = s.len().min(cap.saturating_sub(1));
    if cap > 0 {
        // SAFETY: n < cap.
        unsafe {
            core::ptr::copy_nonoverlapping(s.as_ptr(), out, n);
            *out.add(n) = 0;
        }
    }
    n
}
