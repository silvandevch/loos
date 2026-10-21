/* OS backend for the Linux x86-64 syscall ABI, which LoOS exposes through its Linux-compat layer
 * (see docs/LINUX-COMPAT.md). This is the only file in the libc that issues system calls. */
#include "libc.h"
#include <errno.h>

static inline long sc6(long n, long a, long b, long c, long d, long e, long f)
{
    long ret;
    register long r10 __asm__("r10") = d;
    register long r8 __asm__("r8") = e;
    register long r9 __asm__("r9") = f;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9)
                     : "rcx", "r11", "memory");
    return ret;
}
#define sc1(n, a) sc6(n, (long)(a), 0, 0, 0, 0, 0)
#define sc2(n, a, b) sc6(n, (long)(a), (long)(b), 0, 0, 0, 0)
#define sc3(n, a, b, c) sc6(n, (long)(a), (long)(b), (long)(c), 0, 0, 0)

enum { SYS_read = 0, SYS_write = 1, SYS_open = 2, SYS_close = 3, SYS_lseek = 8, SYS_mmap = 9, SYS_munmap = 11,
       SYS_rt_sigaction = 13, SYS_ioctl = 16, SYS_nanosleep = 35, SYS_getpid = 39, SYS_kill = 62,
       SYS_rename = 82, SYS_unlink = 87, SYS_clock_gettime = 228, SYS_exit_group = 231 };

long __os_read(int fd, void *b, size_t n) { return sc3(SYS_read, fd, b, n); }
long __os_write(int fd, const void *b, size_t n) { return sc3(SYS_write, fd, b, n); }
long __os_open(const char *p, int fl, int m) { return sc3(SYS_open, p, fl, m); }
long __os_close(int fd) { return sc1(SYS_close, fd); }
long __os_lseek(int fd, long o, int w) { return sc3(SYS_lseek, fd, o, w); }
long __os_unlink(const char *p) { return sc1(SYS_unlink, p); }
long __os_rename(const char *a, const char *b) { return sc2(SYS_rename, a, b); }
long __os_getpid(void) { return sc1(SYS_getpid, 0); }
long __os_kill(long pid, int sig) { return sc2(SYS_kill, pid, sig); }
long __os_clock_gettime(int id, struct timespec *ts) { return sc2(SYS_clock_gettime, id, ts); }
long __os_nanosleep(const struct timespec *r, struct timespec *m) { return sc2(SYS_nanosleep, r, m); }

long __os_isatty(int fd)
{
    char termios[64];                                      /* TCGETS = 0x5401 succeeds only on terminals */
    return sc3(SYS_ioctl, fd, 0x5401, termios) == 0;
}

void *__os_map(size_t bytes)
{
    long r = sc6(SYS_mmap, 0, (long)bytes, 3 /*RW*/, 0x22 /*PRIVATE|ANON*/, -1, 0);
    return (r < 0 && r > -4096) ? (void *)0 : (void *)r;
}
void __os_unmap(void *p, size_t bytes) { sc2(SYS_munmap, p, bytes); }

__attribute__((noreturn)) void __os_exit(int status)
{
    for (;;) sc1(SYS_exit_group, status);
}

/* signals: the kernel needs a restorer that issues rt_sigreturn (syscall 15) */
__asm__(".text\n.type __sigreturn_tramp,@function\n__sigreturn_tramp:\n  mov $15,%eax\n  syscall\n");
extern void __sigreturn_tramp(void);

struct ksigaction { void (*handler)(int); unsigned long flags; void (*restorer)(void); unsigned long mask; };

long __os_sigaction(int sig, void (*handler)(int))
{
    struct ksigaction sa = { handler, 0x04000000UL /*SA_RESTORER*/, __sigreturn_tramp, 0 };
    return sc6(SYS_rt_sigaction, sig, (long)&sa, 0, 8, 0, 0);
}

long __syscall_ret(long r)
{
    if (r < 0 && r > -4096) { errno = (int)-r; return -1; }
    return r;
}

long __os_rmdir(const char *p) { return sc1(84, p); }

/* ---- generic entry for the POSIX layer (src/posix.c): raw syscall with the kernel's -errno convention ---- */
long __os_syscall(long n, long a, long b, long c, long d, long e, long f) { return sc6(n, a, b, c, d, e, f); }
