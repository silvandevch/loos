/* libdispatch (Grand Central Dispatch) for macOS programs: queues (main, global, serial, concurrent),
 * async/sync/after/apply/barrier, groups, semaphores, queue-specific data, and dispatch sources
 * (timers and the DATA_ADD/OR/REPLACE kinds; the other kinds can be created but never fire).
 *
 * Dispatch objects are C structs whose first word is an Objective-C isa (OS_dispatch_* classes
 * below), so ARC code can retain and release them as it does on macOS. The main queue and the
 * global queues are static and never freed: the SDK's dispatch_get_main_queue() is the address of
 * the data symbol _dispatch_main_q.
 *
 * Work runs on a pool of worker threads. Global queues hand each item to the pool; a serial queue
 * (and, here, a custom concurrent queue: barriers are then trivially right) is drained by one pool
 * job at a time. The main queue is drained on the main thread by the run loop (cfrunloop.m) or by
 * dispatch_main(). One timer thread serves dispatch_after and timer sources. Times are absolute
 * wall-clock nanoseconds (both dispatch_time and dispatch_walltime). */
#include "objc.h"

typedef unsigned long NSUInteger;
__attribute__((objc_root_class))
@interface NSObject { Class isa; }
+ (Class)class;
@end

/* ---- pthread / libc (macOS layouts; implemented in pthread.c) ---- */
typedef struct { long sig; char opaque[56]; } pthread_mutex_t;
typedef struct { long sig; char opaque[40]; } pthread_cond_t;
typedef struct pthread *pthread_t;
typedef struct { long sig; char opaque[56]; } pthread_attr_t;
typedef unsigned long pthread_key_t;
#define MUTEX_INIT {0x32AAABA7, {0}}
#define COND_INIT {0x3CB0B1BB, {0}}
struct timespec { long tv_sec; long tv_nsec; };
struct timeval { long tv_sec; int tv_usec; };
int pthread_mutex_lock(pthread_mutex_t *);
int pthread_mutex_unlock(pthread_mutex_t *);
int pthread_mutex_init(pthread_mutex_t *, const void *);
int pthread_cond_init(pthread_cond_t *, const void *);
int pthread_cond_wait(pthread_cond_t *, pthread_mutex_t *);
int pthread_cond_timedwait(pthread_cond_t *, pthread_mutex_t *, const struct timespec *);
int pthread_cond_signal(pthread_cond_t *);
int pthread_cond_broadcast(pthread_cond_t *);
int pthread_create(pthread_t *, const pthread_attr_t *, void *(*)(void *), void *);
int pthread_detach(pthread_t);
int pthread_main_np(void);
int pthread_key_create(pthread_key_t *, void (*)(void *));
void *pthread_getspecific(pthread_key_t);
int pthread_setspecific(pthread_key_t, const void *);
int gettimeofday(struct timeval *, void *);
void *calloc(size_t, size_t);
void free(void *);
void exit(int);
int snprintf(char *, size_t, const char *, ...);
void *objc_autoreleasePoolPush(void);
void objc_autoreleasePoolPop(void *);

/* ---- blocks ---- */
struct block { void *isa; int flags, reserved; void (*invoke)(void *, ...); };
void *_Block_copy(const void *);
void _Block_release(const void *);
static void call_block(void *b) { ((void (*)(void *))((struct block *)b)->invoke)(b); }
static void call_release_block(void *b) { call_block(b); _Block_release(b); }

typedef uint64_t dispatch_time_t;
#define TIME_NOW 0ull
#define TIME_FOREVER (~0ull)
typedef void (*fn_t)(void *);

/* ---- objects ---- */
enum { K_QUEUE = 1, K_GROUP, K_SEMA, K_SOURCE };
struct dobj {
    void *isa;
    volatile long rc;
    int kind, immortal;
    void *ctx;
    fn_t finalizer;
};
struct item { struct item *next; fn_t fn; void *ctx; };
struct spec { struct spec *next; const void *key; void *ctx; fn_t dtor; };
struct dq {
    struct dobj o;
    pthread_mutex_t mu;
    struct item *head, *tail;
    int global;       /* items go straight to the pool */
    int scheduled;    /* a drain job is queued or running */
    int suspended;
    int main;
    struct spec *specs;
    char label[64];
};
struct dgroup { struct dobj o; pthread_mutex_t mu; pthread_cond_t cv; long count; struct item *notify; };
struct dsema { struct dobj o; pthread_mutex_t mu; pthread_cond_t cv; long value, waiters, orig; };
struct dsource {
    struct dobj o;
    pthread_mutex_t mu;
    const void *type;
    struct dq *q;
    void *handler, *cancel_handler, *reg_handler; /* blocks (copied) or functions */
    int handler_is_f, cancel_is_f;
    uint64_t start, interval;
    unsigned long data, pending, mask;
    int suspended, cancelled, fire_queued, armed, activated;
    uint64_t gen; /* bumps on set_timer, so stale timer entries are ignored */
};

@interface OS_dispatch_object : NSObject @end
@interface OS_dispatch_queue : OS_dispatch_object @end
@interface OS_dispatch_group : OS_dispatch_object @end
@interface OS_dispatch_semaphore : OS_dispatch_object @end
@interface OS_dispatch_source : OS_dispatch_object @end

extern char cls_queue __asm__("_OBJC_CLASS_$_OS_dispatch_queue");

/* The main queue and the global queues (one per QoS: background, utility, default, user-initiated). */
#define STATIC_QUEUE(lbl, g, m) {{&cls_queue, 1, K_QUEUE, 1, 0, 0}, MUTEX_INIT, 0, 0, g, 0, 0, m, 0, lbl}
struct dq _dispatch_main_q = STATIC_QUEUE("com.apple.main-thread", 0, 1);
static struct dq globals[4] = {
    STATIC_QUEUE("com.apple.root.background-qos", 1, 0), STATIC_QUEUE("com.apple.root.utility-qos", 1, 0),
    STATIC_QUEUE("com.apple.root.default-qos", 1, 0), STATIC_QUEUE("com.apple.root.user-initiated-qos", 1, 0),
};
/* Queue attributes and source types are compared by address only. */
const long _dispatch_queue_attr_concurrent[4] = {1};
const long _dispatch_source_type_timer[4] = {1}, _dispatch_source_type_data_add[4] = {2}, _dispatch_source_type_data_or[4] = {3},
    _dispatch_source_type_data_replace[4] = {4}, _dispatch_source_type_read[4] = {5}, _dispatch_source_type_write[4] = {6},
    _dispatch_source_type_signal[4] = {7}, _dispatch_source_type_proc[4] = {8}, _dispatch_source_type_vnode[4] = {9},
    _dispatch_source_type_mach_recv[4] = {10}, _dispatch_source_type_mach_send[4] = {11}, _dispatch_source_type_memorypressure[4] = {12};
void *_dispatch_data_destructor_free = (void *)1, *_dispatch_data_destructor_none = (void *)2, *_dispatch_data_destructor_munmap = (void *)3;

static void *alloc_obj(Class c, size_t size, int kind) {
    struct dobj *o = calloc(1, size);
    o->isa = c;
    o->rc = 1;
    o->kind = kind;
    return o;
}
static void obj_retain(void *p) { struct dobj *o = p; if (o && !o->immortal) __atomic_add_fetch(&o->rc, 1, __ATOMIC_RELAXED); }
static void obj_free(struct dobj *o);
static void obj_release(void *p) {
    struct dobj *o = p;
    if (!o || o->immortal) return;
    if (__atomic_sub_fetch(&o->rc, 1, __ATOMIC_ACQ_REL) == 0) obj_free(o);
}

@implementation OS_dispatch_object
- (id)retain { obj_retain(self); return self; }
- (oneway void)release { obj_release(self); }
- (id)autorelease { extern id _prism_objc_rootAutorelease(id); return _prism_objc_rootAutorelease(self); }
- (NSUInteger)retainCount { return ((struct dobj *)self)->immortal ? ~0ul : (NSUInteger)((struct dobj *)self)->rc; }
- (void)dealloc {}
@end
@implementation OS_dispatch_queue @end
@implementation OS_dispatch_group @end
@implementation OS_dispatch_semaphore @end
@implementation OS_dispatch_source @end

/* ---- time ---- */
static uint64_t now_ns(void) {
    struct timeval tv;
    gettimeofday(&tv, 0);
    return (uint64_t)tv.tv_sec * 1000000000ull + (uint64_t)tv.tv_usec * 1000ull;
}
static struct timespec abs_ts(uint64_t ns) { return (struct timespec){(long)(ns / 1000000000ull), (long)(ns % 1000000000ull)}; }
dispatch_time_t dispatch_time(dispatch_time_t when, int64_t delta) {
    if (when == TIME_FOREVER) return TIME_FOREVER;
    uint64_t base = when == TIME_NOW ? now_ns() : when;
    if (delta < 0 && (uint64_t)-delta > base) return 1;
    uint64_t r = base + (uint64_t)delta;
    return r >= TIME_FOREVER ? TIME_FOREVER - 1 : r;
}
dispatch_time_t dispatch_walltime(const struct timespec *ts, int64_t delta) {
    uint64_t base = ts ? (uint64_t)ts->tv_sec * 1000000000ull + (uint64_t)ts->tv_nsec : now_ns();
    return dispatch_time(base ? base : 1, delta);
}
/* Wait on `cv` until `deadline` (dispatch time). Returns nonzero on timeout. */
static int wait_until(pthread_cond_t *cv, pthread_mutex_t *mu, dispatch_time_t deadline) {
    if (deadline == TIME_FOREVER) { pthread_cond_wait(cv, mu); return 0; }
    uint64_t when = deadline == TIME_NOW ? now_ns() : deadline;
    if (now_ns() >= when) return 1;
    struct timespec ts = abs_ts(when);
    pthread_cond_timedwait(cv, mu, &ts);
    return now_ns() >= when;
}

/* ---- the worker pool ---- */
struct job { struct job *next; fn_t fn; void *ctx; struct dq *q; };
static pthread_mutex_t pool_mu = MUTEX_INIT;
static pthread_cond_t pool_cv = COND_INIT;
static struct job *jobs_head, *jobs_tail;
static int workers, idle;
static volatile long outstanding; /* items queued or running anywhere, plus armed timers */
#define MAX_WORKERS 16
static pthread_key_t cur_key;
static long cur_key_once;

static void init_keys(void) {
    long z = 0;
    if (__atomic_compare_exchange_n(&cur_key_once, &z, 1, 0, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE)) {
        pthread_key_create(&cur_key, 0);
        __atomic_store_n(&cur_key_once, 2, __ATOMIC_RELEASE);
    }
    while (__atomic_load_n(&cur_key_once, __ATOMIC_ACQUIRE) != 2) {}
}
static struct dq *current_queue(void) { init_keys(); struct dq *q = pthread_getspecific(cur_key); return q ? q : pthread_main_np() ? &_dispatch_main_q : &globals[2]; }
static void run_on(struct dq *q, fn_t fn, void *ctx) {
    init_keys();
    void *prev = pthread_getspecific(cur_key);
    pthread_setspecific(cur_key, q);
    void *pool = objc_autoreleasePoolPush();
    fn(ctx);
    objc_autoreleasePoolPop(pool);
    pthread_setspecific(cur_key, prev);
}

static void *worker(void *arg) {
    (void)arg;
    pthread_mutex_lock(&pool_mu);
    for (;;) {
        while (!jobs_head) {
            idle++;
            pthread_cond_wait(&pool_cv, &pool_mu);
            idle--;
        }
        struct job *j = jobs_head;
        jobs_head = j->next;
        if (!jobs_head) jobs_tail = 0;
        pthread_mutex_unlock(&pool_mu);
        run_on(j->q, j->fn, j->ctx);
        free(j);
        pthread_mutex_lock(&pool_mu);
    }
    return 0;
}
static void pool_submit(struct dq *q, fn_t fn, void *ctx) {
    struct job *j = calloc(1, sizeof *j);
    j->fn = fn; j->ctx = ctx; j->q = q;
    pthread_mutex_lock(&pool_mu);
    if (jobs_tail) jobs_tail->next = j; else jobs_head = j;
    jobs_tail = j;
    if (idle > 0) pthread_cond_signal(&pool_cv);
    else if (workers < MAX_WORKERS) {
        pthread_t t;
        if (pthread_create(&t, 0, worker, 0) == 0) { pthread_detach(t); workers++; }
    }
    pthread_mutex_unlock(&pool_mu);
}

/* ---- queues ---- */
static pthread_cond_t main_cv = COND_INIT; /* signalled (under _dispatch_main_q.mu) when main gets work */

struct counted { fn_t fn; void *ctx; };
static void run_counted(void *p) {
    struct counted c = *(struct counted *)p;
    free(p);
    c.fn(c.ctx);
    __atomic_sub_fetch(&outstanding, 1, __ATOMIC_ACQ_REL);
}

static void drain(void *p) {
    struct dq *q = p;
    for (int n = 0;; n++) {
        pthread_mutex_lock(&q->mu);
        struct item *it = q->suspended ? 0 : q->head;
        if (it && n == 256) { /* fairness: let other jobs run, keep `scheduled` */
            pthread_mutex_unlock(&q->mu);
            pool_submit(q, drain, q);
            return;
        }
        if (!it) { q->scheduled = 0; pthread_mutex_unlock(&q->mu); obj_release(q); return; }
        q->head = it->next;
        if (!q->head) q->tail = 0;
        pthread_mutex_unlock(&q->mu);
        it->fn(it->ctx);
        free(it);
        __atomic_sub_fetch(&outstanding, 1, __ATOMIC_ACQ_REL);
    }
}

static void enqueue(struct dq *q, fn_t fn, void *ctx) {
    __atomic_add_fetch(&outstanding, 1, __ATOMIC_ACQ_REL);
    if (q->global) {
        struct counted *c = calloc(1, sizeof *c);
        c->fn = fn; c->ctx = ctx;
        pool_submit(q, run_counted, c);
        return;
    }
    struct item *it = calloc(1, sizeof *it);
    it->fn = fn; it->ctx = ctx;
    pthread_mutex_lock(&q->mu);
    if (q->tail) q->tail->next = it; else q->head = it;
    q->tail = it;
    int go = !q->main && !q->scheduled && !q->suspended;
    if (go) { q->scheduled = 1; obj_retain(q); }
    if (q->main) pthread_cond_broadcast(&main_cv);
    pthread_mutex_unlock(&q->mu);
    if (go) pool_submit(q, drain, q);
}

/* Run what the main queue holds now (main thread only). Returns the number of items run. */
int _prism_dispatch_main_drain(void) {
    struct dq *q = &_dispatch_main_q;
    pthread_mutex_lock(&q->mu);
    struct item *list = q->suspended ? 0 : q->head;
    if (list) q->head = q->tail = 0;
    pthread_mutex_unlock(&q->mu);
    int n = 0;
    while (list) {
        struct item *it = list;
        list = it->next;
        run_on(q, it->fn, it->ctx);
        free(it);
        __atomic_sub_fetch(&outstanding, 1, __ATOMIC_ACQ_REL);
        n++;
    }
    return n;
}
/* Is dispatch work queued, running or timed (so the run loop should keep polling)? */
int _prism_dispatch_busy(void) { return __atomic_load_n(&outstanding, __ATOMIC_ACQUIRE) > 0; }
int _prism_dispatch_main_pending(void) { return _dispatch_main_q.head != 0; }

void dispatch_main(void) {
    struct dq *q = &_dispatch_main_q;
    for (;;) {
        _prism_dispatch_main_drain();
        pthread_mutex_lock(&q->mu);
        if (!q->head || q->suspended) pthread_cond_wait(&main_cv, &q->mu);
        pthread_mutex_unlock(&q->mu);
    }
}

void *dispatch_get_global_queue(long prio, unsigned long flags) {
    (void)flags;
    /* DISPATCH_QUEUE_PRIORITY_HIGH 2, DEFAULT 0, LOW -2, BACKGROUND INT16_MIN; QoS classes 0x21.. */
    switch (prio) {
    case 2: case 0x21: case 0x19: return &globals[3];
    case -2: case 0x11: return &globals[1];
    case -32768: case 0x09: return &globals[0];
    default: return &globals[2];
    }
}
void *dispatch_get_main_queue(void) { return &_dispatch_main_q; }
void *dispatch_get_current_queue(void) { return current_queue(); }

void *dispatch_queue_create(const char *label, const void *attr) {
    (void)attr; /* serial or concurrent: both drained in order (see the top of the file) */
    struct dq *q = alloc_obj([OS_dispatch_queue class], sizeof(struct dq), K_QUEUE);
    pthread_mutex_init(&q->mu, 0);
    if (label) snprintf(q->label, sizeof q->label, "%s", label);
    return q;
}
void *dispatch_queue_create_with_target(const char *label, const void *attr, void *target) { (void)target; return dispatch_queue_create(label, attr); }
const void *dispatch_queue_attr_make_with_qos_class(const void *attr, int qos, int rel) { (void)qos; (void)rel; return attr; }
const void *dispatch_queue_attr_make_initially_inactive(const void *attr) { return attr; }
const void *dispatch_queue_attr_make_with_autorelease_frequency(const void *attr, unsigned long f) { (void)f; return attr; }
const char *dispatch_queue_get_label(void *q) { return ((struct dq *)(q ? q : current_queue()))->label; }
int dispatch_queue_get_qos_class(void *q, int *rel) { (void)q; if (rel) *rel = 0; return 0x15; }
void dispatch_set_target_queue(void *o, void *q) { (void)o; (void)q; }
void dispatch_assert_queue(void *q) { (void)q; }
void dispatch_assert_queue_not(void *q) { (void)q; }
void dispatch_assert_queue_barrier(void *q) { (void)q; }

void dispatch_async_f(void *q, void *ctx, fn_t fn) { enqueue(q, fn, ctx); }
void dispatch_async(void *q, void *block) { enqueue(q, call_release_block, _Block_copy(block)); }
void dispatch_barrier_async_f(void *q, void *ctx, fn_t fn) { enqueue(q, fn, ctx); }
void dispatch_barrier_async(void *q, void *block) { dispatch_async(q, block); }

struct sync { fn_t fn; void *ctx; pthread_mutex_t mu; pthread_cond_t cv; int done; };
static void run_sync(void *p) {
    struct sync *s = p;
    s->fn(s->ctx);
    pthread_mutex_lock(&s->mu);
    s->done = 1;
    pthread_cond_signal(&s->cv);
    pthread_mutex_unlock(&s->mu);
}
void dispatch_sync_f(void *qp, void *ctx, fn_t fn) {
    struct dq *q = qp;
    /* Global queues, and the main queue asked from the main thread (which would deadlock on
     * macOS), run the work right here. */
    if (q->global || (q->main && pthread_main_np())) { run_on(q, fn, ctx); return; }
    struct sync s = {fn, ctx, MUTEX_INIT, COND_INIT, 0};
    enqueue(q, run_sync, &s);
    pthread_mutex_lock(&s.mu);
    while (!s.done) {
        if (pthread_main_np()) {
            /* Work on another queue may itself need the main queue: keep draining it. */
            pthread_mutex_unlock(&s.mu);
            _prism_dispatch_main_drain();
            pthread_mutex_lock(&s.mu);
            if (!s.done) wait_until(&s.cv, &s.mu, dispatch_time(0, 2000000));
        } else pthread_cond_wait(&s.cv, &s.mu);
    }
    pthread_mutex_unlock(&s.mu);
}
void dispatch_sync(void *q, void *block) { dispatch_sync_f(q, block, call_block); }
void dispatch_barrier_sync_f(void *q, void *ctx, fn_t fn) { dispatch_sync_f(q, ctx, fn); }
void dispatch_barrier_sync(void *q, void *block) { dispatch_sync(q, block); }
void dispatch_async_and_wait(void *q, void *block) { dispatch_sync(q, block); }
void dispatch_async_and_wait_f(void *q, void *ctx, fn_t fn) { dispatch_sync_f(q, ctx, fn); }

/* dispatch_apply: iterations run on the calling thread, in order (allowed: only completion of
 * all of them is promised). */
void dispatch_apply_f(size_t n, void *q, void *ctx, void (*fn)(void *, size_t)) { (void)q; for (size_t i = 0; i < n; i++) fn(ctx, i); }
void dispatch_apply(size_t n, void *q, void *block) {
    (void)q;
    struct block *b = block;
    for (size_t i = 0; i < n; i++) ((void (*)(void *, size_t))b->invoke)(b, i);
}

/* ---- queue-specific data ---- */
void dispatch_queue_set_specific(void *qp, const void *key, void *ctx, fn_t dtor) {
    struct dq *q = qp;
    pthread_mutex_lock(&q->mu);
    struct spec **pp = &q->specs;
    while (*pp && (*pp)->key != key) pp = &(*pp)->next;
    if (*pp) {
        if ((*pp)->dtor && (*pp)->ctx) (*pp)->dtor((*pp)->ctx);
        if (ctx) { (*pp)->ctx = ctx; (*pp)->dtor = dtor; }
        else { struct spec *s = *pp; *pp = s->next; free(s); }
    } else if (ctx) {
        struct spec *s = calloc(1, sizeof *s);
        s->key = key; s->ctx = ctx; s->dtor = dtor; s->next = q->specs;
        q->specs = s;
    }
    pthread_mutex_unlock(&q->mu);
}
void *dispatch_queue_get_specific(void *qp, const void *key) {
    struct dq *q = qp;
    void *r = 0;
    pthread_mutex_lock(&q->mu);
    for (struct spec *s = q->specs; s; s = s->next) if (s->key == key) { r = s->ctx; break; }
    pthread_mutex_unlock(&q->mu);
    return r;
}
void *dispatch_get_specific(const void *key) { return dispatch_queue_get_specific(current_queue(), key); }

/* ---- timers ---- */
struct timer { struct timer *next; uint64_t when; struct dq *q; fn_t fn; void *ctx; struct dsource *src; uint64_t gen; };
static pthread_mutex_t tm_mu = MUTEX_INIT;
static pthread_cond_t tm_cv = COND_INIT;
static struct timer *timers;
static int tm_started;
static void source_timer_fired(struct dsource *s, uint64_t gen);

static void *timer_thread(void *arg) {
    (void)arg;
    pthread_mutex_lock(&tm_mu);
    for (;;) {
        if (!timers) { pthread_cond_wait(&tm_cv, &tm_mu); continue; }
        if (timers->when > now_ns()) { wait_until(&tm_cv, &tm_mu, timers->when); continue; }
        struct timer *t = timers;
        timers = t->next;
        pthread_mutex_unlock(&tm_mu);
        if (t->src) source_timer_fired(t->src, t->gen);
        else enqueue(t->q, t->fn, t->ctx);
        __atomic_sub_fetch(&outstanding, 1, __ATOMIC_ACQ_REL);
        free(t);
        pthread_mutex_lock(&tm_mu);
    }
    return 0;
}
static void add_timer(struct timer *t) {
    __atomic_add_fetch(&outstanding, 1, __ATOMIC_ACQ_REL);
    pthread_mutex_lock(&tm_mu);
    struct timer **pp = &timers;
    while (*pp && (*pp)->when <= t->when) pp = &(*pp)->next;
    t->next = *pp;
    *pp = t;
    if (!tm_started) {
        pthread_t th;
        if (pthread_create(&th, 0, timer_thread, 0) == 0) { pthread_detach(th); tm_started = 1; }
    }
    pthread_cond_signal(&tm_cv);
    pthread_mutex_unlock(&tm_mu);
}
void dispatch_after_f(dispatch_time_t when, void *q, void *ctx, fn_t fn) {
    if (when == TIME_FOREVER) return;
    struct timer *t = calloc(1, sizeof *t);
    t->when = when == TIME_NOW ? now_ns() : when;
    t->q = q; t->fn = fn; t->ctx = ctx;
    add_timer(t);
}
void dispatch_after(dispatch_time_t when, void *q, void *block) { dispatch_after_f(when, q, _Block_copy(block), call_release_block); }

/* ---- groups ---- */
void *dispatch_group_create(void) {
    struct dgroup *g = alloc_obj([OS_dispatch_group class], sizeof(struct dgroup), K_GROUP);
    pthread_mutex_init(&g->mu, 0);
    pthread_cond_init(&g->cv, 0);
    return g;
}
void dispatch_group_enter(void *gp) { struct dgroup *g = gp; pthread_mutex_lock(&g->mu); g->count++; pthread_mutex_unlock(&g->mu); }
struct notify { struct dq *q; fn_t fn; void *ctx; };
void dispatch_group_leave(void *gp) {
    struct dgroup *g = gp;
    pthread_mutex_lock(&g->mu);
    struct item *list = 0;
    if (--g->count <= 0) {
        g->count = 0;
        list = g->notify;
        g->notify = 0;
        pthread_cond_broadcast(&g->cv);
    }
    pthread_mutex_unlock(&g->mu);
    while (list) {
        struct item *it = list;
        list = it->next;
        struct notify *n = it->ctx;
        enqueue(n->q, n->fn, n->ctx);
        obj_release(n->q);
        free(n);
        free(it);
    }
}
long dispatch_group_wait(void *gp, dispatch_time_t timeout) {
    struct dgroup *g = gp;
    pthread_mutex_lock(&g->mu);
    long r = 0;
    while (g->count > 0) {
        if (pthread_main_np()) {
            /* Work in the group may target the main queue. */
            pthread_mutex_unlock(&g->mu);
            _prism_dispatch_main_drain();
            pthread_mutex_lock(&g->mu);
            if (g->count <= 0) break;
            uint64_t step = dispatch_time(0, 2000000);
            if (timeout != TIME_FOREVER && step > timeout) step = timeout;
            if (wait_until(&g->cv, &g->mu, step) && timeout != TIME_FOREVER && now_ns() >= timeout && g->count > 0) { r = 49; break; }
        } else if (wait_until(&g->cv, &g->mu, timeout) && g->count > 0) { r = 49; break; }
    }
    pthread_mutex_unlock(&g->mu);
    return r;
}
void dispatch_group_notify_f(void *gp, void *q, void *ctx, fn_t fn) {
    struct dgroup *g = gp;
    pthread_mutex_lock(&g->mu);
    if (g->count == 0) {
        pthread_mutex_unlock(&g->mu);
        enqueue(q, fn, ctx);
        return;
    }
    struct notify *n = calloc(1, sizeof *n);
    n->q = q; n->fn = fn; n->ctx = ctx;
    obj_retain(q);
    struct item *it = calloc(1, sizeof *it);
    it->ctx = n;
    struct item **pp = &g->notify;
    while (*pp) pp = &(*pp)->next;
    *pp = it;
    pthread_mutex_unlock(&g->mu);
}
void dispatch_group_notify(void *g, void *q, void *block) { dispatch_group_notify_f(g, q, _Block_copy(block), call_release_block); }
struct gwork { struct dgroup *g; fn_t fn; void *ctx; };
static void run_group(void *p) {
    struct gwork w = *(struct gwork *)p;
    free(p);
    w.fn(w.ctx);
    dispatch_group_leave(w.g);
    obj_release(w.g);
}
void dispatch_group_async_f(void *g, void *q, void *ctx, fn_t fn) {
    dispatch_group_enter(g);
    obj_retain(g);
    struct gwork *w = calloc(1, sizeof *w);
    w->g = g; w->fn = fn; w->ctx = ctx;
    enqueue(q, run_group, w);
}
void dispatch_group_async(void *g, void *q, void *block) { dispatch_group_async_f(g, q, _Block_copy(block), call_release_block); }

/* ---- semaphores ---- */
void *dispatch_semaphore_create(long value) {
    if (value < 0) return 0;
    struct dsema *s = alloc_obj([OS_dispatch_semaphore class], sizeof(struct dsema), K_SEMA);
    pthread_mutex_init(&s->mu, 0);
    pthread_cond_init(&s->cv, 0);
    s->value = s->orig = value;
    return s;
}
long dispatch_semaphore_wait(void *sp, dispatch_time_t timeout) {
    struct dsema *s = sp;
    pthread_mutex_lock(&s->mu);
    long r = 0;
    s->waiters++;
    while (s->value <= 0) {
        if (pthread_main_np() && timeout != TIME_NOW) {
            pthread_mutex_unlock(&s->mu);
            _prism_dispatch_main_drain();
            pthread_mutex_lock(&s->mu);
            if (s->value > 0) break;
            uint64_t step = dispatch_time(0, 2000000);
            if (timeout != TIME_FOREVER && step > timeout) step = timeout;
            wait_until(&s->cv, &s->mu, step);
            if (s->value <= 0 && timeout != TIME_FOREVER && now_ns() >= timeout) { r = 49; break; }
        } else if (wait_until(&s->cv, &s->mu, timeout) && s->value <= 0) { r = 49; break; }
    }
    s->waiters--;
    if (!r) s->value--;
    pthread_mutex_unlock(&s->mu);
    return r;
}
long dispatch_semaphore_signal(void *sp) {
    struct dsema *s = sp;
    pthread_mutex_lock(&s->mu);
    s->value++;
    long woke = s->waiters > 0;
    if (woke) pthread_cond_signal(&s->cv);
    pthread_mutex_unlock(&s->mu);
    return woke;
}

/* ---- sources ---- */
#define T_TIMER ((const void *)_dispatch_source_type_timer)
static int is_data_type(const void *t) { return t == _dispatch_source_type_data_add || t == _dispatch_source_type_data_or || t == _dispatch_source_type_data_replace; }

void *dispatch_source_create(const void *type, uintptr_t handle, unsigned long mask, void *q) {
    (void)handle;
    struct dsource *s = alloc_obj([OS_dispatch_source class], sizeof(struct dsource), K_SOURCE);
    pthread_mutex_init(&s->mu, 0);
    s->type = type;
    s->mask = mask;
    s->q = q ? q : &globals[2];
    obj_retain(s->q);
    s->suspended = 1; /* sources start suspended */
    s->interval = TIME_FOREVER;
    s->start = TIME_FOREVER;
    return s;
}
static void src_call(void *h, int is_f, void *ctx) {
    if (!h) return;
    if (is_f) ((fn_t)h)(ctx);
    else call_block(h);
}
static void src_fire(void *p) {
    struct dsource *s = p;
    pthread_mutex_lock(&s->mu);
    s->data = s->pending;
    s->pending = 0;
    s->fire_queued = 0;
    int run = !s->cancelled && !s->suspended;
    void *h = s->handler;
    int f = s->handler_is_f;
    pthread_mutex_unlock(&s->mu);
    if (run) src_call(h, f, s->o.ctx);
    obj_release(s);
}
/* Queue the event handler unless it is already queued (events coalesce, as on macOS). */
static void src_signal_locked(struct dsource *s) {
    if (s->fire_queued || s->suspended || s->cancelled) return;
    s->fire_queued = 1;
    obj_retain(s);
    enqueue(s->q, src_fire, s);
}
static void arm_timer_locked(struct dsource *s) {
    if (s->type != T_TIMER || s->suspended || s->cancelled || s->start == TIME_FOREVER) return;
    struct timer *t = calloc(1, sizeof *t);
    t->when = s->start == TIME_NOW ? now_ns() : s->start;
    t->src = s;
    t->gen = s->gen;
    obj_retain(s);
    add_timer(t);
}
static void source_timer_fired(struct dsource *s, uint64_t gen) {
    pthread_mutex_lock(&s->mu);
    if (gen == s->gen && !s->cancelled) {
        if (s->suspended) {
            s->armed = 0; /* re-armed on resume */
        } else {
            s->pending++;
            src_signal_locked(s);
            if (s->interval != TIME_FOREVER && s->interval) {
                uint64_t base = s->start == TIME_NOW ? now_ns() : s->start;
                s->start = base + s->interval;
                if (s->start < now_ns()) s->start = now_ns() + s->interval; /* missed periods coalesce */
                arm_timer_locked(s);
            } else s->start = TIME_FOREVER;
        }
    }
    pthread_mutex_unlock(&s->mu);
    obj_release(s);
}
void dispatch_source_set_timer(void *sp, dispatch_time_t start, uint64_t interval, uint64_t leeway) {
    (void)leeway;
    struct dsource *s = sp;
    pthread_mutex_lock(&s->mu);
    s->gen++;
    s->start = start;
    s->interval = interval;
    arm_timer_locked(s);
    pthread_mutex_unlock(&s->mu);
}
static void set_handler(void **slot, int *is_f, void *h, int f) {
    if (*slot && !*is_f) _Block_release(*slot);
    *slot = h && !f ? _Block_copy(h) : h;
    *is_f = f;
}
void dispatch_source_set_event_handler(void *sp, void *b) { struct dsource *s = sp; pthread_mutex_lock(&s->mu); set_handler(&s->handler, &s->handler_is_f, b, 0); pthread_mutex_unlock(&s->mu); }
void dispatch_source_set_event_handler_f(void *sp, fn_t f) { struct dsource *s = sp; pthread_mutex_lock(&s->mu); set_handler(&s->handler, &s->handler_is_f, (void *)f, 1); pthread_mutex_unlock(&s->mu); }
void dispatch_source_set_cancel_handler(void *sp, void *b) { struct dsource *s = sp; pthread_mutex_lock(&s->mu); set_handler(&s->cancel_handler, &s->cancel_is_f, b, 0); pthread_mutex_unlock(&s->mu); }
void dispatch_source_set_cancel_handler_f(void *sp, fn_t f) { struct dsource *s = sp; pthread_mutex_lock(&s->mu); set_handler(&s->cancel_handler, &s->cancel_is_f, (void *)f, 1); pthread_mutex_unlock(&s->mu); }
void dispatch_source_set_registration_handler(void *sp, void *b) { struct dsource *s = sp; int f = 0; pthread_mutex_lock(&s->mu); set_handler(&s->reg_handler, &f, b, 0); pthread_mutex_unlock(&s->mu); }
static void src_cancelled(void *p) {
    struct dsource *s = p;
    src_call(s->cancel_handler, s->cancel_is_f, s->o.ctx);
    obj_release(s);
}
void dispatch_source_cancel(void *sp) {
    struct dsource *s = sp;
    pthread_mutex_lock(&s->mu);
    int first = !s->cancelled;
    s->cancelled = 1;
    s->gen++;
    pthread_mutex_unlock(&s->mu);
    if (first && s->cancel_handler) { obj_retain(s); enqueue(s->q, src_cancelled, s); }
}
long dispatch_source_testcancel(void *s) { return ((struct dsource *)s)->cancelled; }
unsigned long dispatch_source_get_data(void *s) { return ((struct dsource *)s)->data; }
unsigned long dispatch_source_get_mask(void *s) { return ((struct dsource *)s)->mask; }
uintptr_t dispatch_source_get_handle(void *s) { (void)s; return 0; }
void dispatch_source_merge_data(void *sp, unsigned long v) {
    struct dsource *s = sp;
    if (!is_data_type(s->type)) return;
    pthread_mutex_lock(&s->mu);
    if (s->type == _dispatch_source_type_data_add) s->pending += v;
    else if (s->type == _dispatch_source_type_data_or) s->pending |= v;
    else s->pending = v;
    if (v || s->type == _dispatch_source_type_data_replace) src_signal_locked(s);
    pthread_mutex_unlock(&s->mu);
}

/* ---- generic object calls ---- */
void dispatch_retain(void *o) { obj_retain(o); }
void dispatch_release(void *o) { obj_release(o); }
void *dispatch_get_context(void *o) { return ((struct dobj *)o)->ctx; }
void dispatch_set_context(void *o, void *c) { ((struct dobj *)o)->ctx = c; }
void dispatch_set_finalizer_f(void *o, fn_t f) { ((struct dobj *)o)->finalizer = f; }
void dispatch_suspend(void *o) {
    struct dobj *d = o;
    if (d->kind == K_QUEUE) { struct dq *q = o; pthread_mutex_lock(&q->mu); q->suspended++; pthread_mutex_unlock(&q->mu); }
    else if (d->kind == K_SOURCE) { struct dsource *s = o; pthread_mutex_lock(&s->mu); s->suspended++; pthread_mutex_unlock(&s->mu); }
}
void dispatch_resume(void *o) {
    struct dobj *d = o;
    if (d->kind == K_QUEUE) {
        struct dq *q = o;
        pthread_mutex_lock(&q->mu);
        if (q->suspended > 0) q->suspended--;
        int go = !q->suspended && q->head && !q->scheduled && !q->main && !q->global;
        if (go) { q->scheduled = 1; obj_retain(q); }
        if (q->main) pthread_cond_broadcast(&main_cv);
        pthread_mutex_unlock(&q->mu);
        if (go) pool_submit(q, drain, q);
    } else if (d->kind == K_SOURCE) {
        struct dsource *s = o;
        pthread_mutex_lock(&s->mu);
        if (s->suspended > 0) s->suspended--;
        if (!s->suspended) {
            if (!s->activated) {
                s->activated = 1;
                if (s->reg_handler) { void *h = s->reg_handler; s->reg_handler = 0; obj_retain(s); enqueue(s->q, call_release_block, h); obj_release(s); }
            }
            s->gen++;
            arm_timer_locked(s);
            if (s->pending) src_signal_locked(s);
        }
        pthread_mutex_unlock(&s->mu);
    }
}
void dispatch_activate(void *o) {
    struct dobj *d = o;
    if (d->kind == K_SOURCE && !((struct dsource *)o)->activated) dispatch_resume(o);
}

static void obj_free(struct dobj *o) {
    if (o->finalizer) o->finalizer(o->ctx);
    switch (o->kind) {
    case K_QUEUE: {
        struct dq *q = (struct dq *)o;
        for (struct spec *s = q->specs, *n; s; s = n) { n = s->next; if (s->dtor && s->ctx) s->dtor(s->ctx); free(s); }
        break;
    }
    case K_SOURCE: {
        struct dsource *s = (struct dsource *)o;
        if (s->handler && !s->handler_is_f) _Block_release(s->handler);
        if (s->cancel_handler && !s->cancel_is_f) _Block_release(s->cancel_handler);
        if (s->reg_handler) _Block_release(s->reg_handler);
        obj_release(s->q);
        break;
    }
    }
    free(o);
}
