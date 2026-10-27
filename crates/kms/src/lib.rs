//! Display class core (task #228), shaped like Linux DRM/KMS so Linux userspace (wlroots, X11 modesetting,
//! Mesa) and the LoOS compositor can drive the same driver interface.
//! * [`edid`] – EDID 1.x base-block parser (identity, detailed timings, standard/established timings).
//! * [`mode`] – display modes, CVT reduced-blanking generation, DRM-style names, mode picking.
//! * [`commit`] – atomic commit validation (scanout buffer, scaling, formats, shared bandwidth).
//! * [`layout`] – multi-monitor arrangement (extended desktop, no overlap, gap detection, DPI scale).
//!
//! Pure logic with no hardware access: the miniport (virtio-gpu, GOP framebuffer, native GPU) supplies EDID
//! bytes and executes validated commits.
#![no_std]

extern crate alloc;

pub mod commit;
pub mod edid;
pub mod layout;
pub mod mode;
