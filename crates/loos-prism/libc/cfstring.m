/* NSString / CFString: one concrete class, __NSCFString, for every string Prism makes, and
 * __NSCFConstantString for the compiler's @"..." and CFSTR constants (the same layout, never
 * freed). The text logic is in src/cf/string.rs. */
#include "foundation.h"

@interface __NSCFString : NSMutableString { @public uint64_t _flags; void *_data; uint64_t _count, _cap; }
@end
@interface __NSCFConstantString : __NSCFString
@end
/* The class word of constant strings: a copy of __NSCFConstantString (objc.c fills it in). */
uintptr_t __CFConstantStringClassReference[5];

static Class cfstring;
static id new_string(uint64_t flags) {
    if (!cfstring) cfstring = objc_getClass("__NSCFString");
    id s = class_createInstance(cfstring, 0);
    CF(s)->flags = flags;
    return s;
}
/* The fields of any string: its own if it is one of ours, else a UTF-16 copy in `tmp`. */
static struct cfobj *view(id s, struct cfobj *tmp) {
    if (!cfstring) cfstring = objc_getClass("__NSCFString");
    if (cf_is(s, cfstring)) return CF(s);
    NSUInteger n = [s length];
    unichar *b = malloc(n * 2 + 2);
    [s getCharacters:b range:(NSRange){0, n}];
    *tmp = (struct cfobj){0, CF_UTF16, b, n, 0};
    return tmp;
}
#define VIEW(name, s) struct cfobj name##_tmp, *name = view(s, &name##_tmp)
#define DONE(name) do { if (name == &name##_tmp) free(name##_tmp.data); } while (0)

id _prism_cf_string(const char *utf8) {
    id s = new_string(0);
    if (!__prism_cf_str_init(s, utf8, strlen(utf8), 4)) __prism_cf_str_init(s, utf8, strlen(utf8), 5);
    return [s autorelease];
}

/* A malloc'ed buffer that lives until the autorelease pool drains (-UTF8String and friends). */
@interface _PrismBuffer : NSObject { @public void *p; }
@end
@implementation _PrismBuffer
- (void)dealloc { free(p); [super dealloc]; }
@end
void *_prism_cf_autoreleased_buffer(void *p) {
    _PrismBuffer *b = [[_PrismBuffer new] autorelease];
    b->p = p;
    return p;
}

@implementation NSObject (CoreFoundation)
- (id)description {
    char b[128];
    snprintf(b, sizeof b, "<%s: %p>", object_getClassName(self), (void *)self);
    return _prism_cf_string(b);
}
+ (id)description { return _prism_cf_string(object_getClassName(self)); }
- (id)debugDescription { return [self description]; }
- (CFTypeID)_cfTypeID { return 1; }
@end
id _prism_cf_describe(id o) { return o ? [o description] : _prism_cf_string("(null)"); }

@implementation NSString
+ (id)allocWithZone:(NSZone *)z { (void)z; return new_string(0); }
+ (id)alloc { return new_string(0); }
+ (id)string { return [new_string(0) autorelease]; }
+ (id)stringWithUTF8String:(const char *)s { return [[[self alloc] initWithUTF8String:s] autorelease]; }
+ (id)stringWithCString:(const char *)s encoding:(NSUInteger)e { return [[[self alloc] initWithCString:s encoding:e] autorelease]; }
+ (id)stringWithString:(NSString *)s { return [[[self alloc] initWithString:s] autorelease]; }
+ (id)stringWithCharacters:(const unichar *)c length:(NSUInteger)n { return [[[self alloc] initWithCharacters:c length:n] autorelease]; }
+ (id)stringWithFormat:(NSString *)f, ... {
    va_list ap;
    va_start(ap, f);
    id s = _prism_cf_format(f, ap);
    va_end(ap);
    return s;
}
- (id)init { return self; }
- (id)initWithBytes:(const void *)b length:(NSUInteger)n encoding:(NSUInteger)e {
    if (__prism_cf_str_init(self, b, n, e)) return self;
    [self release];
    return nil;
}
- (id)initWithUTF8String:(const char *)s { return s ? [self initWithBytes:s length:strlen(s) encoding:4] : ([self release], nil); }
- (id)initWithCString:(const char *)s encoding:(NSUInteger)e { return s ? [self initWithBytes:s length:strlen(s) encoding:e] : ([self release], nil); }
- (id)initWithCharacters:(const unichar *)c length:(NSUInteger)n { __prism_cf_str_init_chars(self, c, n); return self; }
- (id)initWithString:(NSString *)s { VIEW(v, s); __prism_cf_str_sub(self, v, 0, v->count); DONE(v); return self; }
- (id)initWithFormat:(NSString *)f, ... {
    va_list ap;
    va_start(ap, f);
    id s = _prism_cf_format(f, ap);
    va_end(ap);
    return [self initWithString:s];
}
- (id)initWithFormat:(NSString *)f arguments:(va_list)ap { return [self initWithString:_prism_cf_format(f, ap)]; }

/* The two methods a subclass must provide; everything else works through them. */
- (NSUInteger)length { return 0; }
- (unichar)characterAtIndex:(NSUInteger)i { (void)i; return 0; }
- (void)getCharacters:(unichar *)buf range:(NSRange)r { for (NSUInteger i = 0; i < r.length; i++) buf[i] = [self characterAtIndex:r.location + i]; }
- (void)getCharacters:(unichar *)buf { [self getCharacters:buf range:(NSRange){0, [self length]}]; }

- (CFTypeID)_cfTypeID { return 7; }
- (id)description { return self; }
- (id)copyWithZone:(NSZone *)z { (void)z; return [[NSString alloc] initWithString:self]; }
- (id)mutableCopyWithZone:(NSZone *)z {
    (void)z;
    id s = new_string(CF_MUTABLE);
    return [s initWithString:self];
}
- (NSUInteger)hash { VIEW(v, self); NSUInteger h = __prism_cf_str_hash(v); DONE(v); return h; }
- (NSInteger)compare:(NSString *)o options:(NSUInteger)opts range:(NSRange)r {
    if (!o) return 1;
    VIEW(a, self); VIEW(b, o);
    long c = __prism_cf_str_compare(a, r.location, r.length, b, opts);
    DONE(a); DONE(b);
    return c;
}
- (NSInteger)compare:(NSString *)o options:(NSUInteger)opts { return [self compare:o options:opts range:(NSRange){0, [self length]}]; }
- (NSInteger)compare:(NSString *)o { return [self compare:o options:0]; }
- (NSInteger)caseInsensitiveCompare:(NSString *)o { return [self compare:o options:1]; }
- (NSInteger)localizedCompare:(NSString *)o { return [self compare:o options:0]; }
- (NSInteger)localizedCaseInsensitiveCompare:(NSString *)o { return [self compare:o options:1]; }
- (NSInteger)localizedStandardCompare:(NSString *)o { return [self compare:o options:1 | 64]; }
- (BOOL)isEqualToString:(NSString *)o {
    if (o == self) return YES;
    if (!o || [o length] != [self length]) return NO;
    return [self compare:o options:0] == 0;
}
- (BOOL)isEqual:(id)o {
    if (o == self) return YES;
    static Class string;
    if (!string) string = objc_getClass("NSString");
    return cf_is(o, string) && [self isEqualToString:o];
}
- (NSRange)rangeOfString:(NSString *)o options:(NSUInteger)opts range:(NSRange)r {
    NSRange out = {NSNotFound, 0};
    if (!o) return out;
    VIEW(a, self); VIEW(b, o);
    __prism_cf_str_find(a, r.location, r.length, b, opts, &out);
    DONE(a); DONE(b);
    return out;
}
- (NSRange)rangeOfString:(NSString *)o options:(NSUInteger)opts { return [self rangeOfString:o options:opts range:(NSRange){0, [self length]}]; }
- (NSRange)rangeOfString:(NSString *)o { return [self rangeOfString:o options:0]; }
- (id)componentsSeparatedByString:(NSString *)sep {
    id out = [NSMutableArray array];
    NSUInteger at = 0, n = [self length];
    for (;;) {
        NSRange r = [sep length] ? [self rangeOfString:sep options:0 range:(NSRange){at, n - at}] : (NSRange){NSNotFound, 0};
        if (r.location == NSNotFound) { [out addObject:[self substringFromIndex:at]]; return out; }
        [out addObject:[self substringWithRange:(NSRange){at, r.location - at}]];
        at = r.location + r.length;
    }
}
- (BOOL)containsString:(NSString *)o { return [self rangeOfString:o].length != 0; }
- (BOOL)hasPrefix:(NSString *)o { return [self rangeOfString:o options:8].length != 0; }
- (BOOL)hasSuffix:(NSString *)o { return [self rangeOfString:o options:8 | 4].length != 0; }
- (id)substringWithRange:(NSRange)r {
    if (r.location > [self length] || r.length > [self length] - r.location)
        _prism_raise("NSRangeException", "*** -[%s substringWithRange:]: Range {%lu, %lu} out of bounds; string length %lu", object_getClassName(self), r.location, r.length, [self length]);
    id s = new_string(0);
    VIEW(v, self);
    __prism_cf_str_sub(s, v, r.location, r.length);
    DONE(v);
    return [s autorelease];
}
- (id)substringFromIndex:(NSUInteger)i {
    if (i > [self length]) _prism_raise("NSRangeException", "*** -[%s substringFromIndex:]: Index %lu out of bounds; string length %lu", object_getClassName(self), i, [self length]);
    return [self substringWithRange:(NSRange){i, [self length] - i}];
}
- (id)substringToIndex:(NSUInteger)i { return [self substringWithRange:(NSRange){0, i}]; }
- (id)stringByAppendingString:(NSString *)o {
    NSMutableString *s = [[self mutableCopyWithZone:0] autorelease];
    [s appendString:o];
    return s;
}
- (id)stringByAppendingFormat:(NSString *)f, ... {
    va_list ap;
    va_start(ap, f);
    id t = _prism_cf_format(f, ap);
    va_end(ap);
    return [self stringByAppendingString:t];
}
- (id)stringByReplacingOccurrencesOfString:(NSString *)a withString:(NSString *)b {
    NSMutableString *s = [[self mutableCopyWithZone:0] autorelease];
    [(id)s replaceOccurrencesOfString:a withString:b options:0 range:(NSRange){0, [s length]}];
    return s;
}
static id mapped(id self, unsigned long mode) {
    id s = new_string(0);
    VIEW(v, self);
    __prism_cf_str_map(s, v, mode);
    DONE(v);
    return [s autorelease];
}
- (id)uppercaseString { return mapped(self, 0); }
- (id)lowercaseString { return mapped(self, 1); }
- (id)capitalizedString { return mapped(self, 2); }
- (long long)longLongValue { VIEW(v, self); long long n = __prism_cf_str_i64(v); DONE(v); return n; }
- (NSInteger)integerValue { return [self longLongValue]; }
- (int)intValue { long long n = [self longLongValue]; return n > 0x7fffffff ? 0x7fffffff : n < -0x7fffffff - 1 ? -0x7fffffff - 1 : (int)n; }
- (double)doubleValue { VIEW(v, self); double d = __prism_cf_str_f64(v); DONE(v); return d; }
- (float)floatValue { return [self doubleValue]; }
- (BOOL)boolValue {
    NSUInteger n = [self length], i = 0;
    while (i < n && ([self characterAtIndex:i] == ' ' || [self characterAtIndex:i] == '\t')) i++;
    unichar c = i < n ? [self characterAtIndex:i] : 0;
    return c == 'Y' || c == 'y' || c == 'T' || c == 't' || [self longLongValue] != 0;
}
/* Bytes of the whole string in `e`, NUL-terminated (two NULs cover UTF-16); *len without them. */
- (char *)_bytesUsingEncoding:(NSUInteger)e loss:(long)loss length:(NSUInteger *)len {
    VIEW(v, self);
    long n = __prism_cf_str_encode(v, 0, v->count, e, loss, 0, 0);
    char *b = n < 0 ? 0 : malloc(n + 4);
    if (b) { __prism_cf_str_encode(v, 0, v->count, e, loss, b, n); memset(b + n, 0, 4); }
    DONE(v);
    if (len) *len = n < 0 ? 0 : n;
    return b;
}
- (const char *)cStringUsingEncoding:(NSUInteger)e {
    char *b = [self _bytesUsingEncoding:e loss:-1 length:0];
    return b ? _prism_cf_autoreleased_buffer(b) : 0;
}
- (const char *)UTF8String { return [self cStringUsingEncoding:4]; }
- (id)dataUsingEncoding:(NSUInteger)e allowLossyConversion:(BOOL)lossy {
    NSUInteger n;
    char *b = [self _bytesUsingEncoding:e loss:lossy ? '?' : -1 length:&n];
    if (!b) return nil;
    id d = [NSData dataWithBytes:b length:n];
    free(b);
    return d;
}
- (id)dataUsingEncoding:(NSUInteger)e { return [self dataUsingEncoding:e allowLossyConversion:NO]; }
- (id)initWithData:(NSData *)d encoding:(NSUInteger)e { return [self initWithBytes:[d bytes] length:[d length] encoding:e]; }
- (BOOL)getCString:(char *)buf maxLength:(NSUInteger)max encoding:(NSUInteger)e {
    NSUInteger n;
    char *b = [self _bytesUsingEncoding:e loss:-1 length:&n];
    BOOL ok = b && n < max;
    if (ok) memcpy(buf, b, n + 1);
    free(b);
    return ok;
}
- (NSUInteger)lengthOfBytesUsingEncoding:(NSUInteger)e {
    VIEW(v, self);
    long n = __prism_cf_str_encode(v, 0, v->count, e, -1, 0, 0);
    DONE(v);
    return n < 0 ? 0 : n;
}
- (NSUInteger)maximumLengthOfBytesUsingEncoding:(NSUInteger)e { return [self length] * (e == 4 ? 3 : e == 10 ? 2 : 4); }
- (BOOL)canBeConvertedToEncoding:(NSUInteger)e {
    VIEW(v, self);
    long n = __prism_cf_str_encode(v, 0, v->count, e, -1, 0, 0);
    DONE(v);
    return n >= 0;
}
@end

@implementation NSMutableString
+ (id)allocWithZone:(NSZone *)z { (void)z; return new_string(CF_MUTABLE); }
+ (id)alloc { return new_string(CF_MUTABLE); }
+ (id)string { return [new_string(CF_MUTABLE) autorelease]; }
+ (id)stringWithCapacity:(NSUInteger)n { (void)n; return [new_string(CF_MUTABLE) autorelease]; }
- (id)initWithCapacity:(NSUInteger)n { (void)n; return self; }
- (id)copyWithZone:(NSZone *)z { (void)z; return [new_string(0) initWithString:self]; }
- (void)replaceCharactersInRange:(NSRange)r withString:(NSString *)o {
    if (r.location > [self length] || r.length > [self length] - r.location)
        _prism_raise("NSRangeException", "*** -[%s replaceCharactersInRange:withString:]: Range or index out of bounds", object_getClassName(self));
    VIEW(v, o);
    __prism_cf_str_splice(self, r.location, r.length, v);
    DONE(v);
}
- (void)appendString:(NSString *)o { if (o) [self replaceCharactersInRange:(NSRange){[self length], 0} withString:o]; }
- (void)appendFormat:(NSString *)f, ... {
    va_list ap;
    va_start(ap, f);
    id t = _prism_cf_format(f, ap);
    va_end(ap);
    [self appendString:t];
}
- (void)insertString:(NSString *)o atIndex:(NSUInteger)i { [self replaceCharactersInRange:(NSRange){i, 0} withString:o]; }
- (void)deleteCharactersInRange:(NSRange)r { __prism_cf_str_splice(self, r.location, r.length, 0); }
- (void)setString:(NSString *)o { [self replaceCharactersInRange:(NSRange){0, [self length]} withString:o]; }
- (NSUInteger)replaceOccurrencesOfString:(NSString *)a withString:(NSString *)b options:(NSUInteger)opts range:(NSRange)r {
    NSUInteger n = 0, end = r.location + r.length;
    for (;;) {
        NSRange f = [self rangeOfString:a options:opts & ~4ul range:(NSRange){r.location, end - r.location}];
        if (!f.length) return n;
        [self replaceCharactersInRange:f withString:b];
        NSUInteger blen = [b length];
        r.location = f.location + blen;
        end = end - f.length + blen;
        n++;
    }
}
@end

@implementation __NSCFString
- (NSUInteger)length { return _count; }
- (unichar)characterAtIndex:(NSUInteger)i {
    if (i >= _count) _prism_raise("NSRangeException", "*** -[%s characterAtIndex:]: Range or index out of bounds", object_getClassName(self));
    return _flags & CF_UTF16 ? ((unichar *)_data)[i] : ((unsigned char *)_data)[i];
}
- (void)getCharacters:(unichar *)buf range:(NSRange)r { __prism_cf_str_chars(self, r.location, r.length, buf); }
- (const char *)UTF8String { return _flags & CF_UTF16 ? [super UTF8String] : _data ? _data : ""; }
- (id)copyWithZone:(NSZone *)z { (void)z; return _flags & CF_MUTABLE ? [new_string(0) initWithString:self] : [self retain]; }
- (void)dealloc {
    if (_flags & CF_OWNED) free(_data);
    [super dealloc];
}
@end

@implementation __NSCFConstantString
- (id)retain { return self; }
- (oneway void)release {}
- (id)autorelease { return self; }
- (NSUInteger)retainCount { return ~0ul >> 1; }
- (id)copyWithZone:(NSZone *)z { (void)z; return self; }
- (void)dealloc { if (0) [super dealloc]; }
@end

/* ------------------------------------------------------------------ formatting */

static void append_utf8(id out, const char *s, size_t n) {
    id t = new_string(0);
    if (!__prism_cf_str_init(t, s, n, 4)) __prism_cf_str_init(t, s, n, 5);
    [out appendString:t];
    [t release];
}

/* printf formatting with %@ (an object's description), %C and %S (UTF-16). Everything else goes
 * through snprintf one conversion at a time. */
id _prism_cf_format(id format, va_list ap) {
    id out = [new_string(CF_MUTABLE) autorelease];
    if (!format) return out;
    NSUInteger n = [format length];
    unichar *f = malloc((n + 1) * 2);
    [format getCharacters:f range:(NSRange){0, n}];
    f[n] = 0;
    for (NSUInteger i = 0; i < n;) {
        NSUInteger j = i;
        while (j < n && f[j] != '%') j++;
        if (j > i) {
            id t = new_string(0);
            __prism_cf_str_init_chars(t, f + i, j - i);
            [out appendString:t];
            [t release];
        }
        if (j >= n) break;
        char spec[48];
        int k = 0, width = -1, prec = -1, lng = 0;
        spec[k++] = '%';
        j++;
        for (; j < n && k < 40; j++) {
            unichar c = f[j];
            if (c == '*') {
                int v = va_arg(ap, int);
                if (spec[k - 1] == '.') prec = v; else width = v;
                k += snprintf(spec + k, 12, "%d", v);
            } else if (c == 'l' || c == 'q' || c == 'z' || c == 't' || c == 'j') { lng++; spec[k++] = c == 'q' ? 'l' : c; if (c == 'q') { spec[k++] = 'l'; } }
            else if (c == 'h' || c == 'L' || c == '-' || c == '+' || c == ' ' || c == '#' || c == '.' || c == '\'' || (c >= '0' && c <= '9') || c == '$') spec[k++] = c;
            else break;
        }
        (void)width; (void)prec;
        unichar conv = j < n ? f[j++] : 0;
        char buf[512], *big = 0;
        int len = 0;
        if (conv == '@') {
            [out appendString:_prism_cf_describe(va_arg(ap, id))];
        } else if (conv == '%') {
            append_utf8(out, "%", 1);
        } else if (conv == 'C') {
            unichar c = (unichar)va_arg(ap, int);
            id t = new_string(0);
            __prism_cf_str_init_chars(t, &c, 1);
            [out appendString:t];
            [t release];
        } else if (conv == 'S') {
            const unichar *s = va_arg(ap, const unichar *);
            NSUInteger m = 0;
            while (s && s[m]) m++;
            id t = new_string(0);
            __prism_cf_str_init_chars(t, s, m);
            [out appendString:t];
            [t release];
        } else if (conv) {
            spec[k++] = (char)conv;
            spec[k] = 0;
            if (conv == 's') {
                const char *s = va_arg(ap, const char *);
                len = snprintf(0, 0, spec, s);
                big = malloc(len + 1);
                snprintf(big, len + 1, spec, s);
            } else if (conv == 'f' || conv == 'F' || conv == 'e' || conv == 'E' || conv == 'g' || conv == 'G' || conv == 'a' || conv == 'A') {
                len = snprintf(buf, sizeof buf, spec, va_arg(ap, double));
            } else if (conv == 'p') {
                len = snprintf(buf, sizeof buf, spec, va_arg(ap, void *));
            } else if (conv == 'D' || conv == 'U' || conv == 'O' || lng) {
                len = snprintf(buf, sizeof buf, spec, va_arg(ap, long));
            } else {
                len = snprintf(buf, sizeof buf, spec, va_arg(ap, int));
            }
            if (len > 0) append_utf8(out, big ? big : buf, big ? (size_t)len : (size_t)len < sizeof buf ? (size_t)len : sizeof buf - 1);
            free(big);
        }
        i = j;
    }
    free(f);
    return out;
}

/* ------------------------------------------------------------------ CFString */

#define S(x) ((NSString *)(x))
static unsigned long long nothing;
const void *kCFAllocatorDefault = 0, *kCFAllocatorSystemDefault = &nothing, *kCFAllocatorMalloc = &nothing,
    *kCFAllocatorMallocZone = &nothing, *kCFAllocatorNull = &nothing + 1, *kCFAllocatorUseContext = &nothing;

CFTypeID CFStringGetTypeID(void) { return 7; }
const void *CFStringCreateWithBytes(CFAllocatorRef a, const void *bytes, CFIndex n, CFStringEncoding e, unsigned char external) {
    (void)a; (void)external;
    return [[NSString alloc] initWithBytes:bytes length:n encoding:e];
}
const void *CFStringCreateWithCString(CFAllocatorRef a, const char *s, CFStringEncoding e) { return s ? CFStringCreateWithBytes(a, s, strlen(s), e, 0) : 0; }
const void *CFStringCreateWithCharacters(CFAllocatorRef a, const unichar *c, CFIndex n) { (void)a; return [[NSString alloc] initWithCharacters:c length:n]; }
const void *CFStringCreateCopy(CFAllocatorRef a, const void *s) { (void)a; return [S(s) copyWithZone:0]; }
const void *CFStringCreateMutable(CFAllocatorRef a, CFIndex max) { (void)a; (void)max; return new_string(CF_MUTABLE); }
const void *CFStringCreateMutableCopy(CFAllocatorRef a, CFIndex max, const void *s) { (void)a; (void)max; return [S(s) mutableCopyWithZone:0]; }
const void *CFStringCreateWithSubstring(CFAllocatorRef a, const void *s, CFRange r) { (void)a; return [[S(s) substringWithRange:(NSRange){r.location, r.length}] retain]; }
const void *CFStringCreateWithFormatAndArguments(CFAllocatorRef a, const void *opts, const void *f, va_list ap) { (void)a; (void)opts; return [_prism_cf_format((id)f, ap) retain]; }
const void *CFStringCreateWithFormat(CFAllocatorRef a, const void *opts, const void *f, ...) {
    va_list ap;
    va_start(ap, f);
    const void *s = CFStringCreateWithFormatAndArguments(a, opts, f, ap);
    va_end(ap);
    return s;
}
/* CFSTR() when the compiler does not emit a constant: one string per C string, kept forever. */
const void *__CFStringMakeConstantString(const char *s) {
    static struct { const char *k; id v; } *tab;
    static unsigned n, cap;
    for (unsigned i = 0; i < n; i++) if (tab[i].k == s || !strcmp(tab[i].k, s)) return tab[i].v;
    if (n == cap) { cap = cap ? cap * 2 : 32; tab = realloc(tab, cap * sizeof *tab); }
    tab[n].k = s;
    tab[n].v = [[NSString alloc] initWithUTF8String:s];
    return tab[n++].v;
}
CFIndex CFStringGetLength(const void *s) { return [S(s) length]; }
unichar CFStringGetCharacterAtIndex(const void *s, CFIndex i) { return [S(s) characterAtIndex:i]; }
void CFStringGetCharacters(const void *s, CFRange r, unichar *buf) { [S(s) getCharacters:buf range:(NSRange){r.location, r.length}]; }
unsigned char CFStringGetCString(const void *s, char *buf, CFIndex size, CFStringEncoding e) { return [S(s) getCString:buf maxLength:size encoding:e]; }
const char *CFStringGetCStringPtr(const void *s, CFStringEncoding e) {
    if (!cfstring) cfstring = objc_getClass("__NSCFString");
    BOOL eight = e == 0x0600 || e == 0x08000100 || e == 0 || e == 0x0201;
    return eight && cf_is((id)s, cfstring) && !(CF(s)->flags & CF_UTF16) && CF(s)->data ? CF(s)->data : 0;
}
const unichar *CFStringGetCharactersPtr(const void *s) {
    if (!cfstring) cfstring = objc_getClass("__NSCFString");
    return cf_is((id)s, cfstring) && (CF(s)->flags & CF_UTF16) ? CF(s)->data : 0;
}
CFIndex CFStringGetBytes(const void *s, CFRange r, CFStringEncoding e, unsigned char loss, unsigned char external, unsigned char *buf, CFIndex max, CFIndex *used) {
    (void)external;
    VIEW(v, (id)s);
    long n = __prism_cf_str_encode(v, r.location, r.length, e, loss ? loss : -1, buf, buf ? max : 0);
    DONE(v);
    if (n < 0 || (buf && n > max)) { if (used) *used = 0; return 0; }
    if (used) *used = n;
    return r.length;
}
CFIndex CFStringGetMaximumSizeForEncoding(CFIndex len, CFStringEncoding e) { return len * (e == 0x08000100 ? 3 : e == 0x0100 ? 2 : e == 0x0600 || e == 0 || e == 0x0201 ? 1 : 4); }
CFStringEncoding CFStringGetSystemEncoding(void) { return 0; }
CFStringEncoding CFStringGetFastestEncoding(const void *s) { return CFStringGetCharactersPtr(s) ? 0x0100 : 0x0600; }
CFStringEncoding CFStringGetSmallestEncoding(const void *s) { return CFStringGetFastestEncoding(s); }
unsigned char CFStringIsEncodingAvailable(CFStringEncoding e) { return __prism_cf_str_valid_encoding(e) != 0; }
unsigned long CFStringConvertEncodingToNSStringEncoding(CFStringEncoding e) {
    switch (e) { case 0x0600: return 1; case 0x08000100: return 4; case 0x0201: return 5; case 0: return 30; case 0x0100: return 10; default: return e | 0x80000000ul; }
}
CFStringEncoding CFStringConvertNSStringEncodingToEncoding(unsigned long e) {
    switch (e) { case 1: return 0x0600; case 4: return 0x08000100; case 5: return 0x0201; case 30: return 0; case 10: return 0x0100; default: return (CFStringEncoding)(e & 0x7ffffffful); }
}
CFIndex CFStringCompareWithOptions(const void *a, const void *b, CFRange r, CFOptionFlags opts) { return [S(a) compare:S(b) options:opts range:(NSRange){r.location, r.length}]; }
CFIndex CFStringCompare(const void *a, const void *b, CFOptionFlags opts) { return [S(a) compare:S(b) options:opts]; }
unsigned char CFStringFindWithOptions(const void *s, const void *find, CFRange r, CFOptionFlags opts, CFRange *out) {
    NSRange f = [S(s) rangeOfString:S(find) options:opts range:(NSRange){r.location, r.length}];
    if (!f.length) return 0;
    if (out) *out = (CFRange){f.location, f.length};
    return 1;
}
CFRange CFStringFind(const void *s, const void *find, CFOptionFlags opts) {
    NSRange f = [S(s) rangeOfString:S(find) options:opts];
    return f.length ? (CFRange){f.location, f.length} : (CFRange){-1, 0};
}
unsigned char CFStringHasPrefix(const void *s, const void *p) { return [S(s) hasPrefix:S(p)]; }
unsigned char CFStringHasSuffix(const void *s, const void *p) { return [S(s) hasSuffix:S(p)]; }
int CFStringGetIntValue(const void *s) { return [S(s) intValue]; }
double CFStringGetDoubleValue(const void *s) { return [S(s) doubleValue]; }
void CFStringAppend(void *s, const void *o) { [(NSMutableString *)s appendString:S(o)]; }
void CFStringAppendCString(void *s, const char *c, CFStringEncoding e) { id t = (id)CFStringCreateWithCString(0, c, e); [(NSMutableString *)s appendString:t]; [t release]; }
void CFStringAppendCharacters(void *s, const unichar *c, CFIndex n) { id t = (id)CFStringCreateWithCharacters(0, c, n); [(NSMutableString *)s appendString:t]; [t release]; }
void CFStringAppendFormat(void *s, const void *opts, const void *f, ...) {
    (void)opts;
    va_list ap;
    va_start(ap, f);
    [(NSMutableString *)s appendString:_prism_cf_format((id)f, ap)];
    va_end(ap);
}
void CFStringInsert(void *s, CFIndex i, const void *o) { [(id)s insertString:S(o) atIndex:i]; }
void CFStringDelete(void *s, CFRange r) { [(id)s deleteCharactersInRange:(NSRange){r.location, r.length}]; }
void CFStringReplace(void *s, CFRange r, const void *o) { [(id)s replaceCharactersInRange:(NSRange){r.location, r.length} withString:S(o)]; }
void CFStringReplaceAll(void *s, const void *o) { [(id)s setString:S(o)]; }
void CFStringUppercase(void *s, const void *locale) { (void)locale; [(id)s setString:[S(s) uppercaseString]]; }
void CFStringLowercase(void *s, const void *locale) { (void)locale; [(id)s setString:[S(s) lowercaseString]]; }

/* NSLog: the message and a newline on standard error. */
void NSLogv(id format, va_list ap) {
    id s = _prism_cf_format(format, ap);
    const char *u = [s UTF8String];
    fprintf(stderr, "%s\n", u ? u : "");
}
void NSLog(id format, ...) {
    va_list ap;
    va_start(ap, format);
    NSLogv(format, ap);
    va_end(ap);
}
