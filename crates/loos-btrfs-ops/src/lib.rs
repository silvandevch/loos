//! btrfs operations layer on top of `loos-btrfs` (format parsing).
//!
//! * [`raid`] – logical→physical mapping for every profile + RAID5/6 parity math.
//! * [`consts`] – on-disk constants.
//! * [`dev`] – member-device abstraction.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
#[cfg(any(feature = "std", test))]
extern crate std;

pub mod balance;
pub mod check;
pub mod compress;
pub mod consts;
pub mod dedupe;
pub mod defrag;
pub mod dev;
pub mod error;
pub mod extents;
pub mod fs;
pub mod ioctl;
pub mod ioctl_abi;
pub mod ioctl_fs;
pub mod qgroup;
pub mod raid;
pub mod rescue;
pub mod resize;
pub mod restore;
pub mod scrub;
pub mod swapfile;
pub mod trim;
pub mod volume;
pub mod xattr;
