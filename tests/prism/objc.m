/* Objective-C through Prism's runtime (ARC): classes, ivars and properties, inheritance and super,
 * categories, +load/+initialize, protocols, dealloc, weak references, autorelease pools, blocks,
 * dispatch_once, forwarding and dynamic methods. No SDK: the interfaces are declared here. */
int printf(const char *, ...);
typedef unsigned long NSUInteger;
#if __arm64__
typedef _Bool BOOL;
#else
typedef signed char BOOL;
#endif
#define nil ((id)0)
typedef struct objc_class *Class_;
id objc_getClass(const char *);
const char *object_getClassName(id);
void *objc_getProtocol(const char *);
void objc_setAssociatedObject(id, const void *, id, unsigned long);
id objc_getAssociatedObject(id, const void *);
BOOL class_addMethod(Class, SEL, void *, const char *);
typedef long dispatch_once_t;
void dispatch_once(dispatch_once_t *, void (^)(void));

__attribute__((objc_root_class))
@interface NSObject { Class isa; }
+ (instancetype)alloc;
+ (instancetype)new;
+ (Class)class;
- (instancetype)init;
- (Class)class;
- (BOOL)isKindOfClass:(Class)c;
- (BOOL)respondsToSelector:(SEL)s;
- (BOOL)conformsToProtocol:(id)p;
- (id)performSelector:(SEL)s;
- (id)copy;
@end

static int log_dealloc;
@protocol Shape
- (int)area;
@end

@interface Rect : NSObject <Shape>
@property int w, h;
@property (copy) NSObject *tag;
- (instancetype)initW:(int)w h:(int)h;
@end
@implementation Rect
+ (void)load { printf("load: Rect\n"); }
+ (void)initialize { if (self == [Rect class]) printf("initialize: Rect\n"); }
- (instancetype)initW:(int)w h:(int)h { if ((self = [super init])) { _w = w; _h = h; } return self; }
- (int)area { return _w * _h; }
- (void)dealloc { log_dealloc++; }
@end

@interface Square : Rect
- (instancetype)initSide:(int)s;
@end
@implementation Square
- (instancetype)initSide:(int)s { return [super initW:s h:s]; }
- (int)area { return [super area] + 0; }
@end

@interface Rect (Describe)
- (int)perimeter;
@end
@implementation Rect (Describe)
- (int)perimeter { return 2 * (self.w + self.h); }
@end

@interface Helper : NSObject
- (int)helped;
@end
@implementation Helper
- (int)helped { return 42; }
@end
@interface Forwarder : NSObject @end
@implementation Forwarder
- (id)forwardingTargetForSelector:(SEL)s { (void)s; return [Helper new]; }
@end

@interface Oops : NSObject
@property int code;
@end
@implementation Oops @end
@interface Other : NSObject @end
@implementation Other @end
static int cleanups;
@interface Holder : NSObject @end
@implementation Holder
- (void)dealloc { cleanups++; }
@end
static void thrower(int code) {
    Holder *h = [Holder new]; /* released by the unwinder's cleanup */
    (void)h;
    Oops *o = [Oops new];
    o.code = code;
    @throw o;
}
static int middle(int code) {
    @try { thrower(code); }
    @catch (Other *x) { return -1; }
    @finally { cleanups += 10; }
    return 0;
}

static int dyn(id self, SEL _cmd) { (void)self; (void)_cmd; return 7; }
@interface Dynamic : NSObject @end
@implementation Dynamic
+ (BOOL)resolveInstanceMethod:(SEL)s { return class_addMethod(self, s, (void *)dyn, "i@:"); }
@end

int main(void) {
    int area, sq, per, weak_gone, kinds, blocks, fwd, dynv, assoc;
    __block int once = 0;
    @autoreleasepool {
        Rect *r = [[Rect alloc] initW:3 h:4];
        Square *s = [[Square alloc] initSide:5];
        area = [r area];
        sq = s.area;
        per = [s perimeter];
        kinds = [s isKindOfClass:[Rect class]] * 100 + [r isKindOfClass:[Square class]] * 10 + [s conformsToProtocol:(__bridge id)objc_getProtocol("Shape")];
        __weak Rect *w = r;
        r = nil;
        weak_gone = w == nil;
        __block int counter = 0;
        int (^add)(int) = ^(int k) { counter += k; return counter + s.w; };
        add(1);
        int (^heap)(int) = [add copy];
        blocks = heap(2) * 10 + counter;
        static dispatch_once_t pred;
        for (int i = 0; i < 3; i++) dispatch_once(&pred, ^{ once++; });
        fwd = (int)(long)[(id)[Forwarder new] performSelector:@selector(helped)];
        dynv = (int)(long)[(id)[Dynamic new] performSelector:@selector(anything)];
        static char key;
        objc_setAssociatedObject(s, &key, [Helper new], 1);
        assoc = objc_getAssociatedObject(s, &key) != nil;
    }
    int caught = 0, rethrown = 0;
    @try { middle(5); } @catch (Oops *o) { caught = o.code; }
    @try {
        @try { thrower(9); } @catch (id e) { @throw; }
    } @catch (Oops *o) { rethrown = o.code; }
    printf("exc: caught=%d rethrown=%d cleanups=%d\n", caught, rethrown, cleanups);
    printf("objc: area=%d sq=%d per=%d kinds=%d weak=%d blocks=%d once=%d fwd=%d dyn=%d assoc=%d dealloc=%d class=%s\n",
           area, sq, per, kinds, weak_gone, blocks, once, fwd, dynv, assoc, log_dealloc, object_getClassName(objc_getClass("Square")));
    return 0;
}
