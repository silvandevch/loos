/* NSNotificationCenter / NSNotification and CFNotificationCenter's local center. Observers are
 * held weakly (as on macOS since 10.11), so one that is deallocated simply stops being called;
 * delivery is synchronous on the posting thread. */
#include "foundation.h"

id objc_loadWeakRetained(id *);
id objc_initWeak(id *, id);
void objc_destroyWeak(id *);

@interface NSNotification : NSObject { @public id _name, _object, _info; }
@end
@implementation NSNotification
+ (id)notificationWithName:(id)n object:(id)o { return [self notificationWithName:n object:o userInfo:nil]; }
+ (id)notificationWithName:(id)n object:(id)o userInfo:(id)u {
    NSNotification *x = [[self alloc] init];
    x->_name = [n copy];
    x->_object = [o retain];
    x->_info = [u copy];
    return [x autorelease];
}
- (id)initWithName:(id)n object:(id)o userInfo:(id)u {
    _name = [n copy];
    _object = [o retain];
    _info = [u copy];
    return self;
}
- (id)name { return _name; }
- (id)object { return _object; }
- (id)userInfo { return _info; }
- (id)copyWithZone:(NSZone *)z { (void)z; return [self retain]; }
- (id)description { return [NSString stringWithFormat:CFSTR("NSConcreteNotification %p {name = %@; object = %@}"), self, _name, _object]; }
- (void)dealloc { [_name release]; [_object release]; [_info release]; [super dealloc]; }
@end

/* One registration: a target + selector (weak) or a block (owned by its token), filtered by
 * name and sender (nil matches any). */
@interface __NSObserver : NSObject {
@public
    id _target; /* weak */
    SEL _sel;
    void (^_block)(id);
    id _name, _sender; /* the sender is compared by identity, not retained */
    void (*_cf)(id, void *, id, const void *, id);
    void *_cfobserver;
    BOOL _removed;
}
@end
@implementation __NSObserver
- (void)dealloc {
    objc_destroyWeak(&_target);
    [_block release];
    [_name release];
    [super dealloc];
}
@end

static NSNotificationCenter *default_center;

@implementation NSNotificationCenter
+ (id)defaultCenter {
    if (!default_center) default_center = [NSNotificationCenter new];
    return default_center;
}
- (id)init { _observers = [NSMutableArray new]; return self; }
static void add(NSNotificationCenter *c, __NSObserver *o, id name, id sender) {
    o->_name = [name copy];
    o->_sender = sender;
    [c->_observers addObject:o];
}
- (void)addObserver:(id)target selector:(SEL)sel name:(id)name object:(id)sender {
    __NSObserver *o = [[__NSObserver new] autorelease];
    objc_initWeak(&o->_target, target);
    o->_sel = sel;
    add(self, o, name, sender);
}
- (id)addObserverForName:(id)name object:(id)sender queue:(id)queue usingBlock:(void (^)(id))block {
    (void)queue; /* delivered on the posting thread: Prism has one thread */
    __NSObserver *o = [[__NSObserver new] autorelease];
    o->_block = [block copy];
    add(self, o, name, sender);
    return o; /* the token: removeObserver: with it unregisters */
}
static void remove_matching(NSNotificationCenter *c, id target, id name, id sender) {
    for (NSUInteger i = [c->_observers count]; i-- > 0;) {
        __NSObserver *o = [c->_observers objectAtIndex:i];
        id t = objc_loadWeakRetained(&o->_target);
        BOOL match = (o == target || (t && t == target) || (o->_cfobserver && o->_cfobserver == (void *)target)) &&
                     (!name || [o->_name isEqual:name]) && (!sender || o->_sender == sender);
        [t release];
        if (match) {
            o->_removed = YES;
            [c->_observers removeObjectAtIndex:i];
        }
    }
}
- (void)removeObserver:(id)target { remove_matching(self, target, nil, nil); }
- (void)removeObserver:(id)target name:(id)name object:(id)sender { remove_matching(self, target, name, sender); }
- (void)postNotification:(NSNotification *)n {
    /* a snapshot, so observers may add or remove registrations while being called */
    id snapshot = [[_observers copy] autorelease];
    for (__NSObserver *o in snapshot) {
        if (o->_removed) continue;
        if (o->_name && ![o->_name isEqual:n->_name]) continue;
        if (o->_sender && o->_sender != n->_object) continue;
        if (o->_block) {
            o->_block(n);
        } else if (o->_cf) {
            o->_cf(self, o->_cfobserver, n->_name, n->_object, n->_info);
        } else {
            id t = objc_loadWeakRetained(&o->_target);
            if (t) ((void (*)(id, SEL, id))objc_msgSend)(t, o->_sel, n);
            else { o->_removed = YES; [_observers removeObjectIdenticalTo:o]; }
            [t release];
        }
    }
}
- (void)postNotificationName:(id)name object:(id)sender { [self postNotification:[NSNotification notificationWithName:name object:sender userInfo:nil]]; }
- (void)postNotificationName:(id)name object:(id)sender userInfo:(id)info { [self postNotification:[NSNotification notificationWithName:name object:sender userInfo:info]]; }
@end

/* NSNotificationQueue: posting "when idle" or "as soon as possible" is the next run-loop pass. */
@interface NSNotificationQueue : NSObject @end
@implementation NSNotificationQueue
+ (id)defaultQueue { static id q; if (!q) q = [NSNotificationQueue new]; return q; }
- (void)enqueueNotification:(id)n postingStyle:(NSUInteger)style {
    /* NSPostWhenIdle 1, NSPostASAP 2, NSPostNow 3 */
    if (style == 3) { [[NSNotificationCenter defaultCenter] postNotification:n]; return; }
    extern void CFRunLoopPerformBlock(id, id, void (^)(void));
    extern id CFRunLoopGetCurrent(void);
    id held = [n retain];
    CFRunLoopPerformBlock(CFRunLoopGetCurrent(), nil, ^{
        [[NSNotificationCenter defaultCenter] postNotification:held];
        [held release];
    });
}
- (void)enqueueNotification:(id)n postingStyle:(NSUInteger)s coalesceMask:(NSUInteger)m forModes:(id)modes {
    (void)m; (void)modes;
    [self enqueueNotification:n postingStyle:s];
}
@end

/* ------------------------------------------------------------------ CFNotificationCenter */

typedef void (*CFNotificationCallback)(id center, void *observer, id name, const void *object, id userInfo);
static NSNotificationCenter *local_center;
id CFNotificationCenterGetLocalCenter(void) { return [NSNotificationCenter defaultCenter]; }
/* No other processes to talk to: the distributed and Darwin centers deliver within this one. */
id CFNotificationCenterGetDistributedCenter(void) {
    if (!local_center) local_center = [NSNotificationCenter new];
    return local_center;
}
id CFNotificationCenterGetDarwinNotifyCenter(void) { return CFNotificationCenterGetDistributedCenter(); }
void CFNotificationCenterAddObserver(id center, const void *observer, CFNotificationCallback cb, id name, const void *object, CFIndex behavior) {
    (void)behavior;
    __NSObserver *o = [[__NSObserver new] autorelease];
    o->_cf = cb;
    o->_cfobserver = (void *)observer;
    add(center, o, name, (id)object);
}
void CFNotificationCenterRemoveObserver(id center, const void *observer, id name, const void *object) { remove_matching(center, (id)observer, name, (id)object); }
void CFNotificationCenterRemoveEveryObserver(id center, const void *observer) { remove_matching(center, (id)observer, nil, nil); }
void CFNotificationCenterPostNotification(id center, id name, const void *object, id info, Boolean immediate) {
    (void)immediate;
    [center postNotificationName:name object:(id)object userInfo:info];
}
void CFNotificationCenterPostNotificationWithOptions(id center, id name, const void *object, id info, unsigned long opts) {
    (void)opts;
    [center postNotificationName:name object:(id)object userInfo:info];
}
