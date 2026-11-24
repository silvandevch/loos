/* Sockets and kqueue: socketpair, TCP over loopback (client on a thread), sockaddr layout,
 * socket options, inet_pton/ntop, and kqueue read / timer / user events. No SDK headers. */
typedef unsigned int socklen_t;
typedef long ssize_t;
typedef unsigned long size_t;
typedef struct _opaque_pthread *pthread_t;
struct sockaddr_in { unsigned char len, family; unsigned short port; unsigned int addr; char zero[8]; };
struct kevent { unsigned long ident; short filter; unsigned short flags; unsigned int fflags; long data; void *udata; };
struct timespec { long sec, nsec; };
int socket(int, int, int);
int socketpair(int, int, int, int *);
int bind(int, const void *, socklen_t);
int listen(int, int);
int accept(int, void *, socklen_t *);
int connect(int, const void *, socklen_t);
int getsockname(int, void *, socklen_t *);
int setsockopt(int, int, int, const void *, socklen_t);
int getsockopt(int, int, int, void *, socklen_t *);
ssize_t send(int, const void *, size_t, int);
ssize_t recv(int, void *, size_t, int);
ssize_t write(int, const void *, size_t);
ssize_t read(int, void *, size_t);
int close(int);
int inet_pton(int, const char *, void *);
const char *inet_ntop(int, const void *, char *, socklen_t);
unsigned short htons(unsigned short);
int kqueue(void);
int kevent(int, const struct kevent *, int, struct kevent *, int, const struct timespec *);
int pthread_create(pthread_t *, const void *, void *(*)(void *), void *);
int pthread_join(pthread_t, void **);
int printf(const char *, ...);
int *__error(void);
#define errno (*__error())
int strcmp(const char *, const char *);

#define PORT 8471
static void *client(void *arg) {
    (void)arg;
    int c = socket(2, 1, 0);
    struct sockaddr_in a = { 16, 2, htons(PORT), 0, {0} };
    inet_pton(2, "127.0.0.1", &a.addr);
    long ok = connect(c, &a, sizeof a) == 0 && send(c, "ping", 4, 0) == 4;
    char b[8] = {0};
    ok = ok && recv(c, b, 4, 0) == 4 && b[0] == 'p' && b[1] == 'o';
    close(c);
    return (void *)ok;
}

int main(void) {
    int sv[2];
    char buf[16] = {0};
    int pair = socketpair(1, 1, 0, sv) == 0 && write(sv[0], "hey", 3) == 3 && read(sv[1], buf, 3) == 3 && buf[0] == 'h';

    int s = socket(2, 1, 0), one = 1, got = 0;
    socklen_t gl = sizeof got;
    int opt = setsockopt(s, 0xffff, 0x4, &one, sizeof one) == 0 && getsockopt(s, 0xffff, 0x1008, &got, &gl) == 0 && got == 1;
    struct sockaddr_in a = { 16, 2, htons(PORT), 0, {0} };
    int br = bind(s, &a, sizeof a), be = errno, lr = listen(s, 4);
    int bound = br == 0 && lr == 0;
    if (!bound) printf("net: bind=%d errno=%d listen=%d errno=%d sock=%d\n", br, be, lr, errno, s);
    struct sockaddr_in me; socklen_t ml = sizeof me;
    int name = getsockname(s, &me, &ml) == 0 && me.len == 16 && me.family == 2 && me.port == htons(PORT);

    int kq = kqueue();
    struct kevent ch = { (unsigned long)s, -1, 0x1, 0, 0, (void *)7 }, ev[4];
    kevent(kq, &ch, 1, 0, 0, 0);
    pthread_t t;
    pthread_create(&t, 0, client, 0);
    int n = kevent(kq, 0, 0, ev, 4, 0);
    int kq_accept = n == 1 && ev[0].ident == (unsigned long)s && ev[0].filter == -1 && ev[0].udata == (void *)7;
    struct sockaddr_in peer; socklen_t pl = sizeof peer;
    int c = accept(s, &peer, &pl);
    int peer_ok = c >= 0 && peer.family == 2 && peer.len == 16;
    struct kevent rd = { (unsigned long)c, -1, 0x1 | 0x10, 0, 0, 0 };
    n = kevent(kq, &rd, 1, ev, 4, 0);
    int kq_read = n == 1 && ev[0].ident == (unsigned long)c && ev[0].data == 4;
    int echo = recv(c, buf, 4, 0) == 4 && send(c, "pong", 4, 0) == 4;
    void *cr;
    pthread_join(t, &cr);
    struct timespec zero = {0, 0};
    n = kevent(kq, 0, 0, ev, 4, &zero); /* oneshot read knote is gone */
    int oneshot = n == 0 || ev[0].ident != (unsigned long)c;

    struct kevent tm = { 1, -7, 0x1 | 0x10, 0, 10, 0 };
    n = kevent(kq, &tm, 1, ev, 4, 0);
    int timer = n == 1 && ev[0].filter == -7 && ev[0].data >= 1;
    struct kevent us[2] = { { 5, -10, 0x1 | 0x20, 0, 0, 0 }, { 5, -10, 0, 0x01000000, 0, 0 } };
    n = kevent(kq, us, 2, ev, 4, 0);
    int user = n == 1 && ev[0].filter == -10 && ev[0].ident == 5;
    struct kevent bad = { 0, -4, 0x1, 0, 0, 0 };
    n = kevent(kq, &bad, 1, ev, 4, &zero);
    int unsupported = n == 1 && (ev[0].flags & 0x4000) && ev[0].data == 45;

    unsigned char v6[16]; char txt[64];
    int addr = inet_pton(30, "fe80::1:2", v6) == 1 && strcmp(inet_ntop(30, v6, txt, sizeof txt), "fe80::1:2") == 0;
    printf("net: pair=%d opt=%d bound=%d name=%d kq_accept=%d peer=%d kq_read=%d echo=%d client=%ld oneshot=%d timer=%d user=%d unsupported=%d addr=%d\n",
           pair, opt, bound, name, kq_accept, peer_ok, kq_read, echo, (long)cr, oneshot, timer, user, unsupported, addr);
    return 0;
}
