//! Swap-file validation and extent mapping (what `btrfs_swap_activate` enforces).
//!
//! A btrfs swap file must be a regular, non-COW (`nodatacow`), uncompressed file whose every page is
//! backed by an allocated, unshared extent in a `single`-profile block group on a single-device
//! filesystem. The result is the list of contiguous physical runs the swap subsystem will use.
use crate::consts::*;
use crate::dev::Dev;
use crate::extents::inventory;
use crate::fs::{ExtentKind, Fs, Kind, Node};
use crate::raid::Profile;
use alloc::vec::Vec;

/// Why a file cannot be a swap file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapError {
    /// Not a regular file.
    NotRegular,
    /// The file is copy-on-write (missing `nodatacow`).
    Cow,
    /// An extent is compressed (or the inode forces compression).
    Compressed,
    /// A hole or unallocated range exists inside the file.
    Hole,
    /// An extent is shared with another file/snapshot (reflink/snapshot).
    Shared,
    /// The extent lives in a DUP/RAID block group.
    UnsupportedProfile,
    /// The filesystem has more than one device.
    MultiDevice,
    /// The file size or an extent is not page aligned, or the file is empty.
    Unaligned,
    /// An inline extent (data stored in the leaf).
    Inline,
    /// The filesystem could not be read.
    Io,
}

/// A contiguous run of swap pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwapExtent {
    /// First page of the file.
    pub start_page: u64,
    /// Number of pages.
    pub pages: u64,
    /// Physical start in 512-byte sectors on the device.
    pub sector: u64,
}

/// Validated swap file layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapMap {
    /// Device holding the file.
    pub devid: u64,
    /// Page size used (4096).
    pub page_size: u64,
    /// Total pages.
    pub pages: u64,
    /// Contiguous runs, merged.
    pub extents: Vec<SwapExtent>,
}

/// Validates `node` as a swap file and returns its physical layout.
pub fn map_swapfile<D: Dev>(fs: &mut Fs<D>, node: Node) -> Result<SwapMap, SwapError> {
    const PAGE: u64 = 4096;
    let st = fs.stat(node).map_err(|_| SwapError::Io)?;
    if st.kind() != Kind::File {
        return Err(SwapError::NotRegular);
    }
    if st.flags & INODE_NODATACOW == 0 {
        return Err(SwapError::Cow);
    }
    if st.flags & INODE_COMPRESS != 0 {
        return Err(SwapError::Compressed);
    }
    if fs.vol.devids().len() != 1 || fs.vol.sb.num_devices != 1 {
        return Err(SwapError::MultiDevice);
    }
    if st.size == 0 || st.size % PAGE != 0 {
        return Err(SwapError::Unaligned);
    }
    let inv = inventory(&mut fs.vol).map_err(|_| SwapError::Io)?;
    let exts = fs.extents(node, 0, st.size - 1).map_err(|_| SwapError::Io)?;
    let mut pos = 0u64;
    let mut out: Vec<SwapExtent> = Vec::new();
    let mut devid = 0u64;
    for e in exts {
        match &e.kind {
            ExtentKind::Inline { .. } => return Err(SwapError::Inline),
            ExtentKind::Regular { compression, disk_bytenr, offset, num_bytes, prealloc: _, .. } => {
                if *compression != COMPRESS_NONE {
                    return Err(SwapError::Compressed);
                }
                if e.file_offset != pos || *disk_bytenr == 0 {
                    return Err(SwapError::Hole);
                }
                if e.file_offset % PAGE != 0 || num_bytes % PAGE != 0 || (disk_bytenr + offset) % PAGE != 0 {
                    return Err(SwapError::Unaligned);
                }
                // sharing: the extent item must have exactly one reference
                let idx = inv.extents.partition_point(|x| x.start < *disk_bytenr);
                match inv.extents.get(idx) {
                    Some(x) if x.start == *disk_bytenr && x.refs == 1 => {}
                    Some(x) if x.start == *disk_bytenr => return Err(SwapError::Shared),
                    _ => return Err(SwapError::Io),
                }
                let logical = disk_bytenr + offset;
                let mut done = 0u64;
                while done < *num_bytes {
                    let ch = fs.vol.chunk_for(logical + done).ok_or(SwapError::Io)?;
                    if ch.profile() != Some(Profile::Single) {
                        return Err(SwapError::UnsupportedProfile);
                    }
                    let m = ch.map(logical + done).map_err(|_| SwapError::Io)?;
                    let n = m.len.min(*num_bytes - done);
                    devid = m.copies[0].devid;
                    let sector = m.copies[0].physical / 512;
                    let start_page = (pos + done) / PAGE;
                    let pages = n / PAGE;
                    match out.last_mut() {
                        Some(l) if l.start_page + l.pages == start_page && l.sector + l.pages * (PAGE / 512) == sector => l.pages += pages,
                        _ => out.push(SwapExtent { start_page, pages, sector }),
                    }
                    done += n;
                }
                pos += num_bytes;
            }
        }
    }
    if pos < st.size {
        return Err(SwapError::Hole);
    }
    Ok(SwapMap { devid, page_size: PAGE, pages: st.size / PAGE, extents: out })
}
