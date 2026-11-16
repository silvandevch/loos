/* Foundation and the rest of CoreFoundation through Prism's runtime (ARC): NSException,
 * NSSet/CFSet, NSDate, CFRunLoop/NSRunLoop/NSTimer, NSURL/CFURL, paths, property lists and
 * NSBundle. Run as Found.app/Contents/MacOS/found. No SDK: the interfaces are declared here. */
int printf(const char *, ...);
typedef unsigned long NSUInteger;
typedef long NSInteger, CFIndex;
#if __arm64__
typedef _Bool BOOL;
#else
typedef signed char BOOL;
#endif
typedef unsigned char Boolean;
#define nil 0
#define YES __objc_yes
#define NO __objc_no
typedef const void *CFTypeRef;
typedef double CFAbsoluteTime;
#define CFSTR(s) ((CFTypeRef)__builtin___CFStringMakeConstantString(s))
extern const void *kCFTypeSetCallBacks;
extern id const NSRangeException, NSDefaultRunLoopMode;
extern CFTypeRef kCFRunLoopDefaultMode;
const char *object_getClassName(id);
CFTypeRef CFSetCreateMutable(const void *, CFIndex, const void *);
void CFSetAddValue(CFTypeRef, const void *);
Boolean CFSetContainsValue(CFTypeRef, const void *);
CFIndex CFSetGetCount(CFTypeRef);
void CFRelease(CFTypeRef);
CFAbsoluteTime CFAbsoluteTimeGetCurrent(void);
CFTypeRef CFRunLoopGetCurrent(void);
int CFRunLoopRunInMode(CFTypeRef, double, Boolean);
void CFRunLoopStop(CFTypeRef);
typedef struct { CFIndex version; void *info; const void *r, *rl, *d; } TimerCtx;
CFTypeRef CFRunLoopTimerCreate(const void *, CFAbsoluteTime, double, unsigned long, CFIndex, void (*)(CFTypeRef, void *), TimerCtx *);
void CFRunLoopAddTimer(CFTypeRef, CFTypeRef, CFTypeRef);
void CFRunLoopTimerInvalidate(CFTypeRef);
typedef struct { CFIndex version; void *info; const void *r, *rl, *d, *eq, *h, *sched, *cancel; void (*perform)(void *); } SourceCtx;
CFTypeRef CFRunLoopSourceCreate(const void *, CFIndex, SourceCtx *);
void CFRunLoopAddSource(CFTypeRef, CFTypeRef, CFTypeRef);
void CFRunLoopSourceSignal(CFTypeRef);
void CFRunLoopSourceInvalidate(CFTypeRef);
CFTypeRef CFRunLoopObserverCreateWithHandler(const void *, unsigned long, Boolean, CFIndex, void (^)(CFTypeRef, unsigned long));
void CFRunLoopAddObserver(CFTypeRef, CFTypeRef, CFTypeRef);
CFTypeRef CFURLCreateWithString(const void *, CFTypeRef, CFTypeRef);
CFTypeRef CFURLCopyHostName(CFTypeRef);
int CFURLGetPortNumber(CFTypeRef);
CFTypeRef CFBundleGetMainBundle(void);
CFTypeRef CFBundleGetIdentifier(CFTypeRef);
CFTypeRef CFBundleGetValueForInfoDictionaryKey(CFTypeRef, CFTypeRef);
CFTypeRef CFPropertyListCreateWithData(const void *, CFTypeRef, unsigned long, CFIndex *, void *);

@class NSArray, NSString, NSData, NSDate, NSSet, NSURL, NSNumber;
__attribute__((objc_root_class))
@interface NSObject { Class isa; }
+ (instancetype)alloc;
+ (instancetype)new;
- (instancetype)init;
- (id)description;
- (BOOL)isEqual:(id)o;
- (void)performSelector:(SEL)s withObject:(id)o afterDelay:(double)d;
- (id)mutableCopy;
@end
@interface NSString : NSObject
+ (instancetype)stringWithFormat:(NSString *)f, ...;
+ (instancetype)stringWithContentsOfFile:(NSString *)p encoding:(NSUInteger)e error:(id *)err;
- (const char *)UTF8String;
- (NSUInteger)length;
- (NSString *)substringFromIndex:(NSUInteger)i;
- (NSString *)lastPathComponent;
- (NSString *)pathExtension;
- (NSString *)stringByDeletingLastPathComponent;
- (NSString *)stringByAppendingPathComponent:(NSString *)c;
- (NSString *)stringByDeletingPathExtension;
- (NSString *)stringByStandardizingPath;
- (NSArray *)pathComponents;
- (NSString *)stringByRemovingPercentEncoding;
- (BOOL)writeToFile:(NSString *)p atomically:(BOOL)a encoding:(NSUInteger)e error:(id *)err;
- (BOOL)hasSuffix:(NSString *)s;
@end
@interface NSNumber : NSObject
- (int)intValue;
@end
@interface NSArray : NSObject
- (NSUInteger)count;
- (id)objectAtIndex:(NSUInteger)i;
- (id)objectAtIndexedSubscript:(NSUInteger)i;
- (NSString *)componentsJoinedByString:(NSString *)s;
- (NSArray *)sortedArrayUsingSelector:(SEL)s;
@end
@interface NSArray (Literals)
+ (instancetype)arrayWithObjects:(const id[])o count:(NSUInteger)n;
@end
@interface NSDictionary : NSObject
- (id)objectForKeyedSubscript:(id)k;
- (NSUInteger)count;
+ (instancetype)dictionaryWithObjects:(const id[])o forKeys:(const id[])k count:(NSUInteger)n;
+ (instancetype)dictionaryWithContentsOfFile:(NSString *)p;
- (BOOL)writeToFile:(NSString *)p atomically:(BOOL)a;
@end
@interface NSMutableDictionary : NSDictionary
- (void)setObject:(id)o forKeyedSubscript:(id)k;
@end
@interface NSData : NSObject
- (NSUInteger)length;
+ (instancetype)dataWithBytes:(const void *)b length:(NSUInteger)n;
@end
@interface NSSet : NSObject
+ (instancetype)setWithArray:(NSArray *)a;
- (NSUInteger)count;
- (BOOL)containsObject:(id)o;
- (NSArray *)allObjects;
- (BOOL)isSubsetOfSet:(NSSet *)s;
- (NSUInteger)countByEnumeratingWithState:(void *)st objects:(id __unsafe_unretained *)b count:(NSUInteger)n;
@end
@interface NSMutableSet : NSSet
- (void)addObject:(id)o;
- (void)removeObject:(id)o;
- (void)intersectSet:(NSSet *)s;
@end
@interface NSDate : NSObject
+ (instancetype)dateWithTimeIntervalSinceReferenceDate:(double)t;
+ (instancetype)dateWithTimeIntervalSinceNow:(double)t;
- (double)timeIntervalSinceReferenceDate;
- (NSDate *)dateByAddingTimeInterval:(double)t;
- (NSInteger)compare:(NSDate *)o;
@end
@interface NSException : NSObject
+ (void)raise:(NSString *)name format:(NSString *)f, ...;
- (NSString *)name;
- (NSString *)reason;
@end
@interface NSTimer : NSObject
+ (NSTimer *)scheduledTimerWithTimeInterval:(double)s repeats:(BOOL)r block:(void (^)(NSTimer *))b;
- (void)invalidate;
@end
@interface NSRunLoop : NSObject
+ (NSRunLoop *)currentRunLoop;
- (void)runUntilDate:(NSDate *)d;
- (void)run;
@end
@interface NSURL : NSObject
+ (instancetype)URLWithString:(NSString *)s;
+ (instancetype)URLWithString:(NSString *)s relativeToURL:(NSURL *)b;
+ (instancetype)fileURLWithPath:(NSString *)p;
- (NSString *)absoluteString;
- (NSString *)scheme;
- (NSString *)host;
- (NSNumber *)port;
- (NSString *)path;
- (NSString *)query;
- (NSString *)fragment;
- (NSString *)lastPathComponent;
- (NSURL *)URLByAppendingPathComponent:(NSString *)c;
- (NSURL *)URLByDeletingLastPathComponent;
- (BOOL)isFileURL;
@end
@interface NSPropertyListSerialization : NSObject
+ (NSData *)dataWithPropertyList:(id)p format:(NSUInteger)f options:(NSUInteger)o error:(id *)e;
+ (id)propertyListWithData:(NSData *)d options:(NSUInteger)o format:(NSUInteger *)f error:(id *)e;
@end
@interface NSBundle : NSObject
+ (NSBundle *)mainBundle;
- (NSString *)bundlePath;
- (NSString *)bundleIdentifier;
- (NSString *)pathForResource:(NSString *)n ofType:(NSString *)t;
- (id)objectForInfoDictionaryKey:(NSString *)k;
- (NSString *)localizedStringForKey:(NSString *)k value:(NSString *)v table:(NSString *)t;
@end
@interface NSNumber (Literals)
+ (NSNumber *)numberWithInt:(int)v;
+ (NSNumber *)numberWithDouble:(double)v;
+ (NSNumber *)numberWithBool:(BOOL)v;
@end

static const char *c(id o) { return [[o description] UTF8String]; }

@interface Ticker : NSObject
@end
@implementation Ticker
- (void)ping:(NSString *)what { printf("delayed: %s\n", [what UTF8String]); }
@end

static int timer_hits, source_hits;
static void on_timer(CFTypeRef t, void *info) {
    (void)info;
    if (++timer_hits == 2) CFRunLoopTimerInvalidate(t);
}
static void on_source(void *info) { source_hits += *(int *)info; }

int main(void) {
    @autoreleasepool {
        /* Exceptions: Foundation's own and a program's. */
        NSArray *a = @[ @1, @2, @3 ];
        NSString *caught = nil;
        @try {
            (void)a[7];
        } @catch (NSException *e) {
            caught = [NSString stringWithFormat:@"%@: %@", [e name], [e reason]];
        }
        NSString *custom = nil;
        @try {
            [NSException raise:@"MyError" format:@"code %d", 42];
        } @catch (NSException *e) {
            custom = [e reason];
        } @finally {
            custom = [NSString stringWithFormat:@"%@!", custom];
        }
        NSString *sel = nil;
        @try {
            [(id)@"str" performSelector:@selector(count) withObject:nil afterDelay:-1];
            [(NSArray *)(id)@"str" count];
        } @catch (NSException *e) {
            sel = [[e reason] substringFromIndex:0];
        }
        printf("exc: %s | %s | %d\n", [caught UTF8String], [custom UTF8String], sel != nil);

        /* Sets. */
        NSMutableSet *s = [[NSSet setWithArray:@[ @"a", @"b", @"c", @"a" ]] mutableCopy];
        [s addObject:@"d"];
        [s removeObject:@"b"];
        [s intersectSet:[NSSet setWithArray:@[ @"a", @"c", @"d", @"z" ]]];
        int n = 0;
        for (id x in s) n += (int)[x length];
        CFTypeRef cs = CFSetCreateMutable(0, 0, &kCFTypeSetCallBacks);
        CFSetAddValue(cs, CFSTR("x"));
        CFSetAddValue(cs, CFSTR("x"));
        printf("set: count=%lu has=%d n=%d sorted=%s sub=%d cf=%ld/%d\n", [s count], [s containsObject:@"c"], n,
               [[[[s allObjects] sortedArrayUsingSelector:@selector(compare:)] componentsJoinedByString:@","] UTF8String],
               [[NSSet setWithArray:@[ @"a" ]] isSubsetOfSet:s], CFSetGetCount(cs), CFSetContainsValue(cs, CFSTR("x")));
        CFRelease(cs);

        /* Dates. */
        NSDate *d0 = [NSDate dateWithTimeIntervalSinceReferenceDate:0];
        printf("date: %s %s cmp=%ld\n", c(d0), c([d0 dateByAddingTimeInterval:86400 * 365.25 * 25]), [d0 compare:[NSDate dateWithTimeIntervalSinceNow:0]]);

        /* Run loop: an NSTimer, a CF timer, a source, an observer, a delayed message. */
        __block int ticks = 0;
        [NSTimer scheduledTimerWithTimeInterval:0.05 repeats:YES block:^(NSTimer *t) {
            if (++ticks == 3) [t invalidate];
        }];
        CFTypeRef rl = CFRunLoopGetCurrent();
        CFTypeRef ct = CFRunLoopTimerCreate(0, CFAbsoluteTimeGetCurrent() + 0.02, 0.02, 0, 0, on_timer, 0);
        CFRunLoopAddTimer(rl, ct, kCFRunLoopDefaultMode);
        static int weight = 5;
        SourceCtx sc = {0, &weight, 0, 0, 0, 0, 0, 0, 0, on_source};
        CFTypeRef src = CFRunLoopSourceCreate(0, 0, &sc);
        CFRunLoopAddSource(rl, src, kCFRunLoopDefaultMode);
        CFRunLoopSourceSignal(src);
        __block int entries = 0;
        CFTypeRef obs = CFRunLoopObserverCreateWithHandler(0, 1, YES, 0, ^(CFTypeRef o, unsigned long act) { (void)o; (void)act; entries++; });
        CFRunLoopAddObserver(rl, obs, kCFRunLoopDefaultMode);
        [[Ticker new] performSelector:@selector(ping:) withObject:@"hi" afterDelay:0.01];
        double t0 = CFAbsoluteTimeGetCurrent();
        int first = CFRunLoopRunInMode(kCFRunLoopDefaultMode, 10, YES); /* the signaled source: handled */
        CFRunLoopSourceInvalidate(src);
        [[NSRunLoop currentRunLoop] run]; /* until no timers are left */
        double took = CFAbsoluteTimeGetCurrent() - t0;
        int timeout = CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, NO); /* nothing scheduled: finished */
        printf("runloop: first=%d ticks=%d cf=%d source=%d entries>=%d took=%d finished=%d\n", first, ticks, timer_hits, source_hits,
               entries >= 2, took >= 0.14 && took < 2, timeout);
        CFRelease(ct);
        CFRelease(src);
        CFRelease(obs);

        /* URLs and paths. */
        NSURL *base = [NSURL URLWithString:@"https://user@example.com:8443/docs/a/index.html?q=1#top"];
        NSURL *rel = [NSURL URLWithString:@"../img/logo%20big.png" relativeToURL:base];
        CFTypeRef cu = CFURLCreateWithString(0, CFSTR("ftp://files.example.org/pub/"), 0);
        CFTypeRef host = CFURLCopyHostName(cu);
        printf("url: %s %s %d %s %s %s | %s %s | %s %d\n", [[base scheme] UTF8String], [[base host] UTF8String], [[base port] intValue],
               [[base path] UTF8String], [[base query] UTF8String], [[base fragment] UTF8String], [[rel absoluteString] UTF8String],
               [[rel lastPathComponent] UTF8String], c((__bridge id)host), CFURLGetPortNumber(cu));
        CFRelease(host);
        CFRelease(cu);
        NSURL *f = [NSURL fileURLWithPath:@"/tmp/some dir/file.txt"];
        printf("file: %s %s %d %s %s\n", [[f absoluteString] UTF8String], [[f path] UTF8String], [f isFileURL],
               [[[f URLByDeletingLastPathComponent] absoluteString] UTF8String], [[[f URLByAppendingPathComponent:@"x"] path] UTF8String]);
        NSString *p = @"/usr/local/../lib//libfoo.1.dylib";
        printf("path: %s %s %s %s %s %s %s\n", [[p lastPathComponent] UTF8String], [[p pathExtension] UTF8String],
               [[p stringByDeletingLastPathComponent] UTF8String], [[p stringByStandardizingPath] UTF8String],
               [[[p lastPathComponent] stringByDeletingPathExtension] UTF8String], [[@"a" stringByAppendingPathComponent:@"b"] UTF8String],
               [[[@"/x/y/" pathComponents] componentsJoinedByString:@"|"] UTF8String]);

        /* Property lists: XML and binary round trips, files. */
        NSDictionary *pl = @{
            @"name" : @"Prism",
            @"version" : @3,
            @"ratio" : @0.5,
            @"on" : @YES,
            @"list" : @[ @"x", @1 ],
            @"blob" : [NSData dataWithBytes:"\x01\x02\xff" length:3],
            @"when" : [NSDate dateWithTimeIntervalSinceReferenceDate:3600],
        };
        NSData *xml = [NSPropertyListSerialization dataWithPropertyList:pl format:100 options:0 error:nil];
        NSData *bin = [NSPropertyListSerialization dataWithPropertyList:pl format:200 options:0 error:nil];
        NSUInteger fx = 0, fb = 0;
        id px = [NSPropertyListSerialization propertyListWithData:xml options:0 format:&fx error:nil];
        id pb = [NSPropertyListSerialization propertyListWithData:bin options:0 format:&fb error:nil];
        [pl writeToFile:@"/tmp/prism.plist" atomically:YES];
        NSDictionary *back = [NSDictionary dictionaryWithContentsOfFile:@"/tmp/prism.plist"];
        CFIndex sf = 0;
        CFTypeRef step = CFPropertyListCreateWithData(0, (__bridge CFTypeRef)[NSData dataWithBytes:"{ a = (1, 2); b = \"two words\"; }" length:31], 0, &sf, 0);
        printf("plist: xml=%d/%lu bin=%d/%lu file=%d %s %s on=%s step=%ld/%lu\n", [px isEqual:pl], fx, [pb isEqual:pl], fb, [back isEqual:pl],
               c(back[@"when"]), c(back[@"blob"]), c(back[@"on"]), sf, [(__bridge NSDictionary *)step count]);
        CFRelease(step);

        /* The main bundle: Found.app. */
        NSBundle *b = [NSBundle mainBundle];
        NSString *res = [b pathForResource:@"greeting" ofType:@"txt"];
        CFTypeRef mb = CFBundleGetMainBundle();
        printf("bundle: %s %s %s %s \"%s\" %s %s\n", [[[b bundlePath] lastPathComponent] UTF8String], [[b bundleIdentifier] UTF8String],
               c([b objectForInfoDictionaryKey:@"CFBundleShortVersionString"]), [[res lastPathComponent] UTF8String],
               [[NSString stringWithContentsOfFile:res encoding:4 error:nil] UTF8String], [[b localizedStringForKey:@"hello" value:nil table:nil] UTF8String],
               c((__bridge id)CFBundleGetValueForInfoDictionaryKey(mb, CFSTR("CFBundleName"))));
        (void)CFBundleGetIdentifier(mb);
    }
    /* Uncaught: Foundation's message, then the abort. */
    [NSException raise:@"Custom" format:@"bye %d", 3];
    return 0;
}
