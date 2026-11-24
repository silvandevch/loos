/* statfs, fstatfs, getmntinfo, getfsstat (64-bit-inode struct statfs). No SDK headers. */
struct statfs { unsigned bsize; int iosize; unsigned long blocks, bfree, bavail, files, ffree; int fsid[2];
                unsigned owner, type, flags, fssubtype; char fstypename[16], mntonname[1024], mntfromname[1024]; unsigned flags_ext, reserved[7]; };
#if defined(__x86_64__)
#define I64(n) __asm__("_" #n "$INODE64")
#else
#define I64(n)
#endif
int statfs(const char *, struct statfs *) I64(statfs);
int fstatfs(int, struct statfs *) I64(fstatfs);
int getmntinfo(struct statfs **, int) I64(getmntinfo);
int getfsstat(struct statfs *, int, int) I64(getfsstat);
int open(const char *, int, ...);
int printf(const char *, ...);
int strcmp(const char *, const char *);
int main(void) {
    struct statfs s, f, g, *m;
    int a = statfs("/tmp", &s) == 0, fd = open("/", 0), b = fstatfs(fd, &f) == 0;
    int n = getmntinfo(&m, 2), k = getfsstat(&g, sizeof g, 2);
    printf("fs: statfs=%d fstatfs=%d type=%s on=%s bsize=%d free<=total=%d mnt=%d/%s fsstat=%d/%s\n", a, b, s.fstypename, s.mntonname,
           s.bsize == 4096, s.bfree <= s.blocks && s.blocks > 0, n, n ? m[0].mntonname : "-", k, g.fstypename);
    return 0;
}
