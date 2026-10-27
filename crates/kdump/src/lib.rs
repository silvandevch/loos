//! Kernel crash dump for LoOS (task #223).
//!
//! * [`bugcheck`] – stop codes (NT-compatible numbering where one exists) and names.
//! * [`format`] – the on-disk dump format: a streaming, allocation-free [`format::DumpWriter`]
//!   usable from the bugcheck path, and a verifying [`format::DumpReader`].
//! * [`stack`] – frame-pointer stack walk over a caller-supplied memory reader.
//! * [`report`] – human-readable crash report and the retention policy for stored dumps.
//!
//! Everything stays on the machine: there is no upload path and no network type anywhere in
//! this crate. Reports are rendered to text for the local user only.
#![no_std]

extern crate alloc;

pub mod bugcheck;
pub mod format;
pub mod report;
pub mod stack;

pub use bugcheck::BugCheck;
pub use format::{Context, DumpError, DumpReader, DumpWriter, Header, Sink, SliceSink};
