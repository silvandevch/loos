//! Decompression of btrfs extents: zlib (miniz_oxide) and LZO1X (btrfs framing).
use crate::consts::*;
use crate::error::{Error, Result};
use alloc::vec;
use alloc::vec::Vec;

/// Decompresses one extent to exactly `ram_bytes` bytes (zero-padded if the stream ends early, as the
/// kernel does for the tail of a compressed extent).
pub fn decompress(kind: u8, data: &[u8], ram_bytes: usize, sectorsize: usize) -> Result<Vec<u8>> {
    let mut out = match kind {
        COMPRESS_ZLIB => miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, ram_bytes).map_err(|_| Error::Corrupt("zlib stream"))?,
        COMPRESS_ZSTD => return Err(Error::Unsupported("zstd decoder")),
        COMPRESS_LZO => lzo_extent(data, ram_bytes, sectorsize)?,
        _ => return Err(Error::Unsupported("compression type")),
    };
    if out.len() > ram_bytes {
        return Err(Error::Corrupt("decompressed size"));
    }
    out.resize(ram_bytes, 0);
    Ok(out)
}

/// btrfs LZO framing: `u32 total_len`, then segments `u32 seg_len` + LZO1X block, with segment
/// headers never straddling a sector boundary.
fn lzo_extent(data: &[u8], ram_bytes: usize, sectorsize: usize) -> Result<Vec<u8>> {
    if data.len() < 4 {
        return Err(Error::Corrupt("lzo header"));
    }
    let total = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if total > data.len() || total < 4 {
        return Err(Error::Corrupt("lzo total length"));
    }
    let mut pos = 4usize;
    let mut out = Vec::with_capacity(ram_bytes);
    while pos < total && out.len() < ram_bytes {
        let in_sector = sectorsize - (pos % sectorsize);
        if in_sector < 4 {
            pos += in_sector; // header would straddle: skip padding
            continue;
        }
        if pos + 4 > total {
            return Err(Error::Corrupt("lzo segment header"));
        }
        let seg = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if seg == 0 || pos + seg > total {
            return Err(Error::Corrupt("lzo segment length"));
        }
        let want = core::cmp::min(sectorsize, ram_bytes - out.len());
        let mut buf = vec![0u8; sectorsize + 64];
        let n = lzo1x_decompress(&data[pos..pos + seg], &mut buf)?;
        let last = pos + seg >= total;
        if n < want && !last {
            return Err(Error::Corrupt("lzo short segment"));
        }
        // the final segment may stop at EOF while ram_bytes is rounded up to a sector
        out.extend_from_slice(&buf[..core::cmp::min(n, want)]);
        pos += seg;
    }
    Ok(out)
}

/// LZO1X-1 block decompression with full bounds checking. Returns the output length.
pub fn lzo1x_decompress(src: &[u8], dst: &mut [u8]) -> Result<usize> {
    #[derive(Clone, Copy)]
    enum St {
        Top,
        FirstLiteralRun,
        Match(usize),
        MatchNext(usize),
    }
    let bad = || Error::Corrupt("lzo stream");
    let mut ip = 0usize;
    let mut op = 0usize;
    macro_rules! rd {
        () => {{
            let b = *src.get(ip).ok_or_else(bad)?;
            ip += 1;
            b
        }};
    }
    macro_rules! copy_lit {
        ($n:expr) => {{
            let n = $n;
            if ip + n > src.len() || op + n > dst.len() {
                return Err(bad());
            }
            dst[op..op + n].copy_from_slice(&src[ip..ip + n]);
            ip += n;
            op += n;
        }};
    }
    macro_rules! copy_match {
        ($dist:expr, $n:expr) => {{
            let dist = $dist;
            let n = $n;
            if dist == 0 || dist > op || op + n > dst.len() {
                return Err(bad());
            }
            for _ in 0..n {
                dst[op] = dst[op - dist];
                op += 1;
            }
        }};
    }
    let mut st;
    if src.is_empty() {
        return Err(bad());
    }
    if src[0] > 17 {
        let t = rd!() as usize - 17;
        if t < 4 {
            st = St::MatchNext(t);
        } else {
            copy_lit!(t);
            st = St::FirstLiteralRun;
        }
    } else {
        st = St::Top;
    }
    loop {
        match st {
            St::Top => {
                let mut t = rd!() as usize;
                if t >= 16 {
                    st = St::Match(t);
                    continue;
                }
                if t == 0 {
                    while *src.get(ip).ok_or_else(bad)? == 0 {
                        t += 255;
                        ip += 1;
                    }
                    t += 15 + rd!() as usize;
                }
                copy_lit!(t + 3);
                st = St::FirstLiteralRun;
            }
            St::FirstLiteralRun => {
                let t = rd!() as usize;
                if t >= 16 {
                    st = St::Match(t);
                    continue;
                }
                // short match of 3 bytes after a literal run
                let dist = 1 + 0x0800 + (t >> 2) + ((rd!() as usize) << 2);
                copy_match!(dist, 3);
                // match_done
                let n = src[ip - 2] as usize & 3;
                if n == 0 {
                    st = St::Top;
                } else {
                    st = St::MatchNext(n);
                }
            }
            St::MatchNext(n) => {
                copy_lit!(n);
                let t = rd!() as usize;
                st = St::Match(t);
            }
            St::Match(mut t) => {
                let (dist, len);
                if t >= 64 {
                    // M2
                    dist = 1 + ((t >> 2) & 7) + ((rd!() as usize) << 3);
                    len = (t >> 5) - 1 + 2;
                } else if t >= 32 {
                    // M3
                    t &= 31;
                    if t == 0 {
                        while *src.get(ip).ok_or_else(bad)? == 0 {
                            t += 255;
                            ip += 1;
                        }
                        t += 31 + rd!() as usize;
                    }
                    let a = rd!() as usize;
                    let b = rd!() as usize;
                    dist = 1 + (a >> 2) + (b << 6);
                    len = t + 2;
                } else if t >= 16 {
                    // M4
                    let high = (t & 8) << 11;
                    t &= 7;
                    if t == 0 {
                        while *src.get(ip).ok_or_else(bad)? == 0 {
                            t += 255;
                            ip += 1;
                        }
                        t += 7 + rd!() as usize;
                    }
                    let a = rd!() as usize;
                    let b = rd!() as usize;
                    let d = (a >> 2) + (b << 6);
                    if high == 0 && d == 0 {
                        // end-of-stream marker
                        return Ok(op);
                    }
                    dist = 0x4000 + high + d;
                    len = t + 2;
                } else {
                    // M1 (2 byte match after a literal run of >= 4)
                    dist = 1 + (t >> 2) + ((rd!() as usize) << 2);
                    copy_match!(dist, 2);
                    let n = src[ip - 2] as usize & 3;
                    st = if n == 0 { St::Top } else { St::MatchNext(n) };
                    continue;
                }
                copy_match!(dist, len);
                let n = src[ip - 2] as usize & 3;
                st = if n == 0 { St::Top } else { St::MatchNext(n) };
            }
        }
    }
}
