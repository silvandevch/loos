/* kqueue / kevent on the Linux layer's epoll. A kqueue is an epoll descriptor plus a list of
 * knotes. EVFILT_READ / EVFILT_WRITE knotes on a descriptor become one epoll registration for it
 * (the union of the enabled filters; EPOLLET when one asks for EV_CLEAR). EVFILT_TIMER knotes own
 * a timerfd and EVFILT_USER knotes an eventfd, registered with the knote as epoll data.
 * Not yet: EVFILT_VNODE, EVFILT_PROC, EVFILT_SIGNAL, EVFILT_MACHPORT (registration fails with
 * ENOTSUP). Calls go to Linux through Prism's passthrough system calls (darwin.rs). */
#include "prism.h"

long __syscall(long n, long a, long b, long c, long d, long e, long f);
#define lx(nr, a, b, c, d, e, f) __syscall(0x10000 + (nr), (long)(a), (long)(b), (long)(c), (long)(d), (long)(e), (long)(f))

typedef struct { uint32_t word; } os_unfair_lock;
void os_unfair_lock_lock(os_unfair_lock *);
void os_unfair_lock_unlock(os_unfair_lock *);

#define EVFILT_READ (-1)
#define EVFILT_WRITE (-2)
#define EVFILT_TIMER (-7)
#define EVFILT_USER (-10)
#define EV_ADD 0x1
#define EV_DELETE 0x2
#define EV_ENABLE 0x4
#define EV_DISABLE 0x8
#define EV_ONESHOT 0x10
#define EV_CLEAR 0x20
#define EV_RECEIPT 0x40
#define EV_DISPATCH 0x80
#define EV_ERROR 0x4000
#define EV_EOF 0x8000
#define NOTE_TRIGGER 0x01000000u
#define NOTE_FFCTRLMASK 0xc0000000u
#define NOTE_FFLAGSMASK 0x00ffffffu
#define NOTE_SECONDS 0x1
#define NOTE_USECONDS 0x2
#define NOTE_NSECONDS 0x4
#define ENOTSUP 45

#define L_EPOLLIN 0x1
#define L_EPOLLOUT 0x4
#define L_EPOLLERR 0x8
#define L_EPOLLHUP 0x10
#define L_EPOLLRDHUP 0x2000
#define L_EPOLLET 0x80000000u
#define L_NONBLOCK_CLOEXEC (0x800 | 0x80000)
#define AUX_TAG (1ull << 63)

struct kevent { uintptr_t ident; int16_t filter; uint16_t flags; uint32_t fflags; intptr_t data; void *udata; };
struct kevent64_s { uint64_t ident; int16_t filter; uint16_t flags; uint32_t fflags; int64_t data; uint64_t udata; uint64_t ext[2]; };
struct epoll_event { uint32_t events; uint64_t data; } __attribute__((packed));
struct timespec_ { long sec; long nsec; };

struct knote {
    uint64_t ident;
    int16_t filter;
    uint16_t flags;
    uint32_t fflags;
    int64_t data;
    uint64_t udata;
    int enabled;
    int aux; /* timerfd / eventfd, or -1 */
    struct knote *next;
};
struct kq { struct knote *notes; os_unfair_lock lock; };

#define MAX_KQ 4096
static struct kq *kqs[MAX_KQ];

static struct kq *kq_of(int fd) {
    if (fd < 0 || fd >= MAX_KQ || !kqs[fd]) { errno = EBADF; return 0; }
    return kqs[fd];
}

int kqueue(void) {
    int fd = (int)lx(291, 0x80000, 0, 0, 0, 0, 0); /* epoll_create1(EPOLL_CLOEXEC) */
    if (fd < 0) return -1;
    if (fd >= MAX_KQ) { lx(3, fd, 0, 0, 0, 0, 0); errno = 24; return -1; } /* EMFILE */
    struct kq *q = kqs[fd];
    if (q) { /* the descriptor was a kqueue before: forget the old one's knotes */
        for (struct knote *k = q->notes, *n; k; k = n) { n = k->next; if (k->aux >= 0) lx(3, k->aux, 0, 0, 0, 0, 0); free(k); }
    } else {
        q = calloc(1, sizeof *q);
        if (!q) { lx(3, fd, 0, 0, 0, 0, 0); errno = ENOMEM; return -1; }
    }
    q->notes = 0;
    q->lock.word = 0;
    kqs[fd] = q;
    return fd;
}

static struct knote *find(struct kq *q, uint64_t ident, int16_t filter) {
    for (struct knote *k = q->notes; k; k = k->next)
        if (k->ident == ident && k->filter == filter) return k;
    return 0;
}

/* Re-register descriptor `fd` with the union of its enabled READ / WRITE knotes. */
static int sync_fd(int ep, struct kq *q, uint64_t fd) {
    uint32_t ev = 0;
    for (struct knote *k = q->notes; k; k = k->next) {
        if (k->ident != fd || !k->enabled || (k->filter != EVFILT_READ && k->filter != EVFILT_WRITE)) continue;
        ev |= k->filter == EVFILT_READ ? L_EPOLLIN | L_EPOLLRDHUP : L_EPOLLOUT;
        if (k->flags & EV_CLEAR) ev |= L_EPOLLET;
    }
    struct epoll_event e = { ev, fd };
    if (!ev) {
        lx(233, ep, 2, fd, &e, 0, 0); /* EPOLL_CTL_DEL; not registered is fine */
        return 0;
    }
    if (lx(233, ep, 3, fd, &e, 0, 0) == 0) return 0; /* MOD */
    return lx(233, ep, 1, fd, &e, 0, 0) == 0 ? 0 : errno; /* ADD */
}

static int64_t timer_ns(const struct knote *k) {
    int64_t v = k->data;
    if (k->fflags & NOTE_NSECONDS) return v;
    if (k->fflags & NOTE_USECONDS) return v * 1000;
    if (k->fflags & NOTE_SECONDS) return v * 1000000000;
    return v * 1000000; /* milliseconds */
}

static int arm_aux(int ep, struct knote *k) {
    if (k->filter == EVFILT_TIMER) {
        int64_t ns = timer_ns(k);
        if (ns <= 0) ns = 1;
        struct timespec_ its[2]; /* interval, value */
        its[1].sec = ns / 1000000000; its[1].nsec = ns % 1000000000;
        its[0] = (k->flags & EV_ONESHOT) ? (struct timespec_){ 0, 0 } : its[1];
        if (!k->enabled) its[1] = its[0] = (struct timespec_){ 0, 0 };
        if (lx(286, k->aux, 0, its, 0, 0, 0) < 0) return errno;
    }
    struct epoll_event e = { L_EPOLLIN, AUX_TAG | (uint64_t)(uintptr_t)k };
    if (lx(233, ep, 3, k->aux, &e, 0, 0) == 0) return 0;
    return lx(233, ep, 1, k->aux, &e, 0, 0) == 0 ? 0 : errno;
}

static void drop(int ep, struct kq *q, struct knote *k) {
    struct knote **pp = &q->notes;
    while (*pp && *pp != k) pp = &(*pp)->next;
    if (*pp) *pp = k->next;
    if (k->aux >= 0) {
        lx(233, ep, 2, k->aux, 0, 0, 0);
        lx(3, k->aux, 0, 0, 0, 0, 0);
    } else {
        sync_fd(ep, q, k->ident);
    }
    free(k);
}

/* Apply one change; returns 0 or a Darwin errno. */
static int apply(int ep, struct kq *q, const struct kevent64_s *c) {
    struct knote *k = find(q, c->ident, c->filter);
    int f = c->filter;
    if (f != EVFILT_READ && f != EVFILT_WRITE && f != EVFILT_TIMER && f != EVFILT_USER)
        return f < 0 && f >= -17 ? ENOTSUP : EINVAL;
    if (c->flags & EV_DELETE) {
        if (!k) return ENOENT;
        drop(ep, q, k);
        return 0;
    }
    if (!k) {
        if (!(c->flags & EV_ADD)) return ENOENT;
        k = calloc(1, sizeof *k);
        if (!k) return ENOMEM;
        k->ident = c->ident;
        k->filter = c->filter;
        k->aux = -1;
        k->enabled = 1;
        if (f == EVFILT_TIMER) k->aux = (int)lx(283, 1, L_NONBLOCK_CLOEXEC, 0, 0, 0, 0); /* CLOCK_MONOTONIC */
        if (f == EVFILT_USER) k->aux = (int)lx(290, 0, L_NONBLOCK_CLOEXEC, 0, 0, 0, 0);
        if ((f == EVFILT_TIMER || f == EVFILT_USER) && k->aux < 0) { int e = errno; free(k); return e; }
        k->next = q->notes;
        q->notes = k;
    }
    k->flags = (uint16_t)(c->flags & (EV_ONESHOT | EV_CLEAR | EV_DISPATCH));
    k->udata = c->udata;
    if (c->flags & EV_ENABLE) k->enabled = 1;
    if (c->flags & EV_DISABLE) k->enabled = 0;
    if (f == EVFILT_USER) {
        uint32_t ff = c->fflags & NOTE_FFLAGSMASK;
        switch (c->fflags & NOTE_FFCTRLMASK) {
        case 0x40000000u: k->fflags &= ff; break;  /* NOTE_FFAND */
        case 0x80000000u: k->fflags |= ff; break;  /* NOTE_FFOR */
        case 0xc0000000u: k->fflags = ff; break;   /* NOTE_FFCOPY */
        default: break;
        }
        if (c->fflags & NOTE_TRIGGER) { uint64_t one = 1; lx(1, k->aux, &one, 8, 0, 0, 0); }
        return arm_aux(ep, k);
    }
    if (f == EVFILT_TIMER) {
        k->fflags = c->fflags;
        k->data = c->data;
        return arm_aux(ep, k);
    }
    k->fflags = c->fflags;
    int e = sync_fd(ep, q, k->ident);
    if (e) { drop(ep, q, k); return e; }
    return 0;
}

static int bytes_ready(uint64_t fd) {
    int n = 0;
    if (lx(16, fd, 0x541B, &n, 0, 0, 0) < 0) return 1; /* FIONREAD; a listener has no byte count */
    return n;
}

/* Turn epoll's ready list into kevents. */
static int collect(int ep, struct kq *q, const struct epoll_event *ev, int n, struct kevent64_s *out, int max) {
    int o = 0;
    for (int i = 0; i < n && o < max; i++) {
        if (ev[i].data & AUX_TAG) {
            struct knote *k = (struct knote *)(uintptr_t)(ev[i].data & ~AUX_TAG);
            uint64_t cnt = 0;
            lx(0, k->aux, &cnt, 8, 0, 0, 0);
            if (!k->enabled) continue;
            out[o] = (struct kevent64_s){ k->ident, k->filter, k->flags, k->fflags, k->filter == EVFILT_TIMER ? (int64_t)cnt : 0, k->udata, { 0, 0 } };
            o++;
            if (k->flags & EV_ONESHOT) drop(ep, q, k);
            else if (k->flags & EV_DISPATCH) { k->enabled = 0; if (k->filter == EVFILT_TIMER) arm_aux(ep, k); }
            continue;
        }
        uint64_t fd = ev[i].data;
        uint32_t e = ev[i].events;
        for (int pass = 0; pass < 2 && o < max; pass++) {
            int16_t filt = pass ? EVFILT_WRITE : EVFILT_READ;
            struct knote *k = find(q, fd, filt);
            if (!k || !k->enabled) continue;
            uint32_t want = pass ? L_EPOLLOUT | L_EPOLLHUP | L_EPOLLERR : L_EPOLLIN | L_EPOLLHUP | L_EPOLLERR | L_EPOLLRDHUP;
            if (!(e & want)) continue;
            uint16_t fl = k->flags;
            if (e & (L_EPOLLHUP | L_EPOLLRDHUP)) fl |= EV_EOF;
            out[o] = (struct kevent64_s){ fd, filt, fl, 0, pass ? 65536 : bytes_ready(fd), k->udata, { 0, 0 } };
            o++;
            if (k->flags & EV_ONESHOT) drop(ep, q, k);
            else if (k->flags & EV_DISPATCH) { k->enabled = 0; sync_fd(ep, q, fd); }
        }
    }
    return o;
}

static int kevent_core(int kq, const struct kevent64_s *ch, int nch, struct kevent64_s *out, int nout, const struct timespec_ *ts) {
    struct kq *q = kq_of(kq);
    if (!q) return -1;
    int o = 0;
    os_unfair_lock_lock(&q->lock);
    for (int i = 0; i < nch; i++) {
        int e = apply(kq, q, &ch[i]);
        if (e || (ch[i].flags & EV_RECEIPT)) {
            if (o < nout) {
                out[o] = ch[i];
                out[o].flags = EV_ERROR;
                out[o].data = e;
                o++;
            } else if (e) {
                os_unfair_lock_unlock(&q->lock);
                errno = e;
                return -1;
            }
        }
    }
    os_unfair_lock_unlock(&q->lock);
    if (o || nout <= 0) return o;
    int ms = -1;
    if (ts) {
        long long t = (long long)ts->sec * 1000 + (ts->nsec + 999999) / 1000000;
        ms = t > 0x7fffffff ? 0x7fffffff : (int)t;
    }
    struct epoll_event ev[64];
    int max = nout < 64 ? nout : 64;
    for (;;) {
        int n = (int)lx(232, kq, ev, max, ms, 0, 0);
        if (n < 0) return -1;
        os_unfair_lock_lock(&q->lock);
        o = collect(kq, q, ev, n, out, nout);
        os_unfair_lock_unlock(&q->lock);
        /* Every ready entry may have been filtered (disabled knote, other filter): wait again. */
        if (o || n == 0 || ms == 0) return o;
    }
}

int kevent64(int kq, const struct kevent64_s *ch, int nch, struct kevent64_s *out, int nout, unsigned flags, const struct timespec_ *ts) {
    (void)flags;
    return kevent_core(kq, ch, nch, out, nout, ts);
}

int kevent(int kq, const struct kevent *ch, int nch, struct kevent *out, int nout, const struct timespec_ *ts) {
    struct kevent64_s c64[64], o64[64];
    int done = 0;
    /* Changes in batches; the events come back with the last batch. */
    do {
        int n = nch - done > 64 ? 64 : nch - done;
        for (int i = 0; i < n; i++) {
            const struct kevent *c = &ch[done + i];
            c64[i] = (struct kevent64_s){ c->ident, c->filter, c->flags, c->fflags, c->data, (uint64_t)(uintptr_t)c->udata, { 0, 0 } };
        }
        done += n;
        int last = done >= nch;
        int max = last ? (nout < 64 ? nout : 64) : 0;
        int r = kevent_core(kq, c64, n, o64, max, last ? ts : &(struct timespec_){ 0, 0 });
        if (r < 0) return -1;
        if (last) {
            for (int i = 0; i < r; i++)
                out[i] = (struct kevent){ (uintptr_t)o64[i].ident, o64[i].filter, o64[i].flags, o64[i].fflags, (intptr_t)o64[i].data, (void *)(uintptr_t)o64[i].udata };
            return r;
        }
    } while (1);
}
