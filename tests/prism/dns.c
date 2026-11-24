/* Name resolution: numeric hosts, localhost, /etc/hosts, /etc/services, passive wildcard,
 * errors, getnameinfo, gethostbyname, and DNS (a CNAME chain, AAAA, NXDOMAIN) answered by a
 * small DNS server on 127.0.0.1:53 in a thread (the boot image's /etc/resolv.conf points there).
 * No SDK headers. */
typedef unsigned int socklen_t;
typedef unsigned long size_t;
typedef long ssize_t;
typedef struct _opaque_pthread *pthread_t;
struct addrinfo { int flags, family, socktype, protocol; socklen_t addrlen; char *canonname; void *addr; struct addrinfo *next; };
struct sockaddr_in { unsigned char len, family; unsigned short port; unsigned char addr[4]; char zero[8]; };
struct sockaddr_in6 { unsigned char len, family; unsigned short port; unsigned flow; unsigned char addr[16]; unsigned scope; };
struct hostent { char *name; char **aliases; int addrtype, length; char **addr_list; };
int getaddrinfo(const char *, const char *, const struct addrinfo *, struct addrinfo **);
void freeaddrinfo(struct addrinfo *);
int getnameinfo(const void *, socklen_t, char *, socklen_t, char *, socklen_t, int);
struct hostent *gethostbyname(const char *);
const char *inet_ntop(int, const void *, char *, socklen_t);
int inet_pton(int, const char *, void *);
int socket(int, int, int);
int bind(int, const void *, socklen_t);
ssize_t sendto(int, const void *, size_t, int, const void *, socklen_t);
ssize_t recvfrom(int, void *, size_t, int, void *, socklen_t *);
unsigned short htons(unsigned short);
int pthread_create(pthread_t *, const void *, void *(*)(void *), void *);
int printf(const char *, ...);
int strcmp(const char *, const char *);
void *memcpy(void *, const void *, size_t);
int usleep(unsigned);

static int server_ready;
static int put_name(unsigned char *p, const char *n) {
    int o = 0;
    while (*n) {
        const char *d = n;
        while (*d && *d != '.') d++;
        p[o++] = (unsigned char)(d - n);
        memcpy(p + o, n, (size_t)(d - n));
        o += (int)(d - n);
        n = *d ? d + 1 : d;
    }
    p[o++] = 0;
    return o;
}
static int rr(unsigned char *p, const char *owner, int type, const unsigned char *data, int len) {
    int o = put_name(p, owner);
    unsigned char h[10] = { 0, (unsigned char)type, 0, 1, 0, 0, 0, 60, 0, (unsigned char)len };
    memcpy(p + o, h, 10);
    memcpy(p + o + 10, data, (size_t)len);
    return o + 10 + len;
}
static void *dns_server(void *arg) {
    (void)arg;
    int s = socket(2, 2, 0);
    struct sockaddr_in a = { 16, 2, htons(53), { 127, 0, 0, 1 }, { 0 } };
    bind(s, &a, sizeof a);
    __atomic_store_n(&server_ready, 1, __ATOMIC_RELEASE);
    for (;;) {
        unsigned char q[512], r[512];
        struct sockaddr_in from;
        socklen_t fl = sizeof from;
        ssize_t n = recvfrom(s, q, sizeof q, 0, &from, &fl);
        if (n < 17) continue;
        char name[256];
        int i = 12, o = 0;
        while (q[i]) { if (o) name[o++] = '.'; memcpy(name + o, q + i + 1, q[i]); o += q[i]; i += q[i] + 1; }
        name[o] = 0;
        int qtype = q[i + 2], qend = i + 5;
        memcpy(r, q, (size_t)qend);
        r[2] = 0x81; r[3] = 0x80; r[6] = 0; r[7] = 0; r[8] = r[9] = r[10] = r[11] = 0;
        int len = qend, an = 0;
        if (!strcmp(name, "web.test") && qtype == 1) {
            unsigned char cname[64];
            int cl = put_name(cname, "real.test");
            len += rr(r + len, "web.test", 5, cname, cl); an++;
            unsigned char v4[4] = { 192, 0, 2, 7 };
            len += rr(r + len, "real.test", 1, v4, 4); an++;
        } else if (!strcmp(name, "real6.test") && qtype == 28) {
            unsigned char v6[16];
            inet_pton(30, "2001:db8::9", v6);
            len += rr(r + len, "real6.test", 28, v6, 16); an++;
        } else {
            r[3] = 0x83; /* NXDOMAIN */
        }
        r[7] = (unsigned char)an;
        sendto(s, r, (size_t)len, 0, &from, fl);
    }
    return 0;
}

static const char *v4(struct addrinfo *ai, char *b) { return inet_ntop(2, ((struct sockaddr_in *)ai->addr)->addr, b, 64); }

int main(void) {
    pthread_t t;
    pthread_create(&t, 0, dns_server, 0);
    while (!__atomic_load_n(&server_ready, __ATOMIC_ACQUIRE)) usleep(1000);
    struct addrinfo h = { 0 }, *r, *x;
    char b1[64], b2[64], b3[64];
    int n;

    h.socktype = 1;
    getaddrinfo("192.168.1.20", "80", &h, &r);
    printf("dns: num=%d/%d/%d/%u", r->family, r->socktype, htons(((struct sockaddr_in *)r->addr)->port), r->addrlen);
    freeaddrinfo(r);
    getaddrinfo("10.0.0.1", "https", 0, &r);
    for (n = 0, x = r; x; x = x->next) n++;
    printf(" both=%d/%d", n, htons(((struct sockaddr_in *)r->addr)->port));
    freeaddrinfo(r);
    h.socktype = 0; h.family = 2;
    getaddrinfo("localhost", 0, &h, &r);
    printf(" local=%s", v4(r, b1));
    freeaddrinfo(r);
    h.flags = 2; /* AI_CANONNAME */
    getaddrinfo("printer", "ssh", &h, &r);
    printf(" hosts=%s/%s/%d", v4(r, b1), r->canonname, htons(((struct sockaddr_in *)r->addr)->port));
    freeaddrinfo(r);
    h.flags = 0; h.family = 30;
    n = getaddrinfo("six.lan", 0, &h, &r);
    printf(" six=%d/%s/%u", n, n ? "-" : inet_ntop(30, ((struct sockaddr_in6 *)r->addr)->addr, b1, 64), n ? 0 : r->addrlen);
    if (!n) freeaddrinfo(r);
    h.family = 0; h.socktype = 1;
    getaddrinfo("127.0.0.1", "myproto", &h, &r);
    printf(" svc=%d", htons(((struct sockaddr_in *)r->addr)->port));
    freeaddrinfo(r);
    h.flags = 1; /* AI_PASSIVE */
    getaddrinfo(0, "8080", &h, &r);
    printf(" passive=%s", v4(r, b1));
    freeaddrinfo(r);
    h.flags = 0;
    int e1 = getaddrinfo("127.0.0.1", "nosuchservice", &h, &r);
    h.family = 99;
    int e2 = getaddrinfo("127.0.0.1", 0, &h, &r);
    h.family = 0; h.flags = 4; /* AI_NUMERICHOST */
    int e3 = getaddrinfo("printer", 0, &h, &r);
    printf(" err=%d/%d/%d", e1, e2, e3);
    struct sockaddr_in sa = { 16, 2, htons(443), { 10, 1, 2, 3 }, { 0 } };
    getnameinfo(&sa, sizeof sa, b1, 64, b2, 64, 0);
    getnameinfo(&sa, sizeof sa, b3, 64, 0, 0, 2 /* NI_NUMERICHOST */);
    printf(" ni=%s/%s/%s", b1, b2, b3);
    struct hostent *he = gethostbyname("printer");
    printf(" hbn=%s/%s", he ? he->name : "-", he ? inet_ntop(2, he->addr_list[0], b1, 64) : "-");
    h.flags = 2; h.family = 2; h.socktype = 1;
    n = getaddrinfo("web.test", "80", &h, &r);
    printf(" cname=%s/%s", n ? "-" : v4(r, b1), n ? "-" : r->canonname);
    if (!n) freeaddrinfo(r);
    h.flags = 0; h.family = 30;
    n = getaddrinfo("real6.test", 0, &h, &r);
    printf(" aaaa=%s", n ? "-" : inet_ntop(30, ((struct sockaddr_in6 *)r->addr)->addr, b1, 64));
    if (!n) freeaddrinfo(r);
    h.family = 2;
    printf(" nx=%d\n", getaddrinfo("missing.test", 0, &h, &r));
    return 0;
}
