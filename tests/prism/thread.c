/* Threads: create/join with results, a contended mutex, a condition variable handoff,
 * thread-specific data, per-thread errno and thread-local variables. */
/* No SDK headers: the macOS ABI types are declared here. */
typedef struct { long sig; char opaque[56]; } pthread_mutex_t;
typedef struct { long sig; char opaque[40]; } pthread_cond_t;
typedef struct _opaque_pthread *pthread_t;
typedef unsigned long pthread_key_t;
#define PTHREAD_MUTEX_INITIALIZER {0x32AAABA7, {0}}
#define PTHREAD_COND_INITIALIZER {0x3CB0B1BB, {0}}
int pthread_create(pthread_t *, const void *, void *(*)(void *), void *);
int pthread_join(pthread_t, void **);
int pthread_mutex_lock(pthread_mutex_t *);
int pthread_mutex_unlock(pthread_mutex_t *);
int pthread_cond_wait(pthread_cond_t *, pthread_mutex_t *);
int pthread_cond_signal(pthread_cond_t *);
int pthread_key_create(pthread_key_t *, void (*)(void *));
int pthread_setspecific(pthread_key_t, const void *);
void *pthread_getspecific(pthread_key_t);
int pthread_main_np(void);
int pthread_detach(pthread_t);
struct timespec { long sec, nsec; };
int pthread_cond_timedwait(pthread_cond_t *, pthread_mutex_t *, const struct timespec *);
int gettimeofday(void *, void *);
static long long now_us(void) { struct { long s; int us; } tv; gettimeofday(&tv, 0); return (long long)tv.s * 1000000 + tv.us; }
static int detached_runs;
static void *detached(void *a) { (void)a; __atomic_fetch_add(&detached_runs, 1, __ATOMIC_RELAXED); return 0; }
int *__error(void);
#define errno (*__error())
int printf(const char *, ...);
int usleep(unsigned);

#define N 4
#define ITER 20000
static pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t cv = PTHREAD_COND_INITIALIZER;
static long counter;
static int ready, tsd_ok = 1, errno_ok = 1, tlv_ok = 1;
static pthread_key_t key;
static __thread int tlv = 5;

static void *work(void *arg) {
    long id = (long)arg;
    pthread_setspecific(key, (void *)(id + 100));
    errno = (int)id + 1;
    tlv = (int)id * 10;
    for (int i = 0; i < ITER; i++) {
        pthread_mutex_lock(&m);
        counter++;
        pthread_mutex_unlock(&m);
    }
    pthread_mutex_lock(&m);
    if ((long)pthread_getspecific(key) != id + 100) tsd_ok = 0;
    if (errno != (int)id + 1) errno_ok = 0;
    if (tlv != (int)id * 10) tlv_ok = 0;
    pthread_mutex_unlock(&m);
    return (void *)(id * id);
}

static void *waiter(void *arg) {
    (void)arg;
    pthread_mutex_lock(&m);
    while (!ready) pthread_cond_wait(&cv, &m);
    pthread_mutex_unlock(&m);
    return (void *)42;
}

int main(void) {
    pthread_key_create(&key, 0);
    pthread_t t[N], w;
    pthread_create(&w, 0, waiter, 0);
    for (long i = 0; i < N; i++) pthread_create(&t[i], 0, work, (void *)i);
    long sum = 0;
    for (int i = 0; i < N; i++) { void *r; pthread_join(t[i], &r); sum += (long)r; }
    usleep(1000);
    pthread_mutex_lock(&m);
    ready = 1;
    pthread_cond_signal(&cv);
    pthread_mutex_unlock(&m);
    void *wr;
    pthread_join(w, &wr);
    printf("thread: counter=%ld sum=%ld cond=%ld tsd=%d errno=%d tlv=%d main_tlv=%d main=%d\n", counter, sum, (long)wr, tsd_ok, errno_ok, tlv_ok, tlv, pthread_main_np());
    /* Timed wait: nobody signals, so it times out after ~50 ms. */
    pthread_cond_t c2 = PTHREAD_COND_INITIALIZER;
    long long t0 = now_us();
    struct timespec dl = { (long)((t0 + 50000) / 1000000), (long)((t0 + 50000) % 1000000) * 1000 };
    pthread_mutex_lock(&m);
    int tr = pthread_cond_timedwait(&c2, &m, &dl);
    pthread_mutex_unlock(&m);
    long long took = (now_us() - t0) / 1000;
    /* Detached threads, in waves: later creates unmap the stacks of finished ones. */
    int det_ok = 1;
    for (int wave = 0; wave < 3; wave++) {
        for (int i = 0; i < 20; i++) {
            pthread_t d;
            if (pthread_create(&d, 0, detached, 0) || pthread_detach(d)) det_ok = 0;
        }
        usleep(20000);
    }
    usleep(20000);
    printf("thread2: timedwait=%d took_ok=%d detached=%d runs=%d\n", tr, took >= 40 && took < 1000, det_ok, detached_runs);
    return 0;
}
