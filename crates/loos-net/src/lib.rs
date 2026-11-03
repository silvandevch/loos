//! LoOS network stack, written from the RFCs (791, 792, 793, 826, 4443, 4291, 4861, 768).
//!
//! Pure safe `no_std` code with no I/O of its own: frames go in through [`stack::Stack::rx`] and come
//! out of [`stack::Stack::pop_tx`], so the NIC drivers in `loos-drivers` plug in unchanged and the whole
//! stack is exercised on the host by wiring two `Stack`s together.
//!
//! Scope: single-host stack (no IP forwarding, no fragmentation/reassembly, no IPv6 extension headers,
//! no TCP options beyond a fixed MSS, no SACK/window scaling). Fragments and extension headers are dropped.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod route;
pub mod stack;
pub mod tcp;
pub mod wire;
