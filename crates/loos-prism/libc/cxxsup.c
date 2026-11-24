/* Odds and ends the C++ standard library (libc++) imports from libSystem: compiler-rt's 128-bit
 * division, __ulock_wait/__ulock_wake (std::atomic::wait), the *at file calls, statvfs,
 * fcopyfile (std::filesystem::copy_file) and the message-catalog API (std::messages, which finds
 * no catalogs). */
#include "prism.h"

/* ---------------------------------------------------------------- 128-bit division */
typedef unsigned __int128 u128;
typedef __int128 i128;
static u128 udivmod(u128 n, u128 d, u128 *rem) {
    if (d == 0) __builtin_trap();
    if (n < d) { if (rem) *rem = n; return 0; }
    if ((n >> 64) == 0 && (d >> 64) == 0) {
        uint64_t a = (uint64_t)n, b = (uint64_t)d;
        if (rem) *rem = a % b;
        return a / b;
    }
    int shift = __builtin_clzll((uint64_t)(d >> 64) ? (uint64_t)(d >> 64) : 1) + ((d >> 64) ? 0 : 64)
              - (__builtin_clzll((uint64_t)(n >> 64) ? (uint64_t)(n >> 64) : 1) + ((n >> 64) ? 0 : 64));
    u128 q = 0;
    d <<= shift;
    for (; shift >= 0; shift--, d >>= 1) {
        q <<= 1;
        if (n >= d) { n -= d; q |= 1; }
    }
    if (rem) *rem = n;
    return q;
}
u128 __udivti3(u128 n, u128 d) { return udivmod(n, d, 0); }
u128 __umodti3(u128 n, u128 d) { u128 r; udivmod(n, d, &r); return r; }
i128 __divti3(i128 n, i128 d) {
    int neg = (n < 0) != (d < 0);
    u128 q = udivmod(n < 0 ? -(u128)n : (u128)n, d < 0 ? -(u128)d : (u128)d, 0);
    return neg ? -(i128)q : (i128)q;
}
i128 __modti3(i128 n, i128 d) {
    u128 r;
    udivmod(n < 0 ? -(u128)n : (u128)n, d < 0 ? -(u128)d : (u128)d, &r);
    return n < 0 ? -(i128)r : (i128)r;
}

/* ---------------------------------------------------------------- __ulock (Darwin 515 / 516) */
int __ulock_wait(uint32_t op, void *addr, uint64_t value, uint32_t timeout_us) {
    return (int)__syscall(515, op, (long)addr, (long)value, timeout_us, 0, 0);
}
int __ulock_wake(uint32_t op, void *addr, uint64_t wake_value) {
    return (int)__syscall(516, op, (long)addr, (long)wake_value, 0, 0, 0);
}

/* ---------------------------------------------------------------- *at calls (Linux passthrough) */
#define LX(nr) (0x10000 + (nr))
#define AT_FDCWD_DARWIN -2
static long dfd(int fd) { return fd == AT_FDCWD_DARWIN ? -100 : fd; }
int openat(int fd, const char *path, int flags, ...) {
    va_list a; va_start(a, flags); int mode = va_arg(a, int); va_end(a);
    return (int)__syscall(463, fd, (long)path, flags, mode, 0, 0);
}
int unlinkat(int fd, const char *path, int flags) {
    return (int)__syscall(LX(263), dfd(fd), (long)path, (flags & 0x80) ? 0x200 : 0, 0, 0, 0);
}
int fchmodat(int fd, const char *path, int mode, int flags) {
    (void)flags;
    return (int)__syscall(LX(268), dfd(fd), (long)path, mode, 0, 0, 0);
}
int fstat(int, void *);
void *fdopendir_inode64(int) __asm("_fdopendir$INODE64");
#if defined(__aarch64__)
void *fdopendir(int fd) { return fdopendir_inode64(fd); }
#endif

/* statvfs over the Linux statfs (x86-64 layout, the Linux layer's ABI on both architectures). */
struct lstatfs { long type, bsize, blocks, bfree, bavail, files, ffree; int fsid[2]; long namelen, frsize, flags, spare[4]; };
struct statvfs { unsigned long bsize, frsize; unsigned blocks, bfree, bavail, files, ffree, favail; unsigned long fsid, flag, namemax; };
static unsigned clamp32(long v) { return v < 0 ? 0 : v > 0xffffffffL ? 0xffffffffu : (unsigned)v; }
static int to_vfs(long r, struct lstatfs *l, struct statvfs *v) {
    if (r < 0) return -1;
    /* Darwin's 32-bit block counts: scale the block size up until they fit. */
    unsigned long f = l->frsize ? l->frsize : l->bsize;
    long blocks = l->blocks, bfree = l->bfree, bavail = l->bavail;
    while (blocks > 0xffffffffL) { blocks >>= 1; bfree >>= 1; bavail >>= 1; f <<= 1; }
    memset(v, 0, sizeof *v);
    v->bsize = l->bsize; v->frsize = f;
    v->blocks = clamp32(blocks); v->bfree = clamp32(bfree); v->bavail = clamp32(bavail);
    v->files = clamp32(l->files); v->ffree = v->favail = clamp32(l->ffree);
    v->fsid = (unsigned)l->fsid[0]; v->flag = l->flags & 1; v->namemax = l->namelen;
    return 0;
}
int statvfs(const char *path, struct statvfs *v) { struct lstatfs l; return to_vfs(__syscall(LX(137), (long)path, (long)&l, 0, 0, 0, 0), &l, v); }
int fstatvfs(int fd, struct statvfs *v) { struct lstatfs l; return to_vfs(__syscall(LX(138), fd, (long)&l, 0, 0, 0, 0), &l, v); }

/* ---------------------------------------------------------------- copyfile (data only) */
typedef struct _copyfile_state { int unused; } *copyfile_state_t;
copyfile_state_t copyfile_state_alloc(void) { return calloc(1, sizeof(struct _copyfile_state)); }
int copyfile_state_free(copyfile_state_t s) { free(s); return 0; }
ssize_t read(int, void *, size_t);
ssize_t write(int, const void *, size_t);
int fcopyfile(int from, int to, copyfile_state_t s, uint32_t flags) {
    (void)s; (void)flags;
    char buf[65536];
    for (;;) {
        ssize_t n = read(from, buf, sizeof buf);
        if (n < 0) return -1;
        if (n == 0) return 0;
        for (ssize_t done = 0; done < n;) {
            ssize_t w = write(to, buf + done, n - done);
            if (w < 0) return -1;
            done += w;
        }
    }
}

/* ---------------------------------------------------------------- message catalogs: none */
typedef struct __nl_cat_d *nl_catd;
nl_catd catopen(const char *name, int flag) { (void)name; (void)flag; errno = ENOENT; return (nl_catd)-1; }
char *catgets(nl_catd c, int set, int msg, const char *def) { (void)c; (void)set; (void)msg; return (char *)def; }
int catclose(nl_catd c) { (void)c; return 0; }
