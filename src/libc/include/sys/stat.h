#ifndef _SYS_STAT_H
#define _SYS_STAT_H
#include "../features.h"
#include "types.h"
#include "../time.h"
__LOOS_BEGIN
struct stat {                       /* matches the kernel's x86-64 struct stat */
    dev_t st_dev; ino_t st_ino; nlink_t st_nlink; mode_t st_mode; uid_t st_uid; gid_t st_gid; int __pad0;
    dev_t st_rdev; off_t st_size; blksize_t st_blksize; blkcnt_t st_blocks;
    struct timespec st_atim, st_mtim, st_ctim; long __unused[3];
};
#define S_IFMT 0170000
#define S_IFSOCK 0140000
#define S_IFLNK 0120000
#define S_IFREG 0100000
#define S_IFBLK 0060000
#define S_IFDIR 0040000
#define S_IFCHR 0020000
#define S_IFIFO 0010000
#define S_ISREG(m) (((m) & S_IFMT) == S_IFREG)
#define S_ISDIR(m) (((m) & S_IFMT) == S_IFDIR)
#define S_ISLNK(m) (((m) & S_IFMT) == S_IFLNK)
#define S_ISCHR(m) (((m) & S_IFMT) == S_IFCHR)
#define S_ISBLK(m) (((m) & S_IFMT) == S_IFBLK)
#define S_ISFIFO(m) (((m) & S_IFMT) == S_IFIFO)
#define S_ISSOCK(m) (((m) & S_IFMT) == S_IFSOCK)
#define S_ISUID 04000
#define S_ISGID 02000
#define S_ISVTX 01000
#define S_IRWXU 0700
#define S_IRUSR 0400
#define S_IWUSR 0200
#define S_IXUSR 0100
#define S_IRWXG 070
#define S_IRGRP 040
#define S_IWGRP 020
#define S_IXGRP 010
#define S_IRWXO 07
#define S_IROTH 04
#define S_IWOTH 02
#define S_IXOTH 01
int stat(const char *restrict, struct stat *restrict);
int lstat(const char *restrict, struct stat *restrict);
int fstat(int, struct stat *);
int mkdir(const char *, mode_t);
int chmod(const char *, mode_t);
mode_t umask(mode_t);
__LOOS_END
#endif
