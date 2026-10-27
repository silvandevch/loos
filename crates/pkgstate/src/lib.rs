//! Package state shared by every package front-end (winget/MSI/MSIX/dpkg):
//! * [`trust`] – download policy: https only, host allowlist, hash required, publisher allowlist
//!   (tasks #176 + privacy gate).
//! * [`sha256`] – SHA-256 for verifying downloads.
//! * [`ownership`] – path → owning package(s), uninstall plans, repair reports (#177).
//! * [`journal`] – install transaction journal and rollback plan (#177).
//!
//! All pure logic; the package manager performs the I/O.
#![no_std]

extern crate alloc;

pub mod journal;
pub mod ownership;
pub mod sha256;
pub mod trust;
