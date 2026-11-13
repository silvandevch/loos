/* Buffered stdio over the BSD FILE layout macOS programs are compiled against: the SDK's
 * getc_unlocked/putc_unlocked macros use _p/_r/_w/_lbfsize and call __srget/__swbuf. */
#include "prism.h"

#define BUFSIZ 4096
#define _IOFBF 0
#define _IOLBF 1
#define _IONBF 2

static int sread(void *c, char *b, int n) { return (int)read(((FILE *)c)->_file, b, n); }
static int swrite(void *c, const char *b, int n) { return (int)write(((FILE *)c)->_file, b, n); }
static fpos_t sseek(void *c, fpos_t o, int w) { return lseek(((FILE *)c)->_file, o, w); }
static int sclose(void *c) { return close(((FILE *)c)->_file); }

static FILE std_files[3] = {
    {._file = 0, ._flags = __SRD, ._cookie = &std_files[0], ._read = sread, ._write = swrite, ._seek = sseek, ._close = sclose},
    {._file = 1, ._flags = __SWR, ._cookie = &std_files[1], ._read = sread, ._write = swrite, ._seek = sseek, ._close = sclose},
    {._file = 2, ._flags = __SWR | __SNBF, ._cookie = &std_files[2], ._read = sread, ._write = swrite, ._seek = sseek, ._close = sclose},
};
FILE *__stdinp = &std_files[0];
FILE *__stdoutp = &std_files[1];
FILE *__stderrp = &std_files[2];

/* Every stream that may hold buffered output (for fflush(NULL) and exit). */
#define MAX_STREAMS 1024
static FILE *streams[MAX_STREAMS] = {&std_files[0], &std_files[1], &std_files[2]};

static void track(FILE *fp, int on) {
    for (int i = 0; i < MAX_STREAMS; i++) {
        if (on ? streams[i] == NULL : streams[i] == fp) {
            streams[i] = on ? fp : NULL;
            return;
        }
    }
}

/* Give the stream a buffer: line-buffered on terminals (as BSD's __smakebuf). */
static void makebuf(FILE *fp) {
    if (fp->_bf._base)
        return;
    if (fp->_flags & __SNBF) {
        fp->_bf._base = fp->_p = fp->_nbuf;
        fp->_bf._size = 1;
        return;
    }
    unsigned char *b = malloc(BUFSIZ);
    if (!b) {
        fp->_flags |= __SNBF;
        fp->_bf._base = fp->_p = fp->_nbuf;
        fp->_bf._size = 1;
        return;
    }
    fp->_flags |= __SMBF;
    fp->_bf._base = fp->_p = b;
    fp->_bf._size = BUFSIZ;
    if (fp->_file >= 0 && isatty(fp->_file))
        fp->_flags |= __SLBF;
}

int __sflush(FILE *fp) {
    if (!(fp->_flags & __SWR) || !fp->_bf._base)
        return 0;
    unsigned char *p = fp->_bf._base;
    int n = (int)(fp->_p - p);
    fp->_p = p;
    fp->_w = (fp->_flags & (__SLBF | __SNBF)) ? 0 : fp->_bf._size;
    while (n > 0) {
        int t = fp->_write ? fp->_write(fp->_cookie, (char *)p, n) : -1;
        if (t <= 0) {
            fp->_flags |= __SERR;
            return EOF;
        }
        p += t;
        n -= t;
    }
    return 0;
}

/* Switch to writing. Nonzero: the stream cannot be written. */
static int wsetup(FILE *fp) {
    if (!(fp->_flags & __SWR)) {
        if (!(fp->_flags & __SRW))
            return (errno = EBADF), fp->_flags |= __SERR, EOF;
        if (fp->_flags & __SRD) {
            if (fp->_r > 0 && fp->_seek)
                fp->_seek(fp->_cookie, -(fpos_t)fp->_r, SEEK_CUR);
            fp->_r = 0;
            fp->_flags &= ~(__SRD | __SEOF);
        }
        fp->_flags |= __SWR;
    }
    if (!fp->_bf._base)
        makebuf(fp);
    if (fp->_r)
        fp->_r = 0;
    if (fp->_p == fp->_bf._base) {
        if (fp->_flags & __SLBF) {
            fp->_w = 0;
            fp->_lbfsize = -fp->_bf._size;
        } else {
            fp->_w = (fp->_flags & __SNBF) ? 0 : fp->_bf._size;
            fp->_lbfsize = 0;
        }
    }
    return 0;
}

int __swbuf(int c, FILE *fp) {
    if (wsetup(fp))
        return EOF;
    fp->_w = fp->_lbfsize;
    c = (unsigned char)c;
    int n = (int)(fp->_p - fp->_bf._base);
    if (n >= fp->_bf._size) {
        if (__sflush(fp))
            return EOF;
        n = 0;
    }
    fp->_w--;
    *fp->_p++ = (unsigned char)c;
    if (++n == fp->_bf._size || ((fp->_flags & __SLBF) && c == '\n'))
        if (__sflush(fp))
            return EOF;
    return c;
}

static void flush_line_buffered(void) {
    for (int i = 0; i < MAX_STREAMS; i++) {
        FILE *f = streams[i];
        if (f && (f->_flags & (__SLBF | __SWR)) == (__SLBF | __SWR))
            __sflush(f);
    }
}

/* Refill the read buffer. 0, or EOF at end of file / error. */
int __srefill(FILE *fp) {
    if (!(fp->_flags & __SRD)) {
        if (fp->_flags & __SWR) {
            if (__sflush(fp))
                return EOF;
            fp->_flags &= ~__SWR;
            fp->_w = 0;
            fp->_lbfsize = 0;
        } else if (!(fp->_flags & __SRW) && !fp->_read) {
            return (errno = EBADF), fp->_flags |= __SERR, EOF;
        }
        fp->_flags |= __SRD;
    }
    if (fp->_flags & __SEOF)
        return EOF;
    if (!fp->_bf._base)
        makebuf(fp);
    /* Reading from a terminal or an unbuffered stream: show pending prompts first. */
    if (fp->_flags & (__SLBF | __SNBF))
        flush_line_buffered();
    fp->_p = fp->_bf._base;
    int n = fp->_read ? fp->_read(fp->_cookie, (char *)fp->_p, fp->_bf._size) : -1;
    if (n <= 0) {
        fp->_r = 0;
        fp->_flags |= n == 0 ? __SEOF : __SERR;
        return EOF;
    }
    fp->_r = n;
    return 0;
}

int __srget(FILE *fp) {
    if (__srefill(fp) == 0) {
        fp->_r--;
        return *fp->_p++;
    }
    return EOF;
}

#define GETC(fp) (--(fp)->_r < 0 ? __srget(fp) : (int)(*(fp)->_p++))
#define PUTC(c, fp) (--(fp)->_w >= 0 || ((fp)->_w >= (fp)->_lbfsize && (char)(c) != '\n') ? (int)(*(fp)->_p++ = (unsigned char)(c)) : __swbuf((c), (fp)))

static FILE *new_file(int fd, int flags) {
    FILE *fp = calloc(1, sizeof(FILE));
    if (!fp)
        return NULL;
    fp->_file = (short)fd;
    fp->_flags = (short)flags;
    fp->_cookie = fp;
    fp->_read = sread;
    fp->_write = swrite;
    fp->_seek = sseek;
    fp->_close = sclose;
    track(fp, 1);
    return fp;
}

/* fopen mode → open flags and stream flags. */
static int mode_flags(const char *m, int *oflags) {
    int f, o;
    switch (*m++) {
    case 'r': f = __SRD; o = O_RDONLY; break;
    case 'w': f = __SWR; o = O_WRONLY | O_CREAT | O_TRUNC; break;
    case 'a': f = __SWR | __SAPP; o = O_WRONLY | O_CREAT | O_APPEND; break;
    default: errno = EINVAL; return 0;
    }
    for (; *m; m++) {
        if (*m == '+') {
            f = __SRW | (f & __SAPP);
            o = (o & ~O_ACCMODE) | O_RDWR;
        } else if (*m == 'x') {
            o |= O_EXCL;
        } else if (*m == 'e') {
            o |= O_CLOEXEC;
        }
    }
    *oflags = o;
    return f;
}

FILE *fopen(const char *path, const char *mode) {
    int o, f = mode_flags(mode, &o);
    if (!f)
        return NULL;
    int fd = open(path, o, 0666);
    if (fd < 0)
        return NULL;
    FILE *fp = new_file(fd, f);
    if (!fp)
        close(fd);
    return fp;
}

FILE *fdopen(int fd, const char *mode) {
    int o, f = mode_flags(mode, &o);
    return f ? new_file(fd, f) : NULL;
}

static void release(FILE *fp) {
    if (fp->_flags & __SMBF)
        free(fp->_bf._base);
    fp->_bf._base = fp->_p = NULL;
    fp->_bf._size = 0;
    fp->_r = fp->_w = 0;
    fp->_flags = 0;
}

int fclose(FILE *fp) {
    int r = (fp->_flags & __SWR) ? __sflush(fp) : 0;
    if (fp->_close && fp->_close(fp->_cookie) < 0)
        r = EOF;
    release(fp);
    track(fp, 0);
    if (fp < &std_files[0] || fp > &std_files[2])
        free(fp);
    return r;
}

FILE *freopen(const char *path, const char *mode, FILE *fp) {
    int o, f = mode_flags(mode, &o);
    if (!f)
        return NULL;
    if (fp->_flags & __SWR)
        __sflush(fp);
    int want = fp->_file;
    if (fp->_close)
        fp->_close(fp->_cookie);
    release(fp);
    int fd = path ? open(path, o, 0666) : -1;
    if (fd < 0)
        return NULL;
    /* Keep the descriptor number the stream had (freopen("x", "w", stdout) redirects fd 1). */
    if (want >= 0 && fd != want && SYS2(90, fd, want) >= 0) {
        close(fd);
        fd = want;
    }
    fp->_file = (short)fd;
    fp->_flags = (short)f;
    fp->_cookie = fp;
    fp->_read = sread;
    fp->_write = swrite;
    fp->_seek = sseek;
    fp->_close = sclose;
    return fp;
}

int fflush(FILE *fp) {
    if (fp)
        return __sflush(fp);
    int r = 0;
    for (int i = 0; i < MAX_STREAMS; i++)
        if (streams[i] && __sflush(streams[i]))
            r = EOF;
    return r;
}

static void flush_all_at_exit(void) { fflush(NULL); }
__attribute__((constructor)) static void stdio_init(void) { atexit(flush_all_at_exit); }

int setvbuf(FILE *fp, char *buf, int mode, size_t size) {
    if (fp->_flags & __SWR)
        __sflush(fp);
    if (fp->_flags & __SMBF)
        free(fp->_bf._base);
    fp->_flags &= ~(__SLBF | __SNBF | __SMBF);
    fp->_bf._base = fp->_p = NULL;
    fp->_r = fp->_w = fp->_lbfsize = 0;
    if (mode == _IONBF) {
        fp->_flags |= __SNBF;
    } else {
        if (mode == _IOLBF)
            fp->_flags |= __SLBF;
        if (buf && size) {
            fp->_bf._base = fp->_p = (unsigned char *)buf;
            fp->_bf._size = (int)size;
        }
    }
    if (!fp->_bf._base) {
        int lbf = fp->_flags & __SLBF;
        makebuf(fp);
        fp->_flags |= lbf;
        if (mode == _IOFBF)
            fp->_flags &= ~__SLBF;
    }
    return 0;
}
void setbuf(FILE *fp, char *buf) { setvbuf(fp, buf, buf ? _IOFBF : _IONBF, BUFSIZ); }
void setbuffer(FILE *fp, char *buf, int size) { setvbuf(fp, buf, buf ? _IOFBF : _IONBF, (size_t)size); }
int setlinebuf(FILE *fp) { return setvbuf(fp, NULL, _IOLBF, 0); }

int fgetc(FILE *fp) { return GETC(fp); }
int getc(FILE *fp) { return GETC(fp); }
int getc_unlocked(FILE *fp) { return GETC(fp); }
int getchar(void) { return GETC(stdin); }
int getchar_unlocked(void) { return GETC(stdin); }
int fputc(int c, FILE *fp) { return PUTC(c, fp); }
int putc(int c, FILE *fp) { return PUTC(c, fp); }
int putc_unlocked(int c, FILE *fp) { return PUTC(c, fp); }
int putchar(int c) { return PUTC(c, stdout); }
int putchar_unlocked(int c) { return PUTC(c, stdout); }
int getw(FILE *fp) {
    int w;
    return fread(&w, sizeof w, 1, fp) == 1 ? w : EOF;
}
int putw(int w, FILE *fp) { return fwrite(&w, sizeof w, 1, fp) == 1 ? 0 : EOF; }

int ungetc(int c, FILE *fp) {
    if (c == EOF)
        return EOF;
    if (!(fp->_flags & __SRD)) {
        if (fp->_flags & __SWR) {
            if (__sflush(fp))
                return EOF;
            fp->_flags &= ~__SWR;
            fp->_w = 0;
            fp->_lbfsize = 0;
        }
        fp->_flags |= __SRD;
    }
    if (!fp->_bf._base) {
        makebuf(fp);
        fp->_p = fp->_bf._base;
        fp->_r = 0;
    }
    if (fp->_r <= 0)
        fp->_p = fp->_bf._base + fp->_bf._size; /* empty: push back from the end */
    if (fp->_p > fp->_bf._base) {
        *--fp->_p = (unsigned char)c;
    } else if (fp->_r < fp->_bf._size) {
        memmove(fp->_p + 1, fp->_p, (size_t)fp->_r);
        *fp->_p = (unsigned char)c;
    } else {
        return EOF;
    }
    fp->_r = fp->_r < 0 ? 1 : fp->_r + 1;
    fp->_flags &= ~__SEOF;
    return (unsigned char)c;
}

char *fgets(char *buf, int n, FILE *fp) {
    if (n <= 0)
        return NULL;
    char *s = buf;
    while (--n > 0) {
        int c = GETC(fp);
        if (c == EOF) {
            if (s == buf)
                return NULL;
            break;
        }
        *s++ = (char)c;
        if (c == '\n')
            break;
    }
    *s = 0;
    return buf;
}

ssize_t getdelim(char **line, size_t *cap, int delim, FILE *fp) {
    if (!line || !cap)
        return (errno = EINVAL), -1;
    size_t n = 0;
    for (;;) {
        int c = GETC(fp);
        if (c == EOF && n == 0)
            return -1;
        if (n + 2 > *cap || !*line) {
            size_t nc = *cap < 128 ? 128 : *cap * 2;
            char *p = realloc(*line, nc);
            if (!p)
                return (errno = ENOMEM), -1;
            *line = p;
            *cap = nc;
        }
        if (c == EOF)
            break;
        (*line)[n++] = (char)c;
        if (c == delim)
            break;
    }
    (*line)[n] = 0;
    return (ssize_t)n;
}
ssize_t getline(char **line, size_t *cap, FILE *fp) { return getdelim(line, cap, '\n', fp); }

/* fgetln: the line stays in the stream's buffer, or in _lb when it spans refills. */
char *fgetln(FILE *fp, size_t *len) {
    size_t n = 0;
    for (;;) {
        int c = GETC(fp);
        if (c == EOF)
            break;
        if (n + 1 > (size_t)fp->_lb._size) {
            int nc = fp->_lb._size < 128 ? 128 : fp->_lb._size * 2;
            unsigned char *p = realloc(fp->_lb._base, (size_t)nc);
            if (!p)
                return NULL;
            fp->_lb._base = p;
            fp->_lb._size = nc;
        }
        fp->_lb._base[n++] = (unsigned char)c;
        if (c == '\n')
            break;
    }
    *len = n;
    return n ? (char *)fp->_lb._base : NULL;
}

size_t fread(void *buf, size_t size, size_t count, FILE *fp) {
    size_t total = size * count;
    if (!total)
        return 0;
    unsigned char *d = buf;
    size_t want = total;
    while (want) {
        if (fp->_r > 0) {
            size_t k = (size_t)fp->_r < want ? (size_t)fp->_r : want;
            memcpy(d, fp->_p, k);
            fp->_p += k;
            fp->_r -= (int)k;
            d += k;
            want -= k;
            continue;
        }
        if (__srefill(fp))
            break;
    }
    return (total - want) / size;
}

size_t fwrite(const void *buf, size_t size, size_t count, FILE *fp) {
    size_t total = size * count;
    if (!total)
        return 0;
    const unsigned char *s = buf;
    if (wsetup(fp))
        return 0;
    if (fp->_flags & __SLBF) {
        for (size_t i = 0; i < total; i++)
            if (PUTC(s[i], fp) == EOF)
                return i / size;
        return count;
    }
    size_t left = total;
    while (left) {
        int room = fp->_bf._size - (int)(fp->_p - fp->_bf._base);
        if (room <= 0 || (fp->_flags & __SNBF)) {
            if (__sflush(fp))
                break;
            if ((fp->_flags & __SNBF) || left >= (size_t)fp->_bf._size) {
                /* Write straight from the caller's buffer. */
                int t = fp->_write(fp->_cookie, (const char *)s, left > INT_MAX ? INT_MAX : (int)left);
                if (t <= 0) {
                    fp->_flags |= __SERR;
                    break;
                }
                s += t;
                left -= (size_t)t;
                continue;
            }
            room = fp->_bf._size;
        }
        size_t k = left < (size_t)room ? left : (size_t)room;
        memcpy(fp->_p, s, k);
        fp->_p += k;
        fp->_w -= (int)k;
        s += k;
        left -= k;
    }
    return (total - left) / size;
}

int fputs(const char *s, FILE *fp) {
    size_t n = strlen(s);
    return fwrite(s, 1, n, fp) == n ? 0 : EOF;
}
int puts(const char *s) { return fputs(s, stdout) == 0 && PUTC('\n', stdout) != EOF ? 0 : EOF; }

int vfprintf(FILE *fp, const char *fmt, va_list ap) {
    char small[512];
    va_list ap2;
    va_copy(ap2, ap);
    int n = vsnprintf(small, sizeof small, fmt, ap);
    char *p = small;
    if (n >= (int)sizeof small) {
        p = malloc((size_t)n + 1);
        if (!p) {
            va_end(ap2);
            return (errno = ENOMEM), -1;
        }
        vsnprintf(p, (size_t)n + 1, fmt, ap2);
    }
    va_end(ap2);
    int r = n >= 0 && fwrite(p, 1, (size_t)n, fp) == (size_t)n ? n : -1;
    if (p != small)
        free(p);
    return r;
}
int fprintf(FILE *fp, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = vfprintf(fp, fmt, ap);
    va_end(ap);
    return r;
}
int vprintf(const char *fmt, va_list ap) { return vfprintf(stdout, fmt, ap); }
int printf(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = vfprintf(stdout, fmt, ap);
    va_end(ap);
    return r;
}
int vasprintf(char **out, const char *fmt, va_list ap) {
    va_list ap2;
    va_copy(ap2, ap);
    int n = vsnprintf(NULL, 0, fmt, ap);
    *out = n >= 0 ? malloc((size_t)n + 1) : NULL;
    if (*out)
        vsnprintf(*out, (size_t)n + 1, fmt, ap2);
    va_end(ap2);
    return *out ? n : -1;
}
int asprintf(char **out, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int r = vasprintf(out, fmt, ap);
    va_end(ap);
    return r;
}

int fseeko(FILE *fp, off_t off, int whence) {
    if (!fp->_seek)
        return (errno = ESPIPE), -1;
    if (fp->_flags & __SWR && __sflush(fp))
        return -1;
    if (whence == SEEK_CUR && (fp->_flags & __SRD) && fp->_r > 0)
        off -= fp->_r;
    if (fp->_seek(fp->_cookie, off, whence) < 0)
        return -1;
    if (fp->_flags & __SRD) {
        fp->_r = 0;
        fp->_p = fp->_bf._base;
    }
    fp->_flags &= ~__SEOF;
    return 0;
}
int fseek(FILE *fp, long off, int whence) { return fseeko(fp, off, whence); }
off_t ftello(FILE *fp) {
    if (!fp->_seek)
        return (errno = ESPIPE), -1;
    off_t pos = fp->_seek(fp->_cookie, 0, SEEK_CUR);
    if (pos < 0)
        return -1;
    if (fp->_flags & __SRD)
        pos -= fp->_r > 0 ? fp->_r : 0;
    else if ((fp->_flags & __SWR) && fp->_bf._base)
        pos += fp->_p - fp->_bf._base;
    return pos;
}
long ftell(FILE *fp) { return (long)ftello(fp); }
void rewind(FILE *fp) {
    fseeko(fp, 0, SEEK_SET);
    fp->_flags &= ~__SERR;
}
int fgetpos(FILE *fp, fpos_t *pos) { return (*pos = ftello(fp)) < 0 ? -1 : 0; }
int fsetpos(FILE *fp, const fpos_t *pos) { return fseeko(fp, *pos, SEEK_SET); }

int feof(FILE *fp) { return (fp->_flags & __SEOF) != 0; }
int ferror(FILE *fp) { return (fp->_flags & __SERR) != 0; }
void clearerr(FILE *fp) { fp->_flags &= ~(__SEOF | __SERR); }
int fileno(FILE *fp) { return fp->_file; }
int feof_unlocked(FILE *fp) { return feof(fp); }
int ferror_unlocked(FILE *fp) { return ferror(fp); }
void clearerr_unlocked(FILE *fp) { clearerr(fp); }
int fileno_unlocked(FILE *fp) { return fp->_file; }
int fpurge(FILE *fp) {
    if (fp->_flags & __SRD)
        fp->_r = 0;
    if (fp->_flags & __SWR) {
        fp->_p = fp->_bf._base;
        fp->_w = (fp->_flags & (__SLBF | __SNBF)) ? 0 : fp->_bf._size;
    }
    return 0;
}
void flockfile(FILE *fp) { (void)fp; }
void funlockfile(FILE *fp) { (void)fp; }
int ftrylockfile(FILE *fp) { (void)fp; return 0; }

FILE *funopen(const void *cookie, int (*r)(void *, char *, int), int (*w)(void *, const char *, int), fpos_t (*s)(void *, fpos_t, int), int (*c)(void *)) {
    int flags = r && w ? __SRW : r ? __SRD : w ? __SWR : 0;
    if (!flags)
        return (errno = EINVAL), (FILE *)NULL;
    FILE *fp = new_file(-1, flags);
    if (!fp)
        return NULL;
    fp->_cookie = (void *)cookie;
    fp->_read = r;
    fp->_write = w;
    fp->_seek = s;
    fp->_close = c;
    return fp;
}

void perror(const char *s) {
    int e = errno;
    if (s && *s)
        fprintf(stderr, "%s: %s\n", s, strerror(e));
    else
        fprintf(stderr, "%s\n", strerror(e));
}
