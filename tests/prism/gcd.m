/* libdispatch, NSThread, the Foundation locks and NSOperationQueue through Prism (ARC). No SDK:
 * the interfaces are declared here with the SDK's names and types. */
int printf(const char *, ...);
typedef unsigned long NSUInteger, size_t, uintptr_t;
typedef long NSInteger, intptr_t;
typedef unsigned long long uint64_t;
typedef long long int64_t;
#if __arm64__
typedef _Bool BOOL;
#else
typedef signed char BOOL;
#endif
#define YES __objc_yes
#define NO __objc_no
#define nil ((void *)0)

/* ---- libdispatch (SDK names; objects are Objective-C objects under ARC) ---- */
__attribute__((objc_root_class))
@interface NSObject
+ (instancetype)alloc;
+ (instancetype)new;
- (instancetype)init;
@end
@protocol OS_dispatch_object @end
typedef NSObject<OS_dispatch_object> *dispatch_object_t;
@protocol OS_dispatch_queue <OS_dispatch_object> @end
@protocol OS_dispatch_group <OS_dispatch_object> @end
@protocol OS_dispatch_semaphore <OS_dispatch_object> @end
@protocol OS_dispatch_source <OS_dispatch_object> @end
typedef id<OS_dispatch_queue> dispatch_queue_t;
typedef id<OS_dispatch_group> dispatch_group_t;
typedef id<OS_dispatch_semaphore> dispatch_semaphore_t;
typedef id<OS_dispatch_source> dispatch_source_t;
typedef uint64_t dispatch_time_t;
typedef const struct dispatch_source_type_s *dispatch_source_type_t;
#define DISPATCH_TIME_NOW 0ull
#define DISPATCH_TIME_FOREVER (~0ull)
#define NSEC_PER_MSEC 1000000ull
extern struct dispatch_queue_s _dispatch_main_q;
#define dispatch_get_main_queue() ((__bridge dispatch_queue_t)(void *)&_dispatch_main_q)
extern const struct dispatch_queue_attr_s _dispatch_queue_attr_concurrent;
#define DISPATCH_QUEUE_CONCURRENT ((__bridge id)(void *)&_dispatch_queue_attr_concurrent)
extern const struct dispatch_source_type_s _dispatch_source_type_timer, _dispatch_source_type_data_add;
dispatch_queue_t dispatch_get_global_queue(long, unsigned long);
dispatch_queue_t dispatch_queue_create(const char *, id);
const char *dispatch_queue_get_label(dispatch_queue_t);
void dispatch_async(dispatch_queue_t, void (^)(void));
void dispatch_sync(dispatch_queue_t, void (^)(void));
void dispatch_barrier_async(dispatch_queue_t, void (^)(void));
void dispatch_after(dispatch_time_t, dispatch_queue_t, void (^)(void));
void dispatch_apply(size_t, dispatch_queue_t, void (^)(size_t));
void dispatch_once(long *, void (^)(void));
dispatch_time_t dispatch_time(dispatch_time_t, int64_t);
dispatch_group_t dispatch_group_create(void);
void dispatch_group_async(dispatch_group_t, dispatch_queue_t, void (^)(void));
void dispatch_group_enter(dispatch_group_t);
void dispatch_group_leave(dispatch_group_t);
long dispatch_group_wait(dispatch_group_t, dispatch_time_t);
void dispatch_group_notify(dispatch_group_t, dispatch_queue_t, void (^)(void));
dispatch_semaphore_t dispatch_semaphore_create(long);
long dispatch_semaphore_wait(dispatch_semaphore_t, dispatch_time_t);
long dispatch_semaphore_signal(dispatch_semaphore_t);
dispatch_source_t dispatch_source_create(dispatch_source_type_t, uintptr_t, unsigned long, dispatch_queue_t);
void dispatch_source_set_timer(dispatch_source_t, dispatch_time_t, uint64_t, uint64_t);
void dispatch_source_set_event_handler(dispatch_source_t, void (^)(void));
void dispatch_source_set_cancel_handler(dispatch_source_t, void (^)(void));
void dispatch_source_cancel(dispatch_source_t);
void dispatch_source_merge_data(dispatch_source_t, unsigned long);
unsigned long dispatch_source_get_data(dispatch_source_t);
void dispatch_resume(dispatch_object_t);
void dispatch_queue_set_specific(dispatch_queue_t, const void *, void *, void (*)(void *));
void *dispatch_get_specific(const void *);
int CFRunLoopRunInMode(const void *, double, unsigned char);
extern const void *kCFRunLoopDefaultMode;

/* ---- Foundation ---- */
@interface NSString : NSObject
- (const char *)UTF8String;
@end
@interface NSDate : NSObject
+ (id)dateWithTimeIntervalSinceNow:(double)s;
- (double)timeIntervalSinceNow;
@end
@interface NSThread : NSObject
+ (NSThread *)currentThread;
+ (BOOL)isMainThread;
+ (void)detachNewThreadWithBlock:(void (^)(void))b;
+ (void)sleepForTimeInterval:(double)s;
- (instancetype)initWithBlock:(void (^)(void))b;
- (void)start;
- (BOOL)isFinished;
- (void)setName:(NSString *)n;
- (NSString *)name;
@end
@interface NSLock : NSObject
- (void)lock;
- (void)unlock;
- (BOOL)tryLock;
@end
@interface NSRecursiveLock : NSLock @end
@interface NSCondition : NSLock
- (void)wait;
- (void)signal;
@end
@interface NSConditionLock : NSLock
- (instancetype)initWithCondition:(NSInteger)c;
- (void)lockWhenCondition:(NSInteger)c;
- (void)unlockWithCondition:(NSInteger)c;
@end
@interface NSOperation : NSObject
- (void)addDependency:(NSOperation *)o;
- (void)setCompletionBlock:(void (^)(void))b;
- (void)cancel;
- (BOOL)isFinished;
- (BOOL)isCancelled;
- (void)waitUntilFinished;
@end
@interface NSBlockOperation : NSOperation
+ (instancetype)blockOperationWithBlock:(void (^)(void))b;
- (void)addExecutionBlock:(void (^)(void))b;
@end
@interface NSOperationQueue : NSObject
+ (NSOperationQueue *)mainQueue;
- (void)addOperation:(NSOperation *)o;
- (void)addOperationWithBlock:(void (^)(void))b;
- (void)setMaxConcurrentOperationCount:(NSInteger)n;
- (void)waitUntilAllOperationsAreFinished;
- (void)setSuspended:(BOOL)s;
- (NSUInteger)operationCount;
@end
@interface NSObject (Perform)
- (void)performSelectorOnMainThread:(SEL)s withObject:(id)o waitUntilDone:(BOOL)w;
@end

static int perform_hits;
@interface Target : NSObject
- (void)hit:(id)o;
@end
@implementation Target
- (void)hit:(id)o { (void)o; if ([NSThread isMainThread]) perform_hits++; }
@end

/* Run the main run loop until `*flag` or 5 s. */
static void spin(volatile int *flag) {
    for (NSDate *end = [NSDate dateWithTimeIntervalSinceNow:5]; !*flag && [end timeIntervalSinceNow] > 0;)
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, YES);
}

int main(void) {
    @autoreleasepool {
        /* Serial queue: order, sync after async, label. */
        dispatch_queue_t sq = dispatch_queue_create("org.loos.serial", nil);
        static int order[50];
        __block int n = 0;
        for (int i = 0; i < 50; i++) dispatch_async(sq, ^{ order[n++] = i; });
        __block int after_sync = 0;
        dispatch_sync(sq, ^{ after_sync = n; });
        int ordered = 1;
        for (int i = 0; i < 50; i++) ordered &= order[i] == i;

        /* Global queue + group: 400 atomic increments on the pool. */
        dispatch_group_t g = dispatch_group_create();
        __block long total = 0;
        dispatch_queue_t gq = dispatch_get_global_queue(0, 0);
        for (int i = 0; i < 400; i++) dispatch_group_async(g, gq, ^{ __atomic_add_fetch(&total, i, __ATOMIC_RELAXED); });
        long gw = dispatch_group_wait(g, DISPATCH_TIME_FOREVER);

        /* Concurrent queue with a barrier. */
        dispatch_queue_t cq = dispatch_queue_create("org.loos.concurrent", DISPATCH_QUEUE_CONCURRENT);
        __block int before = 0, seen = -1;
        for (int i = 0; i < 10; i++) dispatch_async(cq, ^{ __atomic_add_fetch(&before, 1, __ATOMIC_RELAXED); });
        dispatch_barrier_async(cq, ^{ seen = before; });
        dispatch_sync(cq, ^{});

        /* Semaphore handoff and timeout; dispatch_after. */
        dispatch_semaphore_t sem = dispatch_semaphore_create(0);
        long timedout = dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, 20 * NSEC_PER_MSEC));
        NSDate *t0 = [NSDate dateWithTimeIntervalSinceNow:0];
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 60 * NSEC_PER_MSEC), gq, ^{ dispatch_semaphore_signal(sem); });
        long got = dispatch_semaphore_wait(sem, DISPATCH_TIME_FOREVER);
        int waited_ms = (int)(-[t0 timeIntervalSinceNow] * 1000);

        /* dispatch_apply, dispatch_once from many threads, queue-specific data. */
        __block long applied = 0;
        dispatch_apply(10, gq, ^(size_t i) { __atomic_add_fetch(&applied, (long)i, __ATOMIC_RELAXED); });
        static long once;
        __block int once_runs = 0;
        dispatch_group_t og = dispatch_group_create();
        for (int i = 0; i < 8; i++) dispatch_group_async(og, gq, ^{ dispatch_once(&once, ^{ once_runs++; }); });
        dispatch_group_wait(og, DISPATCH_TIME_FOREVER);
        static int key;
        dispatch_queue_set_specific(sq, &key, (void *)42, 0);
        __block intptr_t spec = 0;
        dispatch_sync(sq, ^{ spec = (intptr_t)dispatch_get_specific(&key); });
        printf("gcd: ordered=%d sync=%d label=%s total=%ld gw=%ld barrier=%d timeout=%d after=%ld waited>=50:%d apply=%ld once=%d spec=%ld\n",
               ordered, after_sync, dispatch_queue_get_label(sq), total, gw, seen, timedout != 0, got, waited_ms >= 50, applied, once_runs, (long)spec);

        /* Main queue from a worker, group notify on main, timer and data sources. */
        __block volatile int main_ran = 0, notified = 0;
        dispatch_async(gq, ^{ dispatch_async(dispatch_get_main_queue(), ^{ main_ran = [NSThread isMainThread] ? 1 : 2; }); });
        spin(&main_ran);
        dispatch_group_t ng = dispatch_group_create();
        dispatch_group_enter(ng);
        dispatch_group_notify(ng, dispatch_get_main_queue(), ^{ notified = 1; });
        dispatch_async(gq, ^{ dispatch_group_leave(ng); });
        spin(&notified);
        __block volatile int ticks = 0, cancelled = 0;
        dispatch_source_t timer = dispatch_source_create(&_dispatch_source_type_timer, 0, 0, dispatch_get_main_queue());
        dispatch_source_set_timer(timer, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_MSEC), 20 * NSEC_PER_MSEC, 0);
        __weak dispatch_source_t weak_timer = timer;
        dispatch_source_set_event_handler(timer, ^{ if (++ticks == 3) dispatch_source_cancel(weak_timer); });
        dispatch_source_set_cancel_handler(timer, ^{ cancelled = 1; });
        dispatch_resume(timer);
        spin(&cancelled);
        dispatch_source_t data = dispatch_source_create(&_dispatch_source_type_data_add, 0, 0, sq);
        __block unsigned long dsum = 0;
        dispatch_source_set_event_handler(data, ^{ dsum += dispatch_source_get_data(data); });
        dispatch_resume(data);
        for (int i = 1; i <= 10; i++) dispatch_source_merge_data(data, (unsigned long)i);
        dispatch_sync(sq, ^{});
        dispatch_sync(sq, ^{});
        printf("gcd2: main=%d notify=%d ticks=%d cancelled=%d data=%lu\n", main_ran, notified, ticks, cancelled, dsum);

        /* NSThread and locks. */
        NSLock *lock = [NSLock new];
        __block long shared = 0;
        __block volatile int done = 0;
        __block int bg_main = -1;
        for (int t = 0; t < 4; t++)
            [NSThread detachNewThreadWithBlock:^{
                if (t == 0) bg_main = [NSThread isMainThread];
                for (int i = 0; i < 2000; i++) { [lock lock]; shared++; [lock unlock]; }
                __atomic_add_fetch(&done, 1, __ATOMIC_RELAXED);
            }];
        while (done < 4) [NSThread sleepForTimeInterval:0.005];
        NSRecursiveLock *rl = [NSRecursiveLock new];
        [rl lock]; BOOL again = [rl tryLock]; [rl unlock]; [rl unlock];
        NSConditionLock *cl = [[NSConditionLock alloc] initWithCondition:0];
        __block int stage = 0;
        NSThread *th = [[NSThread alloc] initWithBlock:^{
            [cl lockWhenCondition:1];
            stage = 2;
            [cl unlockWithCondition:2];
        }];
        [th setName:@"worker"];
        [th start];
        [cl lockWhenCondition:0]; stage = 1; [cl unlockWithCondition:1];
        [cl lockWhenCondition:2]; [cl unlockWithCondition:0];
        while (![th isFinished]) [NSThread sleepForTimeInterval:0.002];
        Target *tg = [Target new];
        __block volatile int performed = 0;
        [NSThread detachNewThreadWithBlock:^{ [tg performSelectorOnMainThread:@selector(hit:) withObject:nil waitUntilDone:YES]; performed = 1; }];
        spin(&performed);
        printf("thread: shared=%ld bg_main=%d recursive=%d stage=%d name=%s performed=%d\n", shared, bg_main, again, stage, [[th name] UTF8String], perform_hits);

        /* NSOperationQueue: limits, dependencies, completion, cancel, main queue. */
        NSOperationQueue *oq = [NSOperationQueue new];
        [oq setMaxConcurrentOperationCount:2];
        __block long osum = 0;
        __block volatile int running = 0, peak = 0;
        for (int i = 1; i <= 20; i++)
            [oq addOperationWithBlock:^{
                int r = __atomic_add_fetch(&running, 1, __ATOMIC_RELAXED);
                if (r > peak) peak = r;
                [NSThread sleepForTimeInterval:0.002];
                __atomic_add_fetch(&osum, i, __ATOMIC_RELAXED);
                __atomic_sub_fetch(&running, 1, __ATOMIC_RELAXED);
            }];
        [oq waitUntilAllOperationsAreFinished];
        static char seq[8];
        __block int si = 0;
        NSLock *seqlock = [NSLock new];
        void (^mark)(char) = ^(char c) { [seqlock lock]; seq[si++] = c; [seqlock unlock]; };
        NSBlockOperation *a = [NSBlockOperation blockOperationWithBlock:^{ [NSThread sleepForTimeInterval:0.02]; mark('a'); }];
        NSBlockOperation *b = [NSBlockOperation blockOperationWithBlock:^{ mark('b'); }];
        [b addExecutionBlock:^{ mark('B'); }];
        [b addDependency:a];
        __block volatile int completed = 0;
        [b setCompletionBlock:^{ completed = 1; }];
        NSBlockOperation *c = [NSBlockOperation blockOperationWithBlock:^{ mark('X'); }];
        [c addDependency:b];
        [c cancel];
        NSOperationQueue *q2 = [NSOperationQueue new];
        [q2 setMaxConcurrentOperationCount:4];
        [q2 addOperation:c];
        [q2 addOperation:b];
        [q2 addOperation:a];
        [c waitUntilFinished];
        [b waitUntilFinished];
        for (int i = 0; i < 200 && !completed; i++) [NSThread sleepForTimeInterval:0.005];
        __block volatile int on_main = 0;
        [[NSOperationQueue mainQueue] addOperationWithBlock:^{ on_main = [NSThread isMainThread] ? 1 : 2; }];
        spin(&on_main);
        printf("ops: sum=%ld peak<=2:%d first=%c deps=%d completed=%d cancelled=%d/%d main=%d count=%lu\n", osum, peak <= 2, seq[0],
               seq[0] == 'a' && si == 3, completed, [c isCancelled], [c isFinished], on_main, (unsigned long)[q2 operationCount]);
    }
    return 0;
}
