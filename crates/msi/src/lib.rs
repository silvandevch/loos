//! MSI installer engine, read side (task #172): [`cfb`] OLE compound file reader,
//! [`database`] MSI table decoding (string pool, `_Columns`, row data), [`plan`] directory and
//! file resolution, property substitution and a custom-action audit.
//!
//! The engine never runs custom actions itself; [`plan::Plan::custom_actions`] classifies them
//! so the caller can deny, sandbox, or reimplement the common ones.
#![no_std]

extern crate alloc;

pub mod cfb;
pub mod database;
pub mod plan;

pub use database::{Cell, Database, Table};
