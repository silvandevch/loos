/* POSIX layer on top of the OS backend: file metadata, directories, pipes, processes. */
#include "libc.h"
#include <unistd.h>
#include <fcntl.h>
#include <dirent.h>
#include <errno.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <sys/time.h>

#define SC(n, ...) __syscall_ret(__os_syscall(n, ##__VA_ARGS__, 0, 0, 0, 0, 0, 0))
#define R(x) ((int)(x))
static long sys(long n, long a, long b, long c) { return __syscall_ret(__os_syscall(n, a, b, c, 0, 0, 0)); }

int open(const char *path, int flags, ...)
{
    int mode = 0;
    if (flags & (O_CREAT | 020000000)) { va_list ap; va_start(ap, flags); mode = va_arg(ap, int); va_end(ap); }
    return R(sys(2, (long)path, flags, mode));
}
int creat(const char *p, mode_t m) { return open(p, O_WRONLY | O_CREAT | O_TRUNC, m); }
int fcntl(int fd, int cmd, ...)
{
    va_list ap; va_start(ap, cmd); long arg = va_arg(ap, long); va_end(ap);
    return R(sys(72, fd, cmd, arg));
}
int stat(const char *restrict p, struct stat *restrict st) { return R(sys(4, (long)p, (long)st, 0)); }
int lstat(const char *restrict p, struct stat *restrict st) { return R(sys(6, (long)p, (long)st, 0)); }
int fstat(int fd, struct stat *st) { return R(sys(5, fd, (long)st, 0)); }
int mkdir(const char *p, mode_t m) { return R(sys(83, (long)p, m, 0)); }
int rmdir(const char *p) { return R(sys(84, (long)p, 0, 0)); }
int chmod(const char *p, mode_t m) { return R(sys(90, (long)p, m, 0)); }
mode_t umask(mode_t m) { return (mode_t)__os_syscall(95, m, 0, 0, 0, 0, 0); }
int chdir(const char *p) { return R(sys(80, (long)p, 0, 0)); }
char *getcwd(char *buf, size_t n)
{
    char tmp[4096];
    if (!buf) { buf = malloc(n ? n : sizeof tmp); if (!buf) return 0; if (!n) n = sizeof tmp; }
    long r = sys(79, (long)buf, (long)n, 0);
    (void)tmp;
    return r < 0 ? 0 : buf;
}
int access(const char *p, int m) { return R(sys(21, (long)p, m, 0)); }
int dup(int fd) { return R(sys(32, fd, 0, 0)); }
int dup2(int a, int b) { return R(sys(33, a, b, 0)); }
int pipe(int fds[2]) { return R(sys(22, (long)fds, 0, 0)); }
int fsync(int fd) { return R(sys(74, fd, 0, 0)); }
int ftruncate(int fd, long n) { return R(sys(77, fd, n, 0)); }
int truncate(const char *p, long n) { return R(sys(76, (long)p, n, 0)); }
int symlink(const char *a, const char *b) { return R(sys(88, (long)a, (long)b, 0)); }
int link(const char *a, const char *b) { return R(sys(86, (long)a, (long)b, 0)); }
long readlink(const char *restrict p, char *restrict b, size_t n) { return sys(89, (long)p, (long)b, (long)n); }
int getppid(void) { return R(__os_syscall(110, 0, 0, 0, 0, 0, 0)); }
unsigned getuid(void) { return (unsigned)__os_syscall(102, 0, 0, 0, 0, 0, 0); }
unsigned geteuid(void) { return (unsigned)__os_syscall(107, 0, 0, 0, 0, 0, 0); }
unsigned getgid(void) { return (unsigned)__os_syscall(104, 0, 0, 0, 0, 0, 0); }
unsigned getegid(void) { return (unsigned)__os_syscall(108, 0, 0, 0, 0, 0, 0); }
int usleep(unsigned us) { struct timespec t = { us / 1000000, (long)(us % 1000000) * 1000 }; return R(__syscall_ret(__os_nanosleep(&t, 0))); }
int gettimeofday(struct timeval *restrict tv, void *restrict tz)
{
    (void)tz;
    struct timespec ts;
    if (__os_clock_gettime(0, &ts) < 0) return -1;
    if (tv) { tv->tv_sec = ts.tv_sec; tv->tv_usec = ts.tv_nsec / 1000; }
    return 0;
}
int gethostname(char *buf, size_t n)
{
    char uts[390];                                   /* struct utsname: 6 x 65 bytes; nodename is field 2 */
    if (sys(63, (long)uts, 0, 0) < 0) return -1;
    strlcpy(buf, uts + 65, n);
    return 0;
}
long sysconf(int name)
{
    switch (name) {
    case _SC_PAGESIZE: return 4096;
    case _SC_CLK_TCK: return 100;
    case _SC_NPROCESSORS_ONLN: return 1;
    }
    errno = EINVAL;
    return -1;
}

pid_t fork(void) { return R(__syscall_ret(__os_syscall(57, 0, 0, 0, 0, 0, 0))); }
int execve(const char *p, char *const argv[], char *const envp[]) { return R(sys(59, (long)p, (long)argv, (long)envp)); }
int execv(const char *p, char *const argv[]) { return execve(p, argv, environ); }
int execvp(const char *file, char *const argv[])
{
    if (strchr(file, '/')) return execv(file, argv);
    const char *path = getenv("PATH");
    if (!path) path = "/usr/local/bin:/usr/bin:/bin";
    size_t fl = strlen(file);
    int saw_acces = 0;
    while (*path) {
        size_t dl = strcspn(path, ":");
        char full[4096];
        if (dl + fl + 2 < sizeof full) {
            memcpy(full, path, dl); full[dl] = '/'; memcpy(full + dl + 1, file, fl + 1);
            execv(full, argv);
            if (errno == EACCES) saw_acces = 1; else if (errno != ENOENT && errno != ENOTDIR) return -1;
        }
        path += dl; if (*path == ':') path++;
    }
    errno = saw_acces ? EACCES : ENOENT;
    return -1;
}
pid_t waitpid(pid_t pid, int *st, int opt) { return R(__syscall_ret(__os_syscall(61, pid, (long)st, opt, 0, 0, 0))); }
pid_t wait(int *st) { return waitpid(-1, st, 0); }

/* ---- directories (getdents64) ---- */
struct __loos_dir { int fd; size_t pos, len; struct dirent ent; char buf[4096]; };
DIR *opendir(const char *path)
{
    long fd = __os_open(path, O_RDONLY | O_DIRECTORY | O_CLOEXEC, 0);
    if (fd < 0) { errno = (int)-fd; return 0; }
    DIR *d = calloc(1, sizeof *d);
    if (!d) { __os_close((int)fd); errno = ENOMEM; return 0; }
    d->fd = (int)fd;
    return d;
}
struct dirent *readdir(DIR *d)
{
    if (d->pos >= d->len) {
        long r = __os_syscall(217, d->fd, (long)d->buf, sizeof d->buf, 0, 0, 0);
        if (r < 0) { errno = (int)-r; return 0; }
        if (r == 0) return 0;
        d->pos = 0; d->len = (size_t)r;
    }
    /* kernel record: u64 ino, s64 off, u16 reclen, u8 type, char name[] */
    unsigned char *rec = (unsigned char *)d->buf + d->pos;
    unsigned short reclen; memcpy(&reclen, rec + 16, 2);
    memcpy(&d->ent.d_ino, rec, 8); memcpy(&d->ent.d_off, rec + 8, 8);
    d->ent.d_reclen = sizeof d->ent; d->ent.d_type = rec[18];
    strlcpy(d->ent.d_name, (char *)rec + 19, sizeof d->ent.d_name);
    d->pos += reclen;
    return &d->ent;
}
void rewinddir(DIR *d) { __os_lseek(d->fd, 0, 0); d->pos = d->len = 0; }
int dirfd(DIR *d) { return d->fd; }
int closedir(DIR *d) { int r = R(__syscall_ret(__os_close(d->fd))); free(d); return r; }
