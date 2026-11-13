/* Prism's C runtime (the part of libSystem written in C). Built freestanding for x86_64 and arm64
 * macOS; the only kernel interface is the Darwin system call, which LoOS Prism translates.
 * Types and layouts follow the macOS SDK ABI (FILE, _RuneLocale, struct tm, ...). */
#pragma once
#include <stddef.h>
#include <stdint.h>
#include <stdarg.h>

typedef long ssize_t;
typedef long long off_t;
typedef long long fpos_t;
typedef unsigned short mode_t;
typedef int pid_t;
typedef unsigned int uid_t, gid_t;
typedef long time_t;
typedef int wint_t;
typedef int rune_t;

#define EOF (-1)
#define LONG_MAX 0x7fffffffffffffffL
#define LONG_MIN (-LONG_MAX - 1)
#define INT_MAX 0x7fffffff

/* errno values (Darwin) */
#define EPERM 1
#define ENOENT 2
#define EINTR 4
#define EIO 5
#define EBADF 9
#define ENOMEM 12
#define EACCES 13
#define EFAULT 14
#define EEXIST 17
#define ENOTDIR 20
#define EISDIR 21
#define EINVAL 22
#define ENOSPC 28
#define ESPIPE 29
#define ERANGE 34
#define EAGAIN 35
#define ENOSYS 78
#define EOVERFLOW 84
#define ELAST 106
int *__error(void);
#define errno (*__error())

/* Darwin open flags */
#define O_RDONLY 0
#define O_WRONLY 1
#define O_RDWR 2
#define O_ACCMODE 3
#define O_NONBLOCK 0x4
#define O_APPEND 0x8
#define O_CREAT 0x200
#define O_TRUNC 0x400
#define O_EXCL 0x800
#define O_CLOEXEC 0x1000000
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

/* Implemented by Prism in Rust (stubs into the kernel side). */
void *malloc(size_t);
void *calloc(size_t, size_t);
void *realloc(void *, size_t);
void free(void *);
void *memcpy(void *, const void *, size_t);
void *memmove(void *, const void *, size_t);
void *memset(void *, int, size_t);
int memcmp(const void *, const void *, size_t);
size_t strlen(const char *);
int strcmp(const char *, const char *);
int strncmp(const char *, const char *, size_t);
char *strchr(const char *, int);
char *strrchr(const char *, int);
char *strdup(const char *);
int vsnprintf(char *, size_t, const char *, va_list);
int snprintf(char *, size_t, const char *, ...);
const char *getprogname(void);
char *getenv(const char *);
void abort(void) __attribute__((noreturn));
void exit(int) __attribute__((noreturn));
long strtol(const char *, char **, int);
unsigned long strtoul(const char *, char **, int);
time_t time(time_t *);
int atexit(void (*)(void));

/* Darwin system calls (sys.c): -1 and errno on failure. */
long __syscall(long n, long a, long b, long c, long d, long e, long f);
#define SYS0(n) __syscall(n, 0, 0, 0, 0, 0, 0)
#define SYS1(n, a) __syscall(n, (long)(a), 0, 0, 0, 0, 0)
#define SYS2(n, a, b) __syscall(n, (long)(a), (long)(b), 0, 0, 0, 0)
#define SYS3(n, a, b, c) __syscall(n, (long)(a), (long)(b), (long)(c), 0, 0, 0)
#define SYS4(n, a, b, c, d) __syscall(n, (long)(a), (long)(b), (long)(c), (long)(d), 0, 0)
#define SYS6(n, a, b, c, d, e, f) __syscall(n, (long)(a), (long)(b), (long)(c), (long)(d), (long)(e), (long)(f))
ssize_t read(int, void *, size_t);
ssize_t write(int, const void *, size_t);
int open(const char *, int, ...);
int close(int);
off_t lseek(int, off_t, int);
int isatty(int);

/* stdio: struct __sFILE exactly as in the macOS SDK (the getc/putc macros read it). */
struct __sbuf {
    unsigned char *_base;
    int _size;
};
typedef struct __sFILE {
    unsigned char *_p;
    int _r;
    int _w;
    short _flags;
    short _file;
    struct __sbuf _bf;
    int _lbfsize;
    void *_cookie;
    int (*_close)(void *);
    int (*_read)(void *, char *, int);
    fpos_t (*_seek)(void *, fpos_t, int);
    int (*_write)(void *, const char *, int);
    struct __sbuf _ub;
    void *_extra;
    int _ur;
    unsigned char _ubuf[3];
    unsigned char _nbuf[1];
    struct __sbuf _lb;
    int _blksize;
    fpos_t _offset;
} FILE;
#define __SLBF 0x0001
#define __SNBF 0x0002
#define __SRD 0x0004
#define __SWR 0x0008
#define __SRW 0x0010
#define __SEOF 0x0020
#define __SERR 0x0040
#define __SMBF 0x0080
#define __SAPP 0x0100
#define __SSTR 0x0200
#define __SALC 0x4000
extern FILE *__stdinp, *__stdoutp, *__stderrp;
#define stdin __stdinp
#define stdout __stdoutp
#define stderr __stderrp
int fflush(FILE *);
int fputs(const char *, FILE *);
int fputc(int, FILE *);
int fprintf(FILE *, const char *, ...);
int vfprintf(FILE *, const char *, va_list);
size_t fwrite(const void *, size_t, size_t, FILE *);
size_t fread(void *, size_t, size_t, FILE *);

char *strerror(int);
int vasprintf(char **, const char *, va_list);

struct tm {
    int tm_sec, tm_min, tm_hour, tm_mday, tm_mon, tm_year, tm_wday, tm_yday, tm_isdst;
    long tm_gmtoff;
    char *tm_zone;
};
