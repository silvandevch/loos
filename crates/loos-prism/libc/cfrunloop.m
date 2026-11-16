/* CFRunLoop / NSRunLoop with timers (CFRunLoopTimer, NSTimer, -performSelector:afterDelay:),
 * version-0 sources, observers and queued blocks. Prism runs one thread per process, so there is
 * one run loop; it sleeps with nanosleep until the next timer. Modes are accepted but not told
 * apart: everything is scheduled in the common modes. */
#include "foundation.h"

typedef double CFAbsoluteTime, CFTimeInterval;
double CFAbsoluteTimeGetCurrent(void);
typedef struct { CFIndex version; void *info; const void *(*retain)(const void *); void (*release)(const void *); id (*desc)(const void *); } CFContext;
typedef struct {
    CFIndex version; void *info; const void *(*retain)(const void *); void (*release)(const void *); id (*desc)(const void *);
    Boolean (*equal)(const void *, const void *); CFHashCode (*hash)(const void *);
    void (*schedule)(void *, id, id); void (*cancel)(void *, id, id); void (*perform)(void *);
} CFRunLoopSourceContext;

id const kCFRunLoopDefaultMode = CFSTR("kCFRunLoopDefaultMode");
id const kCFRunLoopCommonModes = CFSTR("kCFRunLoopCommonModes");
id const NSDefaultRunLoopMode = CFSTR("kCFRunLoopDefaultMode");
id const NSRunLoopCommonModes = CFSTR("kCFRunLoopCommonModes");

/* ------------------------------------------------------------------ objects */

@interface NSTimer : NSObject @end
@interface __NSCFTimer : NSTimer {
@public
    double fire, interval;
    BOOL valid, scheduled;
    void (*callout)(id, void *);
    CFContext ctx;
    void (^block)(id);
    id target, userInfo;
    SEL sel;
}
@end
@interface __NSCFRunLoopSource : NSObject {
@public
    CFRunLoopSourceContext ctx;
    BOOL signaled, valid;
    CFIndex order;
}
@end
@interface __NSCFRunLoopObserver : NSObject {
@public
    unsigned long activities;
    BOOL repeats, valid;
    void (*callout)(id, unsigned long, void *);
    void (^block)(id, unsigned long);
    CFContext ctx;
}
@end
@interface __NSCFRunLoop : NSObject {
@public
    id timers, sources, observers, blocks;
    BOOL stopped;
    int depth;
}
@end
@interface NSRunLoop : NSObject { @public __NSCFRunLoop *cf; }
+ (id)currentRunLoop;
@end

static __NSCFRunLoop *the_loop;
static NSRunLoop *the_nsloop;

@implementation __NSCFRunLoop
- (id)init {
    timers = [NSMutableArray new];
    sources = [NSMutableArray new];
    observers = [NSMutableArray new];
    blocks = [NSMutableArray new];
    return self;
}
- (id)retain { return self; }
- (oneway void)release {}
- (id)autorelease { return self; }
- (CFTypeID)_cfTypeID { return 43; }
@end
static __NSCFRunLoop *loop(void) {
    if (!the_loop) the_loop = [[__NSCFRunLoop alloc] init];
    return the_loop;
}

/* ------------------------------------------------------------------ timers */

static void timer_fire(__NSCFTimer *t) {
    [t retain];
    if (t->callout) t->callout(t, t->ctx.info);
    else if (t->block) t->block(t);
    else if (t->target) ((void (*)(id, SEL, id))objc_msgSend)(t->target, t->sel, t);
    [t release];
}
static void timer_unschedule(__NSCFTimer *t) {
    if (!t->scheduled) return;
    t->scheduled = NO;
    [loop()->timers removeObjectIdenticalTo:t];
}

@implementation NSTimer
+ (id)allocWithZone:(NSZone *)z { (void)z; return class_createInstance(objc_getClass("__NSCFTimer"), 0); }
+ (id)alloc { return [self allocWithZone:0]; }
+ (id)timerWithTimeInterval:(double)s repeats:(BOOL)r block:(void (^)(id))b {
    __NSCFTimer *t = [self alloc];
    t->fire = CFAbsoluteTimeGetCurrent() + s;
    t->interval = r ? (s > 0 ? s : 0.0001) : 0;
    t->block = [b copy];
    t->valid = YES;
    return [t autorelease];
}
+ (id)scheduledTimerWithTimeInterval:(double)s repeats:(BOOL)r block:(void (^)(id))b {
    id t = [self timerWithTimeInterval:s repeats:r block:b];
    [[NSRunLoop currentRunLoop] addTimer:t forMode:NSDefaultRunLoopMode];
    return t;
}
+ (id)timerWithTimeInterval:(double)s target:(id)target selector:(SEL)sel userInfo:(id)info repeats:(BOOL)r {
    __NSCFTimer *t = [self alloc];
    t->fire = CFAbsoluteTimeGetCurrent() + s;
    t->interval = r ? (s > 0 ? s : 0.0001) : 0;
    t->target = [target retain];
    t->sel = sel;
    t->userInfo = [info retain];
    t->valid = YES;
    return [t autorelease];
}
+ (id)scheduledTimerWithTimeInterval:(double)s target:(id)target selector:(SEL)sel userInfo:(id)info repeats:(BOOL)r {
    id t = [self timerWithTimeInterval:s target:target selector:sel userInfo:info repeats:r];
    [[NSRunLoop currentRunLoop] addTimer:t forMode:NSDefaultRunLoopMode];
    return t;
}
- (id)initWithFireDate:(NSDate *)d interval:(double)s repeats:(BOOL)r block:(void (^)(id))b {
    __NSCFTimer *t = (__NSCFTimer *)self;
    t->fire = [d timeIntervalSinceReferenceDate];
    t->interval = r ? s : 0;
    t->block = [b copy];
    t->valid = YES;
    return self;
}
- (id)initWithFireDate:(NSDate *)d interval:(double)s target:(id)target selector:(SEL)sel userInfo:(id)info repeats:(BOOL)r {
    __NSCFTimer *t = (__NSCFTimer *)self;
    t->fire = [d timeIntervalSinceReferenceDate];
    t->interval = r ? s : 0;
    t->target = [target retain];
    t->sel = sel;
    t->userInfo = [info retain];
    t->valid = YES;
    return self;
}
- (void)fire {
    __NSCFTimer *t = (__NSCFTimer *)self;
    if (!t->valid) return;
    timer_fire(t);
    if (!t->interval) [self invalidate];
}
- (void)invalidate {
    __NSCFTimer *t = (__NSCFTimer *)self;
    if (!t->valid) return;
    t->valid = NO;
    [self retain];
    timer_unschedule(t);
    if (t->ctx.release && t->ctx.info) t->ctx.release(t->ctx.info);
    t->ctx.info = 0;
    [t->target release];
    t->target = nil;
    [self release];
}
- (BOOL)isValid { return ((__NSCFTimer *)self)->valid; }
- (id)fireDate { return [NSDate dateWithTimeIntervalSinceReferenceDate:((__NSCFTimer *)self)->fire]; }
- (void)setFireDate:(NSDate *)d { ((__NSCFTimer *)self)->fire = [d timeIntervalSinceReferenceDate]; }
- (double)timeInterval { return ((__NSCFTimer *)self)->interval; }
- (id)userInfo { return ((__NSCFTimer *)self)->userInfo; }
- (double)tolerance { return 0; }
- (void)setTolerance:(double)t { (void)t; }
- (CFTypeID)_cfTypeID { return 44; }
@end
@implementation __NSCFTimer
- (void)dealloc {
    [block release];
    [target release];
    [userInfo release];
    if (ctx.release && ctx.info) ctx.release(ctx.info);
    [super dealloc];
}
@end

static void timer_schedule(__NSCFTimer *t) {
    if (t->scheduled || !t->valid) return;
    t->scheduled = YES;
    [loop()->timers addObject:t];
}

/* ------------------------------------------------------------------ sources and observers */

@implementation __NSCFRunLoopSource
- (void)dealloc { if (ctx.release && ctx.info) ctx.release(ctx.info); [super dealloc]; }
- (CFTypeID)_cfTypeID { return 46; }
@end
@implementation __NSCFRunLoopObserver
- (void)dealloc { [block release]; if (ctx.release && ctx.info) ctx.release(ctx.info); [super dealloc]; }
- (CFTypeID)_cfTypeID { return 45; }
@end

static void notify(unsigned long activity) {
    __NSCFRunLoop *rl = loop();
    if (![rl->observers count]) return;
    id obs = [[rl->observers copy] autorelease];
    for (__NSCFRunLoopObserver *o in obs) {
        if (!o->valid || !(o->activities & activity)) continue;
        if (o->callout) o->callout(o, activity, o->ctx.info);
        else if (o->block) o->block(o, activity);
        if (!o->repeats) { o->valid = NO; [rl->observers removeObjectIdenticalTo:o]; }
    }
}

/* ------------------------------------------------------------------ the loop */

enum { FINISHED = 1, STOPPED = 2, TIMED_OUT = 3, HANDLED = 4 };

/* Fire the due timers and signaled sources once; returns whether anything ran. */
static BOOL service(__NSCFRunLoop *rl) {
    BOOL handled = NO;
    if ([rl->blocks count]) {
        id bs = [rl->blocks copy];
        [rl->blocks removeAllObjects];
        for (id b in bs) ((void (^)(void))b)();
        [bs release];
        handled = YES;
    }
    notify(2); /* before timers */
    double now = CFAbsoluteTimeGetCurrent();
    id due = [NSMutableArray array];
    for (__NSCFTimer *t in rl->timers) if (t->fire <= now) [due addObject:t];
    for (__NSCFTimer *t in due) {
        if (!t->valid) continue;
        timer_fire(t);
        handled = YES;
        if (!t->valid) continue;
        if (t->interval > 0) {
            /* the next fire time on the timer's grid, skipping missed ones */
            double next = t->fire + t->interval;
            double n2 = CFAbsoluteTimeGetCurrent();
            if (next <= n2) next += t->interval * floor_((n2 - next) / t->interval + 1);
            t->fire = next;
        } else {
            [t invalidate];
        }
    }
    notify(4); /* before sources */
    id srcs = [[rl->sources copy] autorelease];
    for (__NSCFRunLoopSource *s in srcs) {
        if (!s->valid || !s->signaled) continue;
        s->signaled = NO;
        if (s->ctx.perform) s->ctx.perform(s->ctx.info);
        handled = YES;
    }
    return handled;
}

static int run_in_mode(double seconds, BOOL return_after_source) {
    __NSCFRunLoop *rl = loop();
    double deadline = CFAbsoluteTimeGetCurrent() + (seconds > 0 ? seconds : 0);
    int result;
    rl->depth++;
    notify(1); /* entry */
    for (;;) {
        @autoreleasepool {
            BOOL handled = service(rl);
            if (rl->stopped) { rl->stopped = NO; result = STOPPED; break; }
            if (handled && return_after_source) { result = HANDLED; break; }
            BOOL any_source = NO;
            for (__NSCFRunLoopSource *s in rl->sources) any_source |= s->valid;
            if (![rl->timers count] && !any_source && ![rl->blocks count]) { result = FINISHED; break; }
            double now = CFAbsoluteTimeGetCurrent();
            if (now >= deadline) { result = TIMED_OUT; break; }
            BOOL pending = [rl->blocks count] != 0;
            for (__NSCFRunLoopSource *s in rl->sources) pending |= s->valid && s->signaled;
            if (pending) continue;
            double wake = deadline;
            for (__NSCFTimer *t in rl->timers) if (t->fire < wake) wake = t->fire;
            if (wake > now) {
                notify(32); /* before waiting */
                double d = wake - now;
                if (d > 1e6) d = 1e6;
                struct timespec ts = {(time_t)d, (long)((d - (time_t)d) * 1e9)};
                nanosleep(&ts, 0);
                notify(64); /* after waiting */
            }
        }
    }
    notify(128); /* exit */
    rl->depth--;
    return result;
}

/* ------------------------------------------------------------------ CFRunLoop */

id CFRunLoopGetCurrent(void) { return loop(); }
id CFRunLoopGetMain(void) { return loop(); }
id CFRunLoopCopyCurrentMode(id rl) { (void)rl; return loop()->depth ? [kCFRunLoopDefaultMode retain] : nil; }
CFTypeID CFRunLoopGetTypeID(void) { return 43; }
int CFRunLoopRunInMode(id mode, double seconds, Boolean return_after) { (void)mode; return run_in_mode(seconds, return_after); }
void CFRunLoopRun(void) {
    int r;
    do r = run_in_mode(1e10, NO); while (r != STOPPED && r != FINISHED);
}
void CFRunLoopStop(id rl) { (void)rl; loop()->stopped = YES; }
void CFRunLoopWakeUp(id rl) { (void)rl; }
Boolean CFRunLoopIsWaiting(id rl) { (void)rl; return 0; }
void CFRunLoopPerformBlock(id rl, id mode, void (^b)(void)) {
    (void)rl; (void)mode;
    id c = [b copy];
    [loop()->blocks addObject:c];
    [c release];
}
void CFRunLoopAddCommonMode(id rl, id mode) { (void)rl; (void)mode; }

id CFRunLoopTimerCreate(CFAllocatorRef a, double fire, double interval, unsigned long flags, CFIndex order, void (*cb)(id, void *), CFContext *ctx) {
    (void)a; (void)flags; (void)order;
    __NSCFTimer *t = class_createInstance(objc_getClass("__NSCFTimer"), 0);
    t->fire = fire;
    t->interval = interval > 0 ? interval : 0;
    t->callout = cb;
    t->valid = YES;
    if (ctx) {
        t->ctx = *ctx;
        if (ctx->retain && ctx->info) t->ctx.info = (void *)ctx->retain(ctx->info);
    }
    return t;
}
id CFRunLoopTimerCreateWithHandler(CFAllocatorRef a, double fire, double interval, unsigned long flags, CFIndex order, void (^b)(id)) {
    __NSCFTimer *t = CFRunLoopTimerCreate(a, fire, interval, flags, order, 0, 0);
    t->block = [b copy];
    return t;
}
void CFRunLoopAddTimer(id rl, id t, id mode) { (void)rl; (void)mode; timer_schedule(t); }
void CFRunLoopRemoveTimer(id rl, id t, id mode) { (void)rl; (void)mode; timer_unschedule(t); }
Boolean CFRunLoopContainsTimer(id rl, id t, id mode) { (void)rl; (void)mode; return ((__NSCFTimer *)t)->scheduled; }
void CFRunLoopTimerInvalidate(id t) { [t invalidate]; }
Boolean CFRunLoopTimerIsValid(id t) { return ((__NSCFTimer *)t)->valid; }
Boolean CFRunLoopTimerDoesRepeat(id t) { return ((__NSCFTimer *)t)->interval > 0; }
double CFRunLoopTimerGetInterval(id t) { return ((__NSCFTimer *)t)->interval; }
double CFRunLoopTimerGetNextFireDate(id t) { return ((__NSCFTimer *)t)->fire; }
void CFRunLoopTimerSetNextFireDate(id t, double d) { ((__NSCFTimer *)t)->fire = d; }
void CFRunLoopTimerGetContext(id t, CFContext *out) { *out = ((__NSCFTimer *)t)->ctx; }
CFTypeID CFRunLoopTimerGetTypeID(void) { return 44; }
double CFRunLoopGetNextTimerFireDate(id rl, id mode) {
    (void)rl; (void)mode;
    double best = 0;
    for (__NSCFTimer *t in loop()->timers) if (!best || t->fire < best) best = t->fire;
    return best;
}

id CFRunLoopSourceCreate(CFAllocatorRef a, CFIndex order, CFRunLoopSourceContext *ctx) {
    (void)a;
    __NSCFRunLoopSource *s = [__NSCFRunLoopSource new];
    s->order = order;
    s->valid = YES;
    if (ctx) {
        s->ctx = *ctx;
        if (ctx->retain && ctx->info) s->ctx.info = (void *)ctx->retain(ctx->info);
    }
    return s;
}
void CFRunLoopAddSource(id rl, id s, id mode) {
    (void)rl;
    __NSCFRunLoopSource *src = s;
    if ([loop()->sources indexOfObjectIdenticalTo:s] != NSNotFound) return;
    [loop()->sources addObject:s];
    if (src->ctx.schedule) src->ctx.schedule(src->ctx.info, loop(), mode);
}
void CFRunLoopRemoveSource(id rl, id s, id mode) {
    (void)rl;
    __NSCFRunLoopSource *src = s;
    if ([loop()->sources indexOfObjectIdenticalTo:s] == NSNotFound) return;
    if (src->ctx.cancel) src->ctx.cancel(src->ctx.info, loop(), mode);
    [loop()->sources removeObjectIdenticalTo:s];
}
Boolean CFRunLoopContainsSource(id rl, id s, id mode) { (void)rl; (void)mode; return [loop()->sources indexOfObjectIdenticalTo:s] != NSNotFound; }
void CFRunLoopSourceSignal(id s) { ((__NSCFRunLoopSource *)s)->signaled = YES; }
void CFRunLoopSourceInvalidate(id s) { ((__NSCFRunLoopSource *)s)->valid = NO; CFRunLoopRemoveSource(loop(), s, kCFRunLoopCommonModes); }
Boolean CFRunLoopSourceIsValid(id s) { return ((__NSCFRunLoopSource *)s)->valid; }
CFIndex CFRunLoopSourceGetOrder(id s) { return ((__NSCFRunLoopSource *)s)->order; }
CFTypeID CFRunLoopSourceGetTypeID(void) { return 46; }

id CFRunLoopObserverCreate(CFAllocatorRef a, unsigned long activities, Boolean repeats, CFIndex order, void (*cb)(id, unsigned long, void *), CFContext *ctx) {
    (void)a; (void)order;
    __NSCFRunLoopObserver *o = [__NSCFRunLoopObserver new];
    o->activities = activities;
    o->repeats = repeats;
    o->valid = YES;
    o->callout = cb;
    if (ctx) {
        o->ctx = *ctx;
        if (ctx->retain && ctx->info) o->ctx.info = (void *)ctx->retain(ctx->info);
    }
    return o;
}
id CFRunLoopObserverCreateWithHandler(CFAllocatorRef a, unsigned long activities, Boolean repeats, CFIndex order, void (^b)(id, unsigned long)) {
    __NSCFRunLoopObserver *o = CFRunLoopObserverCreate(a, activities, repeats, order, 0, 0);
    o->block = [b copy];
    return o;
}
void CFRunLoopAddObserver(id rl, id o, id mode) { (void)rl; (void)mode; if ([loop()->observers indexOfObjectIdenticalTo:o] == NSNotFound) [loop()->observers addObject:o]; }
void CFRunLoopRemoveObserver(id rl, id o, id mode) { (void)rl; (void)mode; [loop()->observers removeObjectIdenticalTo:o]; }
void CFRunLoopObserverInvalidate(id o) { ((__NSCFRunLoopObserver *)o)->valid = NO; [loop()->observers removeObjectIdenticalTo:o]; }
Boolean CFRunLoopObserverIsValid(id o) { return ((__NSCFRunLoopObserver *)o)->valid; }
CFTypeID CFRunLoopObserverGetTypeID(void) { return 45; }

/* ------------------------------------------------------------------ NSRunLoop */

@implementation NSRunLoop
+ (id)currentRunLoop {
    if (!the_nsloop) { the_nsloop = [NSRunLoop new]; the_nsloop->cf = loop(); }
    return the_nsloop;
}
+ (id)mainRunLoop { return [self currentRunLoop]; }
- (id)retain { return self; }
- (oneway void)release {}
- (id)autorelease { return self; }
- (id)getCFRunLoop { return cf; }
- (id)currentMode { return cf->depth ? NSDefaultRunLoopMode : nil; }
- (void)addTimer:(NSTimer *)t forMode:(id)mode { (void)mode; timer_schedule((__NSCFTimer *)t); }
- (void)run { while (run_in_mode(1e10, NO) != FINISHED) {} }
- (void)runUntilDate:(NSDate *)d {
    for (;;) {
        double left = [d timeIntervalSinceReferenceDate] - CFAbsoluteTimeGetCurrent();
        if (left <= 0 || run_in_mode(left, NO) == FINISHED) return;
    }
}
- (BOOL)runMode:(id)mode beforeDate:(NSDate *)d {
    (void)mode;
    int r = run_in_mode([d timeIntervalSinceReferenceDate] - CFAbsoluteTimeGetCurrent(), YES);
    return r != FINISHED;
}
- (void)acceptInputForMode:(id)mode beforeDate:(NSDate *)d { [self runMode:mode beforeDate:d]; }
- (void)performBlock:(void (^)(void))b { CFRunLoopPerformBlock(cf, kCFRunLoopDefaultMode, b); }
- (void)performInModes:(id)modes block:(void (^)(void))b { (void)modes; [self performBlock:b]; }
@end

/* ------------------------------------------------------------------ delayed messages */

@interface __NSDelayedPerform : NSObject { @public id target, arg; SEL sel; }
@end
@implementation __NSDelayedPerform
- (void)fire:(id)t { (void)t; ((void (*)(id, SEL, id))objc_msgSend)(target, sel, arg); }
- (void)dealloc { [target release]; [arg release]; [super dealloc]; }
@end

@implementation NSObject (RunLoop)
- (void)performSelector:(SEL)s withObject:(id)arg afterDelay:(double)delay {
    __NSDelayedPerform *p = [[__NSDelayedPerform new] autorelease];
    p->target = [self retain];
    p->arg = [arg retain];
    p->sel = s;
    [NSTimer scheduledTimerWithTimeInterval:delay target:p selector:@selector(fire:) userInfo:nil repeats:NO];
}
- (void)performSelector:(SEL)s withObject:(id)arg afterDelay:(double)delay inModes:(id)modes { (void)modes; [self performSelector:s withObject:arg afterDelay:delay]; }
- (void)performSelectorOnMainThread:(SEL)s withObject:(id)arg waitUntilDone:(BOOL)wait {
    if (wait) { ((void (*)(id, SEL, id))objc_msgSend)(self, s, arg); return; }
    [self performSelector:s withObject:arg afterDelay:0];
}
+ (void)cancelPreviousPerformRequestsWithTarget:(id)target {
    for (__NSCFTimer *t in [[loop()->timers copy] autorelease]) {
        __NSDelayedPerform *p = t->target;
        if (cf_is(p, objc_getClass("__NSDelayedPerform")) && p->target == target) [t invalidate];
    }
}
+ (void)cancelPreviousPerformRequestsWithTarget:(id)target selector:(SEL)s object:(id)arg {
    for (__NSCFTimer *t in [[loop()->timers copy] autorelease]) {
        __NSDelayedPerform *p = t->target;
        if (cf_is(p, objc_getClass("__NSDelayedPerform")) && p->target == target && p->sel == s && (p->arg == arg || [p->arg isEqual:arg])) [t invalidate];
    }
}
@end
