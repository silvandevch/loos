/* statfs / fstatfs / getfsstat / getmntinfo with Darwin's 64-bit-inode struct statfs (the only
 * layout on arm64; the $INODE64 variant on x86_64). LoOS has one filesystem behind the whole tree,
 * reported like a macOS root volume: APFS, mounted on / from /dev/disk1s1. Sizes come from the
 * Linux layer's statfs. */
#include "prism.h"

long __syscall(long n, long a, long b, long c, long d, long e, long f);
#define lx(nr, a, b) __syscall(0x10000 + (nr), (long)(a), (long)(b), 0, 0, 0, 0)

struct dstatfs {
    uint32_t f_bsize;
    int32_t f_iosize;
    uint64_t f_blocks, f_bfree, f_bavail, f_files, f_ffree;
    int32_t f_fsid[2];
    uint32_t f_owner, f_type, f_flags, f_fssubtype;
    char f_fstypename[16];
    char f_mntonname[1024];
    char f_mntfromname[1024];
    uint32_t f_flags_ext;
    uint32_t f_reserved[7];
};
_Static_assert(sizeof(struct dstatfs) == 2168, "Darwin struct statfs");

/* Linux x86-64 struct statfs (the Linux layer's ABI on both architectures). */
struct lstatfs { long type, bsize, blocks, bfree, bavail, files, ffree; int fsid[2]; long namelen, frsize, flags, spare[4]; };

#define MNT_LOCAL 0x1000
#define MNT_ROOTFS 0x4000
#define MNT_DOVOLFS 0x8000
#define MNT_JOURNALED 0x800000

static void fill(struct dstatfs *d, const struct lstatfs *l) {
    memset(d, 0, sizeof *d);
    d->f_bsize = (uint32_t)l->bsize;
    d->f_iosize = 1 << 20;
    d->f_blocks = (uint64_t)l->blocks;
    d->f_bfree = (uint64_t)l->bfree;
    d->f_bavail = (uint64_t)l->bavail;
    d->f_files = (uint64_t)l->files;
    d->f_ffree = (uint64_t)l->ffree;
    d->f_fsid[0] = 0x1000004; /* dev of /dev/disk1s1 */
    d->f_fsid[1] = 26;        /* the APFS type number */
    d->f_type = 26;
    d->f_flags = MNT_LOCAL | MNT_ROOTFS | MNT_DOVOLFS | MNT_JOURNALED;
    memcpy(d->f_fstypename, "apfs", 5);
    memcpy(d->f_mntonname, "/", 2);
    memcpy(d->f_mntfromname, "/dev/disk1s1", 13);
}

static int root(struct dstatfs *d) {
    struct lstatfs l;
    if (lx(137, "/", &l) < 0) return -1;
    fill(d, &l);
    return 0;
}

int statfs(const char *path, struct dstatfs *d) {
    struct lstatfs l;
    if (lx(137, path, &l) < 0) return -1;
    fill(d, &l);
    return 0;
}
int fstatfs(int fd, struct dstatfs *d) {
    struct lstatfs l;
    if (lx(138, fd, &l) < 0) return -1;
    fill(d, &l);
    return 0;
}
int getfsstat(struct dstatfs *buf, int bytes, int flags) {
    (void)flags;
    if (buf && bytes >= (int)sizeof *buf && root(buf) < 0) return -1;
    return 1;
}

/* getmntinfo returns a buffer it owns (and reuses), as on macOS. */
int getmntinfo(struct dstatfs **out, int flags) {
    static struct dstatfs buf;
    (void)flags;
    if (root(&buf) < 0) return 0;
    *out = &buf;
    return 1;
}
int getmntinfo_r_np(struct dstatfs **out, int flags) {
    (void)flags;
    struct dstatfs *b = malloc(sizeof *b);
    if (!b) return 0;
    if (root(b) < 0) { free(b); return 0; }
    *out = b;
    return 1;
}

#if defined(__x86_64__)
/* x86_64 programs import the 64-bit-inode layout under these names. */
__asm__(".globl _statfs$INODE64\n.set _statfs$INODE64, _statfs\n"
        ".globl _fstatfs$INODE64\n.set _fstatfs$INODE64, _fstatfs\n"
        ".globl _getfsstat$INODE64\n.set _getfsstat$INODE64, _getfsstat\n"
        ".globl _getmntinfo$INODE64\n.set _getmntinfo$INODE64, _getmntinfo\n"
        ".globl _getmntinfo_r_np$INODE64\n.set _getmntinfo_r_np$INODE64, _getmntinfo_r_np\n");
#endif
