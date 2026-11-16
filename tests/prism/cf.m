/* CoreFoundation through Prism's runtime (ARC): CF objects are Objective-C objects, so a
 * CFStringRef is an NSString *, @"..." constants have a real class, and the literals @[], @{} and
 * @() build arrays, dictionaries and numbers. No SDK: the interfaces are declared here. */
int printf(const char *, ...);
int strcmp(const char *, const char *);
typedef unsigned long NSUInteger;
typedef long NSInteger, CFIndex;
#if __arm64__
typedef _Bool BOOL;
#else
typedef signed char BOOL;
#endif
typedef unsigned char Boolean, UInt8;
#define nil ((id)0)
#define YES __objc_yes
typedef const struct __CFString *CFStringRef;
typedef struct __CFString *CFMutableStringRef;
typedef const void *CFTypeRef, *CFArrayRef, *CFDictionaryRef, *CFNumberRef, *CFDataRef, *CFBooleanRef;
typedef struct { CFIndex location, length; } CFRange;
const char *object_getClassName(id);
extern const void *kCFTypeArrayCallBacks, *kCFTypeDictionaryKeyCallBacks, *kCFTypeDictionaryValueCallBacks;
extern CFBooleanRef kCFBooleanTrue;
#define CFSTR(s) ((CFStringRef)__builtin___CFStringMakeConstantString(s))
CFTypeRef CFRetain(CFTypeRef);
void CFRelease(CFTypeRef);
CFIndex CFGetRetainCount(CFTypeRef);
unsigned long CFGetTypeID(CFTypeRef);
unsigned long CFStringGetTypeID(void), CFArrayGetTypeID(void), CFNumberGetTypeID(void);
Boolean CFEqual(CFTypeRef, CFTypeRef);
CFIndex CFStringGetLength(CFStringRef);
Boolean CFStringGetCString(CFStringRef, char *, CFIndex, unsigned);
CFMutableStringRef CFStringCreateMutable(const void *, CFIndex);
void CFStringAppend(CFMutableStringRef, CFStringRef);
void CFStringAppendCString(CFMutableStringRef, const char *, unsigned);
CFStringRef CFStringCreateWithFormat(const void *, CFDictionaryRef, CFStringRef, ...);
CFNumberRef CFNumberCreate(const void *, CFIndex, const void *);
Boolean CFNumberGetValue(CFNumberRef, CFIndex, void *);
CFArrayRef CFArrayCreate(const void *, const void **, CFIndex, const void *);
CFIndex CFArrayGetCount(CFArrayRef);
const void *CFArrayGetValueAtIndex(CFArrayRef, CFIndex);
CFDictionaryRef CFDictionaryCreateMutable(const void *, CFIndex, const void *, const void *);
void CFDictionarySetValue(CFDictionaryRef, const void *, const void *);
const void *CFDictionaryGetValue(CFDictionaryRef, const void *);
Boolean CFBooleanGetValue(CFBooleanRef);
CFDataRef CFDataCreate(const void *, const UInt8 *, CFIndex);

__attribute__((objc_root_class))
@interface NSObject { Class isa; }
+ (instancetype)alloc;
+ (instancetype)new;
- (instancetype)init;
- (id)copy;
- (id)mutableCopy;
- (id)description;
- (NSUInteger)hash;
- (BOOL)isEqual:(id)o;
@end
@interface NSString : NSObject
+ (instancetype)stringWithFormat:(NSString *)f, ...;
+ (instancetype)stringWithUTF8String:(const char *)s;
- (NSUInteger)length;
- (const char *)UTF8String;
- (BOOL)hasPrefix:(NSString *)s;
- (NSString *)uppercaseString;
- (NSString *)stringByAppendingString:(NSString *)s;
- (NSString *)substringFromIndex:(NSUInteger)i;
- (BOOL)isEqualToString:(NSString *)s;
- (int)intValue;
@end
@interface NSMutableString : NSString
- (void)appendString:(NSString *)s;
@end
@interface NSNumber : NSObject
+ (NSNumber *)numberWithInt:(int)v;
- (int)intValue;
- (double)doubleValue;
- (BOOL)boolValue;
- (const char *)objCType;
@end
@interface NSArray<T> : NSObject
- (NSUInteger)count;
- (T)objectAtIndexedSubscript:(NSUInteger)i;
- (NSString *)componentsJoinedByString:(NSString *)s;
- (NSArray *)sortedArrayUsingSelector:(SEL)s;
- (BOOL)containsObject:(id)o;
- (NSUInteger)countByEnumeratingWithState:(void *)st objects:(id __unsafe_unretained *)b count:(NSUInteger)n;
@end
@interface NSMutableArray<T> : NSArray<T>
- (void)addObject:(T)o;
- (void)removeObjectAtIndex:(NSUInteger)i;
@end
@interface NSDictionary<K, V> : NSObject
- (NSUInteger)count;
- (V)objectForKeyedSubscript:(K)k;
- (NSArray *)allKeys;
- (NSUInteger)countByEnumeratingWithState:(void *)st objects:(id __unsafe_unretained *)b count:(NSUInteger)n;
@end
@interface NSMutableDictionary<K, V> : NSDictionary<K, V>
- (void)setObject:(V)o forKeyedSubscript:(K)k;
- (void)removeObjectForKey:(K)k;
@end
@interface NSData : NSObject
+ (instancetype)dataWithBytes:(const void *)b length:(NSUInteger)n;
- (NSUInteger)length;
@end
@interface NSMutableData : NSData
- (void)appendBytes:(const void *)b length:(NSUInteger)n;
@end
@interface NSNumber (Literals)
+ (NSNumber *)numberWithDouble:(double)v;
+ (NSNumber *)numberWithBool:(BOOL)v;
+ (NSNumber *)numberWithInteger:(NSInteger)v;
@end
@interface NSArray (Literals)
+ (instancetype)arrayWithObjects:(const id [])o count:(NSUInteger)n;
@end
@interface NSDictionary (Literals)
+ (instancetype)dictionaryWithObjects:(const id [])o forKeys:(const id [])k count:(NSUInteger)n;
@end
@interface NSString (Compare)
- (NSInteger)compare:(NSString *)o;
@end

static const char *c(id o) { return [[o description] UTF8String]; }

int main(void) {
    @autoreleasepool {
        /* Constant strings: a real class, and the same object as the CFSTR one. */
        NSString *k = @"hello";
        CFStringRef cs = CFSTR("hello");
        printf("const: class=%s len=%lu eq=%d cf=%d\n", object_getClassName(k), [k length], [k isEqual:(__bridge id)cs],
               CFGetTypeID(cs) == CFStringGetTypeID());

        /* Toll-free: CF functions on an NSString, NSString methods on a CFString. */
        CFMutableStringRef m = CFStringCreateMutable(0, 0);
        CFStringAppend(m, (__bridge CFStringRef)k);
        CFStringAppendCString(m, ", wörld", 0x08000100);
        NSString *ns = (__bridge_transfer NSString *)m;
        char buf[64];
        CFStringGetCString((__bridge CFStringRef)[ns uppercaseString], buf, sizeof buf, 0x08000100);
        CFStringRef f = CFStringCreateWithFormat(0, 0, CFSTR("%d-%@-%s"), 7, k, "c");
        NSString *g = [NSString stringWithFormat:@"%@ %.2f %05d %x", (__bridge id)f, 2.5, 42, 255];
        CFRelease(f);
        printf("str: %s | %s | len=%ld | %s | prefix=%d sub=%s int=%d\n", [ns UTF8String], buf, CFStringGetLength((__bridge CFStringRef)ns),
               [g UTF8String], [ns hasPrefix:@"hell"], [[ns substringFromIndex:7] UTF8String], [@"  -12x" intValue]);

        /* Numbers. */
        NSNumber *n = @42, *d = @2.5, *b = @YES;
        int v = 0;
        int seven = 7;
        CFNumberRef cn = CFNumberCreate(0, 9, &seven);
        CFNumberGetValue((__bridge CFNumberRef)n, 9, &v);
        printf("num: %s %s %s v=%d eq=%d %s%s bool=%d cf=%s\n", c(n), c(d), c(b), v, [@42 isEqual:@42.0], [n objCType], [d objCType],
               CFBooleanGetValue((__bridge CFBooleanRef)b) && (__bridge CFBooleanRef)b == kCFBooleanTrue, c((__bridge id)cn));
        CFRelease(cn);

        /* Arrays. */
        NSArray<NSString *> *a = @[ @"pear", @"apple", @"fig" ];
        NSMutableArray *ma = [a mutableCopy];
        [ma addObject:@3];
        [ma removeObjectAtIndex:0];
        int total = 0;
        for (id o in ma) total += (int)[[o description] length];
        const void *raw[] = {(__bridge const void *)k, (__bridge const void *)n};
        CFArrayRef ca = CFArrayCreate(0, raw, 2, &kCFTypeArrayCallBacks);
        printf("arr: %s | %s | n=%lu total=%d has=%d cf=%ld %s\n", [[a componentsJoinedByString:@","] UTF8String],
               [[[a sortedArrayUsingSelector:@selector(compare:)] componentsJoinedByString:@" "] UTF8String], [ma count], total,
               [a containsObject:@"fig"], CFArrayGetCount(ca), c((__bridge id)CFArrayGetValueAtIndex(ca, 1)));
        printf("desc: %s\n", c(@[ @1, @"two words", @[ @"x" ] ]));
        CFRelease(ca);

        /* Dictionaries. */
        NSDictionary<NSString *, id> *dict = @{ @"one" : @1, @"two" : @2, @"list" : a };
        NSMutableDictionary *md = [NSMutableDictionary new];
        for (int i = 0; i < 100; i++) md[[NSString stringWithFormat:@"k%d", i]] = @(i * i);
        for (int i = 0; i < 100; i += 2) [md removeObjectForKey:[NSString stringWithFormat:@"k%d", i]];
        long sum = 0;
        for (NSString *key in md) sum += [md[key] intValue];
        CFDictionaryRef cd = CFDictionaryCreateMutable(0, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
        CFDictionarySetValue(cd, CFSTR("x"), (__bridge const void *)@9);
        printf("dict: one=%s list=%lu missing=%d count=%lu sum=%ld cf=%s %s\n", c(dict[@"one"]), [dict[@"list"] count], dict[@"none"] == nil,
               [md count], sum, c((__bridge id)CFDictionaryGetValue(cd, (__bridge const void *)[NSString stringWithUTF8String:"x"])), c(@{ @"a b" : @1 }));
        CFRelease(cd);

        /* Data and reference counts. */
        UInt8 bytes[] = {0xde, 0xad, 0xbe, 0xef, 1};
        CFDataRef cdata = CFDataCreate(0, bytes, 5);
        NSMutableData *md2 = [(__bridge NSData *)cdata mutableCopy];
        [md2 appendBytes:"!" length:1];
        CFTypeRef r = CFRetain((__bridge CFTypeRef)md2);
        printf("data: %s len=%lu eq=%d rc=%ld\n", c((__bridge id)cdata), [md2 length], CFEqual(cdata, (__bridge CFTypeRef)[NSData dataWithBytes:bytes length:5]),
               CFGetRetainCount(r));
        CFRelease(r);
        CFRelease(cdata);
    }
    return 0;
}
