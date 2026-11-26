/* Property lists (CFPropertyList, NSPropertyListSerialization) and reading and writing files
 * for NSData, NSString, NSArray and NSDictionary. The formats are in src/cf/plist.rs; the two
 * sides exchange a tree as records (see there). */
#include "foundation.h"

struct rec { uint64_t kind, n, a, b; };
struct rec *__prism_cf_plist_parse(const void *bytes, unsigned long n, unsigned long *count, unsigned long *format);
void *__prism_cf_plist_write(const struct rec *r, unsigned long count, unsigned long format, unsigned long *len);
double CFAbsoluteTimeGetCurrent(void);
const void *CFNumberCreate(CFAllocatorRef, CFIndex, const void *);
extern struct cfobj __kCFBooleanTrue, __kCFBooleanFalse;

enum { IMMUTABLE = 0, MUTABLE_CONTAINERS = 1, MUTABLE_ALL = 2 };

/* A keyed archive's object reference (binary plist UID), as CF represents it. */
#define T_UID 41
@interface __NSCFKeyedArchiverUID : NSObject { @public uint32_t _v; }
@end
@implementation __NSCFKeyedArchiverUID
- (CFTypeID)_cfTypeID { return T_UID; }
- (BOOL)isEqual:(id)o { return o == self || (cf_is(o, objc_getClass("__NSCFKeyedArchiverUID")) && ((__NSCFKeyedArchiverUID *)o)->_v == _v); }
- (NSUInteger)hash { return _v; }
- (id)copyWithZone:(NSZone *)z { (void)z; return [self retain]; }
- (id)description { return [NSString stringWithFormat:CFSTR("<CFKeyedArchiverUID %p [%p]>{value = %u}"), self, (void *)0, _v]; }
@end
id _prism_uid(uint32_t v) {
    __NSCFKeyedArchiverUID *u = [[__NSCFKeyedArchiverUID new] autorelease];
    u->_v = v;
    return u;
}
/* The value of a UID object, or -1 when `o` is not one. */
long _prism_uid_value(id o) { return cf_is(o, objc_getClass("__NSCFKeyedArchiverUID")) ? ((__NSCFKeyedArchiverUID *)o)->_v : -1; }
uint32_t _CFKeyedArchiverUIDGetValue(id o) { return (uint32_t)_prism_uid_value(o); }
id _CFKeyedArchiverUIDCreate(CFAllocatorRef a, uint32_t v) { (void)a; return [_prism_uid(v) retain]; }
enum { FORMAT_OPENSTEP = 1, FORMAT_XML = 100, FORMAT_BINARY = 200 };

/* ------------------------------------------------------------------ records → objects */

static id build(const struct rec *r, unsigned long n, unsigned long *i, unsigned long opts) {
    if (*i >= n) return nil;
    const struct rec *x = &r[(*i)++];
    switch (x->kind) {
    case 1: {
        id d = [NSMutableDictionary dictionary];
        for (uint64_t k = 0; k < x->n; k++) {
            id key = build(r, n, i, IMMUTABLE), v = build(r, n, i, opts);
            if (!key || !v) return nil;
            [d setObject:v forKey:key];
        }
        return opts == IMMUTABLE ? [[d copy] autorelease] : d;
    }
    case 2: {
        id a = [NSMutableArray array];
        for (uint64_t k = 0; k < x->n; k++) {
            id v = build(r, n, i, opts);
            if (!v) return nil;
            [a addObject:v];
        }
        return opts == IMMUTABLE ? [[a copy] autorelease] : a;
    }
    case 3: {
        id s = [[[NSString alloc] initWithBytes:(void *)x->a length:x->n encoding:4] autorelease];
        return opts == MUTABLE_ALL ? [[s mutableCopy] autorelease] : s;
    }
    case 4: { long long v = (long long)x->a; return [(id)CFNumberCreate(0, 4, &v) autorelease]; }
    case 5: { double v; memcpy(&v, &x->a, 8); return [(id)CFNumberCreate(0, 6, &v) autorelease]; }
    case 6: return x->a ? (id)&__kCFBooleanTrue : (id)&__kCFBooleanFalse;
    case 7: { double v; memcpy(&v, &x->a, 8); return [NSDate dateWithTimeIntervalSinceReferenceDate:v]; }
    case 8: {
        id d = [NSData dataWithBytes:(void *)x->a length:x->n];
        return opts == MUTABLE_ALL ? [[d mutableCopy] autorelease] : d;
    }
    case 9: return _prism_uid((uint32_t)x->a);
    }
    return nil;
}

/* A property list from bytes, or nil. */
static id parse(const void *bytes, NSUInteger len, unsigned long opts, unsigned long *format) {
    unsigned long n = 0, fmt = 0, i = 0;
    struct rec *r = __prism_cf_plist_parse(bytes, len, &n, &fmt);
    if (!r) return nil;
    id o = build(r, n, &i, opts);
    free(r);
    if (format) *format = fmt;
    return o;
}

/* ------------------------------------------------------------------ objects → records */

struct out { struct rec *r; unsigned long n, cap; };
static void push(struct out *o, struct rec x) {
    if (o->n == o->cap) {
        o->cap = o->cap ? o->cap * 2 : 64;
        o->r = realloc(o->r, o->cap * sizeof *o->r);
    }
    o->r[o->n++] = x;
}
static BOOL flatten(id v, struct out *o, int depth) {
    if (!v || depth > 512) return NO;
    CFTypeID t = [v _cfTypeID];
    if (t == 7) {
        const char *s = [v UTF8String];
        push(o, (struct rec){3, strlen(s), (uint64_t)s, 0});
    } else if (t == 21) {
        push(o, (struct rec){6, 0, v == (id)&__kCFBooleanTrue, 0});
    } else if (t == 22) {
        const char *ty = [v objCType];
        if (*ty == 'f' || *ty == 'd') {
            double d = [v doubleValue];
            uint64_t b;
            memcpy(&b, &d, 8);
            push(o, (struct rec){5, 0, b, 0});
        } else {
            push(o, (struct rec){4, 0, (uint64_t)[v longLongValue], 0});
        }
    } else if (t == T_UID) {
        push(o, (struct rec){9, 0, ((__NSCFKeyedArchiverUID *)v)->_v, 0});
    } else if (t == 42) {
        double d = [v timeIntervalSinceReferenceDate];
        uint64_t b;
        memcpy(&b, &d, 8);
        push(o, (struct rec){7, 0, b, 0});
    } else if (t == 20) {
        push(o, (struct rec){8, [v length], (uint64_t)[v bytes], 0});
    } else if (t == 19) {
        push(o, (struct rec){2, [v count], 0, 0});
        for (id x in v) if (!flatten(x, o, depth + 1)) return NO;
    } else if (t == 18) {
        push(o, (struct rec){1, [v count], 0, 0});
        for (id k in v) {
            if ([k _cfTypeID] != 7) return NO;
            if (!flatten(k, o, depth + 1) || !flatten([v objectForKey:k], o, depth + 1)) return NO;
        }
    } else {
        return NO;
    }
    return YES;
}
/* Serialized bytes as an NSData (autoreleased), or nil if `v` is not a property list. */
static id serialize(id v, unsigned long format) {
    struct out o = {0};
    id data = nil;
    @autoreleasepool {
        if (flatten(v, &o, 0)) {
            unsigned long len = 0;
            void *p = __prism_cf_plist_write(o.r, o.n, format == FORMAT_BINARY || format == FORMAT_OPENSTEP ? format : FORMAT_XML, &len);
            if (p) data = [[NSData alloc] initWithBytesNoCopy:p length:len freeWhenDone:YES];
        }
    }
    free(o.r);
    return [data autorelease];
}

/* ------------------------------------------------------------------ files */

static id read_file(id path) {
    int fd = open([path fileSystemRepresentation], 0);
    if (fd < 0) return nil;
    id d = [NSMutableData data];
    char buf[16384];
    long n;
    while ((n = read(fd, buf, sizeof buf)) > 0) [d appendBytes:buf length:n];
    close(fd);
    return n < 0 ? nil : d;
}
static BOOL write_file(id path, id data, BOOL atomically) {
    const char *p = [path fileSystemRepresentation];
    char tmp[1100];
    snprintf(tmp, sizeof tmp, "%s.prism~", p);
    const char *target = atomically ? tmp : p;
    int fd = open(target, 0x601 /* O_WRONLY|O_CREAT|O_TRUNC */, 0644);
    if (fd < 0) return NO;
    const char *b = [data bytes];
    NSUInteger left = [data length];
    while (left) {
        long n = write(fd, b, left);
        if (n <= 0) { close(fd); return NO; }
        b += n;
        left -= n;
    }
    close(fd);
    return !atomically || rename(tmp, p) == 0;
}
id _prism_plist_from_file(id path) {
    id d = read_file(path);
    return d ? parse([d bytes], [d length], IMMUTABLE, 0) : nil;
}

/* ------------------------------------------------------------------ CFPropertyList */

id CFPropertyListCreateWithData(CFAllocatorRef a, id data, unsigned long opts, CFIndex *format, id *error) {
    (void)a;
    unsigned long f = 0;
    id o = parse([data bytes], [data length], opts, &f);
    if (format) *format = f;
    if (error) *error = nil;
    return [o retain];
}
id CFPropertyListCreateFromXMLData(CFAllocatorRef a, id data, unsigned long opts, id *err) { return CFPropertyListCreateWithData(a, data, opts, 0, err); }
id CFPropertyListCreateData(CFAllocatorRef a, id plist, CFIndex format, unsigned long opts, id *error) {
    (void)a; (void)opts;
    if (error) *error = nil;
    return [serialize(plist, format) retain];
}
id CFPropertyListCreateXMLData(CFAllocatorRef a, id plist) { (void)a; return [serialize(plist, FORMAT_XML) retain]; }
id CFPropertyListCreateDeepCopy(CFAllocatorRef a, id plist, unsigned long opts) {
    (void)a;
    id d = serialize(plist, FORMAT_BINARY);
    return d ? [parse([d bytes], [d length], opts, 0) retain] : nil;
}
Boolean CFPropertyListIsValid(id plist, CFIndex format) {
    (void)format;
    struct out o = {0};
    BOOL ok = flatten(plist, &o, 0);
    free(o.r);
    return ok;
}

@implementation NSPropertyListSerialization
+ (id)propertyListWithData:(id)data options:(NSUInteger)opts format:(NSUInteger *)format error:(id *)error {
    unsigned long f = 0;
    id o = parse([data bytes], [data length], opts, &f);
    if (format) *format = f;
    if (error) *error = nil;
    return o;
}
+ (id)propertyListFromData:(id)data mutabilityOption:(NSUInteger)opts format:(NSUInteger *)format errorDescription:(id *)err {
    id o = [self propertyListWithData:data options:opts format:format error:0];
    if (err) *err = o ? nil : CFSTR("Cannot parse a property list from the data");
    return o;
}
+ (id)dataWithPropertyList:(id)plist format:(NSUInteger)format options:(NSUInteger)opts error:(id *)error {
    (void)opts;
    if (error) *error = nil;
    return serialize(plist, format);
}
+ (id)dataFromPropertyList:(id)plist format:(NSUInteger)format errorDescription:(id *)err {
    id d = serialize(plist, format);
    if (err) *err = d ? nil : CFSTR("Property list invalid for format");
    return d;
}
+ (BOOL)propertyListIsValidForFormat:(NSUInteger)format { return format == FORMAT_XML || format == FORMAT_BINARY || format == FORMAT_OPENSTEP; }
+ (BOOL)propertyList:(id)plist isValidForFormat:(NSUInteger)format { return CFPropertyListIsValid(plist, format); }
@end

/* ------------------------------------------------------------------ contents of files */

@implementation NSData (Files)
+ (id)dataWithContentsOfFile:(id)path { return [[read_file(path) copy] autorelease]; }
+ (id)dataWithContentsOfFile:(id)path options:(NSUInteger)o error:(id *)e { (void)o; if (e) *e = nil; return [self dataWithContentsOfFile:path]; }
+ (id)dataWithContentsOfURL:(id)url {
    extern id _prism_http_get(id, id *);
    return [url isFileURL] ? [self dataWithContentsOfFile:[url path]] : url ? _prism_http_get(url, 0) : nil;
}
- (id)initWithContentsOfFile:(id)path {
    id d = read_file(path);
    if (!d) { [self release]; return nil; }
    return [self initWithBytes:[d bytes] length:[d length]];
}
- (id)initWithContentsOfURL:(id)url { return [url isFileURL] ? [self initWithContentsOfFile:[url path]] : ([self release], nil); }
- (BOOL)writeToFile:(id)path atomically:(BOOL)a { return write_file(path, self, a); }
- (BOOL)writeToFile:(id)path options:(NSUInteger)o error:(id *)e { if (e) *e = nil; return write_file(path, self, o & 1); }
- (BOOL)writeToURL:(id)url atomically:(BOOL)a { return [url isFileURL] && write_file([url path], self, a); }
@end

@implementation NSDictionary (StringsFile)
/* `"key" = "value";` lines, the format of .strings files. */
- (id)descriptionInStringsFileFormat {
    id d = serialize(self, FORMAT_OPENSTEP);
    if (!d) return nil;
    id s = [[[NSString alloc] initWithBytes:[d bytes] length:[d length] encoding:4] autorelease];
    /* drop the braces and the indentation of the top-level dictionary */
    id out = [NSMutableString string];
    for (id line in [s componentsSeparatedByString:CFSTR("\n")]) {
        if ([line isEqual:CFSTR("{")] || [line isEqual:CFSTR("}")] || ![line length]) continue;
        [out appendString:[line hasPrefix:CFSTR("    ")] ? [line substringFromIndex:4] : line];
        [out appendString:CFSTR("\n")];
    }
    return out;
}
@end

@implementation NSString (Files)
+ (id)stringWithContentsOfFile:(id)path encoding:(NSUInteger)enc error:(id *)e {
    id d = read_file(path);
    if (e) *e = nil;
    return d ? [[[NSString alloc] initWithBytes:[d bytes] length:[d length] encoding:enc] autorelease] : nil;
}
+ (id)stringWithContentsOfFile:(id)path usedEncoding:(NSUInteger *)enc error:(id *)e {
    if (enc) *enc = 4;
    return [self stringWithContentsOfFile:path encoding:4 error:e];
}
+ (id)stringWithContentsOfFile:(id)path { return [self stringWithContentsOfFile:path encoding:4 error:0]; }
+ (id)stringWithContentsOfURL:(id)url encoding:(NSUInteger)enc error:(id *)e {
    extern id _prism_http_get(id, id *);
    if ([url isFileURL]) return [self stringWithContentsOfFile:[url path] encoding:enc error:e];
    id d = url ? _prism_http_get(url, e) : nil;
    return d ? [[[self alloc] initWithBytes:[d bytes] length:[d length] encoding:enc] autorelease] : nil;
}
- (id)initWithContentsOfFile:(id)path encoding:(NSUInteger)enc error:(id *)e {
    id d = read_file(path);
    if (e) *e = nil;
    return d ? [self initWithBytes:[d bytes] length:[d length] encoding:enc] : ([self release], nil);
}
- (BOOL)writeToFile:(id)path atomically:(BOOL)a encoding:(NSUInteger)enc error:(id *)e {
    if (e) *e = nil;
    const char *s = [self cStringUsingEncoding:enc];
    return s && write_file(path, [NSData dataWithBytes:s length:strlen(s)], a);
}
- (BOOL)writeToURL:(id)url atomically:(BOOL)a encoding:(NSUInteger)enc error:(id *)e { return [url isFileURL] && [self writeToFile:[url path] atomically:a encoding:enc error:e]; }
- (id)propertyList {
    const char *s = [self UTF8String];
    return parse(s, strlen(s), IMMUTABLE, 0);
}
- (id)propertyListFromStringsFileFormat { return [self propertyList]; }
@end

#define COLLECTION_FILES(cls, typeid)                                                          \
    @implementation cls (Files)                                                                \
    + (id)typeid##WithContentsOfFile:(id)path {                                                \
        id o = _prism_plist_from_file(path);                                                   \
        if (!cf_is(o, objc_getClass(#cls))) return nil;                                        \
        return [self isSubclassOfClass:objc_getClass("NSMutable" #typeid)] || [self class] != objc_getClass(#cls) ? [[o mutableCopy] autorelease] : o; \
    }                                                                                          \
    + (id)typeid##WithContentsOfURL:(id)url { return [url isFileURL] ? [self typeid##WithContentsOfFile:[url path]] : nil; } \
    - (id)initWithContentsOfFile:(id)path {                                                    \
        id o = _prism_plist_from_file(path);                                                   \
        [self release];                                                                        \
        return cf_is(o, objc_getClass(#cls)) ? [o mutableCopy] : nil;                          \
    }                                                                                          \
    - (BOOL)writeToFile:(id)path atomically:(BOOL)a {                                          \
        id d = serialize(self, FORMAT_XML);                                                    \
        return d && write_file(path, d, a);                                                    \
    }                                                                                          \
    - (BOOL)writeToURL:(id)url atomically:(BOOL)a { return [url isFileURL] && [self writeToFile:[url path] atomically:a]; } \
    @end
COLLECTION_FILES(NSDictionary, dictionary)
COLLECTION_FILES(NSArray, array)

id _prism_plist_data(id plist, unsigned long format) { return serialize(plist, format); }
BOOL _prism_write_file(id path, id data, BOOL atomically) { return write_file(path, data, atomically); }
