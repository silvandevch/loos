//! Device resize: planning (grow/shrink, which chunks must move) and an **offline** applier that
//! patches the device item and the superblocks. Online shrink needs chunk relocation (the balance
//! engine) and is expressed here as the list of chunk stripes to relocate.
use crate::consts::*;
use crate::dev::Dev;
use crate::error::{Error, Result};
use crate::ioctl::{ResizeKind, ResizeSpec};
use crate::volume::*;
use alloc::vec;
use alloc::vec::Vec;
use loos_btrfs::{digest, ChecksumAlgorithm};

/// Smallest size a device may be shrunk to (the first MiB is reserved).
pub const MIN_DEVICE_BYTES: u64 = 1 << 20;

/// A chunk stripe that lies (partly) beyond the new device end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relocation {
    /// Logical start of the chunk.
    pub chunk: u64,
    /// Physical start of this device's stripe.
    pub physical: u64,
    /// Stripe length.
    pub length: u64,
}

/// Plan for resizing one device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResizePlan {
    /// Device.
    pub devid: u64,
    /// Current recorded size.
    pub old_size: u64,
    /// Target size (sector aligned).
    pub new_size: u64,
    /// Stripes that must be relocated before the device can shrink (empty for grow).
    pub relocate: Vec<Relocation>,
    /// Highest used byte offset of the device (end of its last dev extent).
    pub used_end: u64,
}

impl ResizePlan {
    /// True when the plan needs no data movement.
    pub fn is_direct(&self) -> bool {
        self.relocate.is_empty()
    }
}

/// Builds a resize plan. `device_len` is the real size of the member device (limit for grow/max).
pub fn plan_resize<D: Dev>(vol: &mut Volume<D>, spec: &ResizeSpec, device_len: u64) -> Result<ResizePlan> {
    let devid = spec.devid;
    let item = *vol.dev_items.get(&devid).ok_or(Error::NotFound)?;
    let ss = vol.sectorsize() as u64;
    let old = item.total_bytes;
    let target = match spec.kind {
        ResizeKind::Max => device_len,
        ResizeKind::Absolute(v) => v,
        ResizeKind::Grow(v) => old.checked_add(v).ok_or(Error::Invalid("resize overflow"))?,
        ResizeKind::Shrink(v) => old.checked_sub(v).ok_or(Error::Invalid("shrink below zero"))?,
    };
    let new_size = target / ss * ss;
    if new_size < MIN_DEVICE_BYTES {
        return Err(Error::Invalid("device would be smaller than 1 MiB"));
    }
    if new_size > device_len {
        return Err(Error::Invalid("new size exceeds the device"));
    }
    let mut used_end = 0u64;
    let mut relocate = Vec::new();
    for ch in vol.chunks() {
        let len = ch.stripe_dev_len();
        for s in ch.stripes.iter().filter(|s| s.devid == devid) {
            used_end = used_end.max(s.physical + len);
            if s.physical + len > new_size {
                relocate.push(Relocation { chunk: ch.logical, physical: s.physical, length: len });
            }
        }
    }
    // the superblock mirrors must stay inside the device too
    Ok(ResizePlan { devid, old_size: old, new_size, relocate, used_end })
}

fn reseal(raw: &mut [u8], alg: ChecksumAlgorithm) {
    let d = digest(alg, &raw[32..]);
    raw[..32].fill(0);
    raw[..d.as_bytes().len()].copy_from_slice(d.as_bytes());
}

/// Applies a direct plan (grow, or shrink with nothing to relocate) to an unmounted filesystem:
/// patches the `DEV_ITEM` in the chunk tree (all copies), then the superblocks of every device.
/// A crash between the two steps leaves a state `rescue::fix_device_size` / `check` can repair.
pub fn apply_offline<D: Dev>(vol: &mut Volume<D>, plan: &ResizePlan) -> Result<()> {
    if !plan.is_direct() {
        return Err(Error::Invalid("shrink needs relocation of chunks beyond the new end"));
    }
    if plan.new_size == plan.old_size {
        return Ok(());
    }
    let chunk_root = vol.sb.chunk_root;
    let key = Key::new(DEV_ITEMS_OBJECTID, DEV_ITEM_KEY, plan.devid);
    let (leaf, off, size) = vol.locate_item(chunk_root, key)?.ok_or(Error::NotFound)?;
    if size < 98 {
        return Err(Error::Corrupt("device item size"));
    }
    let new = plan.new_size;
    vol.patch_tree_block(leaf, &mut |b| put64(b, off + 8, new))?;
    // superblocks: fs-wide total_bytes on every device, dev_item.total_bytes on the resized one
    let delta = plan.new_size as i128 - plan.old_size as i128;
    let alg = vol.alg();
    for id in vol.devids() {
        let len = vol.dev_mut(id).map_or(0, |d| d.len());
        for off in Volume::<D>::super_offsets(len) {
            let mut raw = vec![0u8; SUPER_INFO_SIZE];
            {
                let d = vol.dev_mut(id).unwrap();
                if d.read_at(off, &mut raw).is_err() {
                    continue;
                }
            }
            let s = match Super::parse(&raw, off) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let total = (s.total_bytes as i128 + delta) as u64;
            put64(&mut raw, 112, total);
            if s.dev_item.devid == plan.devid {
                put64(&mut raw, 201 + 8, plan.new_size);
            }
            reseal(&mut raw, alg);
            vol.dev_mut(id).unwrap().write_at(off, &raw).map_err(Error::from)?;
        }
    }
    vol.flush()
}
