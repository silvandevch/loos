//! Dump file format.
//!
//! ```text
//! header   (112 bytes, little endian, CRC32 over the first 108)
//! section* tag u32 | len u32 | crc32(payload) u32 | payload[len]
//! END      tag=0xFFFF, len=8: total_len u32 (incl. END) | section_count u32
//! ```
//! The trailer-style END section lets the writer stream into a non-seekable sink while the
//! bugcheck path holds no allocator. The reader verifies every checksum.

use alloc::string::String;
use alloc::vec::Vec;

use crate::bugcheck::BugCheck;

/// File magic.
pub const MAGIC: [u8; 8] = *b"LOOSDUMP";
/// Format version.
pub const VERSION: u32 = 1;
/// Header size in bytes.
pub const HEADER_LEN: usize = 112;

/// Section tags.
pub mod tag {
    /// CPU register context ([`super::Context`]).
    pub const CONTEXT: u32 = 1;
    /// Stack trace: array of u64 return addresses.
    pub const STACK: u32 = 2;
    /// Raw memory range: base u64 + bytes.
    pub const MEMORY: u32 = 3;
    /// Loaded module: base u64, size u64, name bytes.
    pub const MODULE: u32 = 4;
    /// Thread: tid u64, state u32, pad u32, rip u64, rsp u64.
    pub const THREAD: u32 = 5;
    /// Tail of the kernel log (UTF-8).
    pub const LOG: u32 = 6;
    /// End marker.
    pub const END: u32 = 0xFFFF;
}

/// Errors from writing or reading a dump.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DumpError {
    /// Sink is full.
    NoSpace,
    /// Bad magic or unsupported version.
    BadHeader,
    /// Checksum mismatch.
    BadChecksum,
    /// Truncated or inconsistent structure.
    Truncated,
}

/// CRC-32 (IEEE, reflected), bitwise: small and fine for crash-time use.
pub fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    crc = !crc;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Destination for dump bytes (disk dump area, serial, memory).
pub trait Sink {
    /// Append `bytes`, or fail with [`DumpError::NoSpace`].
    fn put(&mut self, bytes: &[u8]) -> Result<(), DumpError>;
}

/// Sink over a fixed slice.
pub struct SliceSink<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl<'a> SliceSink<'a> {
    /// Wrap `buf`.
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, len: 0 }
    }
    /// Bytes written so far.
    pub fn written(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl Sink for SliceSink<'_> {
    fn put(&mut self, bytes: &[u8]) -> Result<(), DumpError> {
        let end = self.len.checked_add(bytes.len()).ok_or(DumpError::NoSpace)?;
        if end > self.buf.len() {
            return Err(DumpError::NoSpace);
        }
        self.buf[self.len..end].copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }
}

impl Sink for Vec<u8> {
    fn put(&mut self, bytes: &[u8]) -> Result<(), DumpError> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}

/// x86-64 register context captured at the fault.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    /// rax, rcx, rdx, rbx, rsp, rbp, rsi, rdi, r8..r15 in that order.
    pub gpr: [u64; 16],
    /// Instruction pointer.
    pub rip: u64,
    /// RFLAGS.
    pub rflags: u64,
    /// CR0, CR2, CR3, CR4.
    pub cr: [u64; 4],
    /// CS, SS selectors.
    pub cs: u16,
    /// SS selector.
    pub ss: u16,
}

/// Index of rsp in [`Context::gpr`].
pub const GPR_RSP: usize = 4;
/// Index of rbp in [`Context::gpr`].
pub const GPR_RBP: usize = 5;

impl Context {
    const LEN: usize = 16 * 8 + 8 + 8 + 32 + 4;

    fn encode(&self) -> [u8; Self::LEN] {
        let mut o = [0u8; Self::LEN];
        let mut p = 0;
        for v in self.gpr.iter().chain([self.rip, self.rflags].iter()).chain(self.cr.iter()) {
            o[p..p + 8].copy_from_slice(&v.to_le_bytes());
            p += 8;
        }
        o[p..p + 2].copy_from_slice(&self.cs.to_le_bytes());
        o[p + 2..p + 4].copy_from_slice(&self.ss.to_le_bytes());
        o
    }

    fn decode(b: &[u8]) -> Result<Self, DumpError> {
        if b.len() != Self::LEN {
            return Err(DumpError::Truncated);
        }
        let q = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
        let mut c = Context::default();
        for i in 0..16 {
            c.gpr[i] = q(i);
        }
        c.rip = q(16);
        c.rflags = q(17);
        for i in 0..4 {
            c.cr[i] = q(18 + i);
        }
        c.cs = u16::from_le_bytes([b[176], b[177]]);
        c.ss = u16::from_le_bytes([b[178], b[179]]);
        Ok(c)
    }

    /// Stack pointer.
    pub fn rsp(&self) -> u64 {
        self.gpr[GPR_RSP]
    }
    /// Frame pointer.
    pub fn rbp(&self) -> u64 {
        self.gpr[GPR_RBP]
    }
}

/// Parsed dump header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// The bugcheck.
    pub bugcheck: BugCheck,
    /// Monotonic time of the crash, ns.
    pub time_ns: u64,
    /// CPU that crashed.
    pub cpu: u32,
    /// Number of online CPUs.
    pub ncpus: u32,
    /// Kernel build id (e.g. truncated commit hash).
    pub build_id: u64,
}

/// Streaming dump writer. All methods are allocation-free.
pub struct DumpWriter<S: Sink> {
    sink: S,
    total: u32,
    sections: u32,
}

impl<S: Sink> DumpWriter<S> {
    /// Write the header and start a dump.
    pub fn begin(mut sink: S, h: &Header) -> Result<Self, DumpError> {
        let mut b = [0u8; HEADER_LEN];
        b[0..8].copy_from_slice(&MAGIC);
        b[8..12].copy_from_slice(&VERSION.to_le_bytes());
        b[12..16].copy_from_slice(&h.bugcheck.code.to_le_bytes());
        for (i, p) in h.bugcheck.params.iter().enumerate() {
            b[16 + i * 8..24 + i * 8].copy_from_slice(&p.to_le_bytes());
        }
        b[48..56].copy_from_slice(&h.time_ns.to_le_bytes());
        b[56..60].copy_from_slice(&h.cpu.to_le_bytes());
        b[60..64].copy_from_slice(&h.ncpus.to_le_bytes());
        b[64..72].copy_from_slice(&h.build_id.to_le_bytes());
        let crc = crc32_update(0, &b[..HEADER_LEN - 4]);
        b[HEADER_LEN - 4..].copy_from_slice(&crc.to_le_bytes());
        sink.put(&b)?;
        Ok(Self { sink, total: HEADER_LEN as u32, sections: 0 })
    }

    fn section(&mut self, tag: u32, parts: &[&[u8]]) -> Result<(), DumpError> {
        let len: usize = parts.iter().map(|p| p.len()).sum();
        let mut crc = 0;
        for p in parts {
            crc = crc32_update(crc, p);
        }
        let mut h = [0u8; 12];
        h[0..4].copy_from_slice(&tag.to_le_bytes());
        h[4..8].copy_from_slice(&(len as u32).to_le_bytes());
        h[8..12].copy_from_slice(&crc.to_le_bytes());
        self.sink.put(&h)?;
        for p in parts {
            self.sink.put(p)?;
        }
        self.total = self.total.wrapping_add(12 + len as u32);
        self.sections += 1;
        Ok(())
    }

    /// Add the register context.
    pub fn context(&mut self, c: &Context) -> Result<(), DumpError> {
        self.section(tag::CONTEXT, &[&c.encode()])
    }

    /// Add a stack trace.
    pub fn stack(&mut self, frames: &[u64]) -> Result<(), DumpError> {
        let mut v = [0u8; 8];
        let mut crc = 0;
        for f in frames {
            v.copy_from_slice(&f.to_le_bytes());
            crc = crc32_update(crc, &v);
        }
        let mut h = [0u8; 12];
        h[0..4].copy_from_slice(&tag::STACK.to_le_bytes());
        h[4..8].copy_from_slice(&((frames.len() * 8) as u32).to_le_bytes());
        h[8..12].copy_from_slice(&crc.to_le_bytes());
        self.sink.put(&h)?;
        for f in frames {
            self.sink.put(&f.to_le_bytes())?;
        }
        self.total = self.total.wrapping_add(12 + (frames.len() * 8) as u32);
        self.sections += 1;
        Ok(())
    }

    /// Add a raw memory range captured from virtual address `base`.
    pub fn memory(&mut self, base: u64, data: &[u8]) -> Result<(), DumpError> {
        self.section(tag::MEMORY, &[&base.to_le_bytes(), data])
    }

    /// Add a loaded module.
    pub fn module(&mut self, base: u64, size: u64, name: &str) -> Result<(), DumpError> {
        self.section(tag::MODULE, &[&base.to_le_bytes(), &size.to_le_bytes(), name.as_bytes()])
    }

    /// Add a thread record.
    pub fn thread(&mut self, tid: u64, state: u32, rip: u64, rsp: u64) -> Result<(), DumpError> {
        self.section(
            tag::THREAD,
            &[&tid.to_le_bytes(), &state.to_le_bytes(), &0u32.to_le_bytes(), &rip.to_le_bytes(), &rsp.to_le_bytes()],
        )
    }

    /// Add the kernel log tail.
    pub fn log(&mut self, text: &[u8]) -> Result<(), DumpError> {
        self.section(tag::LOG, &[text])
    }

    /// Write the END trailer and return the sink and total dump size.
    pub fn finish(mut self) -> Result<(S, u32), DumpError> {
        let total = self.total.wrapping_add(12 + 8);
        let count = self.sections;
        let mut p = [0u8; 8];
        p[0..4].copy_from_slice(&total.to_le_bytes());
        p[4..8].copy_from_slice(&count.to_le_bytes());
        let crc = crc32_update(0, &p);
        let mut h = [0u8; 12];
        h[0..4].copy_from_slice(&tag::END.to_le_bytes());
        h[4..8].copy_from_slice(&8u32.to_le_bytes());
        h[8..12].copy_from_slice(&crc.to_le_bytes());
        self.sink.put(&h)?;
        self.sink.put(&p)?;
        Ok((self.sink, total))
    }
}

/// A verified module record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    /// Load address.
    pub base: u64,
    /// Image size.
    pub size: u64,
    /// Image name.
    pub name: String,
}

/// A thread record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thread {
    /// Thread id.
    pub tid: u64,
    /// Scheduler state.
    pub state: u32,
    /// Instruction pointer.
    pub rip: u64,
    /// Stack pointer.
    pub rsp: u64,
}

/// A fully verified, parsed dump.
#[derive(Clone, Debug)]
pub struct DumpReader {
    /// Header.
    pub header: Header,
    /// Register context, if present.
    pub context: Option<Context>,
    /// Stack trace.
    pub stack: Vec<u64>,
    /// Memory ranges `(base, bytes)`.
    pub memory: Vec<(u64, Vec<u8>)>,
    /// Loaded modules.
    pub modules: Vec<Module>,
    /// Threads.
    pub threads: Vec<Thread>,
    /// Log tail.
    pub log: String,
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64le(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

impl DumpReader {
    /// Parse and verify `data` (header CRC, every section CRC, trailer length and count).
    pub fn parse(data: &[u8]) -> Result<Self, DumpError> {
        if data.len() < HEADER_LEN {
            return Err(DumpError::Truncated);
        }
        if data[0..8] != MAGIC || u32le(data, 8) != VERSION {
            return Err(DumpError::BadHeader);
        }
        if crc32_update(0, &data[..HEADER_LEN - 4]) != u32le(data, HEADER_LEN - 4) {
            return Err(DumpError::BadChecksum);
        }
        let mut params = [0u64; 4];
        for (i, p) in params.iter_mut().enumerate() {
            *p = u64le(data, 16 + i * 8);
        }
        let mut r = DumpReader {
            header: Header {
                bugcheck: BugCheck::new(u32le(data, 12), params),
                time_ns: u64le(data, 48),
                cpu: u32le(data, 56),
                ncpus: u32le(data, 60),
                build_id: u64le(data, 64),
            },
            context: None,
            stack: Vec::new(),
            memory: Vec::new(),
            modules: Vec::new(),
            threads: Vec::new(),
            log: String::new(),
        };
        let mut pos = HEADER_LEN;
        let mut count = 0u32;
        loop {
            if pos + 12 > data.len() {
                return Err(DumpError::Truncated);
            }
            let t = u32le(data, pos);
            let len = u32le(data, pos + 4) as usize;
            let crc = u32le(data, pos + 8);
            let start = pos + 12;
            let end = start.checked_add(len).ok_or(DumpError::Truncated)?;
            if end > data.len() {
                return Err(DumpError::Truncated);
            }
            let p = &data[start..end];
            if crc32_update(0, p) != crc {
                return Err(DumpError::BadChecksum);
            }
            if t == tag::END {
                if len != 8 || u32le(p, 0) as usize != end || u32le(p, 4) != count {
                    return Err(DumpError::Truncated);
                }
                return Ok(r);
            }
            match t {
                tag::CONTEXT => r.context = Some(Context::decode(p)?),
                tag::STACK => {
                    if len % 8 != 0 {
                        return Err(DumpError::Truncated);
                    }
                    r.stack = p.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
                }
                tag::MEMORY => {
                    if len < 8 {
                        return Err(DumpError::Truncated);
                    }
                    r.memory.push((u64le(p, 0), p[8..].to_vec()));
                }
                tag::MODULE => {
                    if len < 16 {
                        return Err(DumpError::Truncated);
                    }
                    r.modules.push(Module {
                        base: u64le(p, 0),
                        size: u64le(p, 8),
                        name: String::from_utf8_lossy(&p[16..]).into_owned(),
                    });
                }
                tag::THREAD => {
                    if len != 32 {
                        return Err(DumpError::Truncated);
                    }
                    r.threads.push(Thread { tid: u64le(p, 0), state: u32le(p, 8), rip: u64le(p, 16), rsp: u64le(p, 24) });
                }
                tag::LOG => r.log = String::from_utf8_lossy(p).into_owned(),
                _ => {} // forward compatible: unknown sections are CRC-checked and skipped
            }
            count += 1;
            pos = end;
        }
    }

    /// Resolve `addr` to `(module name, offset)`.
    pub fn resolve(&self, addr: u64) -> Option<(&str, u64)> {
        self.modules.iter().find(|m| addr >= m.base && addr - m.base < m.size).map(|m| (m.name.as_str(), addr - m.base))
    }
}
