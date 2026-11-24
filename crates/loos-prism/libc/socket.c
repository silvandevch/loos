/* BSD sockets on the Linux layer's sockets (AF_UNIX, AF_INET, AF_INET6). Darwin and Linux differ in
 * the sockaddr header (Darwin: u8 sa_len, u8 sa_family; Linux: u16 family), AF_INET6 (30 / 10),
 * SOL_SOCKET and its option numbers, MSG_* flags and the msghdr layout; everything else (address
 * bytes after the header, unix paths) has the same offsets. Calls go to Linux through Prism's
 * passthrough system calls (darwin.rs LINUX_PASSTHROUGH). */
#include "prism.h"

long __syscall(long n, long a, long b, long c, long d, long e, long f);
#define LX(nr) (0x10000 + (nr))
#define lx(nr, a, b, c, d, e, f) __syscall(LX(nr), (long)(a), (long)(b), (long)(c), (long)(d), (long)(e), (long)(f))

typedef unsigned int socklen_t;
struct iovec { void *base; size_t len; };

#define D_AF_INET6 30
#ifndef ENOPROTOOPT
#define ENOPROTOOPT 42
#endif
#define L_AF_INET6 10

static int fam_to_linux(int f) { return f == D_AF_INET6 ? L_AF_INET6 : f; }
static int fam_to_darwin(int f) { return f == L_AF_INET6 ? D_AF_INET6 : f; }

/* A Darwin sockaddr copied into a Linux one (128 bytes is sockaddr_storage on both). */
static const void *addr_in(const void *a, socklen_t len, unsigned char *out, socklen_t *outlen) {
    if (!a) { *outlen = 0; return 0; }
    if (len > 128) len = 128;
    memcpy(out, a, len);
    unsigned short f = (unsigned short)fam_to_linux(((const unsigned char *)a)[1]);
    memcpy(out, &f, 2);
    *outlen = len;
    return out;
}

/* Rewrite a Linux sockaddr the kernel wrote into a Darwin one, in place. */
static void addr_out(void *a, socklen_t *lenp) {
    if (!a || !lenp || *lenp < 2) return;
    unsigned short f;
    memcpy(&f, a, 2);
    unsigned char *b = a;
    socklen_t n = *lenp;
    if (f == 1) { /* AF_UNIX: sun_len is the length up to the end of the path */
        if (n > 106) n = 106;
    }
    b[0] = (unsigned char)(n > 255 ? 255 : n);
    b[1] = (unsigned char)fam_to_darwin(f);
}

static int msg_flags(int d) {
    int l = d & 7; /* OOB, PEEK, DONTROUTE */
    if (d & 0x8) l |= 0x80;   /* EOR */
    if (d & 0x10) l |= 0x20;  /* TRUNC */
    if (d & 0x20) l |= 0x8;   /* CTRUNC */
    if (d & 0x40) l |= 0x100; /* WAITALL */
    if (d & 0x80) l |= 0x40;  /* DONTWAIT */
    return l;
}
static int msg_flags_out(int l) {
    int d = l & 7;
    if (l & 0x80) d |= 0x8;
    if (l & 0x20) d |= 0x10;
    if (l & 0x8) d |= 0x20;
    return d;
}

int socket(int dom, int type, int proto) { return (int)lx(41, fam_to_linux(dom), type, proto, 0, 0, 0); }
int socketpair(int dom, int type, int proto, int sv[2]) { return (int)lx(53, fam_to_linux(dom), type, proto, sv, 0, 0); }
int listen(int fd, int backlog) { return (int)lx(50, fd, backlog, 0, 0, 0, 0); }
int shutdown(int fd, int how) { return (int)lx(48, fd, how, 0, 0, 0, 0); }

int bind(int fd, const void *a, socklen_t len) {
    unsigned char buf[128]; socklen_t n;
    const void *p = addr_in(a, len, buf, &n);
    return (int)lx(49, fd, p, n, 0, 0, 0);
}
int connect(int fd, const void *a, socklen_t len) {
    unsigned char buf[128]; socklen_t n;
    const void *p = addr_in(a, len, buf, &n);
    return (int)lx(42, fd, p, n, 0, 0, 0);
}
int accept(int fd, void *a, socklen_t *lenp) {
    int r = (int)lx(43, fd, a, a ? lenp : 0, 0, 0, 0);
    if (r >= 0) addr_out(a, lenp);
    return r;
}
int getsockname(int fd, void *a, socklen_t *lenp) {
    int r = (int)lx(51, fd, a, lenp, 0, 0, 0);
    if (r == 0) addr_out(a, lenp);
    return r;
}
int getpeername(int fd, void *a, socklen_t *lenp) {
    int r = (int)lx(52, fd, a, lenp, 0, 0, 0);
    if (r == 0) addr_out(a, lenp);
    return r;
}

ssize_t sendto(int fd, const void *buf, size_t n, int flags, const void *a, socklen_t len) {
    unsigned char sa[128]; socklen_t sn;
    const void *p = addr_in(a, len, sa, &sn);
    return lx(44, fd, buf, n, msg_flags(flags), p, sn);
}
ssize_t send(int fd, const void *buf, size_t n, int flags) { return sendto(fd, buf, n, flags, 0, 0); }
ssize_t recvfrom(int fd, void *buf, size_t n, int flags, void *a, socklen_t *lenp) {
    ssize_t r = lx(45, fd, buf, n, msg_flags(flags), a, a ? lenp : 0);
    if (r >= 0 && a) addr_out(a, lenp);
    return r;
}
ssize_t recv(int fd, void *buf, size_t n, int flags) { return recvfrom(fd, buf, n, flags, 0, 0); }

/* msghdr: Darwin {name, u32 namelen, iov, int iovlen, control, u32 controllen, int flags};
 * Linux has size_t iovlen and controllen. Control messages (cmsg_len u32 vs size_t) are copied
 * through unchanged: only zero-length control is translated exactly. */
struct dmsghdr { void *name; socklen_t namelen; struct iovec *iov; int iovlen; void *control; socklen_t controllen; int flags; };
struct lmsghdr { void *name; socklen_t namelen; struct iovec *iov; size_t iovlen; void *control; size_t controllen; int flags; };

ssize_t sendmsg(int fd, const struct dmsghdr *m, int flags) {
    unsigned char sa[128]; socklen_t sn;
    struct lmsghdr l = { 0 };
    l.name = (void *)addr_in(m->name, m->namelen, sa, &sn);
    l.namelen = sn;
    l.iov = m->iov; l.iovlen = (size_t)m->iovlen;
    l.control = m->control; l.controllen = m->controllen;
    return lx(46, fd, &l, msg_flags(flags), 0, 0, 0);
}
ssize_t recvmsg(int fd, struct dmsghdr *m, int flags) {
    struct lmsghdr l = { m->name, m->namelen, m->iov, (size_t)m->iovlen, m->control, m->controllen, 0 };
    ssize_t r = lx(47, fd, &l, msg_flags(flags), 0, 0, 0);
    if (r >= 0) {
        m->namelen = l.namelen;
        if (m->name) addr_out(m->name, &m->namelen);
        m->controllen = (socklen_t)l.controllen;
        m->flags = msg_flags_out(l.flags);
    }
    return r;
}

/* Socket options: SOL_SOCKET (0xffff / 1) and its numbers differ; IPPROTO_* levels mostly match. */
static int opt_to_linux(int *level, int *opt) {
    if (*level == 0xffff) {
        *level = 1;
        switch (*opt) {
        case 0x4: *opt = 2; break;      /* SO_REUSEADDR */
        case 0x8: *opt = 9; break;      /* SO_KEEPALIVE */
        case 0x10: *opt = 5; break;     /* SO_DONTROUTE */
        case 0x20: *opt = 6; break;     /* SO_BROADCAST */
        case 0x80: *opt = 13; break;    /* SO_LINGER */
        case 0x100: *opt = 10; break;   /* SO_OOBINLINE */
        case 0x200: *opt = 15; break;   /* SO_REUSEPORT */
        case 0x1001: *opt = 7; break;   /* SO_SNDBUF */
        case 0x1002: *opt = 8; break;   /* SO_RCVBUF */
        case 0x1003: *opt = 19; break;  /* SO_SNDLOWAT */
        case 0x1004: *opt = 18; break;  /* SO_RCVLOWAT */
        case 0x1005: *opt = 21; break;  /* SO_SNDTIMEO */
        case 0x1006: *opt = 20; break;  /* SO_RCVTIMEO */
        case 0x1007: *opt = 4; break;   /* SO_ERROR */
        case 0x1008: *opt = 3; break;   /* SO_TYPE */
        case 0x1022: return 1;          /* SO_NOSIGPIPE: accepted, not enforced */
        default: return -1;
        }
    } else if (*level == 41 && *opt == 27) {
        *opt = 26; /* IPV6_V6ONLY */
    } else if (*level == 6 && *opt == 0x10) {
        *opt = 4; /* TCP_KEEPALIVE -> TCP_KEEPIDLE */
    }
    return 0;
}

int setsockopt(int fd, int level, int opt, const void *v, socklen_t len) {
    int r = opt_to_linux(&level, &opt);
    if (r == 1) return 0;
    if (r < 0) { errno = ENOPROTOOPT; return -1; }
    return (int)lx(54, fd, level, opt, v, len, 0);
}
int getsockopt(int fd, int level, int opt, void *v, socklen_t *len) {
    int r = opt_to_linux(&level, &opt);
    if (r == 1) { if (*len >= 4) { *(int *)v = 0; *len = 4; } return 0; }
    if (r < 0) { errno = ENOPROTOOPT; return -1; }
    return (int)lx(55, fd, level, opt, v, len, 0);
}

/* ------------------------------------------------------------------ addresses */

unsigned short htons(unsigned short v) { return (unsigned short)((v << 8) | (v >> 8)); }
unsigned short ntohs(unsigned short v) { return htons(v); }
unsigned int htonl(unsigned int v) { return __builtin_bswap32(v); }
unsigned int ntohl(unsigned int v) { return __builtin_bswap32(v); }

static int pton4(const char *s, unsigned char *out) {
    unsigned char b[4];
    for (int i = 0; i < 4; i++) {
        if (*s < '0' || *s > '9') return 0;
        unsigned v = 0;
        int digits = 0;
        while (*s >= '0' && *s <= '9') { v = v * 10 + (unsigned)(*s++ - '0'); if (++digits > 3 || v > 255) return 0; }
        b[i] = (unsigned char)v;
        if (i < 3 && *s++ != '.') return 0;
    }
    if (*s) return 0;
    memcpy(out, b, 4);
    return 1;
}
static int hexv(int c) { return c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 'a' + 10 : c >= 'A' && c <= 'F' ? c - 'A' + 10 : -1; }
static int pton6(const char *s, unsigned char *out) {
    unsigned short w[8] = { 0 };
    int n = 0, gap = -1;
    if (s[0] == ':' && s[1] == ':') { gap = 0; s += 2; }
    while (*s && n < 8) {
        const char *start = s;
        unsigned v = 0;
        int k = 0;
        while (hexv(*s) >= 0 && k < 4) { v = v * 16 + (unsigned)hexv(*s++); k++; }
        if (*s == '.') { /* embedded IPv4 tail */
            unsigned char b[4];
            if (n > 6 || !pton4(start, b)) return 0;
            w[n++] = (unsigned short)(b[0] << 8 | b[1]);
            w[n++] = (unsigned short)(b[2] << 8 | b[3]);
            s += strlen(s);
            break;
        }
        if (!k) return 0;
        w[n++] = (unsigned short)v;
        if (!*s) break;
        if (*s++ != ':') return 0;
        if (*s == ':') { if (gap >= 0) return 0; gap = n; s++; }
    }
    if (*s) return 0;
    if (gap >= 0) {
        int tail = n - gap;
        for (int i = 0; i < tail; i++) w[7 - i] = w[n - 1 - i];
        for (int i = gap; i < 8 - tail; i++) w[i] = 0;
    } else if (n != 8) {
        return 0;
    }
    for (int i = 0; i < 8; i++) { out[2 * i] = (unsigned char)(w[i] >> 8); out[2 * i + 1] = (unsigned char)w[i]; }
    return 1;
}
int inet_pton(int af, const char *s, void *dst) {
    if (af == 2) return pton4(s, dst);
    if (af == D_AF_INET6) return pton6(s, dst);
    errno = 47; /* EAFNOSUPPORT */
    return -1;
}
unsigned int inet_addr(const char *s) { unsigned char b[4]; return pton4(s, b) ? *(unsigned int *)b : 0xffffffffu; }
int inet_aton(const char *s, void *out) { return pton4(s, out); }

int snprintf(char *, size_t, const char *, ...);
const char *inet_ntop(int af, const void *src, char *dst, socklen_t n) {
    const unsigned char *b = src;
    char tmp[64];
    if (af == 2) {
        snprintf(tmp, sizeof tmp, "%u.%u.%u.%u", b[0], b[1], b[2], b[3]);
    } else if (af == D_AF_INET6) {
        unsigned short w[8];
        for (int i = 0; i < 8; i++) w[i] = (unsigned short)(b[2 * i] << 8 | b[2 * i + 1]);
        int best = -1, bestlen = 1; /* longest run of >= 2 zero words becomes "::" */
        for (int i = 0; i < 8;) {
            int j = i;
            while (j < 8 && !w[j]) j++;
            if (j - i > bestlen) { best = i; bestlen = j - i; }
            i = j == i ? i + 1 : j;
        }
        size_t o = 0;
        for (int i = 0; i < 8; i++) {
            if (i == best) { o += (size_t)snprintf(tmp + o, sizeof tmp - o, "::"); i += bestlen - 1; continue; }
            if (o && tmp[o - 1] != ':') tmp[o++] = ':';
            o += (size_t)snprintf(tmp + o, sizeof tmp - o, "%x", w[i]);
        }
        tmp[o] = 0;
    } else {
        errno = 47;
        return 0;
    }
    size_t l = strlen(tmp);
    if (l + 1 > n) { errno = 28; return 0; } /* ENOSPC */
    memcpy(dst, tmp, l + 1);
    return dst;
}
char *inet_ntoa(unsigned int a) {
    static char buf[16];
    inet_ntop(2, &a, buf, sizeof buf);
    return buf;
}
