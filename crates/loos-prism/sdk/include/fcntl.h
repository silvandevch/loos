#ifndef _FCNTL_H
#define _FCNTL_H
#include <_prism_cdefs.h>
#define O_RDONLY 0x0000
#define O_WRONLY 0x0001
#define O_RDWR 0x0002
#define O_ACCMODE 0x0003
#define O_NONBLOCK 0x0004
#define O_APPEND 0x0008
#define O_SHLOCK 0x0010
#define O_EXLOCK 0x0020
#define O_NOFOLLOW 0x0100
#define O_CREAT 0x0200
#define O_TRUNC 0x0400
#define O_EXCL 0x0800
#define O_NOCTTY 0x20000
#define O_DIRECTORY 0x100000
#define O_SYMLINK 0x200000
#define O_CLOEXEC 0x1000000
#define F_DUPFD 0
#define F_GETFD 1
#define F_SETFD 2
#define F_GETFL 3
#define F_SETFL 4
#define F_GETPATH 50
#define F_DUPFD_CLOEXEC 67
#define FD_CLOEXEC 1
#define AT_FDCWD -2
#define AT_EACCESS 0x0010
#define AT_SYMLINK_NOFOLLOW 0x0020
#define AT_SYMLINK_FOLLOW 0x0040
#define AT_REMOVEDIR 0x0080
__BEGIN_DECLS
int open(const char *, int, ...); int openat(int, const char *, int, ...); int creat(const char *, mode_t);
int fcntl(int, int, ...);
__END_DECLS
#endif
