//! winget support (task #170): [`yaml`] subset parser, [`manifest`] model + installer
//! selection, [`index`] search/version lookup. Parsing only; downloads and hash verification
//! happen in the package manager, which refuses any installer whose SHA-256 does not match.
#![no_std]

extern crate alloc;

pub mod index;
pub mod manifest;
pub mod yaml;
