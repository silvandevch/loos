/* POSIX system-call wrappers: Darwin BSD system calls (carry flag set = errno in the result). */
#include "prism.h"

long __syscall(long n, long a, long b, long c, long d, long e, long f) {
    long ret;
    unsigned char fail;
#if defined(__x86_64__)
    register long r10 __asm__("r10") = d;
    register long r8 __asm__("r8") = e;
    register long r9 __asm__("r9") = f;
    __asm__ volatile("syscall\n setc %1"
                     : "=a"(ret), "=r"(fail), "+d"(c)
                     : "a"(0x2000000 | n), "D"(a), "S"(b), "r"(r10), "r"(r8), "r"(r9)
                     : "rcx", "r11", "memory", "cc");
#else
    register long x0 __asm__("x0") = a;
    register long x1 __asm__("x1") = b;
    register long x2 __asm__("x2") = c;
    register long x3 __asm__("x3") = d;
    register long x4 __asm__("x4") = e;
    register long x5 __asm__("x5") = f;
    register long x16 __asm__("x16") = n;
    __asm__ volatile("svc #0x80\n cset %w1, cs"
                     : "+r"(x0), "=r"(fail), "+r"(x1)
                     : "r"(x2), "r"(x3), "r"(x4), "r"(x5), "r"(x16)
                     : "memory", "cc");
    ret = x0;
#endif
    if (fail) {
        errno = (int)ret;
        return -1;
    }
    return ret;
}

ssize_t read(int fd, void *p, size_t n) { return SYS3(3, fd, p, n); }
ssize_t write(int fd, const void *p, size_t n) { return SYS3(4, fd, p, n); }
int open(const char *path, int flags, ...) {
    va_list ap;
    va_start(ap, flags);
    int mode = (flags & O_CREAT) ? va_arg(ap, int) : 0;
    va_end(ap);
    return (int)SYS3(5, path, flags, mode);
}
int close(int fd) { return (int)SYS1(6, fd); }
off_t lseek(int fd, off_t off, int whence) { return SYS3(199, fd, off, whence); }
int isatty(int fd) {
    char t[64]; /* struct termios; TIOCGETA */
    return SYS3(54, fd, 0x40487413, t) == 0;
}

/* The stack probe: our stacks are mapped up front, so there is nothing to touch. It must preserve
 * every register (callers pass the size in rax / x15). */
#if defined(__x86_64__)
__asm__(".globl ____chkstk_darwin\n____chkstk_darwin:\n ret\n");
#else
__asm__(".globl ____chkstk_darwin\n.p2align 2\n____chkstk_darwin:\n ret\n");
#endif
