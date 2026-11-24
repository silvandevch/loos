#ifndef _SYS_STAT_H
#define _SYS_STAT_H
#include <_prism_cdefs.h>
#include <time.h>
struct stat {
    dev_t st_dev; mode_t st_mode; nlink_t st_nlink; ino_t st_ino; uid_t st_uid; gid_t st_gid; dev_t st_rdev;
    struct timespec st_atimespec, st_mtimespec, st_ctimespec, st_birthtimespec;
    off_t st_size; blkcnt_t st_blocks; blksize_t st_blksize; uint32_t st_flags; uint32_t st_gen;
    int32_t st_lspare; int64_t st_qspare[2];
};
#define st_atime st_atimespec.tv_sec
#define st_mtime st_mtimespec.tv_sec
#define st_ctime st_ctimespec.tv_sec
#define S_IFMT 0170000
#define S_IFIFO 0010000
#define S_IFCHR 0020000
#define S_IFDIR 0040000
#define S_IFBLK 0060000
#define S_IFREG 0100000
#define S_IFLNK 0120000
#define S_IFSOCK 0140000
#define S_ISBLK(m) (((m) & S_IFMT) == S_IFBLK)
#define S_ISCHR(m) (((m) & S_IFMT) == S_IFCHR)
#define S_ISDIR(m) (((m) & S_IFMT) == S_IFDIR)
#define S_ISFIFO(m) (((m) & S_IFMT) == S_IFIFO)
#define S_ISREG(m) (((m) & S_IFMT) == S_IFREG)
#define S_ISLNK(m) (((m) & S_IFMT) == S_IFLNK)
#define S_ISSOCK(m) (((m) & S_IFMT) == S_IFSOCK)
#define S_IRWXU 0000700
#define S_IRUSR 0000400
#define S_IWUSR 0000200
#define S_IXUSR 0000100
#define S_IRWXG 0000070
#define S_IRGRP 0000040
#define S_IWGRP 0000020
#define S_IXGRP 0000010
#define S_IRWXO 0000007
#define S_IROTH 0000004
#define S_IWOTH 0000002
#define S_IXOTH 0000001
#define S_ISUID 0004000
#define S_ISGID 0002000
#define S_ISVTX 0001000
#define UTIME_NOW -1
#define UTIME_OMIT -2
__BEGIN_DECLS
int stat(const char *, struct stat *) __DARWIN_INODE64(stat);
int lstat(const char *, struct stat *) __DARWIN_INODE64(lstat);
int fstat(int, struct stat *) __DARWIN_INODE64(fstat);
int fstatat(int, const char *, struct stat *, int) __DARWIN_INODE64(fstatat);
int chmod(const char *, mode_t); int fchmod(int, mode_t); int fchmodat(int, const char *, mode_t, int);
int mkdir(const char *, mode_t); int mkdirat(int, const char *, mode_t); int mkfifo(const char *, mode_t);
mode_t umask(mode_t); int utimensat(int, const char *, const struct timespec[2], int);
int futimens(int, const struct timespec[2]);
__END_DECLS
#endif
