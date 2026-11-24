/* Prism SDK: the macOS C headers Prism's own C/C++ libraries (libc++) are built against.
 * Layouts and constants follow the Darwin ABI (LP64, x86_64 and arm64). Written for LoOS. */
#ifndef _PRISM_CDEFS_H
#define _PRISM_CDEFS_H
#ifdef __cplusplus
#define __BEGIN_DECLS extern "C" {
#define __END_DECLS }
#else
#define __BEGIN_DECLS
#define __END_DECLS
#endif
#if defined(__x86_64__)
#define __DARWIN_INODE64(sym) __asm("_" #sym "$INODE64")
#define __DARWIN_ALIAS(sym) __asm("_" #sym "$UNIX2003")
#else
#define __DARWIN_INODE64(sym)
#define __DARWIN_ALIAS(sym)
#endif
#define __restrict_ restrict
#include <stddef.h>
#include <stdint.h>
typedef long ssize_t;
typedef long long off_t;
typedef unsigned short mode_t;
typedef int pid_t;
typedef unsigned int uid_t, gid_t, id_t;
typedef long time_t;
typedef int suseconds_t;
typedef unsigned int useconds_t;
typedef int dev_t;
typedef unsigned long long ino_t;
typedef unsigned short nlink_t;
typedef long long blkcnt_t;
typedef int blksize_t;
typedef unsigned int fsblkcnt_t, fsfilcnt_t;
typedef unsigned long clock_t;
typedef int clockid_t;
typedef unsigned int sigset_t;
typedef int wint_t;
typedef int rune_t;
typedef union { char __mbstate8[128]; long long _mbstateL; } __mbstate_t;
typedef __mbstate_t mbstate_t;
typedef struct _xlocale *locale_t;
struct timespec { time_t tv_sec; long tv_nsec; };
struct timeval { time_t tv_sec; suseconds_t tv_usec; };
#endif
