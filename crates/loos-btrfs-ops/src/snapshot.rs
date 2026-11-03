//! Crash-safe snapshot metadata planning.
//!
//! A snapshot shares the source root block and therefore must be published as
//! a new root item in the same transaction as its parent-directory reference.
//! This module computes and validates that root-item state without touching a
//! device; the mounted writer consumes the plan when its COW publication path
//! is available.

use crate::consts::{ROOT_SUBVOL_RDONLY, ROOT_TREE_OBJECTID};
use crate::error::{Error, Result};
use crate::volume::RootItem;

/// Whether a newly created snapshot may be modified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotMode {
    /// Snapshot cannot be opened for mutation.
    ReadOnly,
    /// Snapshot may receive future COW mutations.
    Writable,
}

/// Validated input to [`plan_snapshot`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotRequest {
    /// Existing subvolume object id.
    pub source_id: u64,
    /// New subvolume object id, allocated from the free-objectid range.
    pub destination_id: u64,
    /// New transaction generation.
    pub generation: u64,
    /// New subvolume UUID supplied by the filesystem UUID allocator.
    pub uuid: [u8; 16],
    /// Resulting access mode.
    pub mode: SnapshotMode,
}

/// Root-item state to write in the root tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotPlan {
    /// Source root whose tree blocks are shared by the snapshot.
    pub source_id: u64,
    /// New root-tree object id.
    pub destination_id: u64,
    /// Root item fields for the new subvolume.
    pub root: RootItem,
}

/// Validate and prepare the root item for an atomic snapshot publication.
pub fn plan_snapshot(source: RootItem, request: SnapshotRequest) -> Result<SnapshotPlan> {
    if request.source_id == 0 || request.destination_id < ROOT_TREE_OBJECTID {
        return Err(Error::Invalid("snapshot object id"));
    }
    if request.destination_id == request.source_id {
        return Err(Error::Invalid("snapshot destination aliases source"));
    }
    if request.generation == 0 || request.generation <= source.generation {
        return Err(Error::Invalid("snapshot generation"));
    }
    if source.bytenr == 0 || source.root_dirid == 0 || source.level > 8 {
        return Err(Error::Corrupt("source root item"));
    }
    if request.uuid == [0; 16] || request.uuid == source.uuid {
        return Err(Error::Invalid("snapshot uuid"));
    }

    let mut root = source;
    root.generation = request.generation;
    root.last_snapshot = request.generation;
    root.flags = match request.mode {
        SnapshotMode::ReadOnly => source.flags | ROOT_SUBVOL_RDONLY,
        SnapshotMode::Writable => source.flags & !ROOT_SUBVOL_RDONLY,
    };
    root.refs = 1;
    root.uuid = request.uuid;
    root.parent_uuid = source.uuid;
    root.received_uuid = [0; 16];
    root.ctransid = request.generation;
    root.otransid = request.generation;
    root.stransid = 0;
    root.rtransid = 0;
    Ok(SnapshotPlan {
        source_id: request.source_id,
        destination_id: request.destination_id,
        root,
    })
}
