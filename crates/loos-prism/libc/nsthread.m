/* NSThread, the Foundation locks (NSLock, NSRecursiveLock, NSCondition, NSConditionLock),
 * -performSelector:onThread:/OnMainThread:/InBackground:, and NSOperation / NSBlockOperation /
 * NSOperationQueue. Threads are pthreads (pthread.c); operation queues run on libdispatch
 * (dispatch.m): the main operation queue on the main dispatch queue, others on the global pool. */
#include "foundation.h"

typedef struct { long sig; char opaque[56]; } pthread_mutex_t;
typedef struct { long sig; char opaque[40]; } pthread_cond_t;
typedef struct { long sig; char opaque[8]; } pthread_mutexattr_t;
typedef struct pthread *pthread_t;
typedef unsigned long pthread_key_t;
int pthread_mutex_init(pthread_mutex_t *, const pthread_mutexattr_t *);
int pthread_mutex_destroy(pthread_mutex_t *);
int pthread_mutex_lock(pthread_mutex_t *);
int pthread_mutex_trylock(pthread_mutex_t *);
int pthread_mutex_unlock(pthread_mutex_t *);
int pthread_mutexattr_init(pthread_mutexattr_t *);
int pthread_mutexattr_settype(pthread_mutexattr_t *, int);
int pthread_cond_init(pthread_cond_t *, const void *);
int pthread_cond_destroy(pthread_cond_t *);
int pthread_cond_wait(pthread_cond_t *, pthread_mutex_t *);
int pthread_cond_timedwait(pthread_cond_t *, pthread_mutex_t *, const struct timespec *);
int pthread_cond_signal(pthread_cond_t *);
int pthread_cond_broadcast(pthread_cond_t *);
int pthread_create(pthread_t *, const void *, void *(*)(void *), void *);
int pthread_detach(pthread_t);
int pthread_main_np(void);
void pthread_exit(void *) __attribute__((noreturn));
int pthread_key_create(pthread_key_t *, void (*)(void *));
void *pthread_getspecific(pthread_key_t);
int pthread_setspecific(pthread_key_t, const void *);
void dispatch_once_f(long *, void *, void (*)(void *));
void dispatch_async_f(void *, void *, void (*)(void *));
void dispatch_sync_f(void *, void *, void (*)(void *));
void *dispatch_get_global_queue(long, unsigned long);
extern char _dispatch_main_q;
void *_Block_copy(const void *);
void _Block_release(const void *);
void *calloc(size_t, size_t);
void free(void *);
double CFAbsoluteTimeGetCurrent(void);
#define EPOCH_2001 978307200.0
#define MUTEX_INIT {0x32AAABA7, {0}}
#define COND_INIT {0x3CB0B1BB, {0}}

@interface NSObject (PrismThreadUses)
- (void)addObject:(id)o;
- (void)removeObjectIdenticalTo:(id)o;
- (void)removeObjectAtIndex:(NSUInteger)i;
- (void)insertObject:(id)o atIndex:(NSUInteger)i;
- (NSUInteger)count;
- (id)objectAtIndex:(NSUInteger)i;
- (double)timeIntervalSinceReferenceDate;
+ (id)array;
+ (id)dictionary;
- (id)copy;
- (void)setValue:(id)v forKey:(id)k;
@end

static struct timespec abs_from_date(id date) {
    double t = date ? [date timeIntervalSinceReferenceDate] + EPOCH_2001 : 0;
    if (t < 0) t = 0;
    return (struct timespec){(time_t)t, (long)((t - (double)(time_t)t) * 1e9)};
}
static double now_abs(void) { return CFAbsoluteTimeGetCurrent(); }
static void call_block(void *b) { ((void (*)(void *))((struct { void *isa; int f, r; void (*invoke)(void *); } *)b)->invoke)(b); }

/* ------------------------------------------------------------------ locks */

@interface NSLock : NSObject { @public pthread_mutex_t _m; id _name; }
@end
@implementation NSLock
- (id)init { pthread_mutex_init(&_m, 0); return self; }
- (void)lock { pthread_mutex_lock(&_m); }
- (void)unlock { pthread_mutex_unlock(&_m); }
- (BOOL)tryLock { return pthread_mutex_trylock(&_m) == 0; }
- (BOOL)lockBeforeDate:(id)d {
    double limit = [d timeIntervalSinceReferenceDate];
    for (;;) {
        if (pthread_mutex_trylock(&_m) == 0) return YES;
        if (now_abs() >= limit) return NO;
        struct timespec ts = {0, 1000000};
        nanosleep(&ts, 0);
    }
}
- (id)name { return _name; }
- (void)setName:(id)n { id o = _name; _name = [n copy]; [o release]; }
- (void)dealloc { pthread_mutex_destroy(&_m); [_name release]; [super dealloc]; }
@end

@interface NSRecursiveLock : NSLock
@end
@implementation NSRecursiveLock
- (id)init {
    pthread_mutexattr_t a;
    pthread_mutexattr_init(&a);
    pthread_mutexattr_settype(&a, 2); /* PTHREAD_MUTEX_RECURSIVE */
    pthread_mutex_init(&_m, &a);
    return self;
}
@end

@interface NSCondition : NSLock { @public pthread_cond_t _c; }
@end
@implementation NSCondition
- (id)init { [super init]; pthread_cond_init(&_c, 0); return self; }
- (void)wait { pthread_cond_wait(&_c, &_m); }
- (BOOL)waitUntilDate:(id)d {
    struct timespec ts = abs_from_date(d);
    pthread_cond_timedwait(&_c, &_m, &ts);
    return now_abs() < [d timeIntervalSinceReferenceDate];
}
- (void)signal { pthread_cond_signal(&_c); }
- (void)broadcast { pthread_cond_broadcast(&_c); }
- (void)dealloc { pthread_cond_destroy(&_c); [super dealloc]; }
@end

@interface NSConditionLock : NSCondition { @public NSInteger _cond; }
@end
@implementation NSConditionLock
- (id)initWithCondition:(NSInteger)c { [self init]; _cond = c; return self; }
- (NSInteger)condition { return _cond; }
- (void)lockWhenCondition:(NSInteger)c { pthread_mutex_lock(&_m); while (_cond != c) pthread_cond_wait(&_c, &_m); }
- (BOOL)tryLockWhenCondition:(NSInteger)c {
    if (pthread_mutex_trylock(&_m)) return NO;
    if (_cond == c) return YES;
    pthread_mutex_unlock(&_m);
    return NO;
}
- (BOOL)lockWhenCondition:(NSInteger)c beforeDate:(id)d {
    pthread_mutex_lock(&_m);
    double limit = [d timeIntervalSinceReferenceDate];
    while (_cond != c) {
        if (now_abs() >= limit) { pthread_mutex_unlock(&_m); return NO; }
        struct timespec ts = abs_from_date(d);
        pthread_cond_timedwait(&_c, &_m, &ts);
    }
    return YES;
}
- (void)unlockWithCondition:(NSInteger)c { _cond = c; pthread_cond_broadcast(&_c); pthread_mutex_unlock(&_m); }
- (void)unlock { pthread_cond_broadcast(&_c); pthread_mutex_unlock(&_m); }
@end

/* ------------------------------------------------------------------ NSThread */

@interface NSThread : NSObject {
@public
    id _target; SEL _sel; id _arg; void *_block;
    id _name, _dict;
    NSUInteger _stack;
    volatile int _state; /* 0 new, 1 running, 2 finished */
    volatile BOOL _cancelled, _main;
    double _prio;
}
- (void)main;
@end

static pthread_key_t thread_key;
static long thread_key_once;
static NSThread *main_thread;
static void make_key(void *u) { (void)u; pthread_key_create(&thread_key, 0); }
static void key_init(void) { dispatch_once_f(&thread_key_once, 0, make_key); }

id const NSWillBecomeMultiThreadedNotification = (id)CFSTR("NSWillBecomeMultiThreadedNotification");
id const NSThreadWillExitNotification = (id)CFSTR("NSThreadWillExitNotification");
id const NSDidBecomeSingleThreadedNotification = (id)CFSTR("NSDidBecomeSingleThreadedNotification");

static void *thread_entry(void *p) {
    NSThread *t = p;
    key_init();
    pthread_setspecific(thread_key, t);
    @autoreleasepool { [t main]; }
    t->_state = 2;
    [t release];
    return 0;
}

@implementation NSThread
+ (id)currentThread {
    key_init();
    NSThread *t = pthread_getspecific(thread_key);
    if (t) return t;
    if (pthread_main_np()) {
        if (!main_thread) { main_thread = [NSThread new]; main_thread->_main = YES; main_thread->_state = 1; main_thread->_name = [CFSTR("main") retain]; }
        t = main_thread;
    } else {
        t = [NSThread new]; /* a thread made with pthread_create: owned by its key slot */
        t->_state = 1;
    }
    pthread_setspecific(thread_key, t);
    return t;
}
+ (id)mainThread { if (pthread_main_np()) return [self currentThread]; return main_thread; }
+ (BOOL)isMainThread { return pthread_main_np() != 0; }
+ (BOOL)isMultiThreaded { return YES; }
+ (void)sleepForTimeInterval:(double)s {
    if (s <= 0) return;
    struct timespec ts = {(time_t)s, (long)((s - (double)(time_t)s) * 1e9)};
    nanosleep(&ts, 0);
}
+ (void)sleepUntilDate:(id)d { [self sleepForTimeInterval:[d timeIntervalSinceReferenceDate] - now_abs()]; }
+ (void)exit { NSThread *t = [self currentThread]; t->_state = 2; pthread_exit(0); }
+ (double)threadPriority { return 0.5; }
+ (BOOL)setThreadPriority:(double)p { (void)p; return YES; }
+ (id)callStackSymbols { return [NSArray array]; }
+ (id)callStackReturnAddresses { return [NSArray array]; }
+ (void)detachNewThreadSelector:(SEL)s toTarget:(id)t withObject:(id)a { NSThread *th = [[self alloc] initWithTarget:t selector:s object:a]; [th start]; [th release]; }
+ (void)detachNewThreadWithBlock:(void (^)(void))b { NSThread *th = [[self alloc] initWithBlock:b]; [th start]; [th release]; }
- (id)init { _prio = 0.5; return self; }
- (id)initWithTarget:(id)t selector:(SEL)s object:(id)a { [self init]; _target = [t retain]; _sel = s; _arg = [a retain]; return self; }
- (id)initWithBlock:(void (^)(void))b { [self init]; _block = _Block_copy(b); return self; }
- (void)main {
    if (_block) call_block(_block);
    else if (_target) ((void (*)(id, SEL, id))objc_msgSend)(_target, _sel, _arg);
}
- (void)start {
    if (_state) _prism_raise("NSInvalidArgumentException", "*** -[NSThread start]: attempt to start the thread again");
    _state = 1;
    [self retain]; /* released by the thread when it ends */
    pthread_t pt;
    if (pthread_create(&pt, 0, thread_entry, self) != 0) { _state = 2; [self release]; return; }
    pthread_detach(pt);
}
- (BOOL)isMainThread { return _main; }
- (BOOL)isExecuting { return _state == 1; }
- (BOOL)isFinished { return _state == 2; }
- (BOOL)isCancelled { return _cancelled; }
- (void)cancel { _cancelled = YES; }
- (id)name { return _name; }
- (void)setName:(id)n { id o = _name; _name = [n copy]; [o release]; }
- (NSUInteger)stackSize { return _stack ? _stack : 512 * 1024; }
- (void)setStackSize:(NSUInteger)s { _stack = s; }
- (double)threadPriority { return _prio; }
- (void)setThreadPriority:(double)p { _prio = p; }
- (NSInteger)qualityOfService { return -1; }
- (void)setQualityOfService:(NSInteger)q { (void)q; }
- (id)threadDictionary { if (!_dict) _dict = [NSMutableDictionary new]; return _dict; }
- (id)description { return [NSString stringWithFormat:CFSTR("<NSThread: %p>{number = %d, name = %@}"), self, _main ? 1 : 2, _name]; }
- (void)dealloc { [_target release]; [_arg release]; if (_block) _Block_release(_block); [_name release]; [_dict release]; [super dealloc]; }
@end

/* -performSelector: variants that hop threads. A "thread" other than the main one is served by
 * the global dispatch pool (Prism has no per-thread run loops yet). */
struct perform { id target, arg; SEL sel; };
static void run_perform(void *p) {
    struct perform *x = p;
    @autoreleasepool { ((void (*)(id, SEL, id))objc_msgSend)(x->target, x->sel, x->arg); }
    [x->target release];
    [x->arg release];
    free(x);
}
static struct perform *mkperform(id t, SEL s, id a) {
    struct perform *x = calloc(1, sizeof *x);
    x->target = [t retain]; x->sel = s; x->arg = [a retain];
    return x;
}
@implementation NSObject (PrismThreadPerform)
- (void)performSelectorOnMainThread:(SEL)s withObject:(id)a waitUntilDone:(BOOL)w {
    struct perform *x = mkperform(self, s, a);
    if (w) dispatch_sync_f(&_dispatch_main_q, x, run_perform);
    else dispatch_async_f(&_dispatch_main_q, x, run_perform);
}
- (void)performSelectorOnMainThread:(SEL)s withObject:(id)a waitUntilDone:(BOOL)w modes:(id)m { (void)m; [self performSelectorOnMainThread:s withObject:a waitUntilDone:w]; }
- (void)performSelector:(SEL)s onThread:(NSThread *)t withObject:(id)a waitUntilDone:(BOOL)w {
    if (t && t->_main) { [self performSelectorOnMainThread:s withObject:a waitUntilDone:w]; return; }
    if (t == [NSThread currentThread] && w) { ((void (*)(id, SEL, id))objc_msgSend)(self, s, a); return; }
    struct perform *x = mkperform(self, s, a);
    if (w) dispatch_sync_f(dispatch_get_global_queue(0, 0), x, run_perform);
    else dispatch_async_f(dispatch_get_global_queue(0, 0), x, run_perform);
}
- (void)performSelector:(SEL)s onThread:(NSThread *)t withObject:(id)a waitUntilDone:(BOOL)w modes:(id)m { (void)m; [self performSelector:s onThread:t withObject:a waitUntilDone:w]; }
- (void)performSelectorInBackground:(SEL)s withObject:(id)a { [NSThread detachNewThreadSelector:s toTarget:self withObject:a]; }
@end

/* ------------------------------------------------------------------ operations */

/* One lock for all operation state: operations, their dependencies and queues are few, and it
 * makes dependency changes across queues simple. */
static pthread_mutex_t op_mu = MUTEX_INIT;
static pthread_cond_t op_cv = COND_INIT;
@class NSOperationQueue;
static NSOperationQueue *all_queues; /* live queues, linked through _nextq (not retained) */

@class NSOperationQueue;
@interface NSOperation : NSObject {
@public
    id _deps;          /* NSMutableArray */
    void *_completion; /* block */
    id _name;
    volatile BOOL _executing, _finished, _cancelled, _queued;
    NSInteger _priority;
    double _relPrio;
    NSInteger _qos;
}
- (void)main;
- (void)start;
- (BOOL)isReady;
@end
@interface NSOperationQueue : NSObject {
@public
    id _pending; /* operations not started, in priority/arrival order */
    NSInteger _running, _max, _count;
    BOOL _suspended, _main;
    id _name;
    void *_target; /* dispatch queue */
    NSOperationQueue *_nextq;
}
@end
static void schedule_all_locked(void);

@implementation NSOperation
- (id)init { _deps = [NSMutableArray new]; return self; }
- (void)main {}
- (void)start {
    pthread_mutex_lock(&op_mu);
    if (_executing || _finished) { pthread_mutex_unlock(&op_mu); return; }
    BOOL run = !_cancelled;
    _executing = run;
    pthread_mutex_unlock(&op_mu);
    if (run) @autoreleasepool { [self main]; }
    pthread_mutex_lock(&op_mu);
    _executing = NO;
    _finished = YES;
    void *c = _completion;
    _completion = 0;
    pthread_cond_broadcast(&op_cv);
    schedule_all_locked(); /* dependents may now be ready */
    pthread_mutex_unlock(&op_mu);
    if (c) { call_block(c); _Block_release(c); }
}
- (BOOL)isReady {
    for (NSUInteger i = 0, n = [_deps count]; i < n; i++) {
        NSOperation *d = [_deps objectAtIndex:i];
        if (!d->_finished) return NO;
    }
    return YES;
}
- (BOOL)isExecuting { return _executing; }
- (BOOL)isFinished { return _finished; }
- (BOOL)isCancelled { return _cancelled; }
- (BOOL)isConcurrent { return NO; }
- (BOOL)isAsynchronous { return NO; }
- (void)cancel { pthread_mutex_lock(&op_mu); _cancelled = YES; schedule_all_locked(); pthread_mutex_unlock(&op_mu); }
- (void)addDependency:(NSOperation *)o { pthread_mutex_lock(&op_mu); [_deps addObject:o]; pthread_mutex_unlock(&op_mu); }
- (void)removeDependency:(NSOperation *)o { pthread_mutex_lock(&op_mu); [_deps removeObjectIdenticalTo:o]; schedule_all_locked(); pthread_mutex_unlock(&op_mu); }
- (id)dependencies { pthread_mutex_lock(&op_mu); id d = [[_deps copy] autorelease]; pthread_mutex_unlock(&op_mu); return d; }
- (void)waitUntilFinished {
    pthread_mutex_lock(&op_mu);
    while (!_finished) {
        if (pthread_main_np()) {
            extern int _prism_dispatch_main_drain(void);
            pthread_mutex_unlock(&op_mu);
            _prism_dispatch_main_drain();
            pthread_mutex_lock(&op_mu);
            if (_finished) break;
            double t = now_abs() + 0.002 + EPOCH_2001;
            struct timespec ts = {(time_t)t, (long)((t - (double)(time_t)t) * 1e9)};
            pthread_cond_timedwait(&op_cv, &op_mu, &ts);
        } else pthread_cond_wait(&op_cv, &op_mu);
    }
    pthread_mutex_unlock(&op_mu);
}
- (void (^)(void))completionBlock { return _completion; }
- (void)setCompletionBlock:(void (^)(void))b { void *o = _completion; _completion = b ? _Block_copy(b) : 0; if (o) _Block_release(o); }
- (NSInteger)queuePriority { return _priority; }
- (void)setQueuePriority:(NSInteger)p { _priority = p; }
- (double)threadPriority { return _relPrio; }
- (void)setThreadPriority:(double)p { _relPrio = p; }
- (NSInteger)qualityOfService { return _qos; }
- (void)setQualityOfService:(NSInteger)q { _qos = q; }
- (id)name { return _name; }
- (void)setName:(id)n { id o = _name; _name = [n copy]; [o release]; }
- (void)dealloc { [_deps release]; if (_completion) _Block_release(_completion); [_name release]; [super dealloc]; }
@end

@interface NSBlockOperation : NSOperation { @public id _blocks; }
@end
@implementation NSBlockOperation
+ (id)blockOperationWithBlock:(void (^)(void))b { NSBlockOperation *o = [[self alloc] init]; [o addExecutionBlock:b]; return [o autorelease]; }
- (id)init { [super init]; _blocks = [NSMutableArray new]; return self; }
- (void)addExecutionBlock:(void (^)(void))b {
    if (_executing || _finished) _prism_raise("NSInvalidArgumentException", "*** -[NSBlockOperation addExecutionBlock:]: blocks cannot be added after the operation has started executing or finished");
    id c = [b copy];
    [_blocks addObject:c];
    [c release];
}
- (id)executionBlocks { return [[_blocks copy] autorelease]; }
- (void)main { for (NSUInteger i = 0, n = [_blocks count]; i < n; i++) call_block([_blocks objectAtIndex:i]); }
- (void)dealloc { [_blocks release]; [super dealloc]; }
@end

@interface NSInvocationOperation : NSOperation { @public id _target, _arg, _result; SEL _sel; }
@end
@implementation NSInvocationOperation
- (id)initWithTarget:(id)t selector:(SEL)s object:(id)a { [self init]; _target = [t retain]; _sel = s; _arg = [a retain]; return self; }
- (void)main { _result = [((id (*)(id, SEL, id))objc_msgSend)(_target, _sel, _arg) retain]; }
- (id)result { return _result; }
- (void)dealloc { [_target release]; [_arg release]; [_result release]; [super dealloc]; }
@end

struct opjob { NSOperation *op; NSOperationQueue *q; };
static void run_job(void *p) {
    struct opjob j = *(struct opjob *)p;
    free(p);
    [j.op start];
    pthread_mutex_lock(&op_mu);
    j.q->_running--;
    j.q->_count--;
    pthread_cond_broadcast(&op_cv);
    schedule_all_locked();
    pthread_mutex_unlock(&op_mu);
    [j.op release];
    [j.q release];
}

/* Start every ready operation that fits under its queue's limit (op_mu held). */
static void schedule_queue_locked(NSOperationQueue *q) {
    if (q->_suspended) return;
    NSInteger max = q->_main ? 1 : q->_max < 0 ? 64 : q->_max;
    for (NSUInteger i = 0; i < [q->_pending count] && q->_running < max;) {
        NSOperation *op = [q->_pending objectAtIndex:i];
        if (!op->_cancelled && ![op isReady]) { i++; continue; }
        [op retain];
        [q->_pending removeObjectAtIndex:i];
        q->_running++;
        struct opjob *j = calloc(1, sizeof *j);
        j->op = op; /* the retain above */
        j->q = [q retain];
        dispatch_async_f(q->_target, j, run_job);
    }
}
static void schedule_all_locked(void) {
    for (NSOperationQueue *q = all_queues; q; q = q->_nextq) schedule_queue_locked(q);
}

static NSOperationQueue *main_opq;
@implementation NSOperationQueue
+ (id)mainQueue {
    pthread_mutex_lock(&op_mu);
    if (!main_opq) {
        pthread_mutex_unlock(&op_mu);
        NSOperationQueue *q = [[NSOperationQueue alloc] init];
        pthread_mutex_lock(&op_mu);
        if (!main_opq) { q->_main = YES; q->_max = 1; q->_target = &_dispatch_main_q; q->_name = [CFSTR("NSOperationQueue Main Queue") retain]; main_opq = q; }
        else [q release];
    }
    pthread_mutex_unlock(&op_mu);
    return main_opq;
}
+ (id)currentQueue { return pthread_main_np() ? [self mainQueue] : nil; }
- (id)init {
    _pending = [NSMutableArray new];
    _max = -1;
    _target = dispatch_get_global_queue(0, 0);
    pthread_mutex_lock(&op_mu);
    _nextq = all_queues;
    all_queues = self;
    pthread_mutex_unlock(&op_mu);
    return self;
}
- (void)addOperation:(NSOperation *)op {
    pthread_mutex_lock(&op_mu);
    if (op->_queued || op->_executing || op->_finished) {
        pthread_mutex_unlock(&op_mu);
        _prism_raise("NSInvalidArgumentException", "*** -[NSOperationQueue addOperation:]: operation is finished, is executing, or is already in a queue, and cannot be enqueued");
    }
    op->_queued = YES;
    NSUInteger i = [_pending count];
    while (i > 0 && ((NSOperation *)[_pending objectAtIndex:i - 1])->_priority < op->_priority) i--;
    [_pending insertObject:op atIndex:i];
    _count++;
    schedule_queue_locked(self);
    pthread_mutex_unlock(&op_mu);
}
- (void)addOperationWithBlock:(void (^)(void))b { [self addOperation:[NSBlockOperation blockOperationWithBlock:b]]; }
- (void)addBarrierBlock:(void (^)(void))b {
    /* Runs after everything enqueued so far. */
    NSBlockOperation *bo = [NSBlockOperation blockOperationWithBlock:b];
    pthread_mutex_lock(&op_mu);
    for (NSUInteger i = 0, n = [_pending count]; i < n; i++) [bo->_deps addObject:[_pending objectAtIndex:i]];
    pthread_mutex_unlock(&op_mu);
    [self addOperation:bo];
}
- (void)addOperations:(id)ops waitUntilFinished:(BOOL)w {
    for (NSUInteger i = 0, n = [ops count]; i < n; i++) [self addOperation:[ops objectAtIndex:i]];
    if (w) for (NSUInteger i = 0, n = [ops count]; i < n; i++) [[ops objectAtIndex:i] waitUntilFinished];
}
- (void)waitUntilAllOperationsAreFinished {
    pthread_mutex_lock(&op_mu);
    while (_count > 0) {
        if (pthread_main_np()) {
            extern int _prism_dispatch_main_drain(void);
            pthread_mutex_unlock(&op_mu);
            _prism_dispatch_main_drain();
            pthread_mutex_lock(&op_mu);
            if (_count <= 0) break;
            double t = now_abs() + 0.002 + EPOCH_2001;
            struct timespec ts = {(time_t)t, (long)((t - (double)(time_t)t) * 1e9)};
            pthread_cond_timedwait(&op_cv, &op_mu, &ts);
        } else pthread_cond_wait(&op_cv, &op_mu);
    }
    pthread_mutex_unlock(&op_mu);
}
- (void)cancelAllOperations {
    pthread_mutex_lock(&op_mu);
    for (NSUInteger i = 0, n = [_pending count]; i < n; i++) ((NSOperation *)[_pending objectAtIndex:i])->_cancelled = YES;
    schedule_queue_locked(self);
    pthread_mutex_unlock(&op_mu);
}
- (NSUInteger)operationCount { return (NSUInteger)_count; }
- (id)operations { pthread_mutex_lock(&op_mu); id r = [[_pending copy] autorelease]; pthread_mutex_unlock(&op_mu); return r; }
- (NSInteger)maxConcurrentOperationCount { return _max; }
- (void)setMaxConcurrentOperationCount:(NSInteger)m { pthread_mutex_lock(&op_mu); if (!_main) _max = m; schedule_queue_locked(self); pthread_mutex_unlock(&op_mu); }
- (BOOL)isSuspended { return _suspended; }
- (void)setSuspended:(BOOL)s { pthread_mutex_lock(&op_mu); _suspended = s; schedule_queue_locked(self); pthread_mutex_unlock(&op_mu); }
- (id)name { return _name; }
- (void)setName:(id)n { id o = _name; _name = [n copy]; [o release]; }
- (NSInteger)qualityOfService { return -1; }
- (void)setQualityOfService:(NSInteger)q { (void)q; }
- (void *)underlyingQueue { return _target; }
- (void)setUnderlyingQueue:(void *)q { if (!_main && q) _target = q; }
- (void)dealloc {
    pthread_mutex_lock(&op_mu);
    for (NSOperationQueue **pp = &all_queues; *pp; pp = &(*pp)->_nextq)
        if (*pp == self) { *pp = _nextq; break; }
    pthread_mutex_unlock(&op_mu);
    [_pending release];
    [_name release];
    [super dealloc];
}
@end
