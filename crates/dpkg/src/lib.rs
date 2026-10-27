//! dpkg model for the Linux subsystem package manager (tasks #175, #174 parsing half).
//! * [`version`] – Debian version parsing and ordering (policy §5.6.12).
//! * [`control`] – RFC822-style stanza parser (status db, `Packages` indexes).
//! * [`deps`] – dependency expression parsing (`a | b (>= 1.0), c [amd64]`).
//! * [`resolver`] – install/remove planning against an installed set and a repository index.
#![no_std]

extern crate alloc;

pub mod control;
pub mod deps;
pub mod resolver;
pub mod version;
