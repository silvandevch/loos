/* POSIX threads. A thread is a LoOS (Linux-layer) thread of the same process, made by Prism's
 * bsdthread_create; locks sleep on __ulock_wait / __ulock_wake, which Prism maps to futexes.
 *
 * pthread_t points at `struct pthread`; its Darwin TSD (the 512 pointer slots gs / tpidrro_el0
 * points at) follows the header: slot 0 is pthread_self, slot 1 errno, slots from KEY_FIRST are
 * pthread_key_t values. The main thread's lives in the libSystem data page (libsys.rs TSD_MAIN,
 * PTHREAD_HDR); the `tid` word is just below the TSD (lib.rs `new_thread`). */
#include "prism.h"

long __syscall(long n, long a, long b, long c, long d, long e, long f);
int sched_yield(void);
int gettimeofday(void *, void *);
int getpid(void);

#define SYS_munmap 73
#define SYS_mmap 197
#define SYS_bsdthread_create 360
#define SYS_bsdthread_terminate 361
#define SYS_ulock_wait 515
#define SYS_ulock_wake 516
#define UL_COMPARE_AND_WAIT 1
#define ULF_WAKE_ALL 0x100
#define ETIMEDOUT 60
#define EBUSY 16
#define EDEADLK 11
#define ESRCH 3

#define TSD_SLOTS 512
#define KEY_FIRST 16

struct pthread {
    void *(*fn)(void *);
    void *arg;
    void *ret;
    void *stack;
    size_t stacksize;
    uint32_t detached; /* 0 joinable, 1 detached, 2 exited while joinable */
    uint32_t exited;
    struct pthread *next_zombie; /* detached and exited, stack still to unmap */
    uint32_t tid; /* set by Prism; cleared (futex wake) when the thread ends */
    void *tsd[TSD_SLOTS];
};
_Static_assert(__builtin_offsetof(struct pthread, tsd) == 0x40, "PTHREAD_HDR in libsys.rs");
_Static_assert(__builtin_offsetof(struct pthread, tid) == 0x38, "tid word is tsd - 8");
typedef struct pthread *pthread_t;

static void ulock_wait(uint32_t *a, uint32_t v) { __syscall(SYS_ulock_wait, UL_COMPARE_AND_WAIT, (long)a, v, 0, 0, 0); }
static void ulock_wake(uint32_t *a, int all) { __syscall(SYS_ulock_wake, UL_COMPARE_AND_WAIT | (all ? ULF_WAKE_ALL : 0), (long)a, 0, 0, 0, 0); }

static void **tsd(void) {
    void **t;
#if defined(__x86_64__)
    __asm__("mov %%gs:0, %0" : "=r"(t));
    return ((pthread_t)t)->tsd;
#else
    __asm__("mrs %0, tpidrro_el0" : "=r"(t));
    return t;
#endif
}

pthread_t pthread_self(void) { return (pthread_t)tsd()[0]; }
int pthread_main_np(void) { return pthread_self()->stack == 0 && !pthread_self()->fn; }
int pthread_equal(pthread_t a, pthread_t b) { return a == b; }
uint64_t pthread_mach_thread_np(pthread_t t) { return t->tid; }
int pthread_threadid_np(pthread_t t, uint64_t *id) {
    if (!t) t = pthread_self();
    if (id) *id = t->tid ? t->tid : (uint64_t)getpid();
    return 0;
}
int pthread_setname_np(const char *n) { (void)n; return 0; }
int pthread_getname_np(pthread_t t, char *b, size_t n) { (void)t; if (n) b[0] = 0; return 0; }

/* ------------------------------------------------------------------ attributes */

typedef struct { long sig; size_t stacksize; int detached; char opaque[44]; } pthread_attr_t;
#define ATTR_SIG 0x54484441
int pthread_attr_init(pthread_attr_t *a) { memset(a, 0, sizeof *a); a->sig = ATTR_SIG; return 0; }
int pthread_attr_destroy(pthread_attr_t *a) { a->sig = 0; return 0; }
int pthread_attr_setstacksize(pthread_attr_t *a, size_t n) { if (n < 16384) return EINVAL; a->stacksize = n; return 0; }
int pthread_attr_getstacksize(const pthread_attr_t *a, size_t *n) { *n = a->stacksize ? a->stacksize : 512 * 1024; return 0; }
int pthread_attr_setdetachstate(pthread_attr_t *a, int d) { a->detached = d == 2; return 0; } /* PTHREAD_CREATE_DETACHED */
int pthread_attr_getdetachstate(const pthread_attr_t *a, int *d) { *d = a->detached ? 2 : 1; return 0; }
int pthread_attr_setscope(pthread_attr_t *a, int s) { (void)a; (void)s; return 0; }
int pthread_attr_setschedparam(pthread_attr_t *a, const void *p) { (void)a; (void)p; return 0; }
int pthread_attr_getschedparam(const pthread_attr_t *a, void *p) { (void)a; memset(p, 0, 16); return 0; }
int pthread_attr_setguardsize(pthread_attr_t *a, size_t n) { (void)a; (void)n; return 0; }

/* ------------------------------------------------------------------ keys */

static void (*key_dtor[TSD_SLOTS])(void *);
static uint32_t key_used[TSD_SLOTS];
typedef unsigned long pthread_key_t;

int pthread_key_create(pthread_key_t *k, void (*dtor)(void *)) {
    for (unsigned i = KEY_FIRST; i < TSD_SLOTS; i++) {
        uint32_t z = 0;
        if (__atomic_compare_exchange_n(&key_used[i], &z, 1, 0, __ATOMIC_ACQ_REL, __ATOMIC_RELAXED)) {
            key_dtor[i] = dtor;
            *k = i;
            return 0;
        }
    }
    return EAGAIN;
}
int pthread_key_delete(pthread_key_t k) { if (k < KEY_FIRST || k >= TSD_SLOTS) return EINVAL; key_dtor[k] = 0; __atomic_store_n(&key_used[k], 0, __ATOMIC_RELEASE); return 0; }
void *pthread_getspecific(pthread_key_t k) { return k < TSD_SLOTS ? tsd()[k] : 0; }
int pthread_setspecific(pthread_key_t k, const void *v) { if (k >= TSD_SLOTS) return EINVAL; tsd()[k] = (void *)v; return 0; }

/* ------------------------------------------------------------------ create / exit / join */

static void run_dtors(void) {
    void **t = tsd();
    for (int round = 0; round < 4; round++) {
        int any = 0;
        for (unsigned i = KEY_FIRST; i < TSD_SLOTS; i++) {
            void *v = t[i];
            if (v && key_dtor[i]) { t[i] = 0; key_dtor[i](v); any = 1; }
        }
        if (!any) break;
    }
}

static struct pthread *zombies;

static void unmap_thread(pthread_t t) { __syscall(SYS_munmap, (long)t->stack, (long)t->stacksize, 0, 0, 0, 0); }

/* Unmap queued detached threads that have finished; keep the ones still on their way out. */
static void reap_zombies(void) {
    struct pthread *l = __atomic_exchange_n(&zombies, 0, __ATOMIC_ACQUIRE), *n;
    for (; l; l = n) {
        n = l->next_zombie;
        if (__atomic_load_n(&l->tid, __ATOMIC_ACQUIRE) == 0) {
            unmap_thread(l);
        } else {
            struct pthread *h = __atomic_load_n(&zombies, __ATOMIC_RELAXED);
            do l->next_zombie = h;
            while (!__atomic_compare_exchange_n(&zombies, &h, l, 1, __ATOMIC_RELEASE, __ATOMIC_RELAXED));
        }
    }
}

void pthread_exit(void *v) __attribute__((noreturn));
void pthread_exit(void *v) {
    pthread_t self = pthread_self();
    if (pthread_main_np()) exit(0);
    self->ret = v;
    run_dtors();
    __atomic_store_n(&self->exited, 1, __ATOMIC_RELEASE);
    /* A detached thread cannot unmap the stack it runs on: it queues itself, and a later
     * pthread_create unmaps it once the kernel has cleared its tid (the thread is gone). */
    uint32_t joinable = 0;
    if (!__atomic_compare_exchange_n(&self->detached, &joinable, 2, 0, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE)) {
        struct pthread *h = __atomic_load_n(&zombies, __ATOMIC_RELAXED);
        do self->next_zombie = h;
        while (!__atomic_compare_exchange_n(&zombies, &h, self, 1, __ATOMIC_RELEASE, __ATOMIC_RELAXED));
    }
    for (;;) __syscall(SYS_bsdthread_terminate, 0, 0, 0, 0, 0, 0);
}

static void thread_start(void *arg, void **t) __attribute__((noreturn, used));
static void thread_start(void *arg, void **t) {
    (void)arg;
    pthread_t self = (pthread_t)t[0];
    pthread_exit(self->fn(self->arg));
}

int pthread_create(pthread_t *out, const pthread_attr_t *attr, void *(*fn)(void *), void *arg) {
    reap_zombies();
    size_t ss = attr && attr->stacksize ? attr->stacksize : 512 * 1024;
    ss = (ss + 4095) & ~(size_t)4095;
    size_t total = ss + sizeof(struct pthread);
    long m = __syscall(SYS_mmap, 0, (long)total, 3, 0x1002, -1, 0); /* RW, MAP_PRIVATE|MAP_ANON */
    if (m == -1) return EAGAIN;
    pthread_t t = (pthread_t)((char *)m + ss);
    memset(t, 0, sizeof *t);
    t->fn = fn;
    t->arg = arg;
    t->stack = (void *)m;
    t->stacksize = total;
    t->detached = attr && attr->detached;
    t->tsd[0] = t;
    long r = __syscall(SYS_bsdthread_create, (long)thread_start, (long)arg, (long)t, (long)t->tsd, 0, 0);
    if (r == -1) {
        int e = errno;
        __syscall(SYS_munmap, m, (long)total, 0, 0, 0, 0);
        return e;
    }
    if (out) *out = t;
    return 0;
}

int pthread_join(pthread_t t, void **ret) {
    if (!t) return ESRCH;
    if (t == pthread_self()) return EDEADLK;
    for (;;) {
        uint32_t id = __atomic_load_n(&t->tid, __ATOMIC_ACQUIRE);
        if (!id) break;
        ulock_wait(&t->tid, id);
    }
    if (ret) *ret = t->ret;
    unmap_thread(t);
    return 0;
}

int pthread_detach(pthread_t t) {
    uint32_t joinable = 0;
    if (__atomic_compare_exchange_n(&t->detached, &joinable, 1, 0, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE)) return 0;
    if (joinable == 1) return EINVAL;
    return pthread_join(t, 0); /* already exited: release it now */
}
int pthread_cancel(pthread_t t) { (void)t; return 0; }
void pthread_testcancel(void) {}
int pthread_setcancelstate(int s, int *o) { (void)s; if (o) *o = 0; return 0; }
int pthread_setcanceltype(int s, int *o) { (void)s; if (o) *o = 0; return 0; }
size_t pthread_get_stacksize_np(pthread_t t) { return t->stacksize ? t->stacksize - sizeof *t : 8 << 20; }
void *pthread_get_stackaddr_np(pthread_t t) { return t->stack ? (void *)t : (void *)t; }

/* ------------------------------------------------------------------ once */

typedef struct { long sig; uint32_t state; char opaque[4]; } pthread_once_t;
int pthread_once(pthread_once_t *o, void (*init)(void)) {
    uint32_t z = 0;
    if (__atomic_load_n(&o->state, __ATOMIC_ACQUIRE) == 2) return 0;
    if (__atomic_compare_exchange_n(&o->state, &z, 1, 0, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE)) {
        init();
        __atomic_store_n(&o->state, 2, __ATOMIC_RELEASE);
        ulock_wake(&o->state, 1);
        return 0;
    }
    while ((z = __atomic_load_n(&o->state, __ATOMIC_ACQUIRE)) != 2) ulock_wait(&o->state, z);
    return 0;
}

/* ------------------------------------------------------------------ mutexes */

/* pthread_mutex_t is 64 bytes: {long sig; opaque[56]}. Static initializers only set `sig`, so a
 * zero opaque part is a valid unlocked mutex; the initializer's signature gives its type. */
struct mstate { uint32_t word; uint32_t type; pthread_t owner; uint32_t depth; };
typedef struct { long sig; struct mstate m; char opaque[56 - sizeof(struct mstate)]; } pthread_mutex_t;
typedef struct { long sig; int type; char opaque[4]; } pthread_mutexattr_t;
#define MUTEX_NORMAL 0
#define MUTEX_ERRORCHECK 1
#define MUTEX_RECURSIVE 2

int pthread_mutexattr_init(pthread_mutexattr_t *a) { a->sig = 0x4d545841; a->type = 0; return 0; }
int pthread_mutexattr_destroy(pthread_mutexattr_t *a) { (void)a; return 0; }
int pthread_mutexattr_settype(pthread_mutexattr_t *a, int t) { if (t < 0 || t > 2) return EINVAL; a->type = t; return 0; }
int pthread_mutexattr_gettype(const pthread_mutexattr_t *a, int *t) { *t = a->type; return 0; }
int pthread_mutexattr_setpshared(pthread_mutexattr_t *a, int s) { (void)a; (void)s; return 0; }
int pthread_mutexattr_setpolicy_np(pthread_mutexattr_t *a, int p) { (void)a; (void)p; return 0; }

int pthread_mutex_init(pthread_mutex_t *m, const pthread_mutexattr_t *a) {
    memset(&m->m, 0, sizeof m->m);
    m->sig = 0x4d555458;
    m->m.type = a ? (uint32_t)a->type : MUTEX_NORMAL;
    return 0;
}
int pthread_mutex_destroy(pthread_mutex_t *m) { return m->m.word ? EBUSY : 0; }

static uint32_t mtype(pthread_mutex_t *m) {
    if (m->sig == 0x32AAABA1) return MUTEX_ERRORCHECK;
    if (m->sig == 0x32AAABA2) return MUTEX_RECURSIVE;
    return m->m.type;
}

/* The word: 0 free, 1 locked, 2 locked with (possible) waiters. */
static void word_lock(uint32_t *w) {
    uint32_t c = 0;
    if (__atomic_compare_exchange_n(w, &c, 1, 0, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) return;
    if (c != 2) c = __atomic_exchange_n(w, 2, __ATOMIC_ACQUIRE);
    while (c != 0) {
        ulock_wait(w, 2);
        c = __atomic_exchange_n(w, 2, __ATOMIC_ACQUIRE);
    }
}
static void word_unlock(uint32_t *w) {
    if (__atomic_exchange_n(w, 0, __ATOMIC_RELEASE) == 2) ulock_wake(w, 0);
}

int pthread_mutex_lock(pthread_mutex_t *m) {
    uint32_t t = mtype(m);
    pthread_t me = pthread_self();
    if (t != MUTEX_NORMAL && m->m.owner == me) {
        if (t == MUTEX_ERRORCHECK) return EDEADLK;
        m->m.depth++;
        return 0;
    }
    word_lock(&m->m.word);
    m->m.owner = me;
    m->m.depth = 1;
    return 0;
}
int pthread_mutex_trylock(pthread_mutex_t *m) {
    pthread_t me = pthread_self();
    if (mtype(m) == MUTEX_RECURSIVE && m->m.owner == me) { m->m.depth++; return 0; }
    uint32_t c = 0;
    if (!__atomic_compare_exchange_n(&m->m.word, &c, 1, 0, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) return EBUSY;
    m->m.owner = me;
    m->m.depth = 1;
    return 0;
}
int pthread_mutex_unlock(pthread_mutex_t *m) {
    uint32_t t = mtype(m);
    if (t != MUTEX_NORMAL) {
        if (m->m.owner != pthread_self()) return EPERM;
        if (--m->m.depth) return 0;
    }
    m->m.owner = 0;
    word_unlock(&m->m.word);
    return 0;
}

/* os_unfair_lock: a 32-bit word. */
typedef struct { uint32_t word; } os_unfair_lock;
void os_unfair_lock_lock(os_unfair_lock *l) { word_lock(&l->word); }
int os_unfair_lock_trylock(os_unfair_lock *l) { uint32_t c = 0; return __atomic_compare_exchange_n(&l->word, &c, 1, 0, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED); }
void os_unfair_lock_unlock(os_unfair_lock *l) { word_unlock(&l->word); }
void os_unfair_lock_assert_owner(const os_unfair_lock *l) { (void)l; }
void os_unfair_lock_assert_not_owner(const os_unfair_lock *l) { (void)l; }
void OSSpinLockLock(uint32_t *l) { word_lock(l); }
void OSSpinLockUnlock(uint32_t *l) { word_unlock(l); }
int OSSpinLockTry(uint32_t *l) { uint32_t c = 0; return __atomic_compare_exchange_n(l, &c, 1, 0, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED); }

/* ------------------------------------------------------------------ condition variables */

typedef struct { long sig; uint32_t seq; char opaque[36]; } pthread_cond_t;
typedef struct { long sig; char opaque[8]; } pthread_condattr_t;
int pthread_condattr_init(pthread_condattr_t *a) { a->sig = 0; return 0; }
int pthread_condattr_destroy(pthread_condattr_t *a) { (void)a; return 0; }
int pthread_cond_init(pthread_cond_t *c, const pthread_condattr_t *a) { (void)a; c->sig = 0x434f4e44; c->seq = 0; return 0; }
int pthread_cond_destroy(pthread_cond_t *c) { (void)c; return 0; }
int pthread_cond_signal(pthread_cond_t *c) { __atomic_fetch_add(&c->seq, 1, __ATOMIC_RELEASE); ulock_wake(&c->seq, 0); return 0; }
int pthread_cond_broadcast(pthread_cond_t *c) { __atomic_fetch_add(&c->seq, 1, __ATOMIC_RELEASE); ulock_wake(&c->seq, 1); return 0; }

/* Unlock fully (a recursive mutex may be held several times), wait, relock. */
static uint32_t release_all(pthread_mutex_t *m) { uint32_t d = m->m.depth; m->m.depth = 1; pthread_mutex_unlock(m); return d; }
static void reacquire(pthread_mutex_t *m, uint32_t d) { pthread_mutex_lock(m); m->m.depth = d; }

int pthread_cond_wait(pthread_cond_t *c, pthread_mutex_t *m) {
    uint32_t s = __atomic_load_n(&c->seq, __ATOMIC_ACQUIRE);
    uint32_t d = release_all(m);
    ulock_wait(&c->seq, s);
    reacquire(m, d);
    return 0;
}

struct timespec_ { long tv_sec; long tv_nsec; };
static long long now_us(void) { struct { long s; int us; } tv; gettimeofday(&tv, 0); return (long long)tv.s * 1000000 + tv.us; }

/* Timed waits sleep on the futex with the remaining time (Linux FUTEX_WAIT takes a relative
 * timeout; the deadline is wall-clock time, as on macOS). */
int pthread_cond_timedwait(pthread_cond_t *c, pthread_mutex_t *m, const struct timespec_ *abs) {
    long long until = (long long)abs->tv_sec * 1000000 + abs->tv_nsec / 1000;
    uint32_t s = __atomic_load_n(&c->seq, __ATOMIC_ACQUIRE);
    uint32_t d = release_all(m);
    int r = 0;
    while (__atomic_load_n(&c->seq, __ATOMIC_ACQUIRE) == s) {
        long long left = until - now_us();
        if (left <= 0) { r = ETIMEDOUT; break; }
        struct timespec_ rel = { (long)(left / 1000000), (long)(left % 1000000) * 1000 };
        __syscall(0x10000 + 202, (long)&c->seq, 128, s, (long)&rel, 0, 0); /* FUTEX_WAIT_PRIVATE */
    }
    reacquire(m, d);
    return r;
}
int pthread_cond_timedwait_relative_np(pthread_cond_t *c, pthread_mutex_t *m, const struct timespec_ *rel) {
    long long t = now_us() + (long long)rel->tv_sec * 1000000 + rel->tv_nsec / 1000;
    struct timespec_ abs = { (long)(t / 1000000), (long)(t % 1000000) * 1000 };
    return pthread_cond_timedwait(c, m, &abs);
}

/* ------------------------------------------------------------------ read-write locks */

/* pthread_rwlock_t is 200 bytes. `state`: writer bit 31, reader count below; waiters sleep on `seq`. */
typedef struct { long sig; uint32_t state; uint32_t seq; char opaque[184]; } pthread_rwlock_t;
#define WRITER 0x80000000u
int pthread_rwlock_init(pthread_rwlock_t *l, const void *a) { (void)a; l->sig = 0x52574c4b; l->state = l->seq = 0; return 0; }
int pthread_rwlock_destroy(pthread_rwlock_t *l) { return l->state ? EBUSY : 0; }
int pthread_rwlock_tryrdlock(pthread_rwlock_t *l) {
    uint32_t s = __atomic_load_n(&l->state, __ATOMIC_RELAXED);
    while (!(s & WRITER))
        if (__atomic_compare_exchange_n(&l->state, &s, s + 1, 1, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) return 0;
    return EBUSY;
}
int pthread_rwlock_trywrlock(pthread_rwlock_t *l) {
    uint32_t z = 0;
    return __atomic_compare_exchange_n(&l->state, &z, WRITER, 0, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED) ? 0 : EBUSY;
}
static int rw_wait(pthread_rwlock_t *l, int (*try)(pthread_rwlock_t *)) {
    for (;;) {
        uint32_t q = __atomic_load_n(&l->seq, __ATOMIC_ACQUIRE);
        if (try(l) == 0) return 0;
        ulock_wait(&l->seq, q);
    }
}
int pthread_rwlock_rdlock(pthread_rwlock_t *l) { return rw_wait(l, pthread_rwlock_tryrdlock); }
int pthread_rwlock_wrlock(pthread_rwlock_t *l) { return rw_wait(l, pthread_rwlock_trywrlock); }
int pthread_rwlock_unlock(pthread_rwlock_t *l) {
    uint32_t s = __atomic_load_n(&l->state, __ATOMIC_RELAXED);
    if (s & WRITER) __atomic_store_n(&l->state, 0, __ATOMIC_RELEASE);
    else if (s) __atomic_fetch_sub(&l->state, 1, __ATOMIC_RELEASE);
    else return EPERM;
    __atomic_fetch_add(&l->seq, 1, __ATOMIC_RELEASE);
    ulock_wake(&l->seq, 1);
    return 0;
}
