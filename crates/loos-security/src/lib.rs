//! LoOS security services. Pure `no_std` + `alloc`, `forbid(unsafe_code)`, host-tested.
//!
//! No telemetry anywhere: logs and traces stay on the machine.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod aes;
pub mod bignum;
pub mod codesign;
pub mod evlog;
pub mod fde;
pub mod hash;
pub mod logon;
pub mod tls;
pub mod uac;
pub mod x509;
#[cfg(test)]
mod testcerts;
pub mod x25519;

/// Errors shared across the crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Malformed or truncated input.
    Parse,
    /// A signature, MAC, tag or digest did not verify.
    BadSignature,
    /// Certificate chain not trusted / expired / not a CA.
    Untrusted,
    /// Unsupported algorithm or protocol feature.
    Unsupported,
    /// Policy denied the operation.
    Denied,
    /// Wrong credential / key.
    BadCredential,
    /// Caller passed an invalid argument.
    Invalid,
    /// Account locked out.
    LockedOut,
    /// Storage is full and the log is not circular.
    Full,
}

/// Result alias.
pub type Result<T> = core::result::Result<T, Error>;
