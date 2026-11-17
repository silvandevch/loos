/* CFRunLoop / NSRunLoop with timers (CFRunLoopTimer, NSTimer, -performSelector:afterDelay:),
 * version-0 sources, observers and queued blocks, each scheduled in a set of modes. Running in a
 * mode services only what is scheduled in it, or in kCFRunLoopCommonModes when the mode is one of
 * the common modes (the default mode and any added with CFRunLoopAddCommonMode). Prism runs one
 * thread per process, so there is one run loop; it sleeps with nanosleep until the next timer. */
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
    id modes;
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
    id modes;
}
@end
@interface __NSCFRunLoopObserver : NSObject {
@public
    unsigned long activities;
    BOOL repeats, valid;
    id modes;
    void (*callout)(id, unsigned long, void *);
    void (^block)(id, unsigned long);
    CFContext ctx;
}
@end
@interface __NSCFRunLoop : NSObject {
@public
    id timers, sources, observers, blocks; /* blocks: [mode, block] pairs */
    id common;  /* the common modes */
    id mode;    /* the mode being run, nil outside a run */
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
    common = [[NSMutableSet alloc] initWithObjects:kCFRunLoopDefaultMode, nil];
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
/* Is something scheduled in `modes` serviced when running `mode`? */
static BOOL in_mode(id modes, id mode) {
    return [modes containsObject:mode] || ([modes containsObject:kCFRunLoopCommonModes] && [loop()->common containsObject:mode]);
}
static id mode_or_default(id m) { return m ? m : kCFRunLoopDefaultMode; }

/* ------------------------------------------------------------------ timers */

static void timer_fire(__NSCFTimer *t) {
    [t retain];
    if (t->callout) t->callout(t, t->ctx.info);
    else if (t->block) t->block(t);
    else if (t->target) ((void (*)(id, SEL, id))objc_msgSend)(t->target, t->sel, t);
    [t release];
}
/* Take `t` out of `mode` (nil: every mode); unscheduled once it is in none. */
static void timer_unschedule(__NSCFTimer *t, id mode) {
    if (!t->scheduled) return;
    if (mode) [t->modes removeObject:mode];
    else [t->modes removeAllObjects];
    if ([t->modes count]) return;
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
    timer_unschedule(t, nil);
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
    [modes release];
    [block release];
    [target release];
    [userInfo release];
    if (ctx.release && ctx.info) ctx.release(ctx.info);
    [super dealloc];
}
@end

static void timer_schedule(__NSCFTimer *t, id mode) {
    if (!t->valid) return;
    if (!t->modes) t->modes = [NSMutableSet new];
    [t->modes addObject:mode_or_default(mode)];
    if (t->scheduled) return;
    t->scheduled = YES;
    [loop()->timers addObject:t];
}

/* ------------------------------------------------------------------ sources and observers */

@implementation __NSCFRunLoopSource
- (void)dealloc { [modes release]; if (ctx.release && ctx.info) ctx.release(ctx.info); [super dealloc]; }
- (CFTypeID)_cfTypeID { return 46; }
@end
@implementation __NSCFRunLoopObserver
- (void)dealloc { [modes release]; [block release]; if (ctx.release && ctx.info) ctx.release(ctx.info); [super dealloc]; }
- (CFTypeID)_cfTypeID { return 45; }
@end

static void notify(unsigned long activity) {
    __NSCFRunLoop *rl = loop();
    if (![rl->observers count]) return;
    id obs = [[rl->observers copy] autorelease];
    for (__NSCFRunLoopObserver *o in obs) {
        if (!o->valid || !(o->activities & activity) || !in_mode(o->modes, rl->mode)) continue;
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
    id ready = [NSMutableArray array];
    for (id pair in [[rl->blocks copy] autorelease])
        if (in_mode([pair objectAtIndex:0], rl->mode)) { [ready addObject:[pair objectAtIndex:1]]; [rl->blocks removeObjectIdenticalTo:pair]; }
    for (id b in ready) ((void (^)(void))b)();
    if ([ready count]) handled = YES;
    notify(2); /* before timers */
    double now = CFAbsoluteTimeGetCurrent();
    id due = [NSMutableArray array];
    for (__NSCFTimer *t in rl->timers) if (t->fire <= now && in_mode(t->modes, rl->mode)) [due addObject:t];
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
        if (!s->valid || !s->signaled || !in_mode(s->modes, rl->mode)) continue;
        s->signaled = NO;
        if (s->ctx.perform) s->ctx.perform(s->ctx.info);
        handled = YES;
    }
    return handled;
}

static BOOL blocks_pending(__NSCFRunLoop *rl) {
    for (id pair in rl->blocks) if (in_mode([pair objectAtIndex:0], rl->mode)) return YES;
    return NO;
}
static int run_in_mode(id mode, double seconds, BOOL return_after_source) {
    __NSCFRunLoop *rl = loop();
    mode = mode_or_default(mode);
    if ([mode isEqual:kCFRunLoopCommonModes]) return FINISHED; /* not a mode one can run */
    id outer = rl->mode;
    rl->mode = mode;
    double deadline = CFAbsoluteTimeGetCurrent() + (seconds > 0 ? seconds : 0);
    int result;
    rl->depth++;
    notify(1); /* entry */
    for (;;) {
        @autoreleasepool {
            BOOL handled = service(rl);
            if (rl->stopped) { rl->stopped = NO; result = STOPPED; break; }
            if (handled && return_after_source) { result = HANDLED; break; }
            BOOL any = blocks_pending(rl);
            for (__NSCFRunLoopSource *s in rl->sources) any |= s->valid && in_mode(s->modes, mode);
            for (__NSCFTimer *t in rl->timers) any |= in_mode(t->modes, mode);
            if (!any) { result = FINISHED; break; }
            double now = CFAbsoluteTimeGetCurrent();
            if (now >= deadline) { result = TIMED_OUT; break; }
            BOOL pending = blocks_pending(rl);
            for (__NSCFRunLoopSource *s in rl->sources) pending |= s->valid && s->signaled && in_mode(s->modes, mode);
            if (pending) continue;
            double wake = deadline;
            for (__NSCFTimer *t in rl->timers) if (t->fire < wake && in_mode(t->modes, mode)) wake = t->fire;
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
    rl->mode = outer;
    return result;
}

/* ------------------------------------------------------------------ CFRunLoop */

id CFRunLoopGetCurrent(void) { return loop(); }
id CFRunLoopGetMain(void) { return loop(); }
id CFRunLoopCopyCurrentMode(id rl) { (void)rl; return [loop()->mode retain]; }
id CFRunLoopCopyAllModes(id rl) {
    (void)rl;
    id all = [NSMutableSet setWithSet:loop()->common];
    for (__NSCFTimer *t in loop()->timers) [all unionSet:t->modes];
    for (__NSCFRunLoopSource *s in loop()->sources) [all unionSet:s->modes];
    [all removeObject:kCFRunLoopCommonModes];
    return [[all allObjects] retain];
}
CFTypeID CFRunLoopGetTypeID(void) { return 43; }
int CFRunLoopRunInMode(id mode, double seconds, Boolean return_after) { return run_in_mode(mode, seconds, return_after); }
void CFRunLoopRun(void) {
    int r;
    do r = run_in_mode(kCFRunLoopDefaultMode, 1e10, NO); while (r != STOPPED && r != FINISHED);
}
void CFRunLoopStop(id rl) { (void)rl; loop()->stopped = YES; }
void CFRunLoopWakeUp(id rl) { (void)rl; }
Boolean CFRunLoopIsWaiting(id rl) { (void)rl; return 0; }
void CFRunLoopPerformBlock(id rl, id mode, void (^b)(void)) {
    (void)rl;
    id c = [b copy];
    /* `mode` is a mode name or an array or set of them */
    id modes = !mode ? [NSSet setWithObject:kCFRunLoopCommonModes] : [mode _cfTypeID] == 7 ? [NSSet setWithObject:mode] : [NSSet setWithArray:[mode allObjects]];
    [loop()->blocks addObject:[NSArray arrayWithObjects:modes, c, nil]];
    [c release];
}
void CFRunLoopAddCommonMode(id rl, id mode) { (void)rl; [loop()->common addObject:mode]; }

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
void CFRunLoopAddTimer(id rl, id t, id mode) { (void)rl; timer_schedule(t, mode); }
void CFRunLoopRemoveTimer(id rl, id t, id mode) { (void)rl; timer_unschedule(t, mode_or_default(mode)); }
Boolean CFRunLoopContainsTimer(id rl, id t, id mode) { (void)rl; return ((__NSCFTimer *)t)->scheduled && in_mode(((__NSCFTimer *)t)->modes, mode_or_default(mode)); }
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
    for (__NSCFTimer *t in loop()->timers) if ((!best || t->fire < best) && in_mode(t->modes, mode_or_default(mode))) best = t->fire;
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
    if (!src->modes) src->modes = [NSMutableSet new];
    mode = mode_or_default(mode);
    if ([src->modes containsObject:mode]) return;
    [src->modes addObject:mode];
    if ([loop()->sources indexOfObjectIdenticalTo:s] == NSNotFound) [loop()->sources addObject:s];
    if (src->ctx.schedule) src->ctx.schedule(src->ctx.info, loop(), mode);
}
void CFRunLoopRemoveSource(id rl, id s, id mode) {
    (void)rl;
    __NSCFRunLoopSource *src = s;
    mode = mode_or_default(mode);
    if (![src->modes containsObject:mode]) return;
    if (src->ctx.cancel) src->ctx.cancel(src->ctx.info, loop(), mode);
    [src->modes removeObject:mode];
    if (![src->modes count]) [loop()->sources removeObjectIdenticalTo:s];
}
Boolean CFRunLoopContainsSource(id rl, id s, id mode) { (void)rl; return in_mode(((__NSCFRunLoopSource *)s)->modes, mode_or_default(mode)); }
void CFRunLoopSourceSignal(id s) { ((__NSCFRunLoopSource *)s)->signaled = YES; }
void CFRunLoopSourceInvalidate(id s) {
    __NSCFRunLoopSource *src = s;
    src->valid = NO;
    for (id m in [src->modes allObjects]) CFRunLoopRemoveSource(loop(), s, m);
    [loop()->sources removeObjectIdenticalTo:s];
}
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
void CFRunLoopAddObserver(id rl, id o, id mode) {
    (void)rl;
    __NSCFRunLoopObserver *ob = o;
    if (!ob->modes) ob->modes = [NSMutableSet new];
    [ob->modes addObject:mode_or_default(mode)];
    if ([loop()->observers indexOfObjectIdenticalTo:o] == NSNotFound) [loop()->observers addObject:o];
}
void CFRunLoopRemoveObserver(id rl, id o, id mode) {
    (void)rl;
    __NSCFRunLoopObserver *ob = o;
    [ob->modes removeObject:mode_or_default(mode)];
    if (![ob->modes count]) [loop()->observers removeObjectIdenticalTo:o];
}
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
- (id)currentMode { return cf->mode; }
- (void)addTimer:(NSTimer *)t forMode:(id)mode { timer_schedule((__NSCFTimer *)t, mode); }
- (void)run { while (run_in_mode(NSDefaultRunLoopMode, 1e10, NO) != FINISHED) {} }
- (void)runUntilDate:(NSDate *)d {
    for (;;) {
        double left = [d timeIntervalSinceReferenceDate] - CFAbsoluteTimeGetCurrent();
        if (left <= 0 || run_in_mode(NSDefaultRunLoopMode, left, NO) == FINISHED) return;
    }
}
- (BOOL)runMode:(id)mode beforeDate:(NSDate *)d {
    int r = run_in_mode(mode, [d timeIntervalSinceReferenceDate] - CFAbsoluteTimeGetCurrent(), YES);
    return r != FINISHED;
}
- (id)limitDateForMode:(id)mode {
    double t = CFRunLoopGetNextTimerFireDate(cf, mode);
    return t ? [NSDate dateWithTimeIntervalSinceReferenceDate:t] : nil;
}
- (void)acceptInputForMode:(id)mode beforeDate:(NSDate *)d { [self runMode:mode beforeDate:d]; }
- (void)performBlock:(void (^)(void))b { CFRunLoopPerformBlock(cf, kCFRunLoopCommonModes, b); }
- (void)performInModes:(id)modes block:(void (^)(void))b { CFRunLoopPerformBlock(cf, modes, b); }
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
- (void)performSelector:(SEL)s withObject:(id)arg afterDelay:(double)delay inModes:(id)modes {
    __NSDelayedPerform *p = [[__NSDelayedPerform new] autorelease];
    p->target = [self retain];
    p->arg = [arg retain];
    p->sel = s;
    id t = [NSTimer timerWithTimeInterval:delay target:p selector:@selector(fire:) userInfo:nil repeats:NO];
    for (id m in modes) timer_schedule((__NSCFTimer *)t, m);
}
- (void)performSelectorOnMainThread:(SEL)s withObject:(id)arg waitUntilDone:(BOOL)wait {
    if (wait) { ((void (*)(id, SEL, id))objc_msgSend)(self, s, arg); return; }
    [self performSelector:s withObject:arg afterDelay:0 inModes:[NSArray arrayWithObject:kCFRunLoopCommonModes]];
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
