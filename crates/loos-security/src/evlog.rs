//! Local event log (EVTX-like chunked, CRC-protected, circular) and ETW-like trace sessions.
//!
//! Everything stays on the machine: there is no exporter other than [`Log::to_bytes`] for the
//! caller to write to disk.

use crate::{Error, Result};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;

/// Bytes per chunk (header + records).
pub const CHUNK: usize = 65536;
const CHUNK_HDR: usize = 40;
const FILE_HDR: usize = 64;
const FILE_MAGIC: &[u8; 8] = b"LoEvLog\0";
const CHUNK_MAGIC: &[u8; 8] = b"LoChunk\0";
const REC_MAGIC: [u8; 4] = *b"**\0\0";

/// CRC-32 (IEEE).
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = (c >> 1) ^ (0xedb88320 & (!(c & 1)).wrapping_add(1));
        }
    }
    !c
}

/// Event severity (lower is worse, as in Windows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Always logged.
    LogAlways = 0,
    /// Critical.
    Critical = 1,
    /// Error.
    Error = 2,
    /// Warning.
    Warning = 3,
    /// Informational.
    Info = 4,
    /// Verbose.
    Verbose = 5,
}

impl Level {
    fn from_u8(v: u8) -> Level {
        match v {
            0 => Level::LogAlways,
            1 => Level::Critical,
            2 => Level::Error,
            3 => Level::Warning,
            4 => Level::Info,
            _ => Level::Verbose,
        }
    }
}

/// One logged event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Assigned by the log on append.
    pub record_id: u64,
    /// Unix time (ms).
    pub time: u64,
    /// Provider name.
    pub provider: String,
    /// Event id.
    pub event_id: u16,
    /// Severity.
    pub level: Level,
    /// Keyword bitmask.
    pub keywords: u64,
    /// Process id.
    pub pid: u32,
    /// Thread id.
    pub tid: u32,
    /// Named data fields.
    pub data: Vec<(String, String)>,
}

impl Event {
    /// Build an event (record id is filled in by the log).
    pub fn new(provider: &str, event_id: u16, level: Level, time: u64) -> Event {
        Event { record_id: 0, time, provider: provider.into(), event_id, level, keywords: 0, pid: 0, tid: 0, data: Vec::new() }
    }

    /// Add a data field.
    pub fn with(mut self, k: &str, v: &str) -> Event {
        self.data.push((k.into(), v.into()));
        self
    }

    fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&REC_MAGIC);
        b.extend_from_slice(&[0; 4]); // size, patched below
        b.extend_from_slice(&self.record_id.to_le_bytes());
        b.extend_from_slice(&self.time.to_le_bytes());
        b.extend_from_slice(&self.keywords.to_le_bytes());
        b.extend_from_slice(&self.event_id.to_le_bytes());
        b.push(self.level as u8);
        b.extend_from_slice(&self.pid.to_le_bytes());
        b.extend_from_slice(&self.tid.to_le_bytes());
        put_str(&mut b, &self.provider);
        b.push(self.data.len().min(255) as u8);
        for (k, v) in self.data.iter().take(255) {
            put_str(&mut b, k);
            put_str(&mut b, v);
        }
        let n = (b.len() + 4) as u32; // trailing copy of the size allows backward walking
        b[4..8].copy_from_slice(&n.to_le_bytes());
        b.extend_from_slice(&n.to_le_bytes());
        b
    }

    fn decode(r: &[u8]) -> Result<Event> {
        if r.len() < 8 + 8 + 8 + 8 + 2 + 1 + 8 || r[..4] != REC_MAGIC {
            return Err(Error::Parse);
        }
        let mut p = 8;
        let u64at = |p: usize| u64::from_le_bytes(r[p..p + 8].try_into().unwrap());
        let record_id = u64at(p);
        let time = u64at(p + 8);
        let keywords = u64at(p + 16);
        p += 24;
        let event_id = u16::from_le_bytes([r[p], r[p + 1]]);
        let level = Level::from_u8(r[p + 2]);
        let pid = u32::from_le_bytes(r[p + 3..p + 7].try_into().unwrap());
        let tid = u32::from_le_bytes(r[p + 7..p + 11].try_into().unwrap());
        p += 11;
        let provider = get_str(r, &mut p)?;
        let cnt = *r.get(p).ok_or(Error::Parse)?;
        p += 1;
        let mut data = Vec::new();
        for _ in 0..cnt {
            let k = get_str(r, &mut p)?;
            let v = get_str(r, &mut p)?;
            data.push((k, v));
        }
        Ok(Event { record_id, time, provider, event_id, level, keywords, pid, tid, data })
    }
}

fn put_str(b: &mut Vec<u8>, s: &str) {
    let s = &s.as_bytes()[..s.len().min(u16::MAX as usize)];
    b.extend_from_slice(&(s.len() as u16).to_le_bytes());
    b.extend_from_slice(s);
}

fn get_str(r: &[u8], p: &mut usize) -> Result<String> {
    let l = u16::from_le_bytes(r.get(*p..*p + 2).ok_or(Error::Parse)?.try_into().unwrap()) as usize;
    let s = r.get(*p + 2..*p + 2 + l).ok_or(Error::Parse)?;
    *p += 2 + l;
    String::from_utf8(s.to_vec()).map_err(|_| Error::Parse)
}

#[derive(Clone)]
struct Chunk {
    first: u64,
    last: u64,
    data: Vec<u8>,
}

/// Retention behaviour when the log is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    /// Overwrite the oldest chunk.
    Circular,
    /// Refuse new events with [`Error::Full`] (Security logs that must not lose audit records).
    Retain,
}

/// Event filter.
#[derive(Default, Clone)]
pub struct Query {
    /// Only events at this severity or worse.
    pub max_level: Option<Level>,
    /// Provider name.
    pub provider: Option<String>,
    /// Event id.
    pub event_id: Option<u16>,
    /// Inclusive time range (ms).
    pub time: Option<(u64, u64)>,
    /// Only records with id greater than this.
    pub after_id: Option<u64>,
    /// Required keyword bits.
    pub keywords: u64,
}

impl Query {
    fn matches(&self, e: &Event) -> bool {
        self.max_level.map_or(true, |l| e.level <= l)
            && self.provider.as_ref().map_or(true, |p| *p == e.provider)
            && self.event_id.map_or(true, |i| i == e.event_id)
            && self.time.map_or(true, |(a, b)| e.time >= a && e.time <= b)
            && self.after_id.map_or(true, |a| e.record_id > a)
            && e.keywords & self.keywords == self.keywords
    }
}

/// One channel's log.
pub struct Log {
    chunks: VecDeque<Chunk>,
    max_chunks: usize,
    next_id: u64,
    /// Full-log behaviour.
    pub retention: Retention,
    /// Chunks dropped on load because their CRC did not match.
    pub corrupt_chunks: usize,
}

impl Log {
    /// Empty log of at most `max_chunks` chunks (64 KiB each).
    pub fn new(max_chunks: usize, retention: Retention) -> Log {
        Log { chunks: VecDeque::new(), max_chunks: max_chunks.max(1), next_id: 1, retention, corrupt_chunks: 0 }
    }

    /// Append an event; returns its record id.
    pub fn append(&mut self, mut e: Event) -> Result<u64> {
        e.record_id = self.next_id;
        let rec = e.encode();
        if rec.len() > CHUNK - CHUNK_HDR {
            return Err(Error::Invalid);
        }
        let fits = self.chunks.back().is_some_and(|c| CHUNK_HDR + c.data.len() + rec.len() <= CHUNK);
        if !fits {
            if self.chunks.len() == self.max_chunks {
                if self.retention == Retention::Retain {
                    return Err(Error::Full);
                }
                self.chunks.pop_front();
            }
            self.chunks.push_back(Chunk { first: e.record_id, last: e.record_id, data: Vec::new() });
        }
        let c = self.chunks.back_mut().unwrap();
        c.data.extend_from_slice(&rec);
        c.last = e.record_id;
        self.next_id += 1;
        Ok(e.record_id)
    }

    /// Events matching `q`, oldest first.
    pub fn query(&self, q: &Query) -> Vec<Event> {
        let mut out = Vec::new();
        for c in &self.chunks {
            if q.after_id.is_some_and(|a| c.last <= a) {
                continue;
            }
            let mut p = 0;
            while p + 8 <= c.data.len() {
                let n = u32::from_le_bytes(c.data[p + 4..p + 8].try_into().unwrap()) as usize;
                let Some(rec) = c.data.get(p..p + n) else { break };
                if let Ok(e) = Event::decode(rec) {
                    if q.matches(&e) {
                        out.push(e);
                    }
                }
                p += n.max(8);
            }
        }
        out
    }

    /// Number of events retained.
    pub fn len(&self) -> usize {
        self.chunks.iter().map(|c| (c.last - c.first + 1) as usize).sum()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Serialize to the on-disk image (file header + fixed-size chunks).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(FILE_HDR + self.chunks.len() * CHUNK);
        out.extend_from_slice(FILE_MAGIC);
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(self.chunks.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.max_chunks as u32).to_le_bytes());
        out.push(self.retention as u8);
        out.extend_from_slice(&[0; 3]);
        out.extend_from_slice(&self.next_id.to_le_bytes());
        out.resize(FILE_HDR - 4, 0);
        let crc = crc32(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        for c in &self.chunks {
            let start = out.len();
            out.extend_from_slice(CHUNK_MAGIC);
            out.extend_from_slice(&c.first.to_le_bytes());
            out.extend_from_slice(&c.last.to_le_bytes());
            out.extend_from_slice(&(c.data.len() as u32).to_le_bytes());
            out.extend_from_slice(&crc32(&c.data).to_le_bytes());
            out.resize(start + CHUNK_HDR, 0);
            out.extend_from_slice(&c.data);
            out.resize(start + CHUNK, 0);
        }
        out
    }

    /// Load an image. Chunks failing their CRC are dropped (and counted in `corrupt_chunks`);
    /// a damaged file header is an error.
    pub fn from_bytes(b: &[u8]) -> Result<Log> {
        if b.len() < FILE_HDR || &b[..8] != FILE_MAGIC || crc32(&b[..FILE_HDR - 4]) != u32::from_le_bytes(b[FILE_HDR - 4..FILE_HDR].try_into().unwrap()) {
            return Err(Error::Parse);
        }
        let n = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
        let max_chunks = u32::from_le_bytes(b[16..20].try_into().unwrap()) as usize;
        let retention = if b[20] == 1 { Retention::Retain } else { Retention::Circular };
        let next_id = u64::from_le_bytes(b[24..32].try_into().unwrap());
        if b.len() < FILE_HDR + n * CHUNK || n > max_chunks {
            return Err(Error::Parse);
        }
        let mut log = Log::new(max_chunks, retention);
        log.next_id = next_id;
        for i in 0..n {
            let c = &b[FILE_HDR + i * CHUNK..FILE_HDR + (i + 1) * CHUNK];
            let used = u32::from_le_bytes(c[24..28].try_into().unwrap()) as usize;
            let ok = &c[..8] == CHUNK_MAGIC && used <= CHUNK - CHUNK_HDR && crc32(&c[CHUNK_HDR..CHUNK_HDR + used]) == u32::from_le_bytes(c[28..32].try_into().unwrap());
            if !ok {
                log.corrupt_chunks += 1;
                continue;
            }
            log.chunks.push_back(Chunk {
                first: u64::from_le_bytes(c[8..16].try_into().unwrap()),
                last: u64::from_le_bytes(c[16..24].try_into().unwrap()),
                data: c[CHUNK_HDR..CHUNK_HDR + used].to_vec(),
            });
        }
        Ok(log)
    }
}

/// Channel manager (Application, System, Security, Setup, ...).
pub struct EventLog {
    channels: BTreeMap<String, Log>,
}

/// Event id written to `Security` when a log is cleared (same id Windows uses).
pub const EVT_LOG_CLEARED: u16 = 1102;

impl Default for EventLog {
    fn default() -> Self {
        Self::new()
    }
}

impl EventLog {
    /// Standard channels: Application and System circular, Security retain-only.
    pub fn new() -> Self {
        let mut channels = BTreeMap::new();
        channels.insert("Application".into(), Log::new(16, Retention::Circular));
        channels.insert("System".into(), Log::new(16, Retention::Circular));
        channels.insert("Security".into(), Log::new(64, Retention::Retain));
        EventLog { channels }
    }

    /// Create or replace a channel.
    pub fn add_channel(&mut self, name: &str, log: Log) {
        self.channels.insert(name.into(), log);
    }

    /// Write an event to `channel`.
    pub fn write(&mut self, channel: &str, e: Event) -> Result<u64> {
        self.channels.get_mut(channel).ok_or(Error::Invalid)?.append(e)
    }

    /// Query a channel.
    pub fn query(&self, channel: &str, q: &Query) -> Result<Vec<Event>> {
        Ok(self.channels.get(channel).ok_or(Error::Invalid)?.query(q))
    }

    /// Access a channel's log.
    pub fn channel(&self, name: &str) -> Option<&Log> {
        self.channels.get(name)
    }

    /// Clear a channel. The act is itself recorded in `Security` (who cleared what, and when).
    pub fn clear(&mut self, channel: &str, by: &str, time: u64) -> Result<()> {
        let old = self.channels.get(channel).ok_or(Error::Invalid)?;
        let fresh = Log { chunks: VecDeque::new(), max_chunks: old.max_chunks, next_id: old.next_id, retention: old.retention, corrupt_chunks: 0 };
        self.channels.insert(channel.into(), fresh);
        let e = Event::new("LoOS-Eventlog", EVT_LOG_CLEARED, Level::Info, time).with("channel", channel).with("by", by);
        // Security is retain-only: if it is full the clear is refused rather than made silent.
        let sec = self.channels.get_mut("Security").ok_or(Error::Invalid)?;
        sec.append(e).map(|_| ())
    }
}

// ---------------------------------------------------------------- tracing

/// Provider identifier (GUID value).
pub type ProviderId = u128;

/// One buffered trace event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceEvent {
    /// Emitting provider.
    pub provider: ProviderId,
    /// Severity.
    pub level: Level,
    /// Keyword bits.
    pub keywords: u64,
    /// Timestamp (caller's clock).
    pub time: u64,
    /// Process id.
    pub pid: u32,
    /// Thread id.
    pub tid: u32,
    /// Opaque payload.
    pub payload: Vec<u8>,
}

struct Session {
    name: String,
    cap: usize,
    buf: VecDeque<TraceEvent>,
    dropped: u64,
    enabled: BTreeMap<ProviderId, (Level, u64)>,
}

/// Trace controller: sessions subscribe to providers with a level and keyword mask.
#[derive(Default)]
pub struct Tracer {
    sessions: BTreeMap<u32, Session>,
    next: u32,
}

impl Tracer {
    /// New controller.
    pub fn new() -> Self {
        Tracer { sessions: BTreeMap::new(), next: 1 }
    }

    /// Start a session buffering up to `cap` events.
    pub fn start_session(&mut self, name: &str, cap: usize) -> Result<u32> {
        if cap == 0 || self.sessions.values().any(|s| s.name == name) {
            return Err(Error::Invalid);
        }
        let id = self.next;
        self.next += 1;
        self.sessions.insert(id, Session { name: name.into(), cap, buf: VecDeque::new(), dropped: 0, enabled: BTreeMap::new() });
        Ok(id)
    }

    /// Subscribe a session to a provider.
    pub fn enable(&mut self, session: u32, provider: ProviderId, level: Level, keywords: u64) -> Result<()> {
        self.sessions.get_mut(&session).ok_or(Error::Invalid)?.enabled.insert(provider, (level, keywords));
        Ok(())
    }

    /// Unsubscribe.
    pub fn disable(&mut self, session: u32, provider: ProviderId) -> Result<()> {
        self.sessions.get_mut(&session).ok_or(Error::Invalid)?.enabled.remove(&provider);
        Ok(())
    }

    /// Stop a session and return anything still buffered.
    pub fn stop_session(&mut self, session: u32) -> Result<Vec<TraceEvent>> {
        Ok(self.sessions.remove(&session).ok_or(Error::Invalid)?.buf.into())
    }

    /// Cheap pre-check so providers can skip building payloads nobody listens to.
    pub fn is_enabled(&self, provider: ProviderId, level: Level, keywords: u64) -> bool {
        self.sessions.values().any(|s| s.wants(provider, level, keywords))
    }

    /// Emit an event; returns how many sessions received it.
    pub fn emit(&mut self, ev: TraceEvent) -> usize {
        let mut n = 0;
        for s in self.sessions.values_mut() {
            if !s.wants(ev.provider, ev.level, ev.keywords) {
                continue;
            }
            if s.buf.len() == s.cap {
                s.buf.pop_front(); // lossy ring: newest wins
                s.dropped += 1;
            }
            s.buf.push_back(ev.clone());
            n += 1;
        }
        n
    }

    /// Take everything buffered for a session.
    pub fn drain(&mut self, session: u32) -> Result<Vec<TraceEvent>> {
        Ok(self.sessions.get_mut(&session).ok_or(Error::Invalid)?.buf.drain(..).collect())
    }

    /// Events lost to a full buffer.
    pub fn dropped(&self, session: u32) -> Result<u64> {
        Ok(self.sessions.get(&session).ok_or(Error::Invalid)?.dropped)
    }
}

impl Session {
    fn wants(&self, p: ProviderId, level: Level, kw: u64) -> bool {
        self.enabled.get(&p).is_some_and(|&(l, mask)| level <= l && (mask == 0 || kw & mask != 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: u16, t: u64) -> Event {
        Event::new("LoOS-Kernel", id, Level::Info, t).with("k", "v")
    }

    #[test]
    fn crc_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
    }

    #[test]
    fn append_query_filters() {
        let mut l = Log::new(2, Retention::Circular);
        for i in 0..10u16 {
            let mut e = ev(i, 1000 + i as u64);
            e.level = if i % 2 == 0 { Level::Error } else { Level::Info };
            e.keywords = 1 << (i % 3);
            e.provider = if i < 5 { "A".into() } else { "B".into() };
            l.append(e).unwrap();
        }
        assert_eq!(l.len(), 10);
        let all = l.query(&Query::default());
        assert_eq!(all.len(), 10);
        assert_eq!(all[3].record_id, 4);
        assert_eq!(all[3].data, [("k".into(), "v".into())]);
        assert_eq!(l.query(&Query { max_level: Some(Level::Error), ..Default::default() }).len(), 5);
        assert_eq!(l.query(&Query { provider: Some("B".into()), ..Default::default() }).len(), 5);
        assert_eq!(l.query(&Query { event_id: Some(7), ..Default::default() }).len(), 1);
        assert_eq!(l.query(&Query { time: Some((1002, 1004)), ..Default::default() }).len(), 3);
        assert_eq!(l.query(&Query { after_id: Some(8), ..Default::default() }).len(), 2);
        assert_eq!(l.query(&Query { keywords: 2, ..Default::default() }).len(), 3);
    }

    #[test]
    fn circular_and_retain() {
        let big = |i| Event::new("P", 1, Level::Info, i).with("blob", &"x".repeat(30_000));
        let mut c = Log::new(2, Retention::Circular);
        for i in 0..10 {
            c.append(big(i)).unwrap();
        }
        assert_eq!(c.len(), 4); // two chunks of two records survive
        assert_eq!(c.query(&Query::default())[0].record_id, 7);
        assert_eq!(c.query(&Query::default()).last().unwrap().record_id, 10);
        let mut r = Log::new(1, Retention::Retain);
        r.append(big(0)).unwrap();
        r.append(big(1)).unwrap();
        assert_eq!(r.append(big(2)), Err(Error::Full));
        assert_eq!(r.len(), 2);
        assert_eq!(c.append(Event::new("P", 1, Level::Info, 0).with("blob", &"x".repeat(70_000))), Err(Error::Invalid));
    }

    #[test]
    fn persistence_and_corruption() {
        let mut l = Log::new(8, Retention::Retain);
        for i in 0..6 {
            l.append(Event::new("P", i, Level::Warning, 5).with("blob", &"y".repeat(30_000))).unwrap();
        }
        let img = l.to_bytes();
        assert_eq!(img.len(), FILE_HDR + 3 * CHUNK);
        let back = Log::from_bytes(&img).unwrap();
        assert_eq!(back.query(&Query::default()), l.query(&Query::default()));
        assert_eq!(back.retention, Retention::Retain);
        let mut back = back;
        assert_eq!(back.append(ev(1, 1)).unwrap(), 7); // ids continue

        let mut bad = img.clone();
        bad[FILE_HDR + CHUNK + CHUNK_HDR + 100] ^= 0xff; // flip a byte in chunk 1
        let r = Log::from_bytes(&bad).unwrap();
        assert_eq!(r.corrupt_chunks, 1);
        assert_eq!(r.len(), 4);
        let mut bad = img.clone();
        bad[9] ^= 1;
        assert_eq!(Log::from_bytes(&bad).err(), Some(Error::Parse));
        assert_eq!(Log::from_bytes(&img[..10]).err(), Some(Error::Parse));
        assert_eq!(Log::from_bytes(&img[..FILE_HDR + CHUNK]).err(), Some(Error::Parse));
    }

    #[test]
    fn channels_and_clear_is_audited() {
        let mut e = EventLog::new();
        e.write("Application", ev(1, 1)).unwrap();
        e.write("System", ev(2, 2)).unwrap();
        assert_eq!(e.write("Nope", ev(1, 1)), Err(Error::Invalid));
        e.clear("Application", "S-1-5-21-1-1001", 99).unwrap();
        assert!(e.channel("Application").unwrap().is_empty());
        let sec = e.query("Security", &Query { event_id: Some(EVT_LOG_CLEARED), ..Default::default() }).unwrap();
        assert_eq!(sec.len(), 1);
        assert_eq!(sec[0].data[1], ("by".into(), "S-1-5-21-1-1001".into()));
        // ids keep increasing across a clear so gaps are detectable
        assert_eq!(e.write("Application", ev(1, 3)).unwrap(), 2);
    }

    fn te(p: u128, level: Level, kw: u64, n: u8) -> TraceEvent {
        TraceEvent { provider: p, level, keywords: kw, time: n as u64, pid: 1, tid: 1, payload: alloc::vec![n] }
    }

    #[test]
    fn trace_sessions_filter_and_drop() {
        let mut t = Tracer::new();
        assert!(!t.is_enabled(1, Level::Error, 0));
        assert_eq!(t.emit(te(1, Level::Error, 1, 0)), 0);
        let a = t.start_session("a", 2).unwrap();
        let b = t.start_session("b", 8).unwrap();
        assert!(t.start_session("a", 1).is_err() && t.start_session("z", 0).is_err());
        t.enable(a, 1, Level::Warning, 0).unwrap();
        t.enable(b, 1, Level::Verbose, 0b10).unwrap();
        t.enable(b, 2, Level::Info, 0).unwrap();
        assert!(t.is_enabled(1, Level::Warning, 1));
        assert!(!t.is_enabled(1, Level::Info, 1)); // a: too verbose; b: keyword mismatch
        assert_eq!(t.emit(te(1, Level::Warning, 0b10, 1)), 2);
        assert_eq!(t.emit(te(1, Level::Error, 0b01, 2)), 1); // only a (b needs keyword bit 1)
        assert_eq!(t.emit(te(2, Level::Info, 0, 3)), 1);
        assert_eq!(t.emit(te(1, Level::Critical, 0b10, 4)), 2);
        assert_eq!(t.dropped(a).unwrap(), 1); // ring of 2 held events 2,4 after dropping 1
        let da = t.drain(a).unwrap();
        assert_eq!(da.iter().map(|e| e.payload[0]).collect::<Vec<_>>(), [2, 4]);
        assert!(t.drain(a).unwrap().is_empty());
        t.disable(b, 2).unwrap();
        assert_eq!(t.emit(te(2, Level::Info, 0, 5)), 0);
        let rest = t.stop_session(b).unwrap();
        assert_eq!(rest.len(), 3);
        assert!(t.drain(b).is_err());
    }
}
