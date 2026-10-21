/* LoOS libc: private interfaces between the library's translation units and the OS backend. */
#ifndef LOOS_LIBC_PRIVATE_H
#define LOOS_LIBC_PRIVATE_H
#include <stddef.h>
#include <stdint.h>
#include <time.h>
#include <stdarg.h>

/* ---- OS backend (src/os_linux.c): every call returns a value >= 0 or -errno ---- */
long __os_read(int fd, void *buf, size_t n);
long __os_write(int fd, const void *buf, size_t n);
long __os_open(const char *path, int flags, int mode);
long __os_close(int fd);
long __os_lseek(int fd, long off, int whence);
long __os_unlink(const char *path);
long __os_rmdir(const char *path);
long __os_syscall(long n, long a, long b, long c, long d, long e, long f);
long __os_rename(const char *from, const char *to);
long __os_isatty(int fd);
long __os_getpid(void);
long __os_kill(long pid, int sig);
long __os_clock_gettime(int id, struct timespec *ts);
long __os_nanosleep(const struct timespec *req, struct timespec *rem);
void *__os_map(size_t bytes);                 /* zeroed, page aligned; NULL on failure */
void __os_unmap(void *p, size_t bytes);
long __os_sigaction(int sig, void (*handler)(int));
__attribute__((noreturn)) void __os_exit(int status);

/* open flags understood by __os_open (LoOS = Linux values) */
#define LO_RDONLY 0
#define LO_WRONLY 1
#define LO_RDWR 2
#define LO_CREAT 0100
#define LO_EXCL 0200
#define LO_TRUNC 01000
#define LO_APPEND 02000

/* errno helper: map a backend result (-errno on failure) to the usual -1/errno convention */
long __syscall_ret(long r);
extern char **__environ_ptr(void);

/* ---- printf / scanf engines shared by the stdio and string front ends ---- */
typedef void (*__fmt_sink)(void *ctx, const char *s, size_t n);
int __vformat(__fmt_sink sink, void *ctx, const char *fmt, va_list ap);

/* strtod core shared with scanf and wcstod */
double __strtod_core(const char *s, char **end, int bits, int *range_err);

/* flush every open stream; used by exit() */
void __stdio_flush_all(void);
void __run_atexit(void);
void __libc_init_env(char **envp);
void __libc_init_stdio(void);

#define PAGE_SIZE 4096UL
#endif
