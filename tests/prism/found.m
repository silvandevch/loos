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
#define nil ((void *)0)
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

@class NSNotification, NSCoder, NSDictionary, NSArray, NSString, NSData, NSDate, NSSet, NSURL, NSNumber;
__attribute__((objc_root_class))
@interface NSObject { Class isa; }
+ (instancetype)alloc;
+ (instancetype)new;
+ (Class)class;
- (instancetype)init;
- (id)description;
- (BOOL)isEqual:(id)o;
- (void)performSelector:(SEL)s withObject:(id)o afterDelay:(double)d;
- (id)mutableCopy;
@end
@interface NSString : NSObject
+ (instancetype)stringWithFormat:(NSString *)f, ...;
- (instancetype)initWithBytes:(const void *)b length:(NSUInteger)n encoding:(NSUInteger)e;
- (NSArray *)componentsSeparatedByString:(NSString *)s;
- (NSString *)stringByAppendingString:(NSString *)s;
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
typedef struct { NSUInteger location, length; } NSRange;
@interface NSCharacterSet : NSObject
+ (id)whitespaceAndNewlineCharacterSet;
+ (id)decimalDigitCharacterSet;
+ (id)letterCharacterSet;
+ (id)punctuationCharacterSet;
+ (id)URLQueryAllowedCharacterSet;
+ (id)characterSetWithCharactersInString:(id)s;
- (BOOL)characterIsMember:(unsigned short)c;
- (id)invertedSet;
@end
@interface NSMutableCharacterSet : NSCharacterSet
- (void)addCharactersInString:(id)s;
- (void)formUnionWithCharacterSet:(id)o;
@end
@interface NSScanner : NSObject
+ (id)scannerWithString:(id)s;
- (BOOL)scanInt:(int *)v;
- (BOOL)scanDouble:(double *)v;
- (BOOL)scanHexInt:(unsigned *)v;
- (BOOL)scanString:(id)s intoString:(id *)o;
- (BOOL)scanUpToString:(id)s intoString:(id *)o;
- (BOOL)scanCharactersFromSet:(id)s intoString:(id *)o;
- (BOOL)scanUpToCharactersFromSet:(id)s intoString:(id *)o;
- (BOOL)isAtEnd;
- (NSUInteger)scanLocation;
@end
@interface NSString (CharSets)
- (id)stringByTrimmingCharactersInSet:(id)s;
- (id)componentsSeparatedByCharactersInSet:(id)s;
- (NSRange)rangeOfCharacterFromSet:(id)s;
- (id)stringByAddingPercentEncodingWithAllowedCharacters:(id)s;
@end
@interface NSNumber : NSObject
- (int)intValue;
@end
@interface NSArray : NSObject
- (NSUInteger)count;
- (id)objectAtIndex:(NSUInteger)i;
- (id)objectAtIndexedSubscript:(NSUInteger)i;
- (NSString *)componentsJoinedByString:(NSString *)s;
- (id)firstObject;
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
- (const void *)bytes;
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
- (double)timeIntervalSinceNow;
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
@interface NSFileManager : NSObject
+ (NSFileManager *)defaultManager;
- (BOOL)createDirectoryAtPath:(NSString *)p withIntermediateDirectories:(BOOL)i attributes:(NSDictionary *)a error:(id *)e;
- (BOOL)createFileAtPath:(NSString *)p contents:(NSData *)d attributes:(NSDictionary *)a;
- (NSArray *)contentsOfDirectoryAtPath:(NSString *)p error:(id *)e;
- (NSArray *)subpathsOfDirectoryAtPath:(NSString *)p error:(id *)e;
- (NSDictionary *)attributesOfItemAtPath:(NSString *)p error:(id *)e;
- (BOOL)copyItemAtPath:(NSString *)a toPath:(NSString *)b error:(id *)e;
- (BOOL)moveItemAtPath:(NSString *)a toPath:(NSString *)b error:(id *)e;
- (BOOL)removeItemAtPath:(NSString *)p error:(id *)e;
- (BOOL)fileExistsAtPath:(NSString *)p isDirectory:(BOOL *)d;
- (BOOL)fileExistsAtPath:(NSString *)p;
- (NSString *)currentDirectoryPath;
@end
@interface NSError : NSObject
- (NSInteger)code;
- (NSString *)domain;
@end
typedef struct { NSInteger majorVersion, minorVersion, patchVersion; } NSOperatingSystemVersion;
@interface NSProcessInfo : NSObject
+ (NSProcessInfo *)processInfo;
- (NSString *)processName;
- (NSArray *)arguments;
- (NSDictionary *)environment;
- (int)processIdentifier;
- (NSOperatingSystemVersion)operatingSystemVersion;
- (BOOL)isOperatingSystemAtLeastVersion:(NSOperatingSystemVersion)v;
@end
@interface NSUUID : NSObject
+ (instancetype)UUID;
- (instancetype)initWithUUIDString:(NSString *)s;
- (NSString *)UUIDString;
@end
@interface NSUserDefaults : NSObject
+ (NSUserDefaults *)standardUserDefaults;
- (instancetype)initWithSuiteName:(NSString *)s;
- (void)setObject:(id)o forKey:(NSString *)k;
- (void)setInteger:(NSInteger)v forKey:(NSString *)k;
- (void)setBool:(BOOL)v forKey:(NSString *)k;
- (id)objectForKey:(NSString *)k;
- (NSInteger)integerForKey:(NSString *)k;
- (BOOL)boolForKey:(NSString *)k;
- (NSString *)stringForKey:(NSString *)k;
- (void)registerDefaults:(NSDictionary *)d;
- (void)removeObjectForKey:(NSString *)k;
@end
@interface NSNotification : NSObject
- (NSString *)name;
- (id)object;
- (NSDictionary *)userInfo;
@end
@interface NSNotificationCenter : NSObject
+ (NSNotificationCenter *)defaultCenter;
- (void)addObserver:(id)o selector:(SEL)s name:(NSString *)n object:(id)obj;
- (id)addObserverForName:(NSString *)n object:(id)o queue:(id)q usingBlock:(void (^)(NSNotification *))b;
- (void)removeObserver:(id)o;
- (void)postNotificationName:(NSString *)n object:(id)o userInfo:(NSDictionary *)u;
@end
@interface NSRunLoop (Modes)
- (void)addTimer:(NSTimer *)t forMode:(NSString *)m;
- (BOOL)runMode:(NSString *)m beforeDate:(NSDate *)d;
@end
@interface NSTimer (Make)
+ (NSTimer *)timerWithTimeInterval:(double)s repeats:(BOOL)r block:(void (^)(NSTimer *))b;
@end
@interface NSDictionary (Strings)
- (NSString *)descriptionInStringsFileFormat;
@end
@protocol NSCoding
- (void)encodeWithCoder:(id)c;
- (id)initWithCoder:(id)c;
@end
@interface NSCoder : NSObject
- (void)encodeObject:(id)o forKey:(NSString *)k;
- (void)encodeInteger:(NSInteger)v forKey:(NSString *)k;
- (id)decodeObjectForKey:(NSString *)k;
- (id)decodeObjectOfClass:(Class)c forKey:(NSString *)k;
- (NSInteger)decodeIntegerForKey:(NSString *)k;
@end
@interface NSKeyedArchiver : NSCoder
+ (NSData *)archivedDataWithRootObject:(id)o requiringSecureCoding:(BOOL)s error:(id *)e;
@end
@interface NSKeyedUnarchiver : NSCoder
+ (id)unarchivedObjectOfClasses:(NSSet *)c fromData:(NSData *)d error:(id *)e;
+ (id)unarchivedObjectOfClass:(Class)c fromData:(NSData *)d error:(id *)e;
@end
@interface NSSet (Make)
+ (instancetype)setWithObjects:(id)first, ...;
@end
@interface NSMutableString : NSString
- (void)appendString:(NSString *)s;
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

@interface Point2 : NSObject <NSCoding>
@property NSInteger x, y;
@property (copy) NSString *label;
@end
@implementation Point2
+ (BOOL)supportsSecureCoding { return YES; }
- (void)encodeWithCoder:(NSCoder *)c {
    [c encodeInteger:self.x forKey:@"x"];
    [c encodeInteger:self.y forKey:@"y"];
    [c encodeObject:self.label forKey:@"label"];
}
- (id)initWithCoder:(NSCoder *)c {
    self = [super init];
    self.x = [c decodeIntegerForKey:@"x"];
    self.y = [c decodeIntegerForKey:@"y"];
    self.label = [c decodeObjectOfClass:[NSString class] forKey:@"label"];
    return self;
}
- (id)description { return [NSString stringWithFormat:@"%@(%ld,%ld)", self.label, (long)self.x, (long)self.y]; }
@end

static int changes, pings;
@interface Watcher : NSObject
@end
@implementation Watcher
- (void)ping:(NSNotification *)n { pings += [n.userInfo[@"n"] intValue]; }
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
        CFTypeRef step = CFPropertyListCreateWithData(0, (__bridge CFTypeRef)[NSData dataWithBytes:"{ a = (1, 2); b = \"two words\"; }" length:32], 0, &sf, 0);
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
    @autoreleasepool {
        /* Files. */
        NSFileManager *fm = [NSFileManager defaultManager];
        NSString *root = @"/tmp/fmtest";
        id err = nil;
        BOOL made = [fm createDirectoryAtPath:[root stringByAppendingPathComponent:@"a/b"] withIntermediateDirectories:YES attributes:nil error:&err];
        [fm createFileAtPath:[root stringByAppendingPathComponent:@"a/one.txt"] contents:[NSData dataWithBytes:"hello" length:5] attributes:nil];
        [@"two" writeToFile:[root stringByAppendingPathComponent:@"a/b/two.txt"] atomically:YES encoding:4 error:nil];
        NSDictionary *attrs = [fm attributesOfItemAtPath:[root stringByAppendingPathComponent:@"a/one.txt"] error:nil];
        BOOL copied = [fm copyItemAtPath:[root stringByAppendingPathComponent:@"a"] toPath:[root stringByAppendingPathComponent:@"c"] error:nil];
        BOOL moved = [fm moveItemAtPath:[root stringByAppendingPathComponent:@"c/one.txt"] toPath:[root stringByAppendingPathComponent:@"moved.txt"] error:nil];
        BOOL dir = NO;
        BOOL exists = [fm fileExistsAtPath:[root stringByAppendingPathComponent:@"c/b"] isDirectory:&dir];
        NSString *sub = [[fm subpathsOfDirectoryAtPath:root error:nil] componentsJoinedByString:@","];
        BOOL removed = [fm removeItemAtPath:[root stringByAppendingPathComponent:@"c"] error:nil];
        id missing = nil;
        [fm removeItemAtPath:@"/tmp/no/such" error:&missing];
        printf("fm: made=%d %s size=%s type=%s copied=%d moved=%d dir=%d/%d removed=%d left=%s missing=%ld/%s\n", made, [sub UTF8String],
               c(attrs[@"NSFileSize"]), [attrs[@"NSFileType"] UTF8String], copied, moved, exists, dir, removed,
               [[[fm contentsOfDirectoryAtPath:root error:nil] componentsJoinedByString:@","] UTF8String], [missing code], [[missing domain] UTF8String]);

        /* The process. */
        NSProcessInfo *pi = [NSProcessInfo processInfo];
        NSUUID *u = [NSUUID UUID];
        NSOperatingSystemVersion v = [pi operatingSystemVersion];
        printf("proc: %s args=%lu home=%s pid=%d os>=11:%d/%d uuid=%lu same=%d bad=%d\n", [[pi processName] UTF8String], [[pi arguments] count],
               [[pi environment][@"HOME"] UTF8String], [pi processIdentifier] > 0, v.majorVersion >= 11,
               [pi isOperatingSystemAtLeastVersion:(NSOperatingSystemVersion){11, 0, 0}], [[u UUIDString] length],
               [[[NSUUID alloc] initWithUUIDString:[u UUIDString]] isEqual:u], [[NSUUID alloc] initWithUUIDString:@"nope"] == nil);

        /* Notifications (including the defaults' change notifications below). */
        NSNotificationCenter *nc = [NSNotificationCenter defaultCenter];
        Watcher *w = [Watcher new];
        [nc addObserver:w selector:@selector(ping:) name:@"Ping" object:nil];
        id token = [nc addObserverForName:@"NSUserDefaultsDidChangeNotification" object:nil queue:nil usingBlock:^(NSNotification *n) { (void)n; changes++; }];
        [nc postNotificationName:@"Ping" object:nil userInfo:@{@"n" : @2}];
        [nc postNotificationName:@"Other" object:nil userInfo:@{@"n" : @100}];

        /* Defaults: written to ~/Library/Preferences/<bundle id>.plist and read back by a new instance. */
        NSUserDefaults *ud = [NSUserDefaults standardUserDefaults];
        [ud registerDefaults:@{@"volume" : @7, @"name" : @"default"}];
        [ud setInteger:42 forKey:@"answer"];
        [ud setBool:YES forKey:@"enabled"];
        [ud setObject:@[ @"a", @"b" ] forKey:@"list"];
        [ud setObject:@"gone" forKey:@"temp"];
        [ud removeObjectForKey:@"temp"];
        NSUserDefaults *again = [[NSUserDefaults alloc] initWithSuiteName:@"org.loos.found"];
        [nc removeObserver:token];
        [nc removeObserver:w];
        [nc postNotificationName:@"Ping" object:nil userInfo:@{@"n" : @1000}];
        printf("defaults: answer=%ld enabled=%d list=%lu volume=%ld name=%s temp=%d file=%d changes=%d pings=%d\n", (long)[again integerForKey:@"answer"],
               [again boolForKey:@"enabled"], [[again objectForKey:@"list"] count], (long)[ud integerForKey:@"volume"], [[ud stringForKey:@"name"] UTF8String],
               [again objectForKey:@"temp"] == nil, [fm fileExistsAtPath:@"/tmp/Library/Preferences/org.loos.found.plist"], changes, pings);

        /* Run-loop modes: a timer in a private mode runs only in that mode. */
        __block int mine = 0, common = 0;
        NSRunLoop *rl = [NSRunLoop currentRunLoop];
        [rl addTimer:[NSTimer timerWithTimeInterval:0.01 repeats:NO block:^(NSTimer *t) { (void)t; mine++; }] forMode:@"MyMode"];
        [rl addTimer:[NSTimer timerWithTimeInterval:0.01 repeats:YES block:^(NSTimer *t) { if (++common == 2) [t invalidate]; }] forMode:@"kCFRunLoopCommonModes"];
        /* Until the common-mode timer has fired twice (the interpreter may be slow), at most 5 s. */
        for (NSDate *end = [NSDate dateWithTimeIntervalSinceNow:5]; common < 2 && [end timeIntervalSinceNow] > 0;)
            [rl runMode:@"kCFRunLoopDefaultMode" beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
        int mine_in_default = mine;
        [rl runMode:@"MyMode" beforeDate:[NSDate dateWithTimeIntervalSinceNow:1]];
        printf("modes: default=%d mine=%d common=%d\n", mine_in_default, mine, common);

        /* The old text format. */
        NSData *step = [NSPropertyListSerialization dataWithPropertyList:@{@"k" : @"two words", @"n" : @[ @1, @"x" ]} format:1 options:0 error:nil];
        NSString *stepText = [[NSString alloc] initWithBytes:[step bytes] length:[step length] encoding:4];
        printf("openstep: %s|%s", [[[stepText componentsSeparatedByString:@"\n"] componentsJoinedByString:@"/"] UTF8String],
               [[@{@"hello" : @"Grüezi"} descriptionInStringsFileFormat] UTF8String]);

        /* Keyed archiving: a custom class inside collections, shared references, secure decoding. */
        Point2 *pt = [Point2 new];
        pt.x = 3; pt.y = -4; pt.label = @"P";
        NSMutableString *ms = [NSMutableString new];
        [ms appendString:@"mutable"];
        NSDictionary *graph = @{@"points" : @[ pt, pt ], @"when" : [NSDate dateWithTimeIntervalSinceReferenceDate:60], @"tags" : [NSSet setWithObjects:@"t1", @"t2", nil],
                                @"text" : ms, @"n" : @5, @"url" : [NSURL URLWithString:@"https://example.com/a"]};
        id aerr = nil;
        NSData *arch = [NSKeyedArchiver archivedDataWithRootObject:graph requiringSecureCoding:YES error:&aerr];
        NSDictionary *plistView = [NSPropertyListSerialization propertyListWithData:arch options:0 format:0 error:nil];
        id allowed = [NSSet setWithObjects:[NSDictionary class], [NSArray class], [Point2 class], [NSDate class],
                                            [NSSet class], [NSString class], [NSNumber class], [NSURL class], nil];
        NSDictionary *back = [NSKeyedUnarchiver unarchivedObjectOfClasses:allowed fromData:arch error:nil];
        NSArray *pts = back[@"points"];
        id derr = nil;
        id refused = [NSKeyedUnarchiver unarchivedObjectOfClass:[NSArray class] fromData:arch error:&derr];
        printf("archive: %s %s same=%d %s %s %s %s %s %s refused=%d/%ld\n", [plistView[@"$archiver"] UTF8String], c(pts), pts[0] == pts[1],
               c(back[@"when"]), [[[back[@"tags"] allObjects] sortedArrayUsingSelector:@selector(compare:)] componentsJoinedByString:@"+"].UTF8String,
               object_getClassName(back[@"text"]), c(back[@"n"]), [[back[@"url"] absoluteString] UTF8String], aerr ? "err" : "ok", refused == nil, [derr code]);
        /* NSCharacterSet, NSScanner. */
        {
            id ws = [NSCharacterSet whitespaceAndNewlineCharacterSet];
            NSString *trimmed = [@"  \t hi there \n" stringByTrimmingCharactersInSet:ws];
            NSArray *parts = [@"a,b;;c" componentsSeparatedByCharactersInSet:[NSCharacterSet characterSetWithCharactersInString:@",;"]];
            NSRange dr = [@"abc123" rangeOfCharacterFromSet:[NSCharacterSet decimalDigitCharacterSet]];
            NSMutableCharacterSet *ms2 = [[NSCharacterSet letterCharacterSet] mutableCopy];
            [ms2 addCharactersInString:@"_"];
            int mem = [ms2 characterIsMember:'_'] * 100 + [ms2 characterIsMember:0xe9] * 10 + [[ms2 invertedSet] characterIsMember:'7'];
            NSString *enc = [@"a b&c=ü" stringByAddingPercentEncodingWithAllowedCharacters:[NSCharacterSet URLQueryAllowedCharacterSet]];
            NSScanner *sc = [NSScanner scannerWithString:@"width = 42, ratio=-1.5e1 color: 0xFF00 name \"Zoë\" rest"];
            NSString *key = nil, *name = nil, *word = nil;
            int w = 0; double r = 0; unsigned col = 0;
            BOOL ok = [sc scanUpToString:@"=" intoString:&key] && [sc scanString:@"=" intoString:nil] && [sc scanInt:&w]
                   && [sc scanString:@"," intoString:nil] && [sc scanUpToString:@"=" intoString:nil] && [sc scanString:@"=" intoString:nil]
                   && [sc scanDouble:&r] && [sc scanCharactersFromSet:[NSCharacterSet letterCharacterSet] intoString:&word]
                   && [sc scanString:@":" intoString:nil] && [sc scanHexInt:&col] && [sc scanUpToString:@"\"" intoString:nil]
                   && [sc scanString:@"\"" intoString:nil] && [sc scanUpToString:@"\"" intoString:&name] && [sc scanString:@"\"" intoString:nil];
            BOOL bad = [sc scanInt:&w];
            NSString *restw = nil;
            [sc scanUpToCharactersFromSet:ws intoString:&restw];
            printf("charset: trim=[%s] parts=%s digit=%lu mem=%d enc=%s\n", [trimmed UTF8String], [[parts componentsJoinedByString:@"|"] UTF8String],
                   (unsigned long)dr.location, mem, [enc UTF8String]);
            printf("scanner: ok=%d key=[%s] w=%d r=%g word=%s col=%u name=%s bad=%d rest=%s end=%d\n", ok, [key UTF8String], w, r, [word UTF8String], col,
                   [name UTF8String], bad, [restw UTF8String], [sc isAtEnd]);
        }
    }
    /* Uncaught: Foundation's message, then the abort. */
    [NSException raise:@"Custom" format:@"bye %d", 3];
    return 0;
}
