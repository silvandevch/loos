//! Validation for subvolume identities and parent-directory names.
//!
//! The root-tree and filesystem-tree item writes are performed by the mounted
//! COW writer. Keeping validation here gives every writer the same bounded
//! rules before it allocates or publishes any metadata.

use crate::consts::FIRST_FREE_OBJECTID;
use crate::error::{Error, Result};

/// Inputs needed to create a nested subvolume entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubvolumeRequest<'a> {
    /// New subvolume object id.
    pub id: u64,
    /// Existing parent subvolume object id.
    pub parent_id: u64,
    /// Parent directory inode containing the new entry.
    pub dirid: u64,
    /// Byte name of the new directory entry.
    pub name: &'a [u8],
    /// New subvolume UUID.
    pub uuid: [u8; 16],
}

/// Validated identity for a subvolume item and its parent entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubvolumeIdentity<'a> {
    /// New object id.
    pub id: u64,
    /// Parent subvolume id.
    pub parent_id: u64,
    /// Parent directory inode.
    pub dirid: u64,
    /// Name bytes, unchanged from the request.
    pub name: &'a [u8],
    /// New subvolume UUID.
    pub uuid: [u8; 16],
}

/// Validate a subvolume request before beginning a metadata transaction.
pub fn validate_subvolume(request: SubvolumeRequest<'_>) -> Result<SubvolumeIdentity<'_>> {
    if request.id < FIRST_FREE_OBJECTID || request.parent_id == 0 ||
        request.dirid == 0 || request.id == request.parent_id {
        return Err(Error::Invalid("subvolume identity"));
    }
    if request.name.is_empty() || request.name.len() > 255 ||
        request.name.iter().any(|&b| b == 0 || b == b'/') {
        return Err(Error::Invalid("subvolume name"));
    }
    if request.uuid == [0; 16] {
        return Err(Error::Invalid("subvolume uuid"));
    }
    Ok(SubvolumeIdentity {
        id: request.id,
        parent_id: request.parent_id,
        dirid: request.dirid,
        name: request.name,
        uuid: request.uuid,
    })
}
