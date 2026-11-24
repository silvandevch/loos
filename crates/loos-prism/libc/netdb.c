/* Name resolution: getaddrinfo / freeaddrinfo / gai_strerror / getnameinfo, gethostbyname(2),
 * gethostbyaddr, getservbyname / getservbyport. Order: numeric addresses, "localhost",
 * /etc/hosts, then DNS over UDP to the servers in /etc/resolv.conf (the Linux layer generates it
 * from the DHCP lease), with CNAME chains followed. Services: numbers, /etc/services, a built-in
 * table. Layouts and constants are Darwin's (struct addrinfo has ai_canonname before ai_addr;
 * AF_INET6 is 30; EAI_* numbering). Not yet: mDNS (.local), search domains, DNS over TCP. */
#include "prism.h"

long __syscall(long n, long a, long b, long c, long d, long e, long f);
#define lx(nr, a, b, c) __syscall(0x10000 + (nr), (long)(a), (long)(b), (long)(c), 0, 0, 0)
int socket(int, int, int);
long sendto(int, const void *, size_t, int, const void *, unsigned);
long recvfrom(int, void *, size_t, int, void *, unsigned *);
int close(int);
int inet_pton(int, const char *, void *);
const char *inet_ntop(int, const void *, char *, unsigned);
int snprintf(char *, size_t, const char *, ...);
int strcmp(const char *, const char *);
int strcasecmp(const char *, const char *);
char *strdup(const char *);
long getpid(void);
int atoi(const char *);

#define AF_UNSPEC 0
#define AF_INET 2
#define AF_INET6 30
#define SOCK_STREAM 1
#define SOCK_DGRAM 2
#define AI_PASSIVE 0x1
#define AI_CANONNAME 0x2
#define AI_NUMERICHOST 0x4
#define AI_NUMERICSERV 0x1000
#define NI_NUMERICHOST 0x2
#define NI_NAMEREQD 0x4
#define NI_NUMERICSERV 0x8
#define NI_DGRAM 0x10
enum { EAI_AGAIN = 2, EAI_BADFLAGS, EAI_FAIL, EAI_FAMILY, EAI_MEMORY, EAI_NODATA, EAI_NONAME, EAI_SERVICE, EAI_SOCKTYPE, EAI_SYSTEM, EAI_BADHINTS, EAI_PROTOCOL, EAI_OVERFLOW };

struct addrinfo {
    int ai_flags, ai_family, ai_socktype, ai_protocol;
    unsigned ai_addrlen;
    char *ai_canonname;
    void *ai_addr;
    struct addrinfo *ai_next;
};
struct sockaddr_in { uint8_t len, family; uint16_t port; uint8_t addr[4]; char zero[8]; };
struct sockaddr_in6 { uint8_t len, family; uint16_t port; uint32_t flow; uint8_t addr[16]; uint32_t scope; };

static uint16_t be16(uint16_t v) { return (uint16_t)(v << 8 | v >> 8); }

/* ------------------------------------------------------------------ files */

/* Read a whole small file (NUL-terminated, malloc'ed) or NULL. */
static char *slurp(const char *path) {
    int fd = (int)lx(2, path, 0, 0); /* open O_RDONLY */
    if (fd < 0) return 0;
    size_t cap = 4096, n = 0;
    char *b = malloc(cap + 1);
    for (;;) {
        if (n == cap) { char *nb = realloc(b, cap * 2 + 1); if (!nb) break; b = nb; cap *= 2; }
        long r = lx(0, fd, b + n, cap - n);
        if (r <= 0) break;
        n += (size_t)r;
    }
    lx(3, fd, 0, 0);
    b[n] = 0;
    return b;
}

static int is_space(char c) { return c == ' ' || c == '\t' || c == '\r'; }
/* Split the next line into words (comments dropped); returns the word count, advances *p. */
static int words(char **p, char **w, int max) {
    char *s = *p;
    if (!*s) return -1;
    char *e = s;
    while (*e && *e != '\n') e++;
    *p = *e ? e + 1 : e;
    *e = 0;
    char *h = s;
    while (*h && *h != '#') h++;
    *h = 0;
    int n = 0;
    while (*s && n < max) {
        while (is_space(*s)) s++;
        if (!*s) break;
        w[n++] = s;
        while (*s && !is_space(*s)) s++;
        if (*s) *s++ = 0;
    }
    return n;
}

/* ------------------------------------------------------------------ result list */

struct addr { int family; uint8_t a[16]; };

static struct addrinfo *mk(int family, const uint8_t *a, uint16_t port, int socktype, int proto, int flags) {
    struct addrinfo *r = calloc(1, sizeof *r + sizeof(struct sockaddr_in6));
    if (!r) return 0;
    r->ai_flags = flags;
    r->ai_family = family;
    r->ai_socktype = socktype;
    r->ai_protocol = proto;
    r->ai_addr = r + 1;
    if (family == AF_INET) {
        struct sockaddr_in *s = r->ai_addr;
        s->len = sizeof *s; s->family = AF_INET; s->port = be16(port);
        memcpy(s->addr, a, 4);
        r->ai_addrlen = sizeof *s;
    } else {
        struct sockaddr_in6 *s = r->ai_addr;
        s->len = sizeof *s; s->family = AF_INET6; s->port = be16(port);
        memcpy(s->addr, a, 16);
        r->ai_addrlen = sizeof *s;
    }
    return r;
}

void freeaddrinfo(struct addrinfo *ai) {
    while (ai) { struct addrinfo *n = ai->ai_next; free(ai->ai_canonname); free(ai); ai = n; }
}

const char *gai_strerror(int e) {
    static const char *const m[] = { "Success", "Address family for hostname not supported", "Temporary failure in name resolution",
        "Invalid value for ai_flags", "Non-recoverable failure in name resolution", "ai_family not supported", "Memory allocation failure",
        "No address associated with hostname", "nodename nor servname provided, or not known", "servname not supported for ai_socktype",
        "ai_socktype not supported", "System error", "Invalid value for hints", "Resolved protocol is unknown", "Argument buffer overflow" };
    return e >= 0 && e < (int)(sizeof m / sizeof *m) ? m[e] : "Unknown error";
}

/* ------------------------------------------------------------------ services */

static const struct { const char *name; uint16_t port; } known[] = {
    { "ftp", 21 }, { "ssh", 22 }, { "telnet", 23 }, { "smtp", 25 }, { "domain", 53 }, { "http", 80 }, { "www", 80 },
    { "pop3", 110 }, { "ntp", 123 }, { "imap", 143 }, { "ldap", 389 }, { "https", 443 }, { "submission", 587 },
    { "imaps", 993 }, { "pop3s", 995 }, { "mysql", 3306 }, { "postgresql", 5432 },
};

/* Port of service `s` for `proto` ("tcp"/"udp"/NULL); -1 if unknown. */
static int service_port(const char *s, const char *proto, int numeric_only) {
    char *end;
    long v = 0;
    for (end = (char *)s; *end >= '0' && *end <= '9'; end++) v = v * 10 + (*end - '0');
    if (end != s && !*end) return v <= 65535 ? (int)v : -1;
    if (numeric_only) return -1;
    char *f = slurp("/etc/services");
    if (f) {
        char *p = f, *w[16];
        int n, port = -1;
        while (port < 0 && (n = words(&p, w, 16)) >= 0) {
            if (n < 2) continue;
            char *sl = w[1];
            while (*sl && *sl != '/') sl++;
            if (!*sl) continue;
            *sl++ = 0;
            if (proto && strcmp(sl, proto)) continue;
            for (int i = 0; i < n; i++)
                if (i != 1 && !strcmp(w[i], s)) { port = atoi(w[1]); break; }
        }
        free(f);
        if (port >= 0) return port;
    }
    for (size_t i = 0; i < sizeof known / sizeof *known; i++)
        if (!strcmp(known[i].name, s)) return known[i].port;
    return -1;
}
static const char *service_name(int port) {
    for (size_t i = 0; i < sizeof known / sizeof *known; i++)
        if (known[i].port == port) return known[i].name;
    return 0;
}

/* ------------------------------------------------------------------ hosts file */

/* Addresses for `name` from /etc/hosts (family 0: both); `canon` gets the canonical name. */
static int hosts_lookup(const char *name, int family, struct addr *out, int max, char *canon, size_t clen) {
    char *f = slurp("/etc/hosts");
    if (!f) return 0;
    char *p = f, *w[16];
    int n, k = 0;
    while (k < max && (n = words(&p, w, 16)) >= 0) {
        if (n < 2) continue;
        int hit = 0;
        for (int i = 1; i < n; i++) if (!strcasecmp(w[i], name)) hit = 1;
        if (!hit) continue;
        struct addr a;
        if (inet_pton(AF_INET, w[0], a.a) == 1) a.family = AF_INET;
        else if (inet_pton(AF_INET6, w[0], a.a) == 1) a.family = AF_INET6;
        else continue;
        if (family && a.family != family) continue;
        if (!k && canon) { size_t l = strlen(w[1]); if (l >= clen) l = clen - 1; memcpy(canon, w[1], l); canon[l] = 0; }
        out[k++] = a;
    }
    free(f);
    return k;
}

/* ------------------------------------------------------------------ DNS */

static int put_name(uint8_t *q, const char *name) {
    int o = 0;
    while (*name) {
        const char *dot = name;
        while (*dot && *dot != '.') dot++;
        int l = (int)(dot - name);
        if (l == 0 || l > 63 || o + l + 2 > 255) return -1;
        q[o++] = (uint8_t)l;
        memcpy(q + o, name, (size_t)l);
        o += l;
        name = *dot ? dot + 1 : dot;
    }
    q[o++] = 0;
    return o;
}
/* Skip (or read into `out`) a possibly compressed name at `*p`. */
static int get_name(const uint8_t *m, size_t len, size_t *p, char *out, size_t olen) {
    size_t i = *p, o = 0;
    int jumped = 0, hops = 0;
    for (;;) {
        if (i >= len || ++hops > 128) return -1;
        uint8_t l = m[i];
        if ((l & 0xc0) == 0xc0) {
            if (i + 1 >= len) return -1;
            if (!jumped) *p = i + 2;
            i = (size_t)(l & 0x3f) << 8 | m[i + 1];
            jumped = 1;
            continue;
        }
        if (!l) { if (!jumped) *p = i + 1; break; }
        if (i + 1 + l > len) return -1;
        if (out) {
            if (o + l + 2 > olen) return -1;
            if (o) out[o++] = '.';
            memcpy(out + o, m + i + 1, l);
            o += l;
        }
        i += 1 + l;
    }
    if (out) out[o] = 0;
    return 0;
}

static int nameservers(uint8_t ns[][4], int max) {
    char *f = slurp("/etc/resolv.conf");
    int k = 0;
    if (f) {
        char *p = f, *w[4];
        int n;
        while (k < max && (n = words(&p, w, 4)) >= 0)
            if (n >= 2 && !strcmp(w[0], "nameserver") && inet_pton(AF_INET, w[1], ns[k]) == 1) k++;
        free(f);
    }
    return k;
}

/* Query type `qtype` (1 A, 28 AAAA) for `name`. Returns the number of addresses (canonical name in
 * `canon`), 0 if the name has none of that type, -EAI_NONAME / -EAI_AGAIN / -EAI_FAIL. */
static int dns_query(const char *name, int qtype, struct addr *out, int max, char *canon, size_t clen) {
    uint8_t ns[3][4];
    int nns = nameservers(ns, 3);
    if (!nns) return -EAI_AGAIN;
    uint8_t q[512];
    uint16_t id = (uint16_t)(getpid() * 7919 + (uintptr_t)out);
    q[0] = (uint8_t)(id >> 8); q[1] = (uint8_t)id; q[2] = 1; q[3] = 0; /* RD */
    q[4] = 0; q[5] = 1; memset(q + 6, 0, 6);
    int nl = put_name(q + 12, name);
    if (nl < 0) return -EAI_NONAME;
    int ql = 12 + nl;
    q[ql++] = 0; q[ql++] = (uint8_t)qtype; q[ql++] = 0; q[ql++] = 1;
    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    if (fd < 0) return -EAI_SYSTEM;
    uint8_t m[1500];
    long len = -1;
    for (int attempt = 0; attempt < 2 * nns && len < 0; attempt++) {
        struct sockaddr_in sa = { sizeof sa, AF_INET, be16(53), { 0 }, { 0 } };
        memcpy(sa.addr, ns[attempt % nns], 4);
        if (sendto(fd, q, (size_t)ql, 0, &sa, sizeof sa) != ql) continue;
        struct { int fd; short ev, rev; } pfd = { fd, 1, 0 };
        for (;;) {
            if (lx(7, &pfd, 1, 2000) <= 0) break; /* poll, 2 s */
            long r = recvfrom(fd, m, sizeof m, 0, 0, 0);
            if (r >= 12 && m[0] == q[0] && m[1] == q[1] && (m[2] & 0x80)) { len = r; break; }
        }
    }
    close(fd);
    if (len < 0) return -EAI_AGAIN;
    int rcode = m[3] & 0xf;
    if (rcode == 3) return -EAI_NONAME;
    if (rcode) return rcode == 2 ? -EAI_AGAIN : -EAI_FAIL;
    size_t p = 12;
    int qd = m[4] << 8 | m[5], an = m[6] << 8 | m[7];
    for (int i = 0; i < qd; i++) { if (get_name(m, (size_t)len, &p, 0, 0)) return -EAI_FAIL; p += 4; }
    char want[256];
    size_t wl = strlen(name);
    if (wl >= sizeof want) return -EAI_NONAME;
    memcpy(want, name, wl + 1);
    int k = 0;
    for (int i = 0; i < an && k < max; i++) {
        char owner[256], target[256];
        if (get_name(m, (size_t)len, &p, owner, sizeof owner) || p + 10 > (size_t)len) return k ? k : -EAI_FAIL;
        int type = m[p] << 8 | m[p + 1], rdl = m[p + 8] << 8 | m[p + 9];
        p += 10;
        if (p + (size_t)rdl > (size_t)len) break;
        if (!strcasecmp(owner, want)) {
            if (type == 5) { /* CNAME: the chain continues under the target */
                size_t t = p;
                if (!get_name(m, (size_t)len, &t, target, sizeof target)) { memcpy(want, target, strlen(target) + 1); }
            } else if (type == qtype && type == 1 && rdl == 4) {
                out[k].family = AF_INET; memcpy(out[k].a, m + p, 4); k++;
            } else if (type == qtype && type == 28 && rdl == 16) {
                out[k].family = AF_INET6; memcpy(out[k].a, m + p, 16); k++;
            }
        }
        p += (size_t)rdl;
    }
    if (canon) { size_t l = strlen(want); if (l >= clen) l = clen - 1; memcpy(canon, want, l); canon[l] = 0; }
    return k;
}

/* ------------------------------------------------------------------ getaddrinfo */

/* A numeric address of the other family than asked for. */
#define EAI_ADDRFAMILY_OR_NONAME EAI_NONAME

static int resolve(const char *host, int family, int flags, struct addr *out, int max, char *canon, size_t clen) {
    struct addr a;
    if (inet_pton(AF_INET, host, a.a) == 1) a.family = AF_INET;
    else if (inet_pton(AF_INET6, host, a.a) == 1) a.family = AF_INET6;
    else a.family = 0;
    if (a.family) {
        if (family && family != a.family) return -EAI_ADDRFAMILY_OR_NONAME;
        out[0] = a;
        return 1;
    }
    if (flags & AI_NUMERICHOST) return -EAI_NONAME;
    int k = hosts_lookup(host, family, out, max, canon, clen);
    if (k) return k;
    if (!strcasecmp(host, "localhost")) {
        k = 0;
        if (family != AF_INET6) { out[k].family = AF_INET; memcpy(out[k].a, "\x7f\0\0\1", 4); k++; }
        if (family != AF_INET) { out[k].family = AF_INET6; memset(out[k].a, 0, 16); out[k].a[15] = 1; k++; }
        return k;
    }
    /* AF_UNSPEC asks for IPv4 (the Linux layer's sockets are IPv4); AF_INET6 for AAAA records. */
    return dns_query(host, family == AF_INET6 ? 28 : 1, out, max, canon, clen);
}

int getaddrinfo(const char *host, const char *serv, const struct addrinfo *hints, struct addrinfo **res) {
    int family = hints ? hints->ai_family : AF_UNSPEC;
    int socktype = hints ? hints->ai_socktype : 0;
    int proto = hints ? hints->ai_protocol : 0;
    int flags = hints ? hints->ai_flags : 0;
    *res = 0;
    if (!host && !serv) return EAI_NONAME;
    if (family != AF_UNSPEC && family != AF_INET && family != AF_INET6) return EAI_FAMILY;
    if (socktype && socktype != SOCK_STREAM && socktype != SOCK_DGRAM && socktype != 3) return EAI_SOCKTYPE;
    int port = 0;
    if (serv && *serv) {
        port = service_port(serv, socktype == SOCK_DGRAM ? "udp" : socktype == SOCK_STREAM ? "tcp" : 0, flags & AI_NUMERICSERV);
        if (port < 0) return EAI_NONAME;
    }
    struct addr addrs[16];
    char canon[256] = "";
    int n;
    if (!host) { /* passive: the wildcard address; otherwise loopback */
        n = 0;
        if (family != AF_INET6) { addrs[n].family = AF_INET; memset(addrs[n].a, 0, 4); if (!(flags & AI_PASSIVE)) memcpy(addrs[n].a, "\x7f\0\0\1", 4); n++; }
        if (family == AF_INET6) { addrs[n].family = AF_INET6; memset(addrs[n].a, 0, 16); if (!(flags & AI_PASSIVE)) addrs[n].a[15] = 1; n++; }
    } else {
        n = resolve(host, family, flags, addrs, 16, canon, sizeof canon);
        if (n < 0) return -n;
        if (n == 0) return EAI_NONAME;
    }
    static const int types[2][2] = { { SOCK_STREAM, 6 }, { SOCK_DGRAM, 17 } };
    struct addrinfo *head = 0, **tail = &head;
    for (int i = 0; i < n; i++)
        for (int t = 0; t < 2; t++) {
            if (socktype && socktype != types[t][0]) continue;
            if (proto && proto != types[t][1]) continue;
            struct addrinfo *r = mk(addrs[i].family, addrs[i].a, (uint16_t)port, types[t][0], types[t][1], flags);
            if (!r) { freeaddrinfo(head); return EAI_MEMORY; }
            *tail = r;
            tail = &r->ai_next;
        }
    if (!head) return EAI_SOCKTYPE;
    if ((flags & AI_CANONNAME) && host) head->ai_canonname = strdup(canon[0] ? canon : host);
    *res = head;
    return 0;
}

/* ------------------------------------------------------------------ getnameinfo */

/* Reverse lookup in /etc/hosts only (no PTR queries yet). */
static int hosts_reverse(int family, const uint8_t *a, char *out, size_t olen) {
    char *f = slurp("/etc/hosts");
    if (!f) return 0;
    char *p = f, *w[4];
    int n, found = 0;
    while (!found && (n = words(&p, w, 4)) >= 0) {
        uint8_t b[16];
        if (n < 2 || inet_pton(family, w[0], b) != 1 || memcmp(a, b, family == AF_INET ? 4 : 16)) continue;
        size_t l = strlen(w[1]);
        if (l < olen) { memcpy(out, w[1], l + 1); found = 1; }
    }
    free(f);
    return found;
}

int getnameinfo(const void *sa, unsigned salen, char *host, unsigned hostlen, char *serv, unsigned servlen, int flags) {
    const uint8_t *s = sa;
    int family = s[1];
    const uint8_t *a;
    uint16_t port;
    if (family == AF_INET && salen >= sizeof(struct sockaddr_in)) { const struct sockaddr_in *v = sa; a = v->addr; port = be16(v->port); }
    else if (family == AF_INET6 && salen >= sizeof(struct sockaddr_in6)) { const struct sockaddr_in6 *v = sa; a = v->addr; port = be16(v->port); }
    else return EAI_FAMILY;
    if (host && hostlen) {
        int named = !(flags & NI_NUMERICHOST) && hosts_reverse(family, a, host, hostlen);
        if (!named) {
            if (flags & NI_NAMEREQD) return EAI_NONAME;
            if (!inet_ntop(family, a, host, hostlen)) return EAI_OVERFLOW;
        }
    }
    if (serv && servlen) {
        const char *name = (flags & NI_NUMERICSERV) ? 0 : service_name(port);
        int l = name ? snprintf(serv, servlen, "%s", name) : snprintf(serv, servlen, "%u", port);
        if (l < 0 || (unsigned)l >= servlen) return EAI_OVERFLOW;
    }
    return 0;
}

/* ------------------------------------------------------------------ legacy hostent / servent */

struct hostent { char *h_name; char **h_aliases; int h_addrtype, h_length; char **h_addr_list; };
struct servent { char *s_name; char **s_aliases; int s_port; char *s_proto; };
int h_errno;

static struct hostent *fill_host(const char *name, int family, struct addr *a, int n) {
    static struct hostent h;
    static char hname[256];
    static char *aliases[1];
    static uint8_t bytes[16][16];
    static char *list[17];
    size_t l = strlen(name);
    if (l >= sizeof hname) l = sizeof hname - 1;
    memcpy(hname, name, l);
    hname[l] = 0;
    int k = 0;
    for (int i = 0; i < n && k < 16; i++)
        if (a[i].family == family) { memcpy(bytes[k], a[i].a, 16); list[k] = (char *)bytes[k]; k++; }
    if (!k) { h_errno = 4; return 0; } /* NO_DATA */
    list[k] = 0;
    h = (struct hostent){ hname, aliases, family, family == AF_INET ? 4 : 16, list };
    return &h;
}

struct hostent *gethostbyname2(const char *name, int family) {
    struct addr a[16];
    char canon[256] = "";
    int n = resolve(name, family, 0, a, 16, canon, sizeof canon);
    if (n <= 0) { h_errno = n == -EAI_AGAIN ? 2 : 1; return 0; } /* TRY_AGAIN / HOST_NOT_FOUND */
    return fill_host(canon[0] ? canon : name, family, a, n);
}
struct hostent *gethostbyname(const char *name) { return gethostbyname2(name, AF_INET); }

struct hostent *gethostbyaddr(const void *addr, unsigned len, int family) {
    static char name[256];
    if (!hosts_reverse(family, addr, name, sizeof name)) { h_errno = 1; return 0; }
    struct addr a = { family, { 0 } };
    memcpy(a.a, addr, len > 16 ? 16 : len);
    return fill_host(name, family, &a, 1);
}

const char *hstrerror(int e) {
    switch (e) {
    case 1: return "Unknown host";
    case 2: return "Host name lookup failure";
    case 3: return "Unknown server error";
    case 4: return "No address associated with name";
    default: return "Resolver internal error";
    }
}

struct servent *getservbyname(const char *name, const char *proto) {
    static struct servent s;
    static char sname[64], sproto[8];
    static char *aliases[1];
    int port = service_port(name, proto, 0);
    if (port < 0) return 0;
    snprintf(sname, sizeof sname, "%s", name);
    snprintf(sproto, sizeof sproto, "%s", proto ? proto : "tcp");
    s = (struct servent){ sname, aliases, be16((uint16_t)port), sproto };
    return &s;
}
struct servent *getservbyport(int port_be, const char *proto) {
    static struct servent s;
    static char sproto[8];
    static char *aliases[1];
    const char *name = service_name(be16((uint16_t)port_be));
    if (!name) return 0;
    snprintf(sproto, sizeof sproto, "%s", proto ? proto : "tcp");
    s = (struct servent){ (char *)name, aliases, port_be, sproto };
    return &s;
}
