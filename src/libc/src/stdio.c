/* stdio: buffered FILE streams over a small ops vtable (fd-backed and memory-backed), plus the scanf engine. */
#include "libc.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <ctype.h>
#include <limits.h>
#include <stdarg.h>
#include <unistd.h>
#include <math.h>

enum { F_READ = 1, F_WRITE = 2, F_EOF = 4, F_ERR = 8, F_OWNBUF = 16, F_APPEND = 32, F_STATIC = 64 };

struct __loos_file {
    int fd;
    unsigned flags;
    int bmode;                                  /* _IOFBF / _IOLBF / _IONBF */
    unsigned char *buf;
    size_t bufsize, rpos, rend, wlen;
    int last;                                   /* 0 idle, 1 reading, 2 writing */
    int unget;                                  /* pushed-back byte or -1 */
    long (*rd)(FILE *, void *, size_t);
    long (*wr)(FILE *, const void *, size_t);
    long (*sk)(FILE *, long, int);
    long (*cl)(FILE *);
    void *mem; size_t memsz, mempos, memlen;    /* memory streams */
    FILE *next, *prev;
    unsigned char nobuf[8];
};

static long fd_rd(FILE *f, void *b, size_t n) { return __os_read(f->fd, b, n); }
static long fd_wr(FILE *f, const void *b, size_t n) { return __os_write(f->fd, b, n); }
static long fd_sk(FILE *f, long o, int w) { return __os_lseek(f->fd, o, w); }
static long fd_cl(FILE *f) { return __os_close(f->fd); }

static unsigned char in_buf[BUFSIZ], out_buf[BUFSIZ], err_buf[1];
static FILE stdin_f = { 0, F_READ | F_STATIC, _IOFBF, in_buf, sizeof in_buf, 0, 0, 0, 0, -1, fd_rd, fd_wr, fd_sk, fd_cl, 0, 0, 0, 0, 0, 0, { 0 } };
static FILE stdout_f = { 1, F_WRITE | F_STATIC, _IOFBF, out_buf, sizeof out_buf, 0, 0, 0, 0, -1, fd_rd, fd_wr, fd_sk, fd_cl, 0, 0, 0, 0, 0, 0, { 0 } };
static FILE stderr_f = { 2, F_WRITE | F_STATIC, _IONBF, err_buf, 1, 0, 0, 0, 0, -1, fd_rd, fd_wr, fd_sk, fd_cl, 0, 0, 0, 0, 0, 0, { 0 } };
FILE *stdin = &stdin_f, *stdout = &stdout_f, *stderr = &stderr_f;
static FILE *open_list;

static void list_add(FILE *f) { f->prev = 0; f->next = open_list; if (open_list) open_list->prev = f; open_list = f; }
static void list_del(FILE *f)
{
    if (f->prev) f->prev->next = f->next; else open_list = f->next;
    if (f->next) f->next->prev = f->prev;
}

void __libc_init_stdio(void)
{
    list_add(&stdin_f); list_add(&stdout_f); list_add(&stderr_f);
    if (__os_isatty(1)) stdout_f.bmode = _IOLBF;
}

static int flush_write(FILE *f)
{
    size_t done = 0;
    while (done < f->wlen) {
        long r = f->wr(f, f->buf + done, f->wlen - done);
        if (r < 0) {
            if (r == -EINTR) continue;
            errno = (int)-r; f->flags |= F_ERR; f->wlen = 0; return EOF;
        }
        done += (size_t)r;
    }
    f->wlen = 0;
    return 0;
}

static int ensure_buf(FILE *f)
{
    if (f->buf) return 0;
    if (f->bmode == _IONBF) { f->buf = f->nobuf; f->bufsize = 1; return 0; }
    f->buf = malloc(BUFSIZ);
    if (!f->buf) { errno = ENOMEM; f->flags |= F_ERR; return EOF; }
    f->bufsize = BUFSIZ;
    f->flags |= F_OWNBUF;
    return 0;
}

static int to_write(FILE *f)
{
    if (!(f->flags & F_WRITE)) { errno = EBADF; f->flags |= F_ERR; return EOF; }
    if (f->last == 1) {
        if (f->rend > f->rpos || f->unget >= 0) f->sk(f, -(long)(f->rend - f->rpos + (f->unget >= 0)), SEEK_CUR);
        f->rpos = f->rend = 0; f->unget = -1;
    }
    if (ensure_buf(f)) return EOF;
    f->last = 2;
    return 0;
}

static int to_read(FILE *f)
{
    if (!(f->flags & F_READ)) { errno = EBADF; f->flags |= F_ERR; return EOF; }
    if (f->last == 2 && flush_write(f)) return EOF;
    if (ensure_buf(f)) return EOF;
    f->last = 1;
    return 0;
}

static int fill(FILE *f)
{
    if (f->rpos < f->rend) return 0;
    if (f == &stdin_f) for (FILE *o = open_list; o; o = o->next) if (o->bmode != _IOFBF && o->wlen) flush_write(o);
    long r;
    do r = f->rd(f, f->buf, f->bufsize); while (r == -EINTR);
    if (r < 0) { errno = (int)-r; f->flags |= F_ERR; return EOF; }
    if (r == 0) { f->flags |= F_EOF; return EOF; }
    f->rpos = 0; f->rend = (size_t)r;
    return 0;
}

int fflush(FILE *f)
{
    if (!f) {
        int rc = 0;
        for (FILE *o = open_list; o; o = o->next) if (o->wlen && flush_write(o)) rc = EOF;
        return rc;
    }
    if (f->last == 2 && f->wlen) return flush_write(f);
    if (f->last == 1 && (f->rend > f->rpos || f->unget >= 0) && f->sk) {
        f->sk(f, -(long)(f->rend - f->rpos + (f->unget >= 0)), SEEK_CUR);
        f->rpos = f->rend = 0; f->unget = -1;
    }
    return 0;
}
void __stdio_flush_all(void) { fflush(NULL); }

int fgetc(FILE *f)
{
    if (to_read(f)) return EOF;
    if (f->unget >= 0) { int c = f->unget; f->unget = -1; return c; }
    if (fill(f)) return EOF;
    return f->buf[f->rpos++];
}
int getc(FILE *f) { return fgetc(f); }
int getchar(void) { return fgetc(stdin); }

int ungetc(int c, FILE *f)
{
    if (c == EOF || to_read(f)) return EOF;
    if (f->rpos > 0) f->buf[--f->rpos] = (unsigned char)c;
    else if (f->unget < 0) f->unget = (unsigned char)c;
    else return EOF;
    f->flags &= ~F_EOF;
    return (unsigned char)c;
}

int fputc(int c, FILE *f)
{
    if (to_write(f)) return EOF;
    unsigned char ch = (unsigned char)c;
    if (f->wlen >= f->bufsize && flush_write(f)) return EOF;
    f->buf[f->wlen++] = ch;
    if (f->bmode == _IONBF || (f->bmode == _IOLBF && ch == '\n') || f->wlen >= f->bufsize) {
        if (flush_write(f)) return EOF;
    }
    return ch;
}
int putc(int c, FILE *f) { return fputc(c, f); }
int putchar(int c) { return fputc(c, stdout); }

size_t fwrite(const void *restrict p, size_t sz, size_t n, FILE *restrict f)
{
    size_t total;
    if (!sz || !n) return 0;
    if (__builtin_mul_overflow(sz, n, &total)) { errno = EOVERFLOW; f->flags |= F_ERR; return 0; }
    if (to_write(f)) return 0;
    const unsigned char *s = p;
    size_t left = total;
    while (left) {
        if (f->wlen == f->bufsize && flush_write(f)) break;
        if (f->wlen == 0 && left >= f->bufsize && f->bmode != _IONBF) {      /* large write: bypass the buffer */
            size_t chunk = left - left % f->bufsize;
            size_t saved = f->wlen;
            const unsigned char *sv = f->buf; (void)sv; (void)saved;
            long r;
            do r = f->wr(f, s, chunk); while (r == -EINTR);
            if (r < 0) { errno = (int)-r; f->flags |= F_ERR; break; }
            s += r; left -= (size_t)r;
            continue;
        }
        size_t room = f->bufsize - f->wlen, k = left < room ? left : room;
        if (f->bmode == _IOLBF) {
            const unsigned char *nl = memrchr(s, '\n', k);
            if (nl) {
                k = (size_t)(nl - s) + 1;
                memcpy(f->buf + f->wlen, s, k); f->wlen += k; s += k; left -= k;
                if (flush_write(f)) break;
                continue;
            }
        }
        memcpy(f->buf + f->wlen, s, k); f->wlen += k; s += k; left -= k;
        if (f->bmode == _IONBF && flush_write(f)) break;
    }
    return (total - left) / sz;
}

size_t fread(void *restrict p, size_t sz, size_t n, FILE *restrict f)
{
    size_t total;
    if (!sz || !n) return 0;
    if (__builtin_mul_overflow(sz, n, &total)) { errno = EOVERFLOW; f->flags |= F_ERR; return 0; }
    if (to_read(f)) return 0;
    unsigned char *d = p;
    size_t left = total;
    if (f->unget >= 0 && left) { *d++ = (unsigned char)f->unget; f->unget = -1; left--; }
    while (left) {
        if (f->rpos == f->rend) {
            if (left >= f->bufsize) {                                   /* large read: straight into the caller's buffer */
                long r;
                do r = f->rd(f, d, left); while (r == -EINTR);
                if (r < 0) { errno = (int)-r; f->flags |= F_ERR; break; }
                if (r == 0) { f->flags |= F_EOF; break; }
                d += r; left -= (size_t)r;
                continue;
            }
            if (fill(f)) break;
        }
        size_t k = f->rend - f->rpos < left ? f->rend - f->rpos : left;
        memcpy(d, f->buf + f->rpos, k);
        f->rpos += k; d += k; left -= k;
    }
    return (total - left) / sz;
}

char *fgets(char *restrict s, int n, FILE *restrict f)
{
    if (n <= 0) return 0;
    int i = 0;
    while (i < n - 1) {
        int c = fgetc(f);
        if (c == EOF) { if (i == 0 || (f->flags & F_ERR)) return 0; break; }
        s[i++] = (char)c;
        if (c == '\n') break;
    }
    s[i] = 0;
    return s;
}
int fputs(const char *restrict s, FILE *restrict f) { size_t n = strlen(s); return fwrite(s, 1, n, f) == n ? 0 : EOF; }
int puts(const char *s) { if (fputs(s, stdout) == EOF) return EOF; return fputc('\n', stdout) == EOF ? EOF : 0; }

ssize_t getdelim(char **restrict lp, size_t *restrict np, int delim, FILE *restrict f)
{
    if (!lp || !np) { errno = EINVAL; return -1; }
    if (!*lp || !*np) { *np = 120; *lp = malloc(*np); if (!*lp) return -1; }
    size_t len = 0;
    for (;;) {
        int c = fgetc(f);
        if (c == EOF) { if (len == 0 || (f->flags & F_ERR)) return -1; break; }
        if (len + 2 > *np) {
            size_t nn = *np * 2;
            char *t = realloc(*lp, nn);
            if (!t) return -1;
            *lp = t; *np = nn;
        }
        (*lp)[len++] = (char)c;
        if (c == delim) break;
    }
    (*lp)[len] = 0;
    return (ssize_t)len;
}
ssize_t getline(char **restrict lp, size_t *restrict np, FILE *restrict f) { return getdelim(lp, np, '\n', f); }

/* ---- positioning ---- */
int fseeko(FILE *f, off_t off, int whence)
{
    if (f->last == 2 && flush_write(f)) return -1;
    if (whence == SEEK_CUR) off -= (off_t)(f->rend - f->rpos + (f->unget >= 0));
    long r = f->sk ? f->sk(f, off, whence) : -ESPIPE;
    if (r < 0) { errno = (int)-r; return -1; }
    f->rpos = f->rend = 0; f->unget = -1; f->last = 0; f->flags &= ~F_EOF;
    return 0;
}
int fseek(FILE *f, long off, int whence) { return fseeko(f, off, whence); }
off_t ftello(FILE *f)
{
    long r = f->sk ? f->sk(f, 0, SEEK_CUR) : -ESPIPE;
    if (r < 0) { errno = (int)-r; return -1; }
    if (f->last == 1) r -= (long)(f->rend - f->rpos + (f->unget >= 0));
    else if (f->last == 2) r += (long)f->wlen;
    return r;
}
long ftell(FILE *f) { return ftello(f); }
void rewind(FILE *f) { fseeko(f, 0, SEEK_SET); f->flags &= ~F_ERR; }
int fgetpos(FILE *restrict f, fpos_t *restrict p) { off_t o = ftello(f); if (o < 0) return -1; *p = o; return 0; }
int fsetpos(FILE *f, const fpos_t *p) { return fseeko(f, (off_t)*p, SEEK_SET); }
void clearerr(FILE *f) { f->flags &= ~(F_EOF | F_ERR); }
int feof(FILE *f) { return !!(f->flags & F_EOF); }
int ferror(FILE *f) { return !!(f->flags & F_ERR); }
int fileno(FILE *f) { if (f->fd < 0) { errno = EBADF; return -1; } return f->fd; }

int setvbuf(FILE *restrict f, char *restrict buf, int mode, size_t size)
{
    if (mode != _IOFBF && mode != _IOLBF && mode != _IONBF) return -1;
    if (f->last == 2 && flush_write(f)) return -1;
    if (f->flags & F_OWNBUF) free(f->buf);
    f->flags &= ~F_OWNBUF;
    f->rpos = f->rend = f->wlen = 0; f->last = 0;
    f->bmode = mode;
    if (buf && size && mode != _IONBF) { f->buf = (unsigned char *)buf; f->bufsize = size; }
    else { f->buf = 0; f->bufsize = 0; }                   /* allocated lazily (or the 1-byte unbuffered slot) */
    return 0;
}
void setbuf(FILE *restrict f, char *restrict buf) { setvbuf(f, buf, buf ? _IOFBF : _IONBF, BUFSIZ); }

/* ---- open / close ---- */
static FILE *new_file(int fd, unsigned flags)
{
    FILE *f = calloc(1, sizeof *f);
    if (!f) return 0;
    f->fd = fd; f->flags = flags; f->bmode = _IOFBF; f->unget = -1;
    f->rd = fd_rd; f->wr = fd_wr; f->sk = fd_sk; f->cl = fd_cl;
    if (__os_isatty(fd)) f->bmode = _IOLBF;
    list_add(f);
    return f;
}
static int parse_mode(const char *m, int *oflags, unsigned *fflags)
{
    int plus = strchr(m, '+') != 0, excl = strchr(m, 'x') != 0;
    switch (*m) {
    case 'r': *oflags = plus ? LO_RDWR : LO_RDONLY; *fflags = F_READ | (plus ? F_WRITE : 0); break;
    case 'w': *oflags = (plus ? LO_RDWR : LO_WRONLY) | LO_CREAT | LO_TRUNC | (excl ? LO_EXCL : 0); *fflags = F_WRITE | (plus ? F_READ : 0); break;
    case 'a': *oflags = (plus ? LO_RDWR : LO_WRONLY) | LO_CREAT | LO_APPEND; *fflags = F_WRITE | F_APPEND | (plus ? F_READ : 0); break;
    default: errno = EINVAL; return -1;
    }
    return 0;
}
FILE *fopen(const char *restrict path, const char *restrict mode)
{
    int of; unsigned ff;
    if (parse_mode(mode, &of, &ff)) return 0;
    long fd = __os_open(path, of, 0666);
    if (fd < 0) { errno = (int)-fd; return 0; }
    FILE *f = new_file((int)fd, ff);
    if (!f) { __os_close((int)fd); errno = ENOMEM; }
    return f;
}
FILE *fdopen(int fd, const char *mode)
{
    int of; unsigned ff;
    if (fd < 0) { errno = EBADF; return 0; }
    if (parse_mode(mode, &of, &ff)) return 0;
    return new_file(fd, ff);
}
int fclose(FILE *f)
{
    int rc = 0;
    if (f->last == 2 && flush_write(f)) rc = EOF;
    if (f->cl && f->cl(f) < 0) rc = EOF;
    list_del(f);
    if (f->flags & F_OWNBUF) free(f->buf);
    if (!(f->flags & F_STATIC)) free(f);
    return rc;
}
FILE *freopen(const char *restrict path, const char *restrict mode, FILE *restrict f)
{
    int of; unsigned ff;
    fflush(f);
    if (parse_mode(mode, &of, &ff)) { fclose(f); return 0; }
    if (!path) { f->flags = (f->flags & F_STATIC) | ff; return f; }
    long fd = __os_open(path, of, 0666);
    if (fd < 0) { errno = (int)-fd; fclose(f); return 0; }
    if (f->cl) f->cl(f);
    f->fd = (int)fd; f->flags = (f->flags & (F_STATIC | F_OWNBUF)) | ff;
    f->rpos = f->rend = f->wlen = 0; f->last = 0; f->unget = -1;
    f->rd = fd_rd; f->wr = fd_wr; f->sk = fd_sk; f->cl = fd_cl;
    return f;
}
int remove(const char *path)
{
    long r = __os_unlink(path);
    if (r == -EISDIR || r == -EPERM) r = __os_rmdir(path);
    return (int)__syscall_ret(r) < 0 ? -1 : 0;
}
int rename(const char *a, const char *b) { return (int)__syscall_ret(__os_rename(a, b)) < 0 ? -1 : 0; }
void perror(const char *s)
{
    int e = errno;
    if (s && *s) fprintf(stderr, "%s: ", s);
    fprintf(stderr, "%s\n", strerror(e));
}
int mkstemp(char *tpl)
{
    size_t n = strlen(tpl);
    if (n < 6 || strcmp(tpl + n - 6, "XXXXXX")) { errno = EINVAL; return -1; }
    static const char cs[] = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    for (int tries = 0; tries < 1000; tries++) {
        unsigned long long r = (unsigned long long)rand() * 2654435761u ^ (unsigned long long)__os_getpid() << 20 ^ (unsigned)tries;
        for (int i = 0; i < 6; i++) { tpl[n - 6 + i] = cs[r % 62]; r /= 62; }
        long fd = __os_open(tpl, LO_RDWR | LO_CREAT | LO_EXCL, 0600);
        if (fd >= 0) return (int)fd;
        if (fd != -EEXIST) { errno = (int)-fd; return -1; }
    }
    errno = EEXIST;
    return -1;
}
char *mktemp(char *tpl) { int fd = mkstemp(tpl); if (fd < 0) { *tpl = 0; return tpl; } __os_close(fd); __os_unlink(tpl); return tpl; }
FILE *tmpfile(void)
{
    char name[] = "/tmp/loos_tmpXXXXXX";
    int fd = mkstemp(name);
    if (fd < 0) return 0;
    __os_unlink(name);
    FILE *f = new_file(fd, F_READ | F_WRITE);
    if (!f) __os_close(fd);
    return f;
}
char *tmpnam(char *s)
{
    static char buf[L_tmpnam];
    char name[] = "/tmp/loos_tmpXXXXXX";
    int fd = mkstemp(name);
    if (fd < 0) return 0;
    __os_close(fd); __os_unlink(name);
    if (!s) s = buf;
    strcpy(s, name);
    return s;
}

/* ---- memory streams (fmemopen) ---- */
static long mem_rd(FILE *f, void *b, size_t n)
{
    if (f->mempos >= f->memlen) return 0;
    size_t k = f->memlen - f->mempos < n ? f->memlen - f->mempos : n;
    memcpy(b, (char *)f->mem + f->mempos, k);
    f->mempos += k;
    return (long)k;
}
static long mem_wr(FILE *f, const void *b, size_t n)
{
    if (f->flags & F_APPEND) f->mempos = f->memlen;
    if (f->mempos >= f->memsz) return -ENOSPC;
    size_t k = f->memsz - f->mempos < n ? f->memsz - f->mempos : n;
    memcpy((char *)f->mem + f->mempos, b, k);
    f->mempos += k;
    if (f->mempos > f->memlen) f->memlen = f->mempos;
    if (f->memlen < f->memsz) ((char *)f->mem)[f->memlen] = 0;
    return (long)k;
}
static long mem_sk(FILE *f, long off, int whence)
{
    long base = whence == SEEK_SET ? 0 : whence == SEEK_CUR ? (long)f->mempos : (long)f->memlen;
    if (base + off < 0 || (size_t)(base + off) > f->memsz) return -EINVAL;
    f->mempos = (size_t)(base + off);
    return (long)f->mempos;
}
static long mem_cl(FILE *f) { (void)f; return 0; }
FILE *fmemopen(void *buf, size_t size, const char *mode)
{
    int of; unsigned ff;
    if (!buf || parse_mode(mode, &of, &ff)) { errno = EINVAL; return 0; }
    FILE *f = new_file(-1, ff);
    if (!f) return 0;
    f->bmode = _IOFBF;
    f->rd = mem_rd; f->wr = mem_wr; f->sk = mem_sk; f->cl = mem_cl;
    f->mem = buf; f->memsz = size;
    if (*mode == 'r') f->memlen = size;
    else if (*mode == 'w') { f->memlen = 0; if (size) ((char *)buf)[0] = 0; }
    else { f->memlen = strnlen(buf, size); f->mempos = f->memlen; }
    return f;
}

/* ---- formatted output front ends ---- */
static void file_sink(void *ctx, const char *s, size_t n) { fwrite(s, 1, n, ctx); }
int vfprintf(FILE *restrict f, const char *restrict fmt, va_list ap)
{
    int r = __vformat(file_sink, f, fmt, ap);
    return (f->flags & F_ERR) ? -1 : r;
}
int vprintf(const char *restrict fmt, va_list ap) { return vfprintf(stdout, fmt, ap); }
int fprintf(FILE *restrict f, const char *restrict fmt, ...) { va_list ap; va_start(ap, fmt); int r = vfprintf(f, fmt, ap); va_end(ap); return r; }
int printf(const char *restrict fmt, ...) { va_list ap; va_start(ap, fmt); int r = vfprintf(stdout, fmt, ap); va_end(ap); return r; }

struct sn_ctx { char *buf; size_t cap, pos; };
static void sn_sink(void *c, const char *s, size_t n)
{
    struct sn_ctx *x = c;
    if (x->pos < x->cap) { size_t k = x->cap - x->pos < n ? x->cap - x->pos : n; memcpy(x->buf + x->pos, s, k); }
    x->pos += n;
}
int vsnprintf(char *restrict buf, size_t n, const char *restrict fmt, va_list ap)
{
    struct sn_ctx c = { buf, n ? n - 1 : 0, 0 };
    int r = __vformat(sn_sink, &c, fmt, ap);
    if (n) buf[c.pos < c.cap ? c.pos : c.cap] = 0;
    return r;
}
int vsprintf(char *restrict buf, const char *restrict fmt, va_list ap) { return vsnprintf(buf, SIZE_MAX / 2, fmt, ap); }
int snprintf(char *restrict buf, size_t n, const char *restrict fmt, ...) { va_list ap; va_start(ap, fmt); int r = vsnprintf(buf, n, fmt, ap); va_end(ap); return r; }
int sprintf(char *restrict buf, const char *restrict fmt, ...) { va_list ap; va_start(ap, fmt); int r = vsnprintf(buf, SIZE_MAX / 2, fmt, ap); va_end(ap); return r; }

static void fd_sink(void *c, const char *s, size_t n)
{
    int fd = *(int *)c;
    while (n) { long r = __os_write(fd, s, n); if (r <= 0) { if (r == -EINTR) continue; return; } s += r; n -= (size_t)r; }
}
int vdprintf(int fd, const char *restrict fmt, va_list ap) { return __vformat(fd_sink, &fd, fmt, ap); }
int dprintf(int fd, const char *restrict fmt, ...) { va_list ap; va_start(ap, fmt); int r = vdprintf(fd, fmt, ap); va_end(ap); return r; }
int vasprintf(char **out, const char *restrict fmt, va_list ap)
{
    va_list ap2;
    va_copy(ap2, ap);
    int n = vsnprintf(0, 0, fmt, ap2);
    va_end(ap2);
    if (n < 0) return -1;
    *out = malloc((size_t)n + 1);
    if (!*out) return -1;
    return vsnprintf(*out, (size_t)n + 1, fmt, ap);
}
int asprintf(char **out, const char *restrict fmt, ...) { va_list ap; va_start(ap, fmt); int r = vasprintf(out, fmt, ap); va_end(ap); return r; }

/* ---- scanf engine ---- */
struct src { int (*get)(void *); void (*unget)(void *, int); void *ctx; size_t n; int eof_hit; };
static int f_get(void *c) { return fgetc(c); }
static void f_unget(void *c, int ch) { ungetc(ch, c); }
static int s_get(void *c) { const unsigned char **p = c; return **p ? *(*p)++ : EOF; }
static void s_unget(void *c, int ch) { if (ch != EOF) (*(const unsigned char **)c)--; }

static int nextc(struct src *s) { int c = s->get(s->ctx); if (c == EOF) s->eof_hit = 1; else s->n++; return c; }
static void backc(struct src *s, int c) { if (c != EOF) { s->unget(s->ctx, c); s->n--; } }

static int scan_float_text(struct src *s, char *buf, size_t cap, int width)
{
    size_t n = 0; int c, state = 0, digits = 0, hex = 0;
    if (width <= 0 || width > (int)cap - 1) width = (int)cap - 1;
    #define TAKE(ch) (buf[n++] = (char)(ch))
    c = nextc(s);
    if ((c == '+' || c == '-') && n < (size_t)width) { TAKE(c); c = nextc(s); }
    if (c == 'i' || c == 'I' || c == 'n' || c == 'N') {
        const char *want = (c == 'i' || c == 'I') ? "infinity" : "nan"; size_t k = 0, minlen = (c == 'i' || c == 'I') ? 3 : 3;
        while (want[k] && c != EOF && tolower(c) == want[k] && n < (size_t)width) { TAKE(c); k++; c = nextc(s); }
        backc(s, c);
        if (k < minlen) return 0;
        buf[n] = 0; return 1;
    }
    if (c == '0' && n < (size_t)width) {
        TAKE(c); digits = 1; c = nextc(s);
        if ((c == 'x' || c == 'X') && n < (size_t)width) { TAKE(c); hex = 1; digits = 0; c = nextc(s); }
    }
    for (; c != EOF && n < (size_t)width; c = nextc(s)) {
        if (state == 0 && (hex ? isxdigit(c) : isdigit(c))) { TAKE(c); digits = 1; }
        else if (state == 0 && c == '.') { TAKE(c); state = 1; }
        else if (state == 1 && (hex ? isxdigit(c) : isdigit(c))) { TAKE(c); digits = 1; }
        else if (state < 2 && digits && (hex ? (c == 'p' || c == 'P') : (c == 'e' || c == 'E'))) {
            TAKE(c); state = 2;
            int d = nextc(s);
            if ((d == '+' || d == '-') && n < (size_t)width) { TAKE(d); } else backc(s, d);
        }
        else if (state == 2 && isdigit(c)) TAKE(c);
        else break;
    }
    backc(s, c);
    buf[n] = 0;
    return digits;
    #undef TAKE
}

static int vscan(struct src *s, const char *fmt, va_list ap)
{
    int assigned = 0;
    for (const char *p = fmt; *p; p++) {
        if (isspace((unsigned char)*p)) {
            int c; do c = nextc(s); while (c != EOF && isspace(c));
            backc(s, c);
            continue;
        }
        if (*p != '%') {
            int c = nextc(s);
            if (c != (unsigned char)*p) { backc(s, c); return (assigned || !s->eof_hit) ? assigned : EOF; }
            continue;
        }
        p++;
        if (*p == '%') {
            int c; do c = nextc(s); while (c != EOF && isspace(c));
            if (c != '%') { backc(s, c); return (assigned || !s->eof_hit) ? assigned : EOF; }
            continue;
        }
        int suppress = 0, width = 0, len = 0;
        if (*p == '*') { suppress = 1; p++; }
        while (isdigit((unsigned char)*p)) { width = width * 10 + (*p++ - '0'); if (width > 1 << 24) width = 1 << 24; }
        for (;; p++) {
            if (*p == 'h') len = len == -1 ? -2 : -1;
            else if (*p == 'l') len = len == 1 ? 2 : 1;
            else if (*p == 'j') len = 3; else if (*p == 'z') len = 4; else if (*p == 't') len = 5; else if (*p == 'L') len = 6;
            else break;
        }
        char conv = *p;
        if (!conv) break;
        if (conv == 'n') {
            if (!suppress) {
                void *t = va_arg(ap, void *);
                switch (len) { case -2: *(signed char *)t = (signed char)s->n; break; case -1: *(short *)t = (short)s->n; break;
                case 1: *(long *)t = (long)s->n; break; case 2: *(long long *)t = (long long)s->n; break; default: *(int *)t = (int)s->n; }
            }
            continue;
        }
        if (conv != 'c' && conv != '[') {
            int c; do c = nextc(s); while (c != EOF && isspace(c));
            backc(s, c);
        }
        if (conv == 'c') {
            int w = width ? width : 1; char *d = suppress ? 0 : va_arg(ap, char *);
            for (int i = 0; i < w; i++) {
                int c = nextc(s);
                if (c == EOF) return (assigned || i) ? assigned + 0 : EOF;
                if (d) d[i] = (char)c;
            }
            if (!suppress) assigned++;
        } else if (conv == 's') {
            char *d = suppress ? 0 : va_arg(ap, char *); int n = 0, c;
            while ((c = nextc(s)) != EOF && !isspace(c) && (!width || n < width)) { if (d) d[n] = (char)c; n++; }
            backc(s, c);
            if (!n) return (assigned || !s->eof_hit) ? assigned : EOF;
            if (d) { d[n] = 0; assigned++; }
        } else if (conv == '[') {
            p++;
            int neg = 0; unsigned char set[256] = {0};
            if (*p == '^') { neg = 1; p++; }
            if (*p == ']') { set[']'] = 1; p++; }
            for (; *p && *p != ']'; p++) {
                if (p[1] == '-' && p[2] && p[2] != ']') { for (int ch = (unsigned char)*p; ch <= (unsigned char)p[2]; ch++) set[ch] = 1; p += 2; }
                else set[(unsigned char)*p] = 1;
            }
            char *d = suppress ? 0 : va_arg(ap, char *); int n = 0, c;
            while ((c = nextc(s)) != EOF && (set[c] != neg) && (!width || n < width)) { if (d) d[n] = (char)c; n++; }
            backc(s, c);
            if (!n) return (assigned || !s->eof_hit) ? assigned : EOF;
            if (d) { d[n] = 0; assigned++; }
        } else if (strchr("diouxXp", conv)) {
            char buf[160]; size_t n = 0; int c, base = conv == 'd' || conv == 'u' ? 10 : conv == 'o' ? 8 : conv == 'i' ? 0 : 16, nd = 0;
            int w = width && width < 150 ? width : 150;
            c = nextc(s);
            if ((c == '+' || c == '-') && (int)n < w) { buf[n++] = (char)c; c = nextc(s); }
            if (c == '0' && (int)n < w && (base == 0 || base == 16)) {
                buf[n++] = (char)c; nd = 1; c = nextc(s);
                if ((c == 'x' || c == 'X') && (int)n < w) { buf[n++] = (char)c; nd = 0; base = 16; c = nextc(s); }
                else if (base == 0) base = 8;
            } else if (base == 0) base = 10;
            for (; c != EOF && (int)n < w; c = nextc(s)) {
                int d = isdigit(c) ? c - '0' : isalpha(c) ? tolower(c) - 'a' + 10 : 99;
                if (d >= base) break;
                buf[n++] = (char)c; nd++;
            }
            backc(s, c);
            buf[n] = 0;
            if (!nd) return (assigned || !s->eof_hit) ? assigned : EOF;
            if (!suppress) {
                char *e; int sg = (conv == 'd' || conv == 'i');
                unsigned long long u = sg ? (unsigned long long)strtoll(buf, &e, base) : strtoull(buf, &e, base);
                void *t = va_arg(ap, void *);
                if (conv == 'p') *(void **)t = (void *)(uintptr_t)u;
                else switch (len) {
                case -2: *(signed char *)t = (signed char)u; break; case -1: *(short *)t = (short)u; break;
                case 1: case 2: case 3: *(long long *)t = (long long)u; break;
                case 4: *(size_t *)t = (size_t)u; break; case 5: *(ptrdiff_t *)t = (ptrdiff_t)u; break;
                default: *(int *)t = (int)u;
                }
                assigned++;
            }
        } else if (strchr("aAeEfFgG", conv)) {
            char buf[512];
            if (!scan_float_text(s, buf, sizeof buf, width)) return (assigned || !s->eof_hit) ? assigned : EOF;
            if (!suppress) {
                void *t = va_arg(ap, void *);
                if (len == 6) *(long double *)t = strtold(buf, 0);
                else if (len == 1) *(double *)t = strtod(buf, 0);
                else *(float *)t = strtof(buf, 0);
                assigned++;
            }
        } else return assigned;
    }
    return assigned;
}
int vfscanf(FILE *restrict f, const char *restrict fmt, va_list ap)
{
    struct src s = { f_get, f_unget, f, 0, 0 };
    return vscan(&s, fmt, ap);
}
int vsscanf(const char *restrict str, const char *restrict fmt, va_list ap)
{
    const unsigned char *p = (const unsigned char *)str;
    struct src s = { s_get, s_unget, &p, 0, 0 };
    return vscan(&s, fmt, ap);
}
int vscanf(const char *restrict fmt, va_list ap) { return vfscanf(stdin, fmt, ap); }
int fscanf(FILE *restrict f, const char *restrict fmt, ...) { va_list ap; va_start(ap, fmt); int r = vfscanf(f, fmt, ap); va_end(ap); return r; }
int scanf(const char *restrict fmt, ...) { va_list ap; va_start(ap, fmt); int r = vfscanf(stdin, fmt, ap); va_end(ap); return r; }
int sscanf(const char *restrict s, const char *restrict fmt, ...) { va_list ap; va_start(ap, fmt); int r = vsscanf(s, fmt, ap); va_end(ap); return r; }

/* unistd shims */
ssize_t read(int fd, void *b, size_t n) { return (ssize_t)__syscall_ret(__os_read(fd, b, n)); }
ssize_t write(int fd, const void *b, size_t n) { return (ssize_t)__syscall_ret(__os_write(fd, b, n)); }
int close(int fd) { return (int)__syscall_ret(__os_close(fd)); }
long lseek(int fd, long o, int w) { return __syscall_ret(__os_lseek(fd, o, w)); }
int unlink(const char *p) { return (int)__syscall_ret(__os_unlink(p)); }
int isatty(int fd) { return (int)__os_isatty(fd); }
int getpid(void) { return (int)__os_getpid(); }
