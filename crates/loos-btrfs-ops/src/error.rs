//! Error type shared by the btrfs operations layer.
use crate::dev::DevError;

/// Errors from volume, tree, filesystem and tool operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A device read/write failed.
    Io,
    /// Write attempted on a read-only device or mount.
    ReadOnly,
    /// No valid superblock / not btrfs.
    NotBtrfs,
    /// A checksum did not match.
    Csum,
    /// Structure is inconsistent.
    Corrupt(&'static str),
    /// Item/path/inode does not exist.
    NotFound,
    /// Path component is not a directory.
    NotDir,
    /// Directory operation on a file.
    IsDir,
    /// Valid but not implemented (e.g. an unknown compression type).
    Unsupported(&'static str),
    /// A member device is absent and the data cannot be rebuilt.
    MissingDevice(u64),
    /// Argument out of range or malformed.
    Invalid(&'static str),
    /// Name already exists.
    Exists,
    /// Not enough space.
    NoSpace,
    /// Directory not empty.
    NotEmpty,
    /// Permission denied.
    Access,
    /// Operation would exceed a quota.
    QuotaExceeded,
}

impl From<DevError> for Error {
    fn from(e: DevError) -> Self {
        match e {
            DevError::Io => Error::Io,
            DevError::ReadOnly => Error::ReadOnly,
        }
    }
}

/// Result alias.
pub type Result<T> = core::result::Result<T, Error>;

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Corrupt(w) | Error::Unsupported(w) | Error::Invalid(w) => write!(f, "{:?}: {}", self, w),
            Error::MissingDevice(d) => write!(f, "missing device {}", d),
            other => write!(f, "{:?}", other),
        }
    }
}
