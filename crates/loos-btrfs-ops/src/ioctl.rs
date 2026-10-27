//! The btrfs ioctl surface: number tables (measured from the Linux uapi headers), typed decoding of
//! every argument structure into a [`Request`], encoding of [`Reply`] values back into the caller's
//! buffer, kernel-equivalent argument validation, and the [`Backend`] trait a filesystem engine
//! implements. [`dispatch`] is what the VFS ioctl entry point calls.
//!
//! Layouts come from `ioctl_abi` (generated from the real kernel headers), so field offsets are
//! never hand-typed; the `ioctl_abi_matches_kernel_headers` integration test re-measures them.
use crate::ioctl_abi::layout::*;
use crate::ioctl_abi::IOCTLS;
use crate::volume::{le16, le32, le64, put16, put32, put64};
use alloc::string::String;
use alloc::vec::Vec;

/// ioctl "magic" type of the btrfs ioctls.
pub const BTRFS_IOCTL_MAGIC: u8 = 0x94;

/// `-errno` style error returned to user space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Errno(pub i32);

#[allow(missing_docs)]
impl Errno {
    pub const EPERM: Errno = Errno(1);
    pub const ENOENT: Errno = Errno(2);
    pub const EIO: Errno = Errno(5);
    pub const E2BIG: Errno = Errno(7);
    pub const EBADF: Errno = Errno(9);
    pub const ENOMEM: Errno = Errno(12);
    pub const EACCES: Errno = Errno(13);
    pub const EFAULT: Errno = Errno(14);
    pub const EBUSY: Errno = Errno(16);
    pub const EEXIST: Errno = Errno(17);
    pub const EXDEV: Errno = Errno(18);
    pub const ENODEV: Errno = Errno(19);
    pub const ENOTDIR: Errno = Errno(20);
    pub const EISDIR: Errno = Errno(21);
    pub const EINVAL: Errno = Errno(22);
    pub const ENOTTY: Errno = Errno(25);
    pub const ENOSPC: Errno = Errno(28);
    pub const EROFS: Errno = Errno(30);
    pub const ERANGE: Errno = Errno(34);
    pub const ENAMETOOLONG: Errno = Errno(36);
    pub const ENOTEMPTY: Errno = Errno(39);
    pub const ENODATA: Errno = Errno(61);
    pub const EOPNOTSUPP: Errno = Errno(95);
    pub const EDQUOT: Errno = Errno(122);
}

impl From<crate::error::Error> for Errno {
    fn from(e: crate::error::Error) -> Errno {
        use crate::error::Error as E;
        match e {
            E::Io | E::Csum | E::Corrupt(_) | E::NotBtrfs | E::MissingDevice(_) => Errno::EIO,
            E::ReadOnly => Errno::EROFS,
            E::NotFound => Errno::ENOENT,
            E::NotDir => Errno::ENOTDIR,
            E::IsDir => Errno::EISDIR,
            E::Unsupported(_) => Errno::EOPNOTSUPP,
            E::Invalid(_) => Errno::EINVAL,
            E::Exists => Errno::EEXIST,
            E::NoSpace => Errno::ENOSPC,
            E::NotEmpty => Errno::ENOTEMPTY,
            E::Access => Errno::EACCES,
            E::QuotaExceeded => Errno::EDQUOT,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// ioctl number helpers

/// Direction bits.
pub const IOC_NONE: u32 = 0;
/// Userspace writes (kernel reads) the argument.
pub const IOC_WRITE: u32 = 1;
/// Userspace reads (kernel writes) the argument.
pub const IOC_READ: u32 = 2;

/// Builds an ioctl number (`_IOC`).
pub const fn ioc(dir: u32, ty: u8, nr: u8, size: u32) -> u32 {
    (dir << 30) | (size << 16) | ((ty as u32) << 8) | nr as u32
}
/// Argument size encoded in a command.
pub const fn ioc_size(cmd: u32) -> usize {
    ((cmd >> 16) & 0x3fff) as usize
}
/// Direction encoded in a command.
pub const fn ioc_dir(cmd: u32) -> u32 {
    cmd >> 30
}
/// Type (magic) encoded in a command.
pub const fn ioc_type(cmd: u32) -> u8 {
    (cmd >> 8) as u8
}
/// Command number.
pub const fn ioc_nr(cmd: u32) -> u8 {
    cmd as u8
}

/// Looks an ioctl number up by its symbolic name (`BTRFS_IOC_SNAP_CREATE`, `FICLONE`, ...).
pub fn code(name: &str) -> Option<u32> {
    IOCTLS.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

/// Symbolic name of an ioctl number (the first alias in table order).
pub fn name_of(cmd: u32) -> Option<&'static str> {
    IOCTLS.iter().find(|(_, c)| *c == cmd).map(|(n, _)| *n)
}

fn c(name: &str) -> u32 {
    code(name).unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------
// typed requests / replies

/// Argument limits (`BTRFS_PATH_NAME_MAX`, ...).
pub const PATH_NAME_MAX: usize = 4087;
/// Subvolume name limit of the v2 args.
pub const SUBVOL_NAME_MAX: usize = 4039;
/// Name length limit of directory entries.
pub const VOL_NAME_MAX: usize = 255;

/// `BTRFS_SUBVOL_RDONLY`.
pub const SUBVOL_RDONLY: u64 = 1 << 1;
/// `BTRFS_SUBVOL_QGROUP_INHERIT`.
pub const SUBVOL_QGROUP_INHERIT: u64 = 1 << 2;
/// `BTRFS_DEVICE_SPEC_BY_ID`.
pub const DEVICE_SPEC_BY_ID: u64 = 1 << 3;
/// `BTRFS_SUBVOL_SPEC_BY_ID`.
pub const SUBVOL_SPEC_BY_ID: u64 = 1 << 4;

/// Resize argument (`"+1g"`, `"-512m"`, `"max"`, `"3:+1g"`, ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResizeSpec {
    /// Device to resize (`1` when omitted).
    pub devid: u64,
    /// New size semantics.
    pub kind: ResizeKind,
}

/// How a resize string changes the size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeKind {
    /// Grow to the device size.
    Max,
    /// Set to an absolute byte count.
    Absolute(u64),
    /// Grow by bytes.
    Grow(u64),
    /// Shrink by bytes.
    Shrink(u64),
}

/// Parses the string form used by `BTRFS_IOC_RESIZE` (`[devid:]{max|[+|-]size[kKmMgGtTpPeE]}`).
pub fn parse_resize(s: &str) -> Option<ResizeSpec> {
    let (devid, rest) = match s.split_once(':') {
        Some((d, r)) => (d.parse::<u64>().ok()?, r),
        None => (1, s),
    };
    if rest == "max" {
        return Some(ResizeSpec { devid, kind: ResizeKind::Max });
    }
    let (sign, num) = match rest.as_bytes().first()? {
        b'+' => (1, &rest[1..]),
        b'-' => (-1, &rest[1..]),
        _ => (0, rest),
    };
    let (digits, suffix) = match num.find(|c: char| !c.is_ascii_digit()) {
        Some(p) => (&num[..p], &num[p..]),
        None => (num, ""),
    };
    if digits.is_empty() {
        return None;
    }
    let mult: u64 = match suffix {
        "" => 1,
        "k" | "K" => 1 << 10,
        "m" | "M" => 1 << 20,
        "g" | "G" => 1 << 30,
        "t" | "T" => 1 << 40,
        "p" | "P" => 1 << 50,
        "e" | "E" => 1 << 60,
        _ => return None,
    };
    let v = digits.parse::<u64>().ok()?.checked_mul(mult)?;
    Some(ResizeSpec { devid, kind: match sign { 1 => ResizeKind::Grow(v), -1 => ResizeKind::Shrink(v), _ => ResizeKind::Absolute(v) } })
}

/// A device or subvolume addressed by name or id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Spec {
    /// By path/name bytes.
    Name(Vec<u8>),
    /// By numeric id.
    Id(u64),
}

/// `btrfs_qgroup_limit`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QgroupLimit {
    /// `QGROUP_LIMIT_*` flags.
    pub flags: u64,
    /// Max referenced.
    pub max_rfer: u64,
    /// Max exclusive.
    pub max_excl: u64,
    /// Reserved referenced.
    pub rsv_rfer: u64,
    /// Reserved exclusive.
    pub rsv_excl: u64,
}

/// `btrfs_balance_args`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BalanceArgs {
    /// Profiles bit mask.
    pub profiles: u64,
    /// Usage filter (`usage_min` low word / `usage_max` high word).
    pub usage: u64,
    /// Device id.
    pub devid: u64,
    /// Physical range.
    pub pstart: u64,
    /// Physical range end.
    pub pend: u64,
    /// Virtual range.
    pub vstart: u64,
    /// Virtual range end.
    pub vend: u64,
    /// Convert-to profile.
    pub target: u64,
    /// `BALANCE_ARGS_*` flags.
    pub flags: u64,
    /// Chunk limit.
    pub limit: u64,
    /// Minimum stripes.
    pub stripes_min: u32,
    /// Maximum stripes.
    pub stripes_max: u32,
}

/// `btrfs_scrub_progress` (15 counters).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrubProgress {
    /// All counters in header order.
    pub counters: [u64; 15],
}

/// Search key of `BTRFS_IOC_TREE_SEARCH`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchKey {
    /// Tree id.
    pub tree_id: u64,
    /// Lowest object id.
    pub min_objectid: u64,
    /// Highest object id.
    pub max_objectid: u64,
    /// Lowest offset.
    pub min_offset: u64,
    /// Highest offset.
    pub max_offset: u64,
    /// Lowest transid.
    pub min_transid: u64,
    /// Highest transid.
    pub max_transid: u64,
    /// Lowest item type.
    pub min_type: u32,
    /// Highest item type.
    pub max_type: u32,
    /// Max items to return.
    pub nr_items: u32,
}

/// One tree-search hit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchItem {
    /// Item generation.
    pub transid: u64,
    /// Object id.
    pub objectid: u64,
    /// Offset.
    pub offset: u64,
    /// Type.
    pub ty: u32,
    /// Payload.
    pub data: Vec<u8>,
}

/// One destination of a dedupe request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DedupeDest {
    /// Destination file descriptor.
    pub fd: i64,
    /// Offset in the destination.
    pub offset: u64,
}

/// Result per dedupe destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DedupeResult {
    /// Bytes deduplicated.
    pub bytes: u64,
    /// `0` ok, `1` data differs, negative errno.
    pub status: i32,
}

/// Defrag range parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefragRange {
    /// Start offset.
    pub start: u64,
    /// Length (`u64::MAX` = to EOF).
    pub len: u64,
    /// `DEFRAG_RANGE_*` flags.
    pub flags: u64,
    /// Extents at least this big are left alone (0 = default 32 MiB).
    pub extent_thresh: u32,
    /// Compression type (1 zlib, 2 lzo, 3 zstd).
    pub compress_type: u8,
    /// Compression level.
    pub level: i8,
}

/// `FS_IOC_FIEMAP` request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FiemapReq {
    /// Start byte.
    pub start: u64,
    /// Length.
    pub length: u64,
    /// `FIEMAP_FLAG_*`.
    pub flags: u32,
    /// Slots in the caller's array (0 = just count).
    pub extent_count: u32,
}

/// One FIEMAP extent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FiemapExtent {
    /// Logical file offset.
    pub logical: u64,
    /// Physical (or logical btrfs address).
    pub physical: u64,
    /// Length.
    pub length: u64,
    /// `FIEMAP_EXTENT_*`.
    pub flags: u32,
}

/// `btrfs_ioctl_fs_info_args` reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FsInfo {
    /// Highest device id.
    pub max_id: u64,
    /// Device count.
    pub num_devices: u64,
    /// Filesystem UUID.
    pub fsid: [u8; 16],
    /// Node size.
    pub nodesize: u32,
    /// Sector size.
    pub sectorsize: u32,
    /// Clone alignment.
    pub clone_alignment: u32,
    /// Checksum type.
    pub csum_type: u16,
    /// Checksum size.
    pub csum_size: u16,
    /// Generation.
    pub generation: u64,
    /// Metadata UUID.
    pub metadata_uuid: [u8; 16],
}

/// `btrfs_ioctl_dev_info_args` reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevInfo {
    /// Device id.
    pub devid: u64,
    /// Device UUID.
    pub uuid: [u8; 16],
    /// Allocated bytes.
    pub bytes_used: u64,
    /// Size.
    pub total_bytes: u64,
    /// Filesystem UUID.
    pub fsid: [u8; 16],
    /// Path (max 1023 bytes).
    pub path: Vec<u8>,
}

/// `btrfs_ioctl_get_subvol_info_args` reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubvolInfoReply {
    /// Tree id.
    pub treeid: u64,
    /// Name in the parent.
    pub name: Vec<u8>,
    /// Parent tree.
    pub parent_id: u64,
    /// Directory inode in the parent.
    pub dirid: u64,
    /// Generation.
    pub generation: u64,
    /// Root flags.
    pub flags: u64,
    /// UUID.
    pub uuid: [u8; 16],
    /// Parent UUID.
    pub parent_uuid: [u8; 16],
    /// Received UUID.
    pub received_uuid: [u8; 16],
    /// ctransid, otransid, stransid, rtransid.
    pub transids: [u64; 4],
}

/// Decoded ioctl.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// `BTRFS_IOC_FS_INFO`.
    FsInfo { flags: u64 },
    /// `BTRFS_IOC_DEV_INFO`.
    DevInfo { devid: u64, uuid: [u8; 16] },
    /// `BTRFS_IOC_SPACE_INFO` (`slots` = capacity of the caller's array).
    SpaceInfo { slots: u64 },
    /// `BTRFS_IOC_SUBVOL_CREATE{,_V2}`.
    SubvolCreate { name: Vec<u8>, read_only: bool, qgroups: Option<Vec<u8>> },
    /// `BTRFS_IOC_SNAP_CREATE{,_V2}`.
    SnapCreate { source_fd: i64, name: Vec<u8>, read_only: bool, qgroups: Option<Vec<u8>> },
    /// `BTRFS_IOC_SNAP_DESTROY{,_V2}`.
    SnapDestroy { target: Spec },
    /// `BTRFS_IOC_SUBVOL_GETFLAGS`.
    SubvolGetFlags,
    /// `BTRFS_IOC_SUBVOL_SETFLAGS`.
    SubvolSetFlags { flags: u64 },
    /// `BTRFS_IOC_DEFAULT_SUBVOL`.
    DefaultSubvol { id: u64 },
    /// `FICLONE` (`len == 0` + both offsets 0), `FICLONERANGE`, `BTRFS_IOC_CLONE{,_RANGE}`.
    CloneRange { src_fd: i64, src_offset: u64, src_length: u64, dest_offset: u64 },
    /// `FIDEDUPERANGE` / `BTRFS_IOC_FILE_EXTENT_SAME`.
    Dedupe { src_offset: u64, src_length: u64, dests: Vec<DedupeDest> },
    /// `BTRFS_IOC_DEFRAG{,_RANGE}`.
    Defrag { range: Option<DefragRange> },
    /// `BTRFS_IOC_RESIZE`.
    Resize { spec: ResizeSpec },
    /// `BTRFS_IOC_ADD_DEV`.
    AddDev { path: Vec<u8> },
    /// `BTRFS_IOC_RM_DEV{,_V2}`.
    RmDev { target: Spec },
    /// `BTRFS_IOC_SCAN_DEV`.
    ScanDev { path: Vec<u8> },
    /// `BTRFS_IOC_FORGET_DEV`.
    ForgetDev { path: Vec<u8> },
    /// `BTRFS_IOC_DEVICES_READY`.
    DevicesReady { path: Vec<u8> },
    /// `BTRFS_IOC_BALANCE_V2` / `BTRFS_IOC_BALANCE`.
    Balance { flags: u64, data: BalanceArgs, meta: BalanceArgs, sys: BalanceArgs },
    /// `BTRFS_IOC_BALANCE_CTL` (1 pause, 2 cancel).
    BalanceCtl { cmd: i32 },
    /// `BTRFS_IOC_BALANCE_PROGRESS`.
    BalanceProgress,
    /// `BTRFS_IOC_SCRUB`.
    Scrub { devid: u64, start: u64, end: u64, flags: u64 },
    /// `BTRFS_IOC_SCRUB_CANCEL`.
    ScrubCancel,
    /// `BTRFS_IOC_SCRUB_PROGRESS`.
    ScrubProgress { devid: u64 },
    /// `BTRFS_IOC_QUOTA_CTL`.
    QuotaCtl { cmd: u64 },
    /// `BTRFS_IOC_QGROUP_ASSIGN`.
    QgroupAssign { assign: bool, src: u64, dst: u64 },
    /// `BTRFS_IOC_QGROUP_CREATE`.
    QgroupCreate { create: bool, qgroupid: u64 },
    /// `BTRFS_IOC_QGROUP_LIMIT`.
    QgroupLimit { qgroupid: u64, limit: QgroupLimit },
    /// `BTRFS_IOC_QUOTA_RESCAN`.
    QuotaRescan { flags: u64 },
    /// `BTRFS_IOC_QUOTA_RESCAN_STATUS`.
    QuotaRescanStatus,
    /// `BTRFS_IOC_QUOTA_RESCAN_WAIT`.
    QuotaRescanWait,
    /// `BTRFS_IOC_TREE_SEARCH{,_V2}` (`buf_size` = capacity of the result buffer).
    TreeSearch { key: SearchKey, buf_size: usize },
    /// `BTRFS_IOC_INO_LOOKUP`.
    InoLookup { treeid: u64, objectid: u64 },
    /// `BTRFS_IOC_INO_LOOKUP_USER`.
    InoLookupUser { dirid: u64, treeid: u64 },
    /// `BTRFS_IOC_INO_PATHS`.
    InoPaths { inum: u64, size: u64 },
    /// `BTRFS_IOC_LOGICAL_INO{,_V2}`.
    LogicalIno { logical: u64, size: u64, flags: u64 },
    /// `BTRFS_IOC_GET_FSLABEL` / `FS_IOC_GETFSLABEL`.
    GetLabel,
    /// `BTRFS_IOC_SET_FSLABEL` / `FS_IOC_SETFSLABEL`.
    SetLabel { label: Vec<u8> },
    /// `BTRFS_IOC_GET_FEATURES`.
    GetFeatures,
    /// `BTRFS_IOC_SET_FEATURES` (flags + mask for compat, compat_ro, incompat).
    SetFeatures { flags: [u64; 3], mask: [u64; 3] },
    /// `BTRFS_IOC_GET_SUPPORTED_FEATURES`.
    GetSupportedFeatures,
    /// `BTRFS_IOC_GET_SUBVOL_INFO`.
    GetSubvolInfo,
    /// `BTRFS_IOC_GET_SUBVOL_ROOTREF`.
    GetSubvolRootref { min_treeid: u64 },
    /// `BTRFS_IOC_GET_DEV_STATS`.
    GetDevStats { devid: u64, flags: u64 },
    /// `BTRFS_IOC_SYNC`.
    Sync,
    /// `BTRFS_IOC_START_SYNC`.
    StartSync,
    /// `BTRFS_IOC_WAIT_SYNC`.
    WaitSync { transid: u64 },
    /// `BTRFS_IOC_SUBVOL_SYNC_WAIT`.
    SubvolSyncWait { subvolid: u64, mode: u32 },
    /// `BTRFS_IOC_SEND`.
    Send { send_fd: i64, clone_sources_count: u64, parent_root: u64, flags: u64, version: u32 },
    /// `BTRFS_IOC_SET_RECEIVED_SUBVOL`.
    SetReceivedSubvol { uuid: [u8; 16], stransid: u64, stime_sec: u64, stime_nsec: u32, flags: u64 },
    /// `BTRFS_IOC_DEV_REPLACE` (start/status/cancel).
    DevReplace { cmd: u64, srcdevid: u64, src_name: Vec<u8>, tgt_name: Vec<u8>, mode: u64 },
    /// `FS_IOC_FIEMAP`.
    Fiemap(FiemapReq),
    /// `FS_IOC_GETFLAGS`.
    GetFlags,
    /// `FS_IOC_SETFLAGS`.
    SetFlags { flags: u32 },
    /// `BTRFS_IOC_SHUTDOWN`.
    Shutdown { flags: u32 },
    /// Valid btrfs ioctl without a typed decoder (`ENCODED_READ/WRITE`, `GET_CSUMS`, ...).
    Other { cmd: u32, arg: Vec<u8> },
}

/// Values a backend returns; [`dispatch`] serialises them into the user buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Success, no payload.
    Done,
    /// Scalar (`GETFLAGS`, `GET_SUBVOL_FLAGS`, `START_SYNC` transid, ...).
    Value(u64),
    /// FS info.
    FsInfo(FsInfo),
    /// Device info.
    DevInfo(DevInfo),
    /// `(flags, total_bytes, used_bytes)` per space type; `total` may exceed the caller's slots.
    SpaceInfo(Vec<(u64, u64, u64)>),
    /// Dedupe per-destination results.
    Dedupe(Vec<DedupeResult>),
    /// Scrub counters.
    Scrub(ScrubProgress),
    /// Balance state: flags, state, data/meta/sys args, (expected, considered, completed).
    Balance { flags: u64, state: u64, data: BalanceArgs, meta: BalanceArgs, sys: BalanceArgs, stat: [u64; 3] },
    /// Tree search hits.
    TreeSearch(Vec<SearchItem>),
    /// A path (`INO_LOOKUP`, `INO_LOOKUP_USER`).
    Path { treeid: u64, path: Vec<u8> },
    /// Paths of an inode (`INO_PATHS`).
    Paths(Vec<Vec<u8>>),
    /// `(inode, offset, root)` triples (`LOGICAL_INO`).
    Inodes(Vec<(u64, u64, u64)>),
    /// Filesystem label.
    Label(Vec<u8>),
    /// Feature words: `[compat, compat_ro, incompat]` (supported / enabled / safe-set per call).
    Features([u64; 3]),
    /// Subvolume info.
    SubvolInfo(SubvolInfoReply),
    /// `(treeid, dirid)` rootrefs.
    RootRefs(Vec<(u64, u64)>),
    /// Device statistics (`write, read, flush, corruption, generation`).
    DevStats { values: [u64; 5], nr_items: u64 },
    /// Rescan status: flags, progress.
    Rescan { flags: u64, progress: u64 },
    /// Received subvolume transid and time.
    Received { rtransid: u64, rtime_sec: u64, rtime_nsec: u32 },
    /// FIEMAP result.
    Fiemap { mapped: u32, extents: Vec<FiemapExtent> },
    /// Dev-replace status/result: result code, state, progress per mille, started, stopped, write errs, read errs.
    DevReplace { result: u64, status: [u64; 6] },
    /// Raw bytes written back over the caller's buffer.
    Raw(Vec<u8>),
}

/// A filesystem engine.
pub trait Backend {
    /// Executes one decoded request.
    fn ioctl(&mut self, req: Request) -> Result<Reply, Errno>;
}

// ---------------------------------------------------------------------------------------------
// decoding

fn cstr(b: &[u8]) -> Result<Vec<u8>, Errno> {
    match b.iter().position(|&c| c == 0) {
        Some(p) => Ok(b[..p].to_vec()),
        None => Err(Errno::ENAMETOOLONG),
    }
}

fn need(arg: &[u8], n: usize) -> Result<(), Errno> {
    if arg.len() < n {
        Err(Errno::EFAULT)
    } else {
        Ok(())
    }
}

fn valid_name(n: &[u8]) -> Result<(), Errno> {
    if n.is_empty() || n.len() > VOL_NAME_MAX {
        return Err(Errno::EINVAL);
    }
    if n.contains(&b'/') || n == b"." || n == b".." {
        return Err(Errno::EINVAL);
    }
    Ok(())
}

fn balance_args(b: &[u8], o: usize) -> BalanceArgs {
    use crate::ioctl_abi::layout::btrfs_balance_args as A;
    BalanceArgs {
        profiles: le64(b, o + A::PROFILES),
        usage: le64(b, o + A::USAGE),
        devid: le64(b, o + A::DEVID),
        pstart: le64(b, o + A::PSTART),
        pend: le64(b, o + A::PEND),
        vstart: le64(b, o + A::VSTART),
        vend: le64(b, o + A::VEND),
        target: le64(b, o + A::TARGET),
        flags: le64(b, o + A::FLAGS),
        limit: le64(b, o + A::LIMIT),
        stripes_min: le32(b, o + A::STRIPES_MIN),
        stripes_max: le32(b, o + A::STRIPES_MAX),
    }
}

fn put_balance_args(b: &mut [u8], o: usize, a: &BalanceArgs) {
    use crate::ioctl_abi::layout::btrfs_balance_args as A;
    put64(b, o + A::PROFILES, a.profiles);
    put64(b, o + A::USAGE, a.usage);
    put64(b, o + A::DEVID, a.devid);
    put64(b, o + A::PSTART, a.pstart);
    put64(b, o + A::PEND, a.pend);
    put64(b, o + A::VSTART, a.vstart);
    put64(b, o + A::VEND, a.vend);
    put64(b, o + A::TARGET, a.target);
    put64(b, o + A::FLAGS, a.flags);
    put64(b, o + A::LIMIT, a.limit);
    put32(b, o + A::STRIPES_MIN, a.stripes_min);
    put32(b, o + A::STRIPES_MAX, a.stripes_max);
}

/// Decodes the user argument of `cmd`. `arg` is the caller-provided buffer (at least the
/// size encoded in the command, longer for variable-size requests).
pub fn decode(cmd: u32, arg: &[u8]) -> Result<Request, Errno> {
    if ioc_type(cmd) != BTRFS_IOCTL_MAGIC && !matches!(name_of(cmd), Some(n) if n.starts_with("FS_IOC_")) && !matches!(name_of(cmd), Some("BTRFS_IOC_SHUTDOWN")) {
        return Err(Errno::ENOTTY);
    }
    let known = name_of(cmd).ok_or(Errno::ENOTTY)?;
    // aliases share numbers (e.g. BTRFS_IOC_GET_FSLABEL == FS_IOC_GETFSLABEL): normalise by number
    let is = |n: &str| code(n) == Some(cmd);
    need(arg, ioc_size(cmd))?;

    if is("BTRFS_IOC_FS_INFO") {
        need(arg, btrfs_ioctl_fs_info_args::SIZE_OF)?;
        return Ok(Request::FsInfo { flags: le64(arg, btrfs_ioctl_fs_info_args::FLAGS) });
    }
    if is("BTRFS_IOC_DEV_INFO") {
        need(arg, btrfs_ioctl_dev_info_args::SIZE_OF)?;
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&arg[btrfs_ioctl_dev_info_args::UUID..btrfs_ioctl_dev_info_args::UUID + 16]);
        return Ok(Request::DevInfo { devid: le64(arg, btrfs_ioctl_dev_info_args::DEVID), uuid });
    }
    if is("BTRFS_IOC_SPACE_INFO") {
        need(arg, btrfs_ioctl_space_args::SIZE_OF)?;
        return Ok(Request::SpaceInfo { slots: le64(arg, btrfs_ioctl_space_args::SPACE_SLOTS) });
    }
    if is("BTRFS_IOC_SUBVOL_CREATE") || is("BTRFS_IOC_SNAP_CREATE") {
        need(arg, btrfs_ioctl_vol_args::SIZE_OF)?;
        let name = cstr(&arg[btrfs_ioctl_vol_args::NAME..btrfs_ioctl_vol_args::SIZE_OF])?;
        valid_name(&name)?;
        let fd = le64(arg, btrfs_ioctl_vol_args::FD) as i64;
        return Ok(if is("BTRFS_IOC_SUBVOL_CREATE") {
            Request::SubvolCreate { name, read_only: false, qgroups: None }
        } else {
            Request::SnapCreate { source_fd: fd, name, read_only: false, qgroups: None }
        });
    }
    if is("BTRFS_IOC_SUBVOL_CREATE_V2") || is("BTRFS_IOC_SNAP_CREATE_V2") || is("BTRFS_IOC_SNAP_DESTROY_V2") || is("BTRFS_IOC_RM_DEV_V2") {
        need(arg, btrfs_ioctl_vol_args_v2::SIZE_OF)?;
        let a = btrfs_ioctl_vol_args_v2::SIZE_OF;
        let _ = a;
        let flags = le64(arg, btrfs_ioctl_vol_args_v2::FLAGS);
        let fd = le64(arg, btrfs_ioctl_vol_args_v2::FD) as i64;
        let name_off = btrfs_ioctl_vol_args_v2::NAME;
        if is("BTRFS_IOC_SNAP_DESTROY_V2") {
            if flags & !SUBVOL_SPEC_BY_ID != 0 {
                return Err(Errno::EOPNOTSUPP);
            }
            return Ok(Request::SnapDestroy {
                target: if flags & SUBVOL_SPEC_BY_ID != 0 { Spec::Id(le64(arg, name_off)) } else { Spec::Name(cstr(&arg[name_off..btrfs_ioctl_vol_args_v2::SIZE_OF])?) },
            });
        }
        if is("BTRFS_IOC_RM_DEV_V2") {
            if flags & !DEVICE_SPEC_BY_ID != 0 {
                return Err(Errno::EOPNOTSUPP);
            }
            return Ok(Request::RmDev {
                target: if flags & DEVICE_SPEC_BY_ID != 0 { Spec::Id(le64(arg, name_off)) } else { Spec::Name(cstr(&arg[name_off..btrfs_ioctl_vol_args_v2::SIZE_OF])?) },
            });
        }
        if flags & !(SUBVOL_RDONLY | SUBVOL_QGROUP_INHERIT) != 0 {
            return Err(Errno::EOPNOTSUPP);
        }
        let name = cstr(&arg[name_off..btrfs_ioctl_vol_args_v2::SIZE_OF])?;
        valid_name(&name)?;
        let qgroups = if flags & SUBVOL_QGROUP_INHERIT != 0 {
            // the inherit blob is a user pointer in the real ABI; here callers append it after the struct
            let sz = le64(arg, btrfs_ioctl_vol_args_v2::SIZE) as usize;
            if sz > 64 * 1024 || arg.len() < btrfs_ioctl_vol_args_v2::SIZE_OF + sz {
                return Err(Errno::EINVAL);
            }
            Some(arg[btrfs_ioctl_vol_args_v2::SIZE_OF..btrfs_ioctl_vol_args_v2::SIZE_OF + sz].to_vec())
        } else {
            None
        };
        let read_only = flags & SUBVOL_RDONLY != 0;
        return Ok(if is("BTRFS_IOC_SUBVOL_CREATE_V2") { Request::SubvolCreate { name, read_only, qgroups } } else { Request::SnapCreate { source_fd: fd, name, read_only, qgroups } });
    }
    if is("BTRFS_IOC_SNAP_DESTROY") {
        need(arg, btrfs_ioctl_vol_args::SIZE_OF)?;
        let name = cstr(&arg[btrfs_ioctl_vol_args::NAME..btrfs_ioctl_vol_args::SIZE_OF])?;
        valid_name(&name)?;
        return Ok(Request::SnapDestroy { target: Spec::Name(name) });
    }
    if is("BTRFS_IOC_SUBVOL_GETFLAGS") {
        return Ok(Request::SubvolGetFlags);
    }
    if is("BTRFS_IOC_SUBVOL_SETFLAGS") {
        need(arg, 8)?;
        let f = le64(arg, 0);
        if f & !SUBVOL_RDONLY != 0 {
            return Err(Errno::EOPNOTSUPP);
        }
        return Ok(Request::SubvolSetFlags { flags: f });
    }
    if is("BTRFS_IOC_DEFAULT_SUBVOL") {
        need(arg, 8)?;
        return Ok(Request::DefaultSubvol { id: le64(arg, 0) });
    }
    if is("FICLONE") || is("BTRFS_IOC_CLONE") {
        need(arg, 4)?;
        return Ok(Request::CloneRange { src_fd: le32(arg, 0) as i32 as i64, src_offset: 0, src_length: 0, dest_offset: 0 });
    }
    if is("FICLONERANGE") {
        let m = file_clone_range::SIZE_OF;
        need(arg, m)?;
        return Ok(Request::CloneRange {
            src_fd: le64(arg, file_clone_range::SRC_FD) as i64,
            src_offset: le64(arg, file_clone_range::SRC_OFFSET),
            src_length: le64(arg, file_clone_range::SRC_LENGTH),
            dest_offset: le64(arg, file_clone_range::DEST_OFFSET),
        });
    }
    if is("BTRFS_IOC_CLONE_RANGE") {
        let m = btrfs_ioctl_clone_range_args::SIZE_OF;
        need(arg, m)?;
        return Ok(Request::CloneRange {
            src_fd: le64(arg, btrfs_ioctl_clone_range_args::SRC_FD) as i64,
            src_offset: le64(arg, btrfs_ioctl_clone_range_args::SRC_OFFSET),
            src_length: le64(arg, btrfs_ioctl_clone_range_args::SRC_LENGTH),
            dest_offset: le64(arg, btrfs_ioctl_clone_range_args::DEST_OFFSET),
        });
    }
    if is("FIDEDUPERANGE") || is("BTRFS_IOC_FILE_EXTENT_SAME") {
        let hdr = file_dedupe_range::SIZE_OF;
        let info = file_dedupe_range_info::SIZE_OF;
        need(arg, hdr)?;
        let count = le16(arg, file_dedupe_range::DEST_COUNT) as usize;
        need(arg, hdr + count * info)?;
        if count == 0 {
            return Err(Errno::EINVAL);
        }
        if le16(arg, file_dedupe_range::RESERVED1) != 0 || le32(arg, file_dedupe_range::RESERVED2) != 0 {
            return Err(Errno::EINVAL);
        }
        let dests = (0..count)
            .map(|i| {
                let o = hdr + i * info;
                DedupeDest { fd: le64(arg, o + file_dedupe_range_info::DEST_FD) as i64, offset: le64(arg, o + file_dedupe_range_info::DEST_OFFSET) }
            })
            .collect();
        return Ok(Request::Dedupe { src_offset: le64(arg, file_dedupe_range::SRC_OFFSET), src_length: le64(arg, file_dedupe_range::SRC_LENGTH), dests });
    }
    if is("BTRFS_IOC_DEFRAG") {
        return Ok(Request::Defrag { range: None });
    }
    if is("BTRFS_IOC_DEFRAG_RANGE") {
        need(arg, btrfs_ioctl_defrag_range_args::SIZE_OF)?;
        let flags = le64(arg, btrfs_ioctl_defrag_range_args::FLAGS);
        if flags & !0xf != 0 {
            return Err(Errno::EOPNOTSUPP);
        }
        let ct = le32(arg, btrfs_ioctl_defrag_range_args::COMPRESS_TYPE);
        return Ok(Request::Defrag {
            range: Some(DefragRange {
                start: le64(arg, btrfs_ioctl_defrag_range_args::START),
                len: le64(arg, btrfs_ioctl_defrag_range_args::LEN),
                flags,
                extent_thresh: le32(arg, btrfs_ioctl_defrag_range_args::EXTENT_THRESH),
                compress_type: ct as u8,
                level: (ct >> 8) as i8,
            }),
        });
    }
    if is("BTRFS_IOC_RESIZE") {
        need(arg, btrfs_ioctl_vol_args::SIZE_OF)?;
        let s = cstr(&arg[btrfs_ioctl_vol_args::NAME..btrfs_ioctl_vol_args::SIZE_OF])?;
        let s = core::str::from_utf8(&s).map_err(|_| Errno::EINVAL)?;
        return parse_resize(s).map(|spec| Request::Resize { spec }).ok_or(Errno::EINVAL);
    }
    if is("BTRFS_IOC_ADD_DEV") || is("BTRFS_IOC_SCAN_DEV") || is("BTRFS_IOC_FORGET_DEV") || is("BTRFS_IOC_DEVICES_READY") || is("BTRFS_IOC_RM_DEV") {
        need(arg, btrfs_ioctl_vol_args::SIZE_OF)?;
        let path = cstr(&arg[btrfs_ioctl_vol_args::NAME..btrfs_ioctl_vol_args::SIZE_OF])?;
        return Ok(if is("BTRFS_IOC_ADD_DEV") {
            Request::AddDev { path }
        } else if is("BTRFS_IOC_SCAN_DEV") {
            Request::ScanDev { path }
        } else if is("BTRFS_IOC_FORGET_DEV") {
            Request::ForgetDev { path }
        } else if is("BTRFS_IOC_DEVICES_READY") {
            Request::DevicesReady { path }
        } else {
            Request::RmDev { target: Spec::Name(path) }
        });
    }
    if is("BTRFS_IOC_BALANCE") {
        return Ok(Request::Balance { flags: 0, data: BalanceArgs::default(), meta: BalanceArgs::default(), sys: BalanceArgs::default() });
    }
    if is("BTRFS_IOC_BALANCE_V2") {
        let m = btrfs_ioctl_balance_args::SIZE_OF;
        need(arg, m)?;
        return Ok(Request::Balance {
            flags: le64(arg, btrfs_ioctl_balance_args::FLAGS),
            data: balance_args(arg, btrfs_ioctl_balance_args::DATA),
            meta: balance_args(arg, btrfs_ioctl_balance_args::META),
            sys: balance_args(arg, btrfs_ioctl_balance_args::SYS),
        });
    }
    if is("BTRFS_IOC_BALANCE_CTL") {
        need(arg, 4)?;
        let v = le32(arg, 0) as i32;
        if v != 1 && v != 2 {
            return Err(Errno::EINVAL);
        }
        return Ok(Request::BalanceCtl { cmd: v });
    }
    if is("BTRFS_IOC_BALANCE_PROGRESS") {
        return Ok(Request::BalanceProgress);
    }
    if is("BTRFS_IOC_SCRUB") {
        need(arg, btrfs_ioctl_scrub_args::SIZE_OF)?;
        let flags = le64(arg, btrfs_ioctl_scrub_args::FLAGS);
        if flags & !1 != 0 {
            return Err(Errno::EOPNOTSUPP);
        }
        return Ok(Request::Scrub { devid: le64(arg, btrfs_ioctl_scrub_args::DEVID), start: le64(arg, btrfs_ioctl_scrub_args::START), end: le64(arg, btrfs_ioctl_scrub_args::END), flags });
    }
    if is("BTRFS_IOC_SCRUB_CANCEL") {
        return Ok(Request::ScrubCancel);
    }
    if is("BTRFS_IOC_SCRUB_PROGRESS") {
        need(arg, btrfs_ioctl_scrub_args::SIZE_OF)?;
        return Ok(Request::ScrubProgress { devid: le64(arg, btrfs_ioctl_scrub_args::DEVID) });
    }
    if is("BTRFS_IOC_QUOTA_CTL") {
        need(arg, btrfs_ioctl_quota_ctl_args::SIZE_OF)?;
        let cmd = le64(arg, btrfs_ioctl_quota_ctl_args::CMD);
        if !(1..=4).contains(&cmd) || cmd == 3 {
            return Err(Errno::EINVAL);
        }
        return Ok(Request::QuotaCtl { cmd });
    }
    if is("BTRFS_IOC_QGROUP_ASSIGN") {
        need(arg, btrfs_ioctl_qgroup_assign_args::SIZE_OF)?;
        let src = le64(arg, btrfs_ioctl_qgroup_assign_args::SRC);
        let dst = le64(arg, btrfs_ioctl_qgroup_assign_args::DST);
        if src == dst {
            return Err(Errno::EINVAL);
        }
        return Ok(Request::QgroupAssign { assign: le64(arg, btrfs_ioctl_qgroup_assign_args::ASSIGN) != 0, src, dst });
    }
    if is("BTRFS_IOC_QGROUP_CREATE") {
        need(arg, btrfs_ioctl_qgroup_create_args::SIZE_OF)?;
        let id = le64(arg, btrfs_ioctl_qgroup_create_args::QGROUPID);
        if id == 0 {
            return Err(Errno::EINVAL);
        }
        return Ok(Request::QgroupCreate { create: le64(arg, btrfs_ioctl_qgroup_create_args::CREATE) != 0, qgroupid: id });
    }
    if is("BTRFS_IOC_QGROUP_LIMIT") {
        need(arg, btrfs_ioctl_qgroup_limit_args::SIZE_OF)?;
        let o = btrfs_ioctl_qgroup_limit_args::LIM;
        return Ok(Request::QgroupLimit {
            qgroupid: le64(arg, btrfs_ioctl_qgroup_limit_args::QGROUPID),
            limit: QgroupLimit { flags: le64(arg, o), max_rfer: le64(arg, o + 8), max_excl: le64(arg, o + 16), rsv_rfer: le64(arg, o + 24), rsv_excl: le64(arg, o + 32) },
        });
    }
    if is("BTRFS_IOC_QUOTA_RESCAN") {
        need(arg, btrfs_ioctl_quota_rescan_args::SIZE_OF)?;
        let f = le64(arg, btrfs_ioctl_quota_rescan_args::FLAGS);
        if f != 0 {
            return Err(Errno::EINVAL);
        }
        return Ok(Request::QuotaRescan { flags: f });
    }
    if is("BTRFS_IOC_QUOTA_RESCAN_STATUS") {
        return Ok(Request::QuotaRescanStatus);
    }
    if is("BTRFS_IOC_QUOTA_RESCAN_WAIT") {
        return Ok(Request::QuotaRescanWait);
    }
    if is("BTRFS_IOC_TREE_SEARCH") || is("BTRFS_IOC_TREE_SEARCH_V2") {
        let k = btrfs_ioctl_search_key::SIZE_OF;
        need(arg, k)?;
        let key = SearchKey {
            tree_id: le64(arg, btrfs_ioctl_search_key::TREE_ID),
            min_objectid: le64(arg, btrfs_ioctl_search_key::MIN_OBJECTID),
            max_objectid: le64(arg, btrfs_ioctl_search_key::MAX_OBJECTID),
            min_offset: le64(arg, btrfs_ioctl_search_key::MIN_OFFSET),
            max_offset: le64(arg, btrfs_ioctl_search_key::MAX_OFFSET),
            min_transid: le64(arg, btrfs_ioctl_search_key::MIN_TRANSID),
            max_transid: le64(arg, btrfs_ioctl_search_key::MAX_TRANSID),
            min_type: le32(arg, btrfs_ioctl_search_key::MIN_TYPE),
            max_type: le32(arg, btrfs_ioctl_search_key::MAX_TYPE),
            nr_items: le32(arg, btrfs_ioctl_search_key::NR_ITEMS),
        };
        let buf_size = if is("BTRFS_IOC_TREE_SEARCH") {
            btrfs_ioctl_search_args::SIZE_OF - k
        } else {
            need(arg, btrfs_ioctl_search_args_v2::SIZE_OF)?;
            let bs = le64(arg, btrfs_ioctl_search_args_v2::BUF_SIZE) as usize;
            if bs > arg.len() - btrfs_ioctl_search_args_v2::SIZE_OF {
                return Err(Errno::EFAULT);
            }
            bs
        };
        return Ok(Request::TreeSearch { key, buf_size });
    }
    if is("BTRFS_IOC_INO_LOOKUP") {
        need(arg, btrfs_ioctl_ino_lookup_args::SIZE_OF)?;
        return Ok(Request::InoLookup { treeid: le64(arg, btrfs_ioctl_ino_lookup_args::TREEID), objectid: le64(arg, btrfs_ioctl_ino_lookup_args::OBJECTID) });
    }
    if is("BTRFS_IOC_INO_LOOKUP_USER") {
        need(arg, btrfs_ioctl_ino_lookup_user_args::SIZE_OF)?;
        return Ok(Request::InoLookupUser { dirid: le64(arg, btrfs_ioctl_ino_lookup_user_args::DIRID), treeid: le64(arg, btrfs_ioctl_ino_lookup_user_args::TREEID) });
    }
    if is("BTRFS_IOC_INO_PATHS") {
        need(arg, btrfs_ioctl_ino_path_args::SIZE_OF)?;
        let size = le64(arg, btrfs_ioctl_ino_path_args::SIZE);
        if size < 16 || size > 65536 {
            return Err(Errno::EINVAL);
        }
        return Ok(Request::InoPaths { inum: le64(arg, btrfs_ioctl_ino_path_args::INUM), size });
    }
    if is("BTRFS_IOC_LOGICAL_INO") || is("BTRFS_IOC_LOGICAL_INO_V2") {
        need(arg, btrfs_ioctl_logical_ino_args::SIZE_OF)?;
        let flags = if is("BTRFS_IOC_LOGICAL_INO_V2") { le64(arg, btrfs_ioctl_logical_ino_args::FLAGS) } else { 0 };
        if flags & !1 != 0 {
            return Err(Errno::EINVAL);
        }
        return Ok(Request::LogicalIno { logical: le64(arg, btrfs_ioctl_logical_ino_args::LOGICAL), size: le64(arg, btrfs_ioctl_logical_ino_args::SIZE), flags });
    }
    if is("BTRFS_IOC_GET_FSLABEL") {
        return Ok(Request::GetLabel);
    }
    if is("BTRFS_IOC_SET_FSLABEL") {
        need(arg, 256)?;
        let label = cstr(&arg[..256]).map_err(|_| Errno::EINVAL)?;
        return Ok(Request::SetLabel { label });
    }
    if is("BTRFS_IOC_GET_FEATURES") {
        return Ok(Request::GetFeatures);
    }
    if is("BTRFS_IOC_GET_SUPPORTED_FEATURES") {
        return Ok(Request::GetSupportedFeatures);
    }
    if is("BTRFS_IOC_SET_FEATURES") {
        need(arg, 48)?;
        let f = [le64(arg, 0), le64(arg, 8), le64(arg, 16)];
        let m = [le64(arg, 24), le64(arg, 32), le64(arg, 40)];
        return Ok(Request::SetFeatures { flags: f, mask: m });
    }
    if is("BTRFS_IOC_GET_SUBVOL_INFO") {
        return Ok(Request::GetSubvolInfo);
    }
    if is("BTRFS_IOC_GET_SUBVOL_ROOTREF") {
        need(arg, btrfs_ioctl_get_subvol_rootref_args::SIZE_OF)?;
        return Ok(Request::GetSubvolRootref { min_treeid: le64(arg, btrfs_ioctl_get_subvol_rootref_args::MIN_TREEID) });
    }
    if is("BTRFS_IOC_GET_DEV_STATS") {
        need(arg, btrfs_ioctl_get_dev_stats::SIZE_OF)?;
        return Ok(Request::GetDevStats { devid: le64(arg, btrfs_ioctl_get_dev_stats::DEVID), flags: le64(arg, btrfs_ioctl_get_dev_stats::FLAGS) });
    }
    if is("BTRFS_IOC_SYNC") {
        return Ok(Request::Sync);
    }
    if is("BTRFS_IOC_START_SYNC") {
        return Ok(Request::StartSync);
    }
    if is("BTRFS_IOC_WAIT_SYNC") {
        need(arg, 8)?;
        return Ok(Request::WaitSync { transid: le64(arg, 0) });
    }
    if is("BTRFS_IOC_SUBVOL_SYNC_WAIT") {
        need(arg, 16)?;
        return Ok(Request::SubvolSyncWait { subvolid: le64(arg, 0), mode: le32(arg, 8) });
    }
    if is("BTRFS_IOC_SEND") {
        need(arg, btrfs_ioctl_send_args::SIZE_OF)?;
        let flags = le64(arg, btrfs_ioctl_send_args::FLAGS);
        if flags & !0x1f != 0 {
            return Err(Errno::EOPNOTSUPP);
        }
        return Ok(Request::Send {
            send_fd: le64(arg, btrfs_ioctl_send_args::SEND_FD) as i64,
            clone_sources_count: le64(arg, btrfs_ioctl_send_args::CLONE_SOURCES_COUNT),
            parent_root: le64(arg, btrfs_ioctl_send_args::PARENT_ROOT),
            flags,
            version: le32(arg, btrfs_ioctl_send_args::VERSION),
        });
    }
    if is("BTRFS_IOC_SET_RECEIVED_SUBVOL") {
        need(arg, btrfs_ioctl_received_subvol_args::SIZE_OF)?;
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&arg[btrfs_ioctl_received_subvol_args::UUID..btrfs_ioctl_received_subvol_args::UUID + 16]);
        let st = btrfs_ioctl_received_subvol_args::STIME;
        return Ok(Request::SetReceivedSubvol {
            uuid,
            stransid: le64(arg, btrfs_ioctl_received_subvol_args::STRANSID),
            stime_sec: le64(arg, st),
            stime_nsec: le32(arg, st + 8),
            flags: le64(arg, btrfs_ioctl_received_subvol_args::FLAGS),
        });
    }
    if is("BTRFS_IOC_DEV_REPLACE") {
        need(arg, btrfs_ioctl_dev_replace_args::SIZE_OF)?;
        let cmd_v = le64(arg, btrfs_ioctl_dev_replace_args::CMD);
        if cmd_v > 2 {
            return Err(Errno::EINVAL);
        }
        let so = btrfs_ioctl_dev_replace_args::START;
        let sp = btrfs_ioctl_dev_replace_start_params::SRCDEV_NAME;
        let tp = btrfs_ioctl_dev_replace_start_params::TGTDEV_NAME;
        let (src_name, tgt_name) = if cmd_v == 0 {
            (cstr(&arg[so + sp..so + sp + 1024])?, cstr(&arg[so + tp..so + tp + 1024])?)
        } else {
            (Vec::new(), Vec::new())
        };
        return Ok(Request::DevReplace {
            cmd: cmd_v,
            srcdevid: le64(arg, so + btrfs_ioctl_dev_replace_start_params::SRCDEVID),
            src_name,
            tgt_name,
            mode: le64(arg, so + btrfs_ioctl_dev_replace_start_params::CONT_READING_FROM_SRCDEV_MODE),
        });
    }
    if is("FS_IOC_FIEMAP") {
        need(arg, fiemap::SIZE_OF)?;
        return Ok(Request::Fiemap(FiemapReq {
            start: le64(arg, fiemap::FM_START),
            length: le64(arg, fiemap::FM_LENGTH),
            flags: le32(arg, fiemap::FM_FLAGS),
            extent_count: le32(arg, fiemap::FM_EXTENT_COUNT),
        }));
    }
    if is("FS_IOC_GETFLAGS") {
        return Ok(Request::GetFlags);
    }
    if is("FS_IOC_SETFLAGS") {
        need(arg, 4)?;
        return Ok(Request::SetFlags { flags: le32(arg, 0) });
    }
    if is("BTRFS_IOC_SHUTDOWN") {
        need(arg, 4)?;
        return Ok(Request::Shutdown { flags: le32(arg, 0) });
    }
    let _ = known;
    Ok(Request::Other { cmd, arg: arg.to_vec() })
}

// ---------------------------------------------------------------------------------------------
// encoding

/// Writes `reply` into the caller's buffer for `cmd` (the layout the kernel would have produced).
pub fn encode(cmd: u32, req: &Request, reply: &Reply, arg: &mut [u8]) -> Result<(), Errno> {
    match (reply, req) {
        (Reply::Done, _) => Ok(()),
        (Reply::Value(v), _) => {
            if ioc_dir(cmd) & IOC_READ != 0 {
                if ioc_size(cmd) == 4 && arg.len() >= 4 {
                    put32(arg, 0, *v as u32);
                } else if arg.len() >= 8 {
                    put64(arg, 0, *v);
                } else {
                    return Err(Errno::EFAULT);
                }
            }
            Ok(())
        }
        (Reply::FsInfo(f), _) => {
            need(arg, btrfs_ioctl_fs_info_args::SIZE_OF)?;
            let a = btrfs_ioctl_fs_info_args::SIZE_OF;
            for b in &mut arg[..a] {
                *b = 0;
            }
            put64(arg, btrfs_ioctl_fs_info_args::MAX_ID, f.max_id);
            put64(arg, btrfs_ioctl_fs_info_args::NUM_DEVICES, f.num_devices);
            arg[btrfs_ioctl_fs_info_args::FSID..btrfs_ioctl_fs_info_args::FSID + 16].copy_from_slice(&f.fsid);
            put32(arg, btrfs_ioctl_fs_info_args::NODESIZE, f.nodesize);
            put32(arg, btrfs_ioctl_fs_info_args::SECTORSIZE, f.sectorsize);
            put32(arg, btrfs_ioctl_fs_info_args::CLONE_ALIGNMENT, f.clone_alignment);
            let flags_in = if let Request::FsInfo { flags } = req { *flags } else { 0 };
            put16(arg, btrfs_ioctl_fs_info_args::CSUM_TYPE, f.csum_type);
            put16(arg, btrfs_ioctl_fs_info_args::CSUM_SIZE, f.csum_size);
            // flags out: which optional fields were filled (CSUM_INFO=1, GENERATION=2, METADATA_UUID=4)
            put64(arg, btrfs_ioctl_fs_info_args::FLAGS, flags_in & 7);
            put64(arg, btrfs_ioctl_fs_info_args::GENERATION, f.generation);
            arg[btrfs_ioctl_fs_info_args::METADATA_UUID..btrfs_ioctl_fs_info_args::METADATA_UUID + 16].copy_from_slice(&f.metadata_uuid);
            Ok(())
        }
        (Reply::DevInfo(d), _) => {
            need(arg, btrfs_ioctl_dev_info_args::SIZE_OF)?;
            put64(arg, btrfs_ioctl_dev_info_args::DEVID, d.devid);
            arg[btrfs_ioctl_dev_info_args::UUID..btrfs_ioctl_dev_info_args::UUID + 16].copy_from_slice(&d.uuid);
            put64(arg, btrfs_ioctl_dev_info_args::BYTES_USED, d.bytes_used);
            put64(arg, btrfs_ioctl_dev_info_args::TOTAL_BYTES, d.total_bytes);
            let p = btrfs_ioctl_dev_info_args::PATH;
            for b in &mut arg[p..p + 1024] {
                *b = 0;
            }
            let n = d.path.len().min(1023);
            arg[p..p + n].copy_from_slice(&d.path[..n]);
            Ok(())
        }
        (Reply::SpaceInfo(v), Request::SpaceInfo { slots }) => {
            let hdr = btrfs_ioctl_space_args::SIZE_OF;
            need(arg, hdr)?;
            put64(arg, btrfs_ioctl_space_args::TOTAL_SPACES, v.len() as u64);
            let each = btrfs_ioctl_space_info::SIZE_OF;
            let n = core::cmp::min(*slots as usize, v.len());
            need(arg, hdr + n * each)?;
            for (i, (f, t, u)) in v.iter().take(n).enumerate() {
                let o = hdr + i * each;
                put64(arg, o, *f);
                put64(arg, o + 8, *t);
                put64(arg, o + 16, *u);
            }
            Ok(())
        }
        (Reply::Dedupe(res), _) => {
            let hdr = file_dedupe_range::SIZE_OF;
            let info = file_dedupe_range_info::SIZE_OF;
            for (i, r) in res.iter().enumerate() {
                let o = hdr + i * info;
                need(arg, o + info)?;
                put64(arg, o + file_dedupe_range_info::BYTES_DEDUPED, r.bytes);
                put32(arg, o + file_dedupe_range_info::STATUS, r.status as u32);
            }
            Ok(())
        }
        (Reply::Scrub(p), _) => {
            need(arg, btrfs_ioctl_scrub_args::SIZE_OF)?;
            let o = btrfs_ioctl_scrub_args::PROGRESS;
            for (i, v) in p.counters.iter().enumerate() {
                put64(arg, o + i * 8, *v);
            }
            Ok(())
        }
        (Reply::Balance { flags, state, data, meta, sys, stat }, _) => {
            need(arg, btrfs_ioctl_balance_args::SIZE_OF)?;
            put64(arg, btrfs_ioctl_balance_args::FLAGS, *flags);
            put64(arg, btrfs_ioctl_balance_args::STATE, *state);
            put_balance_args(arg, btrfs_ioctl_balance_args::DATA, data);
            put_balance_args(arg, btrfs_ioctl_balance_args::META, meta);
            put_balance_args(arg, btrfs_ioctl_balance_args::SYS, sys);
            let s = btrfs_ioctl_balance_args::STAT;
            for (i, v) in stat.iter().enumerate() {
                put64(arg, s + i * 8, *v);
            }
            Ok(())
        }
        (Reply::TreeSearch(items), Request::TreeSearch { key, buf_size }) => {
            let k = btrfs_ioctl_search_key::SIZE_OF;
            let buf_start = if cmd == c("BTRFS_IOC_TREE_SEARCH") { k } else { btrfs_ioctl_search_args_v2::SIZE_OF };
            let hdr = btrfs_ioctl_search_header::SIZE_OF;
            need(arg, buf_start + (*buf_size).min(arg.len().saturating_sub(buf_start)))?;
            let mut pos = 0usize;
            let mut n = 0u32;
            for it in items {
                if n >= key.nr_items {
                    break;
                }
                let need_bytes = hdr + it.data.len();
                if pos + need_bytes > *buf_size {
                    if n == 0 {
                        return Err(Errno(75)); // EOVERFLOW: a single item does not fit
                    }
                    break;
                }
                let o = buf_start + pos;
                put64(arg, o + btrfs_ioctl_search_header::TRANSID, it.transid);
                put64(arg, o + btrfs_ioctl_search_header::OBJECTID, it.objectid);
                put64(arg, o + btrfs_ioctl_search_header::OFFSET, it.offset);
                put32(arg, o + btrfs_ioctl_search_header::TYPE, it.ty);
                put32(arg, o + btrfs_ioctl_search_header::LEN, it.data.len() as u32);
                arg[o + hdr..o + hdr + it.data.len()].copy_from_slice(&it.data);
                pos += need_bytes;
                n += 1;
            }
            put32(arg, btrfs_ioctl_search_key::NR_ITEMS, n);
            Ok(())
        }
        (Reply::Path { treeid, path }, Request::InoLookup { .. }) => {
            need(arg, btrfs_ioctl_ino_lookup_args::SIZE_OF)?;
            put64(arg, btrfs_ioctl_ino_lookup_args::TREEID, *treeid);
            let p = btrfs_ioctl_ino_lookup_args::NAME;
            let max = btrfs_ioctl_ino_lookup_args::SIZE_OF - p;
            if path.len() + 1 > max {
                return Err(Errno::ENAMETOOLONG);
            }
            for b in &mut arg[p..p + max] {
                *b = 0;
            }
            arg[p..p + path.len()].copy_from_slice(path);
            Ok(())
        }
        (Reply::Path { treeid, path }, Request::InoLookupUser { .. }) => {
            need(arg, btrfs_ioctl_ino_lookup_user_args::SIZE_OF)?;
            put64(arg, btrfs_ioctl_ino_lookup_user_args::TREEID, *treeid);
            let p = btrfs_ioctl_ino_lookup_user_args::PATH;
            let max = btrfs_ioctl_ino_lookup_user_args::SIZE_OF - p;
            if path.len() + 1 > max {
                return Err(Errno::ENAMETOOLONG);
            }
            for b in &mut arg[p..p + max] {
                *b = 0;
            }
            arg[p..p + path.len()].copy_from_slice(path);
            Ok(())
        }
        (Reply::Label(l), _) => {
            need(arg, 256)?;
            for b in &mut arg[..256] {
                *b = 0;
            }
            let n = l.len().min(255);
            arg[..n].copy_from_slice(&l[..n]);
            Ok(())
        }
        (Reply::Features(f), _) => {
            need(arg, 24)?;
            for (i, v) in f.iter().enumerate() {
                put64(arg, i * 8, *v);
            }
            Ok(())
        }
        (Reply::SubvolInfo(s), _) => {
            use crate::ioctl_abi::layout::btrfs_ioctl_get_subvol_info_args as A;
            need(arg, A::SIZE_OF)?;
            for b in &mut arg[..A::SIZE_OF] {
                *b = 0;
            }
            put64(arg, A::TREEID, s.treeid);
            let n = s.name.len().min(255);
            arg[A::NAME..A::NAME + n].copy_from_slice(&s.name[..n]);
            put64(arg, A::PARENT_ID, s.parent_id);
            put64(arg, A::DIRID, s.dirid);
            put64(arg, A::GENERATION, s.generation);
            put64(arg, A::FLAGS, s.flags);
            arg[A::UUID..A::UUID + 16].copy_from_slice(&s.uuid);
            arg[A::PARENT_UUID..A::PARENT_UUID + 16].copy_from_slice(&s.parent_uuid);
            arg[A::RECEIVED_UUID..A::RECEIVED_UUID + 16].copy_from_slice(&s.received_uuid);
            put64(arg, A::CTRANSID, s.transids[0]);
            put64(arg, A::OTRANSID, s.transids[1]);
            put64(arg, A::STRANSID, s.transids[2]);
            put64(arg, A::RTRANSID, s.transids[3]);
            Ok(())
        }
        (Reply::RootRefs(v), _) => {
            use crate::ioctl_abi::layout::btrfs_ioctl_get_subvol_rootref_args as A;
            need(arg, A::SIZE_OF)?;
            let n = v.len().min(255);
            for (i, (t, d)) in v.iter().take(n).enumerate() {
                put64(arg, 8 + i * 16, *t);
                put64(arg, 16 + i * 16, *d);
            }
            arg[A::NUM_ITEMS] = n as u8;
            Ok(())
        }
        (Reply::DevStats { values, nr_items }, _) => {
            use crate::ioctl_abi::layout::btrfs_ioctl_get_dev_stats as A;
            need(arg, A::SIZE_OF)?;
            put64(arg, A::NR_ITEMS, *nr_items);
            for (i, v) in values.iter().enumerate() {
                put64(arg, A::VALUES + i * 8, *v);
            }
            Ok(())
        }
        (Reply::Rescan { flags, progress }, _) => {
            use crate::ioctl_abi::layout::btrfs_ioctl_quota_rescan_args as A;
            need(arg, A::SIZE_OF)?;
            put64(arg, A::FLAGS, *flags);
            put64(arg, A::PROGRESS, *progress);
            Ok(())
        }
        (Reply::Received { rtransid, rtime_sec, rtime_nsec }, _) => {
            use crate::ioctl_abi::layout::btrfs_ioctl_received_subvol_args as A;
            need(arg, A::SIZE_OF)?;
            put64(arg, A::RTRANSID, *rtransid);
            put64(arg, A::RTIME, *rtime_sec);
            put32(arg, A::RTIME + 8, *rtime_nsec);
            Ok(())
        }
        (Reply::Fiemap { mapped, extents }, Request::Fiemap(rq)) => {
            use crate::ioctl_abi::layout::{fiemap as F, fiemap_extent as E};
            need(arg, F::SIZE_OF)?;
            put32(arg, F::FM_MAPPED_EXTENTS, *mapped);
            let n = core::cmp::min(rq.extent_count as usize, extents.len());
            need(arg, F::SIZE_OF + n * E::SIZE_OF)?;
            for (i, e) in extents.iter().take(n).enumerate() {
                let o = F::SIZE_OF + i * E::SIZE_OF;
                put64(arg, o + E::FE_LOGICAL, e.logical);
                put64(arg, o + E::FE_PHYSICAL, e.physical);
                put64(arg, o + E::FE_LENGTH, e.length);
                put32(arg, o + E::FE_FLAGS, e.flags);
            }
            Ok(())
        }
        (Reply::DevReplace { result, status }, _) => {
            use crate::ioctl_abi::layout::btrfs_ioctl_dev_replace_args as A;
            need(arg, A::SIZE_OF)?;
            put64(arg, A::RESULT, *result);
            for (i, v) in status.iter().enumerate() {
                put64(arg, A::START + i * 8, *v);
            }
            Ok(())
        }
        (Reply::Raw(b), _) => {
            let n = b.len().min(arg.len());
            arg[..n].copy_from_slice(&b[..n]);
            Ok(())
        }
        (Reply::Paths(p), Request::InoPaths { size, .. }) => {
            // btrfs_data_container: 16-byte header, then u64 offsets of NUL-terminated strings
            // packed from the end of the buffer; we emit the documented layout.
            let size = (*size as usize).min(arg.len().saturating_sub(btrfs_ioctl_ino_path_args::SIZE_OF));
            let base = btrfs_ioctl_ino_path_args::SIZE_OF;
            for b in &mut arg[base..base + size] {
                *b = 0;
            }
            let mut strings_end = size;
            let mut elems = 0u32;
            let mut missed = 0u32;
            let mut missing = 0usize;
            for (i, s) in p.iter().enumerate() {
                let slot = 16 + i * 8;
                let new_end = strings_end.checked_sub(s.len() + 1);
                match new_end {
                    Some(ne) if slot + 8 <= ne => {
                        arg[base + ne..base + ne + s.len()].copy_from_slice(s);
                        put64(arg, base + slot, ne as u64);
                        strings_end = ne;
                        elems += 1;
                    }
                    _ => {
                        missed += 1;
                        missing += s.len() + 1 + 8;
                    }
                }
            }
            put32(arg, base, (strings_end.saturating_sub(16 + elems as usize * 8)) as u32);
            put32(arg, base + 4, missing as u32);
            put32(arg, base + 8, elems);
            put32(arg, base + 12, missed);
            // fspath points at the container: here, its offset inside the argument
            put64(arg, btrfs_ioctl_ino_path_args::FSPATH, base as u64);
            Ok(())
        }
        (Reply::Inodes(v), Request::LogicalIno { size, .. }) => {
            let base = btrfs_ioctl_logical_ino_args::SIZE_OF;
            let size = (*size as usize).min(arg.len().saturating_sub(base));
            for b in &mut arg[base..base + size] {
                *b = 0;
            }
            let fit = size.saturating_sub(16) / 24;
            let n = v.len().min(fit);
            for (i, (ino, off, root)) in v.iter().take(n).enumerate() {
                let o = base + 16 + i * 24;
                put64(arg, o, *ino);
                put64(arg, o + 8, *off);
                put64(arg, o + 16, *root);
            }
            put32(arg, base, 0);
            put32(arg, base + 4, ((v.len() - n) * 24) as u32);
            put32(arg, base + 8, (n * 3) as u32);
            put32(arg, base + 12, ((v.len() - n) * 3) as u32);
            put64(arg, btrfs_ioctl_logical_ino_args::INODES, base as u64);
            Ok(())
        }
        _ => Err(Errno::EINVAL),
    }
}

/// Kernel entry point: decode `arg`, run it on `backend`, write the reply into `arg`.
/// Returns the scalar the syscall should return (0 on success).
pub fn dispatch<B: Backend>(backend: &mut B, cmd: u32, arg: &mut [u8]) -> Result<i64, Errno> {
    let req = decode(cmd, arg)?;
    let reply = backend.ioctl(req.clone())?;
    encode(cmd, &req, &reply, arg)?;
    Ok(0)
}

/// FS_IOC_GETFLAGS bits (`FS_*_FL`) for a btrfs inode-flags word.
pub fn inode_flags_to_fs_flags(btrfs_flags: u64, is_dir: bool) -> u32 {
    use crate::consts::*;
    let mut f = 0u32;
    if btrfs_flags & INODE_SYNC != 0 {
        f |= 0x0000_0008;
    }
    if btrfs_flags & INODE_IMMUTABLE != 0 {
        f |= 0x0000_0010;
    }
    if btrfs_flags & INODE_APPEND != 0 {
        f |= 0x0000_0020;
    }
    if btrfs_flags & INODE_NODUMP != 0 {
        f |= 0x0000_0040;
    }
    if btrfs_flags & INODE_NOATIME != 0 {
        f |= 0x0000_0080;
    }
    if btrfs_flags & INODE_DIRSYNC != 0 && is_dir {
        f |= 0x0001_0000;
    }
    if btrfs_flags & INODE_COMPRESS != 0 {
        f |= 0x0000_0004;
    }
    if btrfs_flags & INODE_NOCOMPRESS != 0 {
        f |= 0x0400_0000;
    }
    if btrfs_flags & INODE_NODATACOW != 0 {
        f |= 0x0080_0000;
    }
    f
}

/// Reverse of [`inode_flags_to_fs_flags`].
pub fn fs_flags_to_inode_flags(fs: u32) -> u64 {
    use crate::consts::*;
    let mut f = 0u64;
    for (bit, v) in [(0x8u32, INODE_SYNC), (0x10, INODE_IMMUTABLE), (0x20, INODE_APPEND), (0x40, INODE_NODUMP), (0x80, INODE_NOATIME), (0x1_0000, INODE_DIRSYNC), (0x4, INODE_COMPRESS), (0x400_0000, INODE_NOCOMPRESS), (0x80_0000, INODE_NODATACOW)] {
        if fs & bit != 0 {
            f |= v;
        }
    }
    f
}

/// Convenience: a label string from a reply buffer.
pub fn label_string(l: &[u8]) -> String {
    String::from_utf8_lossy(l).into_owned()
}
