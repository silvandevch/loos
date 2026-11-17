/* CoreFoundation's base functions and the value classes beyond strings: NSNull, NSNumber (with
 * CFBoolean), NSData, NSArray and NSDictionary, each a class cluster with one concrete __NSCF*
 * class, and the CF functions on them. Storage, hashing and number arithmetic are in src/cf. */
#include "foundation.h"

#define T_ARRAY 19
#define T_BOOLEAN 21
#define T_DATA 20
#define T_DICTIONARY 18
#define T_NULL 16
#define T_NUMBER 22

static void cf_fail(id o, SEL s, const char *what, NSUInteger i, NSUInteger n) {
    if (n == 0) _prism_raise("NSRangeException", "*** -[%s %s]: %s %lu beyond bounds for empty array", object_getClassName(o), sel_getName(s), what, i);
    _prism_raise("NSRangeException", "*** -[%s %s]: %s %lu beyond bounds [0 .. %lu]", object_getClassName(o), sel_getName(s), what, i, n - 1);
}

/* ------------------------------------------------------------------ CFType */

const void *CFRetain(const void *o) { return o ? (const void *)[(id)o retain] : o; }
void CFRelease(const void *o) { if (o) [(id)o release]; }
const void *CFAutorelease(const void *o) { return o ? (const void *)[(id)o autorelease] : o; }
CFIndex CFGetRetainCount(const void *o) { return o ? (CFIndex)[(id)o retainCount] : 0; }
CFTypeID CFGetTypeID(const void *o) { return o ? [(id)o _cfTypeID] : 0; }
Boolean CFEqual(const void *a, const void *b) { return a == b || (a && b && [(id)a isEqual:(id)b]); }
CFHashCode CFHash(const void *o) { return o ? [(id)o hash] : 0; }
const void *CFCopyDescription(const void *o) { return o ? (const void *)[[(id)o description] retain] : 0; }
CFAllocatorRef CFGetAllocator(const void *o) { (void)o; return 0; }
CFTypeID CFTypeGetTypeID(void) { return 1; }
const void *CFCopyTypeIDDescription(CFTypeID t) {
    const char *n = t == 7 ? "CFString" : t == T_ARRAY ? "CFArray" : t == T_DICTIONARY ? "CFDictionary" : t == T_DATA ? "CFData"
        : t == T_NUMBER ? "CFNumber" : t == T_BOOLEAN ? "CFBoolean" : t == T_NULL ? "CFNull" : "CFType";
    return [_prism_cf_string(n) retain];
}
void CFShow(const void *o) {
    id d = _prism_cf_describe((id)o);
    fprintf(stderr, "%s\n", [d UTF8String]);
}
extern CFAllocatorRef kCFAllocatorNull; /* cfstring.m */
/* The callback tables: only whether one was passed matters (NULL: neither retain nor compare). */
const uintptr_t kCFTypeArrayCallBacks[5], kCFTypeDictionaryKeyCallBacks[6], kCFCopyStringDictionaryKeyCallBacks[6],
    kCFTypeDictionaryValueCallBacks[5], kCFTypeSetCallBacks[6], kCFCopyStringSetCallBacks[6];

/* Elements of a collection made with NULL callbacks are raw pointers. */
static inline id el_retain(id self, id o) { return CF(self)->flags & CF_UNRETAINED ? o : [o retain]; }
static inline void el_release(id self, id o) { if (!(CF(self)->flags & CF_UNRETAINED)) [o release]; }
static inline BOOL el_equal(id self, id a, id b) { return a == b || (!(CF(self)->flags & CF_UNRETAINED) && [a isEqual:b]); }
static inline NSUInteger el_hash(id self, id o) { return CF(self)->flags & CF_UNRETAINED ? (NSUInteger)o >> 3 : [o hash]; }

static Class concrete(const char *name, Class *cache) { return *cache ? *cache : (*cache = objc_getClass(name)); }
static id make(Class c, uint64_t flags) { id o = class_createInstance(c, 0); CF(o)->flags = flags; return o; }
/* `+alloc` of a class cluster: the concrete class, unless a program subclassed the abstract one. */
#define CLUSTER(abstract, mutable_, conc)                                                      \
    + (id)allocWithZone:(NSZone *)z {                                                        \
        (void)z;                                                                             \
        if (self == objc_getClass(#abstract)) return make(conc, 0);                          \
        if (self == objc_getClass(#mutable_)) return make(conc, CF_MUTABLE);                 \
        return class_createInstance(self, 0);                                                \
    }                                                                                        \
    + (id)alloc { return [self allocWithZone:0]; }

/* ------------------------------------------------------------------ NSNull */

@implementation NSNull
+ (id)null { extern id kCFNull; return kCFNull; }
- (id)retain { return self; }
- (oneway void)release {}
- (id)autorelease { return self; }
- (id)description { return _prism_cf_string("<null>"); }
- (CFTypeID)_cfTypeID { return T_NULL; }
@end
CFTypeID CFNullGetTypeID(void) { return T_NULL; }

/* ------------------------------------------------------------------ NSNumber */

/* A number keeps its CFNumberType in `flags` and its bits (number.rs) in `data`. */
@interface __NSCFNumber : NSNumber { @public uint64_t _type; uint64_t _bits; uint64_t _pad[2]; }
@end
@interface __NSCFBoolean : NSNumber { @public uint64_t _type; uint64_t _bits; uint64_t _pad[2]; }
@end
static Class number_class;
struct cfobj __kCFBooleanTrue, __kCFBooleanFalse;
static struct cfobj null_obj;
#define true_obj __kCFBooleanTrue
#define false_obj __kCFBooleanFalse
const void *kCFBooleanTrue = &true_obj, *kCFBooleanFalse = &false_obj;
id kCFNull = (id)&null_obj;

static id number(uint64_t type, const void *value) {
    id n = make(concrete("__NSCFNumber", &number_class), type);
    CF(n)->data = (void *)__prism_cf_num_read(type, value);
    return n;
}
/* The compiler's constant numbers: @42 is {isa, encoding, value}, @2.5 is {isa, double}. */
@interface NSConstantIntegerNumber : NSNumber @end
@interface NSConstantDoubleNumber : NSNumber @end
@interface NSConstantFloatNumber : NSNumber @end
static Class const_int, const_double, const_float;
static uint64_t num_type(id o) {
    Class c = object_getClass(o);
    if (c == concrete("NSConstantIntegerNumber", &const_int)) {
        switch (*((const char **)o)[1]) {
        case 'c': return 7;
        case 's': return 8;
        case 'i': return 9;
        case 'l': return 10;
        case 'q': return 11;
        default: return 4; /* unsigned and bool: the value as an SInt64 */
        }
    }
    if (c == concrete("NSConstantDoubleNumber", &const_double)) return 13;
    if (c == concrete("NSConstantFloatNumber", &const_float)) return 12;
    return CF(o)->flags & 0xff;
}
static unsigned long num_bits(id o) {
    Class c = object_getClass(o);
    if (c == concrete("NSConstantIntegerNumber", &const_int)) return ((uint64_t *)o)[2];
    if (c == concrete("NSConstantFloatNumber", &const_float)) return __prism_cf_num_read(12, (uint64_t *)o + 1);
    return ((uint64_t *)o)[c == concrete("NSConstantDoubleNumber", &const_double) ? 1 : 2];
}
#define TYPE(o) num_type((id)(o))
#define BITS(o) num_bits((id)(o))
#define NUM(ctype, cftype, sel, initsel)                                                       \
    + (id)sel:(ctype)v { return [number(cftype, &v) autorelease]; }                          \
    - (id)initsel:(ctype)v { [self release]; return number(cftype, &v); }

@implementation NSNumber
+ (id)allocWithZone:(NSZone *)z { (void)z; return self == objc_getClass("NSNumber") ? (id)&false_obj : class_createInstance(self, 0); }
+ (id)alloc { return [self allocWithZone:0]; }
NUM(char, 7, numberWithChar, initWithChar)
NUM(unsigned char, 7, numberWithUnsignedChar, initWithUnsignedChar)
NUM(short, 8, numberWithShort, initWithShort)
NUM(unsigned short, 9, numberWithUnsignedShort, initWithUnsignedShort)
NUM(int, 9, numberWithInt, initWithInt)
NUM(unsigned int, 4, numberWithUnsignedInt, initWithUnsignedInt)
NUM(long, 10, numberWithLong, initWithLong)
NUM(unsigned long, 4, numberWithUnsignedLong, initWithUnsignedLong)
NUM(long long, 11, numberWithLongLong, initWithLongLong)
NUM(unsigned long long, 4, numberWithUnsignedLongLong, initWithUnsignedLongLong)
NUM(NSInteger, 15, numberWithInteger, initWithInteger)
NUM(NSUInteger, 4, numberWithUnsignedInteger, initWithUnsignedInteger)
NUM(float, 12, numberWithFloat, initWithFloat)
NUM(double, 13, numberWithDouble, initWithDouble)
+ (id)numberWithBool:(BOOL)b { return b ? (id)&true_obj : (id)&false_obj; }
- (id)initWithBool:(BOOL)b { return b ? (id)&true_obj : (id)&false_obj; }
- (char)charValue { return (char)__prism_cf_num_i64(TYPE(self), BITS(self)); }
- (unsigned char)unsignedCharValue { return (unsigned char)__prism_cf_num_i64(TYPE(self), BITS(self)); }
- (short)shortValue { return (short)__prism_cf_num_i64(TYPE(self), BITS(self)); }
- (unsigned short)unsignedShortValue { return (unsigned short)__prism_cf_num_i64(TYPE(self), BITS(self)); }
- (int)intValue { return (int)__prism_cf_num_i64(TYPE(self), BITS(self)); }
- (unsigned int)unsignedIntValue { return (unsigned int)__prism_cf_num_i64(TYPE(self), BITS(self)); }
- (long)longValue { return (long)__prism_cf_num_i64(TYPE(self), BITS(self)); }
- (unsigned long)unsignedLongValue { return (unsigned long)__prism_cf_num_i64(TYPE(self), BITS(self)); }
- (long long)longLongValue { return __prism_cf_num_i64(TYPE(self), BITS(self)); }
- (unsigned long long)unsignedLongLongValue { return (unsigned long long)__prism_cf_num_i64(TYPE(self), BITS(self)); }
- (NSInteger)integerValue { return (NSInteger)__prism_cf_num_i64(TYPE(self), BITS(self)); }
- (NSUInteger)unsignedIntegerValue { return (NSUInteger)__prism_cf_num_i64(TYPE(self), BITS(self)); }
- (float)floatValue { return (float)__prism_cf_num_f64(TYPE(self), BITS(self)); }
- (double)doubleValue { return __prism_cf_num_f64(TYPE(self), BITS(self)); }
- (BOOL)boolValue { return __prism_cf_num_f64(TYPE(self), BITS(self)) != 0; }
- (const char *)objCType {
    switch (TYPE(self)) {
    case 1: case 7: return "c";
    case 2: case 8: return "s";
    case 3: case 9: return "i";
    case 5: case 12: return "f";
    case 6: case 13: case 16: return "d";
    case 10: case 14: case 15: return "q";
    default: return "q";
    }
}
- (NSInteger)compare:(NSNumber *)o { return __prism_cf_num_compare(TYPE(self), BITS(self), TYPE(o), BITS(o)); }
- (BOOL)isEqualToNumber:(NSNumber *)o { return o && [self compare:o] == 0; }
- (BOOL)isEqual:(id)o { return o == self || (cf_is(o, objc_getClass("NSNumber")) && [self compare:o] == 0); }
- (NSUInteger)hash { return __prism_cf_num_hash(TYPE(self), BITS(self)); }
- (id)stringValue { return [self description]; }
- (id)description {
    id s = [NSString string];
    __prism_cf_num_str(TYPE(self), BITS(self), s);
    return s;
}
- (id)copyWithZone:(NSZone *)z { (void)z; return [self retain]; }
- (CFTypeID)_cfTypeID { return T_NUMBER; }
@end
@implementation __NSCFNumber @end
#define CONSTANT_OBJECT                                                                        \
    - (id)retain { return self; }                                                            \
    - (oneway void)release {}                                                                \
    - (id)autorelease { return self; }                                                       \
    - (NSUInteger)retainCount { return ~0ul >> 1; }
@implementation NSConstantIntegerNumber CONSTANT_OBJECT @end
@implementation NSConstantDoubleNumber CONSTANT_OBJECT @end
@implementation NSConstantFloatNumber CONSTANT_OBJECT @end
@implementation __NSCFBoolean
- (id)retain { return self; }
- (oneway void)release {}
- (id)autorelease { return self; }
- (id)description { return _prism_cf_string(self == (id)&true_obj ? "1" : "0"); }
- (CFTypeID)_cfTypeID { return T_BOOLEAN; }
@end

CFTypeID CFNumberGetTypeID(void) { return T_NUMBER; }
CFTypeID CFBooleanGetTypeID(void) { return T_BOOLEAN; }
const void *CFNumberCreate(CFAllocatorRef a, CFIndex type, const void *value) { (void)a; return number(type, value); }
CFIndex CFNumberGetType(const void *n) { return TYPE(n); }
CFIndex CFNumberGetByteSize(const void *n) {
    switch (TYPE(n)) { case 1: case 7: return 1; case 2: case 8: return 2; case 3: case 5: case 9: case 12: return 4; default: return 8; }
}
Boolean CFNumberIsFloatType(const void *n) { CFIndex t = TYPE(n); return t == 5 || t == 6 || t == 12 || t == 13 || t == 16; }
Boolean CFNumberGetValue(const void *n, CFIndex type, void *out) { return __prism_cf_num_write(TYPE(n), BITS(n), type, out) != 0; }
CFIndex CFNumberCompare(const void *a, const void *b, void *ctx) { (void)ctx; return __prism_cf_num_compare(TYPE(a), BITS(a), TYPE(b), BITS(b)); }
Boolean CFBooleanGetValue(const void *b) { return b == &true_obj; }

/* Called by objc_init once the classes exist: the static objects get their class. */
void _prism_cf_init(void) {
    extern void _prism_ns_exception_init(void);
    _prism_ns_exception_init();
    true_obj = (struct cfobj){objc_getClass("__NSCFBoolean"), 9, (void *)1, 0, 0};
    false_obj = (struct cfobj){objc_getClass("__NSCFBoolean"), 9, (void *)0, 0, 0};
    null_obj = (struct cfobj){objc_getClass("NSNull"), 0, 0, 0, 0};
}

/* ------------------------------------------------------------------ NSData */

@interface __NSCFData : NSMutableData { @public uint64_t _flags; void *_data; uint64_t _count, _cap; }
@end
static Class data_class;
#define DATA_CLASS concrete("__NSCFData", &data_class)
static id data_with(const void *b, NSUInteger n, uint64_t flags) {
    id d = make(DATA_CLASS, flags);
    __prism_cf_vec_splice(d, 1, 0, 0, b, n);
    return d;
}

@implementation NSData
CLUSTER(NSData, NSMutableData, DATA_CLASS)
+ (id)data { return [[self new] autorelease]; }
+ (id)dataWithBytes:(const void *)b length:(NSUInteger)n { return [[[self alloc] initWithBytes:b length:n] autorelease]; }
+ (id)dataWithData:(NSData *)d { return [self dataWithBytes:[d bytes] length:[d length]]; }
+ (id)dataWithBytesNoCopy:(void *)b length:(NSUInteger)n { return [[[self alloc] initWithBytesNoCopy:b length:n freeWhenDone:YES] autorelease]; }
+ (id)dataWithBytesNoCopy:(void *)b length:(NSUInteger)n freeWhenDone:(BOOL)f { return [[[self alloc] initWithBytesNoCopy:b length:n freeWhenDone:f] autorelease]; }
- (id)initWithBytes:(const void *)b length:(NSUInteger)n { __prism_cf_vec_splice(self, 1, 0, 0, b, n); return self; }
- (id)initWithBytesNoCopy:(void *)b length:(NSUInteger)n freeWhenDone:(BOOL)f {
    if (CF(self)->flags & CF_MUTABLE) { [self initWithBytes:b length:n]; if (f) free(b); return self; }
    CF(self)->data = b;
    CF(self)->count = CF(self)->cap = n;
    if (f) CF(self)->flags |= CF_OWNED;
    return self;
}
- (id)initWithBytesNoCopy:(void *)b length:(NSUInteger)n { return [self initWithBytesNoCopy:b length:n freeWhenDone:YES]; }
- (id)initWithData:(NSData *)d { return [self initWithBytes:[d bytes] length:[d length]]; }
- (NSUInteger)length { return CF(self)->count; }
- (const void *)bytes { return CF(self)->data; }
- (void)getBytes:(void *)b length:(NSUInteger)n { memcpy(b, CF(self)->data, n < CF(self)->count ? n : CF(self)->count); }
- (void)getBytes:(void *)b range:(NSRange)r {
    if (r.location + r.length > CF(self)->count) cf_fail(self, _cmd, "range", r.location + r.length, CF(self)->count + 1);
    memcpy(b, (char *)CF(self)->data + r.location, r.length);
}
- (id)subdataWithRange:(NSRange)r {
    if (r.location + r.length > CF(self)->count) cf_fail(self, _cmd, "range", r.location + r.length, CF(self)->count + 1);
    return [data_with((char *)CF(self)->data + r.location, r.length, 0) autorelease];
}
- (NSRange)rangeOfData:(NSData *)d options:(NSUInteger)opts range:(NSRange)r {
    long i = __prism_cf_bytes_find((char *)CF(self)->data + r.location, r.length, [d bytes], [d length], (opts & 4) != 0);
    return i < 0 ? (NSRange){NSNotFound, 0} : (NSRange){r.location + i, [d length]};
}
- (BOOL)isEqualToData:(NSData *)d { return d && [d length] == CF(self)->count && !memcmp([d bytes], CF(self)->data, CF(self)->count); }
- (BOOL)isEqual:(id)o { return o == self || (cf_is(o, objc_getClass("NSData")) && [self isEqualToData:o]); }
- (NSUInteger)hash { return __prism_cf_bytes_hash(CF(self)->data, CF(self)->count); }
- (id)copyWithZone:(NSZone *)z { (void)z; return CF(self)->flags & CF_MUTABLE ? data_with(CF(self)->data, CF(self)->count, 0) : [self retain]; }
- (id)mutableCopyWithZone:(NSZone *)z { (void)z; return data_with(CF(self)->data, CF(self)->count, CF_MUTABLE); }
- (id)description {
    id s = [NSMutableString string];
    [s appendString:_prism_cf_string("<")];
    char b[4];
    for (NSUInteger i = 0; i < CF(self)->count; i++) {
        snprintf(b, sizeof b, i && !(i % 4) ? " %02x" : "%02x", ((unsigned char *)CF(self)->data)[i]);
        [s appendString:_prism_cf_string(b)];
    }
    [s appendString:_prism_cf_string(">")];
    return s;
}
- (CFTypeID)_cfTypeID { return T_DATA; }
- (void)dealloc { if (CF(self)->flags & CF_OWNED) free(CF(self)->data); [super dealloc]; }
@end

@implementation NSMutableData
+ (id)dataWithCapacity:(NSUInteger)n { (void)n; return [[self new] autorelease]; }
+ (id)dataWithLength:(NSUInteger)n { return [[[self alloc] initWithLength:n] autorelease]; }
- (id)initWithCapacity:(NSUInteger)n { (void)n; return self; }
- (id)initWithLength:(NSUInteger)n { __prism_cf_vec_resize(self, 1, n); return self; }
- (void *)mutableBytes { if (!CF(self)->data) __prism_cf_vec_resize(self, 1, 0); return CF(self)->data; }
- (void)setLength:(NSUInteger)n { __prism_cf_vec_resize(self, 1, n); }
- (void)increaseLengthBy:(NSUInteger)n { __prism_cf_vec_resize(self, 1, CF(self)->count + n); }
- (void)appendBytes:(const void *)b length:(NSUInteger)n { __prism_cf_vec_splice(self, 1, CF(self)->count, 0, b, n); }
- (void)appendData:(NSData *)d { [self appendBytes:[d bytes] length:[d length]]; }
- (void)replaceBytesInRange:(NSRange)r withBytes:(const void *)b length:(NSUInteger)n { __prism_cf_vec_splice(self, 1, r.location, r.length, b, n); }
- (void)replaceBytesInRange:(NSRange)r withBytes:(const void *)b { __prism_cf_vec_splice(self, 1, r.location, r.length, b, r.length); }
- (void)resetBytesInRange:(NSRange)r { memset((char *)[self mutableBytes] + r.location, 0, r.length); }
- (void)setData:(NSData *)d { __prism_cf_vec_splice(self, 1, 0, CF(self)->count, [d bytes], [d length]); }
@end
@implementation __NSCFData @end

CFTypeID CFDataGetTypeID(void) { return T_DATA; }
const void *CFDataCreate(CFAllocatorRef a, const UInt8 *b, CFIndex n) { (void)a; return data_with(b, n, 0); }
const void *CFDataCreateWithBytesNoCopy(CFAllocatorRef a, const UInt8 *b, CFIndex n, CFAllocatorRef dealloc) {
    (void)a;
    return [make(DATA_CLASS, 0) initWithBytesNoCopy:(void *)b length:n freeWhenDone:dealloc != kCFAllocatorNull];
}
const void *CFDataCreateCopy(CFAllocatorRef a, const void *d) { (void)a; return data_with([(id)d bytes], [(id)d length], 0); }
const void *CFDataCreateMutable(CFAllocatorRef a, CFIndex cap) { (void)a; (void)cap; return make(DATA_CLASS, CF_MUTABLE); }
const void *CFDataCreateMutableCopy(CFAllocatorRef a, CFIndex cap, const void *d) { (void)a; (void)cap; return data_with([(id)d bytes], [(id)d length], CF_MUTABLE); }
CFIndex CFDataGetLength(const void *d) { return [(id)d length]; }
const UInt8 *CFDataGetBytePtr(const void *d) { return [(id)d bytes]; }
UInt8 *CFDataGetMutableBytePtr(const void *d) { return [(id)d mutableBytes]; }
void CFDataGetBytes(const void *d, CFRange r, UInt8 *b) { [(id)d getBytes:b range:(NSRange){r.location, r.length}]; }
void CFDataSetLength(const void *d, CFIndex n) { [(id)d setLength:n]; }
void CFDataIncreaseLength(const void *d, CFIndex n) { [(id)d increaseLengthBy:n]; }
void CFDataAppendBytes(const void *d, const UInt8 *b, CFIndex n) { [(id)d appendBytes:b length:n]; }
void CFDataReplaceBytes(const void *d, CFRange r, const UInt8 *b, CFIndex n) { [(id)d replaceBytesInRange:(NSRange){r.location, r.length} withBytes:b length:n]; }
void CFDataDeleteBytes(const void *d, CFRange r) { [(id)d replaceBytesInRange:(NSRange){r.location, r.length} withBytes:0 length:0]; }

/* ------------------------------------------------------------------ NSArray */

@interface __NSCFArray : NSMutableArray { @public uint64_t _flags; void *_data; uint64_t _count, _cap; }
@end
static Class array_class;
#define ARRAY_CLASS concrete("__NSCFArray", &array_class)
#define ITEMS(o) ((id *)CF(o)->data)
/* @[...] of constants: {isa, count, items}. */
@interface NSConstantArray : NSArray @end
static Class const_array;
static inline uint64_t *count_of(id o) { return object_getClass(o) == concrete("NSConstantArray", &const_array) ? (uint64_t *)o + 1 : &CF(o)->count; }
#define COUNT(o) (*count_of(o))

/* Indented description of a collection element, as -description of arrays and dictionaries prints it. */
static void describe_into(id out, id o, int depth) {
    id d = [o respondsToSelector:sel_registerName("descriptionWithLocale:indent:")]
        ? ((id(*)(id, SEL, id, NSUInteger))objc_msgSend)(o, sel_registerName("descriptionWithLocale:indent:"), nil, depth)
        : _prism_cf_describe(o);
    BOOL bare = cf_is(o, objc_getClass("NSString")) ? [d length] > 0 : YES;
    if (cf_is(o, objc_getClass("NSString")))
        for (NSUInteger i = 0; bare && i < [d length]; i++) {
            unichar c = [d characterAtIndex:i];
            bare = (c >= '0' && c <= '9') || (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_' || c == '.' || c == '$' || c == '/';
        }
    if (!bare) [out appendString:_prism_cf_string("\"")];
    [out appendString:d];
    if (!bare) [out appendString:_prism_cf_string("\"")];
}
static void indent(id out, int depth) { for (int i = 0; i < depth; i++) [out appendString:_prism_cf_string("    ")]; }

static id array_with(id const *items, NSUInteger n, uint64_t flags) {
    id a = make(ARRAY_CLASS, flags);
    __prism_cf_vec_splice(a, sizeof(id), 0, 0, items, n);
    for (NSUInteger i = 0; i < n; i++) el_retain(a, items[i]);
    return a;
}

@implementation NSArray
CLUSTER(NSArray, NSMutableArray, ARRAY_CLASS)
+ (id)array { return [[self new] autorelease]; }
+ (id)arrayWithObject:(id)o { return [[[self alloc] initWithObjects:&o count:1] autorelease]; }
+ (id)arrayWithObjects:(const id *)o count:(NSUInteger)n { return [[[self alloc] initWithObjects:o count:n] autorelease]; }
+ (id)arrayWithArray:(NSArray *)a { return [[[self alloc] initWithArray:a] autorelease]; }
+ (id)arrayWithObjects:(id)first, ... {
    id a = [[self new] autorelease];
    va_list ap;
    va_start(ap, first);
    for (id o = first; o; o = va_arg(ap, id)) [a _prismAppend:o];
    va_end(ap);
    return a;
}
- (id)initWithObjects:(const id *)o count:(NSUInteger)n {
    __prism_cf_vec_splice(self, sizeof(id), 0, 0, o, n);
    for (NSUInteger i = 0; i < n; i++) [o[i] retain];
    return self;
}
- (id)initWithObjects:(id)first, ... {
    va_list ap;
    va_start(ap, first);
    for (id o = first; o; o = va_arg(ap, id)) [self _prismAppend:o];
    va_end(ap);
    return self;
}
- (id)initWithArray:(NSArray *)a {
    NSUInteger n = [a count];
    for (NSUInteger i = 0; i < n; i++) [self _prismAppend:[a objectAtIndex:i]];
    return self;
}
- (id)initWithArray:(NSArray *)a copyItems:(BOOL)copy {
    NSUInteger n = [a count];
    for (NSUInteger i = 0; i < n; i++) {
        id o = copy ? [[a objectAtIndex:i] copy] : [a objectAtIndex:i];
        [self _prismAppend:o];
        if (copy) [o release];
    }
    return self;
}
- (void)_prismAppend:(id)o { o = el_retain(self, o); __prism_cf_vec_splice(self, sizeof(id), COUNT(self), 0, &o, 1); }
- (NSUInteger)count { return COUNT(self); }
- (id)objectAtIndex:(NSUInteger)i { if (i >= COUNT(self)) cf_fail(self, _cmd, "index", i, COUNT(self)); return ITEMS(self)[i]; }
- (id)objectAtIndexedSubscript:(NSUInteger)i { return [self objectAtIndex:i]; }
- (id)firstObject { return COUNT(self) ? ITEMS(self)[0] : nil; }
- (id)lastObject { return COUNT(self) ? ITEMS(self)[COUNT(self) - 1] : nil; }
- (void)getObjects:(id *)out range:(NSRange)r {
    if (r.location + r.length > COUNT(self)) cf_fail(self, _cmd, "range", r.location + r.length, COUNT(self) + 1);
    memcpy(out, ITEMS(self) + r.location, r.length * sizeof(id));
}
- (NSUInteger)indexOfObject:(id)o {
    for (NSUInteger i = 0; i < COUNT(self); i++) if (el_equal(self, ITEMS(self)[i], o)) return i;
    return NSNotFound;
}
- (NSUInteger)indexOfObjectIdenticalTo:(id)o {
    for (NSUInteger i = 0; i < COUNT(self); i++) if (ITEMS(self)[i] == o) return i;
    return NSNotFound;
}
- (BOOL)containsObject:(id)o { return [self indexOfObject:o] != NSNotFound; }
- (id)arrayByAddingObject:(id)o { id a = [self mutableCopy]; [a _prismAppend:o]; return [a autorelease]; }
- (id)arrayByAddingObjectsFromArray:(NSArray *)b { id a = [self mutableCopy]; [a addObjectsFromArray:b]; return [a autorelease]; }
- (id)subarrayWithRange:(NSRange)r {
    if (r.location + r.length > COUNT(self)) cf_fail(self, _cmd, "range", r.location + r.length, COUNT(self) + 1);
    return [array_with(ITEMS(self) + r.location, r.length, 0) autorelease];
}
- (id)componentsJoinedByString:(NSString *)sep {
    id s = [NSMutableString string];
    for (NSUInteger i = 0; i < COUNT(self); i++) {
        if (i) [s appendString:sep];
        [s appendString:_prism_cf_describe(ITEMS(self)[i])];
    }
    return s;
}
- (void)makeObjectsPerformSelector:(SEL)s { for (NSUInteger i = 0; i < COUNT(self); i++) ((void (*)(id, SEL))objc_msgSend)(ITEMS(self)[i], s); }
- (void)enumerateObjectsUsingBlock:(void (^)(id, NSUInteger, BOOL *))b {
    BOOL stop = NO;
    for (NSUInteger i = 0; i < COUNT(self) && !stop; i++) b(ITEMS(self)[i], i, &stop);
}
- (id)sortedArrayUsingFunction:(NSInteger (*)(id, id, void *))f context:(void *)ctx {
    id a = [self mutableCopy];
    id *v = ITEMS(a);
    for (NSUInteger i = 1; i < COUNT(a); i++) /* stable insertion sort */
        for (NSUInteger j = i; j > 0 && f(v[j - 1], v[j], ctx) > 0; j--) { id t = v[j]; v[j] = v[j - 1]; v[j - 1] = t; }
    CF(a)->flags &= ~CF_MUTABLE;
    return [a autorelease];
}
static NSInteger by_block(id a, id b, void *blk) { return ((NSInteger (^)(id, id))blk)(a, b); }
static NSInteger by_selector(id a, id b, void *sel) { return ((NSInteger(*)(id, SEL, id))objc_msgSend)(a, (SEL)sel, b); }
- (id)sortedArrayUsingComparator:(NSInteger (^)(id, id))cmp { return [self sortedArrayUsingFunction:by_block context:(void *)cmp]; }
- (id)sortedArrayUsingSelector:(SEL)s { return [self sortedArrayUsingFunction:by_selector context:(void *)s]; }
- (BOOL)isEqualToArray:(NSArray *)o {
    if (!o || [o count] != COUNT(self)) return NO;
    for (NSUInteger i = 0; i < COUNT(self); i++) if (![ITEMS(self)[i] isEqual:[o objectAtIndex:i]]) return NO;
    return YES;
}
- (BOOL)isEqual:(id)o { return o == self || (cf_is(o, objc_getClass("NSArray")) && [self isEqualToArray:o]); }
- (NSUInteger)hash { return COUNT(self); }
- (NSUInteger)countByEnumeratingWithState:(NSFastEnumerationState *)st objects:(id *)buf count:(NSUInteger)n {
    (void)buf; (void)n;
    if (st->state) return 0;
    st->state = 1;
    st->itemsPtr = ITEMS(self);
    st->mutationsPtr = (unsigned long *)&COUNT(self);
    return COUNT(self);
}
- (id)copyWithZone:(NSZone *)z { (void)z; return CF(self)->flags & CF_MUTABLE ? array_with(ITEMS(self), COUNT(self), CF(self)->flags & CF_UNRETAINED) : [self retain]; }
- (id)mutableCopyWithZone:(NSZone *)z {
    (void)z;
    BOOL constant = count_of(self) != &CF(self)->count;
    return array_with(ITEMS(self), COUNT(self), CF_MUTABLE | (constant ? 0 : CF(self)->flags & CF_UNRETAINED));
}
- (id)descriptionWithLocale:(id)l indent:(NSUInteger)depth {
    (void)l;
    id s = [NSMutableString string];
    indent(s, 0);
    [s appendString:_prism_cf_string("(\n")];
    for (NSUInteger i = 0; i < COUNT(self); i++) {
        indent(s, depth + 1);
        describe_into(s, ITEMS(self)[i], depth + 1);
        [s appendString:_prism_cf_string(i + 1 < COUNT(self) ? ",\n" : "\n")];
    }
    indent(s, depth);
    [s appendString:_prism_cf_string(")")];
    return s;
}
- (id)description { return [self descriptionWithLocale:nil indent:0]; }
- (CFTypeID)_cfTypeID { return T_ARRAY; }
- (void)dealloc {
    for (NSUInteger i = 0; i < COUNT(self); i++) el_release(self, ITEMS(self)[i]);
    if (CF(self)->flags & CF_OWNED) free(CF(self)->data);
    [super dealloc];
}
@end

@implementation NSMutableArray
+ (id)arrayWithCapacity:(NSUInteger)n { (void)n; return [[self new] autorelease]; }
- (id)initWithCapacity:(NSUInteger)n { (void)n; return self; }
- (void)addObject:(id)o { if (!o) _prism_raise("NSInvalidArgumentException", "*** -[%s addObject:]: object cannot be nil", object_getClassName(self)); [self _prismAppend:o]; }
- (void)addObjectsFromArray:(NSArray *)a { NSUInteger n = [a count]; for (NSUInteger i = 0; i < n; i++) [self _prismAppend:[a objectAtIndex:i]]; }
- (void)insertObject:(id)o atIndex:(NSUInteger)i {
    if (i > COUNT(self)) cf_fail(self, _cmd, "index", i, COUNT(self) + 1);
    o = el_retain(self, o);
    __prism_cf_vec_splice(self, sizeof(id), i, 0, &o, 1);
}
- (void)removeObjectAtIndex:(NSUInteger)i {
    if (i >= COUNT(self)) cf_fail(self, _cmd, "index", i, COUNT(self));
    id o = ITEMS(self)[i];
    __prism_cf_vec_splice(self, sizeof(id), i, 1, 0, 0);
    el_release(self, o);
}
- (void)removeLastObject { if (COUNT(self)) [self removeObjectAtIndex:COUNT(self) - 1]; }
- (void)removeObject:(id)o { for (NSUInteger i = COUNT(self); i-- > 0;) if (el_equal(self, ITEMS(self)[i], o)) [self removeObjectAtIndex:i]; }
- (void)removeObjectIdenticalTo:(id)o { for (NSUInteger i = COUNT(self); i-- > 0;) if (ITEMS(self)[i] == o) [self removeObjectAtIndex:i]; }
- (void)removeAllObjects { while (COUNT(self)) [self removeObjectAtIndex:COUNT(self) - 1]; }
- (void)replaceObjectAtIndex:(NSUInteger)i withObject:(id)o {
    if (i >= COUNT(self)) cf_fail(self, _cmd, "index", i, COUNT(self));
    id old = ITEMS(self)[i];
    ITEMS(self)[i] = el_retain(self, o);
    el_release(self, old);
}
- (void)setObject:(id)o atIndexedSubscript:(NSUInteger)i { if (i == COUNT(self)) [self addObject:o]; else [self replaceObjectAtIndex:i withObject:o]; }
- (void)exchangeObjectAtIndex:(NSUInteger)a withObjectAtIndex:(NSUInteger)b { id t = ITEMS(self)[a]; ITEMS(self)[a] = ITEMS(self)[b]; ITEMS(self)[b] = t; }
- (void)sortUsingComparator:(NSInteger (^)(id, id))cmp {
    id s = [self sortedArrayUsingComparator:cmp];
    memcpy(ITEMS(self), ITEMS(s), COUNT(self) * sizeof(id));
}
- (void)sortUsingSelector:(SEL)sel {
    id s = [self sortedArrayUsingSelector:sel];
    memcpy(ITEMS(self), ITEMS(s), COUNT(self) * sizeof(id));
}
@end
@implementation __NSCFArray @end
@implementation NSConstantArray
CONSTANT_OBJECT
- (id)copyWithZone:(NSZone *)z { (void)z; return self; }
- (void)dealloc { if (0) [super dealloc]; }
@end

CFTypeID CFArrayGetTypeID(void) { return T_ARRAY; }
const void *CFArrayCreate(CFAllocatorRef a, const void **v, CFIndex n, const void *cb) { (void)a; return array_with((id const *)v, n, cb ? 0 : CF_UNRETAINED); }
const void *CFArrayCreateCopy(CFAllocatorRef a, const void *arr) { (void)a; return [(id)arr copy]; }
const void *CFArrayCreateMutable(CFAllocatorRef a, CFIndex cap, const void *cb) { (void)a; (void)cap; return make(ARRAY_CLASS, CF_MUTABLE | (cb ? 0 : CF_UNRETAINED)); }
const void *CFArrayCreateMutableCopy(CFAllocatorRef a, CFIndex cap, const void *arr) { (void)a; (void)cap; return [(id)arr mutableCopy]; }
CFIndex CFArrayGetCount(const void *a) { return [(id)a count]; }
const void *CFArrayGetValueAtIndex(const void *a, CFIndex i) { return [(id)a objectAtIndex:i]; }
void CFArrayGetValues(const void *a, CFRange r, const void **out) { [(id)a getObjects:(id *)out range:(NSRange){r.location, r.length}]; }
Boolean CFArrayContainsValue(const void *a, CFRange r, const void *v) {
    for (CFIndex i = r.location; i < r.location + r.length; i++) if (el_equal((id)a, [(id)a objectAtIndex:i], (id)v)) return 1;
    return 0;
}
CFIndex CFArrayGetFirstIndexOfValue(const void *a, CFRange r, const void *v) {
    for (CFIndex i = r.location; i < r.location + r.length; i++) if (el_equal((id)a, [(id)a objectAtIndex:i], (id)v)) return i;
    return -1;
}
void CFArrayAppendValue(const void *a, const void *v) { [(id)a _prismAppend:(id)v]; }
void CFArrayInsertValueAtIndex(const void *a, CFIndex i, const void *v) { [(id)a insertObject:(id)v atIndex:i]; }
void CFArraySetValueAtIndex(const void *a, CFIndex i, const void *v) { [(id)a setObject:(id)v atIndexedSubscript:i]; }
void CFArrayRemoveValueAtIndex(const void *a, CFIndex i) { [(id)a removeObjectAtIndex:i]; }
void CFArrayRemoveAllValues(const void *a) { [(id)a removeAllObjects]; }
void CFArrayApplyFunction(const void *a, CFRange r, void (*f)(const void *, void *), void *ctx) {
    for (CFIndex i = r.location; i < r.location + r.length; i++) f([(id)a objectAtIndex:i], ctx);
}

/* ------------------------------------------------------------------ NSDictionary */

/* The table (collection.rs) holds {hash | occupied, key, value} slots. */
@interface __NSCFDictionary : NSMutableDictionary { @public uint64_t _flags; void *_data; uint64_t _count, _cap; }
@end
static Class dict_class;
#define DICT_CLASS concrete("__NSCFDictionary", &dict_class)
struct slot { uint64_t hash; id key, value; };
#define SLOTS(o) ((struct slot *)CF(o)->data)

static long dict_find(id self, id key) {
    NSUInteger h = el_hash(self, key) >> 1;
    for (long i = __prism_cf_dict_probe(self, h, -1); i >= 0; i = __prism_cf_dict_probe(self, h, i + 1))
        if (el_equal(self, SLOTS(self)[i].key, key)) return i;
    return -1;
}
/* `copy`: the key is copied (NSDictionary) rather than retained (CFDictionary). */
static void dict_set(id self, id key, id value, BOOL copy, BOOL replace, BOOL add) {
    long i = dict_find(self, key);
    if (i >= 0) {
        if (!replace) return;
        id old = SLOTS(self)[i].value;
        SLOTS(self)[i].value = el_retain(self, value);
        el_release(self, old);
        return;
    }
    if (!add) return;
    id k = CF(self)->flags & CF_UNRETAINED ? key : copy ? [key copy] : [key retain];
    __prism_cf_dict_insert(self, el_hash(self, key) >> 1, k, el_retain(self, value));
}
static void dict_remove(id self, id key) {
    long i = dict_find(self, key);
    if (i < 0) return;
    struct slot s = SLOTS(self)[i];
    __prism_cf_dict_remove(self, i);
    el_release(self, s.key);
    el_release(self, s.value);
}
static id dict_with(id const *keys, id const *values, NSUInteger n, uint64_t flags, BOOL copy) {
    id d = make(DICT_CLASS, flags);
    for (NSUInteger i = 0; i < n; i++) dict_set(d, keys[i], values[i], copy, YES, YES);
    return d;
}
#define EACH_SLOT(self, s) for (struct slot *s = SLOTS(self), *s##_end = s + (SLOTS(self) ? CF(self)->cap : 0); s < s##_end; s++) if (s->hash)

@implementation NSDictionary
CLUSTER(NSDictionary, NSMutableDictionary, DICT_CLASS)
+ (id)dictionary { return [[self new] autorelease]; }
+ (id)dictionaryWithObject:(id)o forKey:(id)k { return [[[self alloc] initWithObjects:&o forKeys:&k count:1] autorelease]; }
+ (id)dictionaryWithObjects:(const id *)o forKeys:(const id *)k count:(NSUInteger)n { return [[[self alloc] initWithObjects:o forKeys:k count:n] autorelease]; }
+ (id)dictionaryWithDictionary:(NSDictionary *)d { return [[[self alloc] initWithDictionary:d] autorelease]; }
+ (id)dictionaryWithObjectsAndKeys:(id)first, ... {
    id d = [[self new] autorelease];
    va_list ap;
    va_start(ap, first);
    for (id o = first; o; o = va_arg(ap, id)) dict_set(d, va_arg(ap, id), o, YES, YES, YES);
    va_end(ap);
    return d;
}
+ (id)dictionaryWithObjects:(NSArray *)o forKeys:(NSArray *)k { return [[[self alloc] initWithObjects:o forKeys:k] autorelease]; }
- (id)initWithObjects:(const id *)o forKeys:(const id *)k count:(NSUInteger)n {
    for (NSUInteger i = 0; i < n; i++) dict_set(self, k[i], o[i], YES, YES, YES);
    return self;
}
- (id)initWithObjects:(NSArray *)o forKeys:(NSArray *)k {
    for (NSUInteger i = 0; i < [k count]; i++) dict_set(self, [k objectAtIndex:i], [o objectAtIndex:i], YES, YES, YES);
    return self;
}
- (id)initWithDictionary:(NSDictionary *)d {
    for (id k in [d allKeys]) dict_set(self, k, [d objectForKey:k], YES, YES, YES);
    return self;
}
- (id)initWithObjectsAndKeys:(id)first, ... {
    va_list ap;
    va_start(ap, first);
    for (id o = first; o; o = va_arg(ap, id)) dict_set(self, va_arg(ap, id), o, YES, YES, YES);
    va_end(ap);
    return self;
}
- (NSUInteger)count { return CF(self)->count; }
- (id)objectForKey:(id)k { long i = k ? dict_find(self, k) : -1; return i < 0 ? nil : SLOTS(self)[i].value; }
- (id)objectForKeyedSubscript:(id)k { return [self objectForKey:k]; }
- (id)valueForKey:(id)k { return [self objectForKey:k]; }
- (id)allKeys { id a = [NSMutableArray array]; EACH_SLOT(self, s) [a addObject:s->key]; return a; }
- (id)allValues { id a = [NSMutableArray array]; EACH_SLOT(self, s) [a addObject:s->value]; return a; }
- (id)allKeysForObject:(id)o { id a = [NSMutableArray array]; EACH_SLOT(self, s) if ([s->value isEqual:o]) [a addObject:s->key]; return a; }
- (void)getObjects:(id *)o andKeys:(id *)k count:(NSUInteger)n {
    NSUInteger i = 0;
    EACH_SLOT(self, s) {
        if (i == n) break;
        if (o) o[i] = s->value;
        if (k) k[i] = s->key;
        i++;
    }
}
- (void)enumerateKeysAndObjectsUsingBlock:(void (^)(id, id, BOOL *))b {
    BOOL stop = NO;
    EACH_SLOT(self, s) { b(s->key, s->value, &stop); if (stop) break; }
}
- (NSUInteger)countByEnumeratingWithState:(NSFastEnumerationState *)st objects:(id *)buf count:(NSUInteger)n {
    /* state = the next slot to look at */
    NSUInteger k = 0;
    st->itemsPtr = buf;
    st->mutationsPtr = (unsigned long *)&CF(self)->count;
    while (k < n && SLOTS(self) && st->state < CF(self)->cap) {
        struct slot *s = &SLOTS(self)[st->state++];
        if (s->hash) buf[k++] = s->key;
    }
    return k;
}
- (BOOL)isEqualToDictionary:(NSDictionary *)o {
    if (!o || [o count] != CF(self)->count) return NO;
    EACH_SLOT(self, s) if (![s->value isEqual:[o objectForKey:s->key]]) return NO;
    return YES;
}
- (BOOL)isEqual:(id)o { return o == self || (cf_is(o, objc_getClass("NSDictionary")) && [self isEqualToDictionary:o]); }
- (NSUInteger)hash { return CF(self)->count; }
- (id)copyWithZone:(NSZone *)z {
    (void)z;
    if (!(CF(self)->flags & CF_MUTABLE)) return [self retain];
    id d = make(DICT_CLASS, CF(self)->flags & CF_UNRETAINED);
    EACH_SLOT(self, s) dict_set(d, s->key, s->value, NO, YES, YES);
    return d;
}
- (id)mutableCopyWithZone:(NSZone *)z {
    (void)z;
    id d = make(DICT_CLASS, CF_MUTABLE | (CF(self)->flags & CF_UNRETAINED));
    EACH_SLOT(self, s) dict_set(d, s->key, s->value, NO, YES, YES);
    return d;
}
- (id)descriptionWithLocale:(id)l indent:(NSUInteger)depth {
    (void)l;
    id s = [NSMutableString string];
    [s appendString:_prism_cf_string("{\n")];
    EACH_SLOT(self, e) {
        indent(s, depth + 1);
        describe_into(s, e->key, depth + 1);
        [s appendString:_prism_cf_string(" = ")];
        describe_into(s, e->value, depth + 1);
        [s appendString:_prism_cf_string(";\n")];
    }
    indent(s, depth);
    [s appendString:_prism_cf_string("}")];
    return s;
}
- (id)description { return [self descriptionWithLocale:nil indent:0]; }
- (CFTypeID)_cfTypeID { return T_DICTIONARY; }
- (void)dealloc {
    EACH_SLOT(self, s) { el_release(self, s->key); el_release(self, s->value); }
    if (CF(self)->data) free(CF(self)->data);
    [super dealloc];
}
@end

@implementation NSMutableDictionary
+ (id)dictionaryWithCapacity:(NSUInteger)n { (void)n; return [[self new] autorelease]; }
- (id)initWithCapacity:(NSUInteger)n { (void)n; return self; }
- (void)setObject:(id)o forKey:(id)k {
    if (!o || !k) _prism_raise("NSInvalidArgumentException", "*** -[%s setObject:forKey:]: %s cannot be nil", object_getClassName(self), o ? "key" : "object");
    dict_set(self, k, o, YES, YES, YES);
}
- (void)setObject:(id)o forKeyedSubscript:(id)k { if (o) [self setObject:o forKey:k]; else dict_remove(self, k); }
- (void)setValue:(id)o forKey:(id)k { [self setObject:o forKeyedSubscript:k]; }
- (void)removeObjectForKey:(id)k { dict_remove(self, k); }
- (void)removeAllObjects {
    EACH_SLOT(self, s) { el_release(self, s->key); el_release(self, s->value); }
    __prism_cf_dict_clear(self);
}
- (void)addEntriesFromDictionary:(NSDictionary *)d { for (id k in [d allKeys]) dict_set(self, k, [d objectForKey:k], YES, YES, YES); }
- (void)removeObjectsForKeys:(NSArray *)keys { for (id k in keys) dict_remove(self, k); }
@end
@implementation __NSCFDictionary @end

/* @{...} of constants. */
@interface NSConstantDictionary : NSDictionary { @public uint64_t _options, _n; id *_keys, *_values; }
@end
@implementation NSConstantDictionary
CONSTANT_OBJECT
- (NSUInteger)count { return _n; }
- (id)objectForKey:(id)k {
    for (NSUInteger i = 0; i < _n; i++) if ([_keys[i] isEqual:k]) return _values[i];
    return nil;
}
- (id)allKeys { return [NSArray arrayWithObjects:_keys count:_n]; }
- (id)allValues { return [NSArray arrayWithObjects:_values count:_n]; }
- (id)allKeysForObject:(id)o { id a = [NSMutableArray array]; for (NSUInteger i = 0; i < _n; i++) if ([_values[i] isEqual:o]) [a addObject:_keys[i]]; return a; }
- (void)getObjects:(id *)o andKeys:(id *)k count:(NSUInteger)n {
    for (NSUInteger i = 0; i < _n && i < n; i++) { if (o) o[i] = _values[i]; if (k) k[i] = _keys[i]; }
}
- (void)enumerateKeysAndObjectsUsingBlock:(void (^)(id, id, BOOL *))b {
    BOOL stop = NO;
    for (NSUInteger i = 0; i < _n && !stop; i++) b(_keys[i], _values[i], &stop);
}
- (NSUInteger)countByEnumeratingWithState:(NSFastEnumerationState *)st objects:(id *)buf count:(NSUInteger)n {
    (void)buf; (void)n;
    if (st->state) return 0;
    st->state = 1;
    st->itemsPtr = _keys;
    st->mutationsPtr = (unsigned long *)&_n;
    return _n;
}
- (BOOL)isEqualToDictionary:(NSDictionary *)o {
    if (!o || [o count] != _n) return NO;
    for (NSUInteger i = 0; i < _n; i++) if (![_values[i] isEqual:[o objectForKey:_keys[i]]]) return NO;
    return YES;
}
- (NSUInteger)hash { return _n; }
- (id)copyWithZone:(NSZone *)z { (void)z; return self; }
- (id)mutableCopyWithZone:(NSZone *)z { (void)z; return dict_with(_keys, _values, _n, CF_MUTABLE, YES); }
- (id)descriptionWithLocale:(id)l indent:(NSUInteger)depth {
    (void)l;
    id s = [NSMutableString string];
    [s appendString:_prism_cf_string("{\n")];
    for (NSUInteger i = 0; i < _n; i++) {
        indent(s, depth + 1);
        describe_into(s, _keys[i], depth + 1);
        [s appendString:_prism_cf_string(" = ")];
        describe_into(s, _values[i], depth + 1);
        [s appendString:_prism_cf_string(";\n")];
    }
    indent(s, depth);
    [s appendString:_prism_cf_string("}")];
    return s;
}
- (void)dealloc { if (0) [super dealloc]; }
@end

CFTypeID CFDictionaryGetTypeID(void) { return T_DICTIONARY; }
const void *CFDictionaryCreate(CFAllocatorRef a, const void **k, const void **v, CFIndex n, const void *kcb, const void *vcb) {
    (void)a; (void)vcb;
    return dict_with((id const *)k, (id const *)v, n, kcb ? 0 : CF_UNRETAINED, NO);
}
const void *CFDictionaryCreateCopy(CFAllocatorRef a, const void *d) { (void)a; return [(id)d copy]; }
const void *CFDictionaryCreateMutable(CFAllocatorRef a, CFIndex cap, const void *kcb, const void *vcb) {
    (void)a; (void)cap; (void)vcb;
    return make(DICT_CLASS, CF_MUTABLE | (kcb ? 0 : CF_UNRETAINED));
}
const void *CFDictionaryCreateMutableCopy(CFAllocatorRef a, CFIndex cap, const void *d) { (void)a; (void)cap; return [(id)d mutableCopy]; }
CFIndex CFDictionaryGetCount(const void *d) { return [(id)d count]; }
const void *CFDictionaryGetValue(const void *d, const void *k) { long i = dict_find((id)d, (id)k); return i < 0 ? 0 : SLOTS(d)[i].value; }
Boolean CFDictionaryGetValueIfPresent(const void *d, const void *k, const void **v) {
    long i = dict_find((id)d, (id)k);
    if (i >= 0 && v) *v = SLOTS(d)[i].value;
    return i >= 0;
}
Boolean CFDictionaryContainsKey(const void *d, const void *k) { return dict_find((id)d, (id)k) >= 0; }
Boolean CFDictionaryContainsValue(const void *d, const void *v) { EACH_SLOT(d, s) if (el_equal((id)d, s->value, (id)v)) return 1; return 0; }
void CFDictionaryGetKeysAndValues(const void *d, const void **k, const void **v) { [(id)d getObjects:(id *)v andKeys:(id *)k count:CF(d)->count]; }
void CFDictionarySetValue(const void *d, const void *k, const void *v) { dict_set((id)d, (id)k, (id)v, NO, YES, YES); }
void CFDictionaryAddValue(const void *d, const void *k, const void *v) { dict_set((id)d, (id)k, (id)v, NO, NO, YES); }
void CFDictionaryReplaceValue(const void *d, const void *k, const void *v) { dict_set((id)d, (id)k, (id)v, NO, YES, NO); }
void CFDictionaryRemoveValue(const void *d, const void *k) { dict_remove((id)d, (id)k); }
void CFDictionaryRemoveAllValues(const void *d) { [(id)d removeAllObjects]; }
void CFDictionaryApplyFunction(const void *d, void (*f)(const void *, const void *, void *), void *ctx) { EACH_SLOT(d, s) f(s->key, s->value, ctx); }

/* ------------------------------------------------------------------ NSSet */

/* A set is a dictionary table whose slots hold each object as key (retained once) and value. */
@interface __NSCFSet : NSMutableSet { @public uint64_t _flags; void *_data; uint64_t _count, _cap; }
@end
static Class set_class;
#define SET_CLASS concrete("__NSCFSet", &set_class)
#define T_SET 17

static void set_add(id self, id o) {
    if (dict_find(self, o) >= 0) return;
    o = el_retain(self, o);
    __prism_cf_dict_insert(self, el_hash(self, o) >> 1, o, o);
}
static void set_remove(id self, id o) {
    long i = dict_find(self, o);
    if (i < 0) return;
    id k = SLOTS(self)[i].key;
    __prism_cf_dict_remove(self, i);
    el_release(self, k);
}
static id set_with(id const *v, NSUInteger n, uint64_t flags) {
    id s = make(SET_CLASS, flags | CF_SET);
    for (NSUInteger i = 0; i < n; i++) set_add(s, v[i]);
    return s;
}
static id set_copy(id self, uint64_t flags) {
    id s = make(SET_CLASS, flags | CF_SET | (CF(self)->flags & CF_UNRETAINED));
    if (cf_is(self, SET_CLASS)) { EACH_SLOT(self, e) set_add(s, e->key); }
    else for (id o in self) set_add(s, o);
    return s;
}

@implementation NSSet
+ (id)allocWithZone:(NSZone *)z {
    (void)z;
    if (self == objc_getClass("NSSet")) return make(SET_CLASS, CF_SET);
    if (self == objc_getClass("NSMutableSet")) return make(SET_CLASS, CF_SET | CF_MUTABLE);
    return class_createInstance(self, 0);
}
+ (id)alloc { return [self allocWithZone:0]; }
+ (id)set { return [[self new] autorelease]; }
+ (id)setWithObject:(id)o { return [[[self alloc] initWithObjects:&o count:1] autorelease]; }
+ (id)setWithObjects:(const id *)o count:(NSUInteger)n { return [[[self alloc] initWithObjects:o count:n] autorelease]; }
+ (id)setWithArray:(NSArray *)a { return [[[self alloc] initWithArray:a] autorelease]; }
+ (id)setWithSet:(NSSet *)s { return [[[self alloc] initWithSet:s] autorelease]; }
+ (id)setWithObjects:(id)first, ... {
    id s = [[self new] autorelease];
    va_list ap;
    va_start(ap, first);
    for (id o = first; o; o = va_arg(ap, id)) set_add(s, o);
    va_end(ap);
    return s;
}
- (id)initWithObjects:(const id *)o count:(NSUInteger)n { for (NSUInteger i = 0; i < n; i++) set_add(self, o[i]); return self; }
- (id)initWithArray:(NSArray *)a { for (id o in a) set_add(self, o); return self; }
- (id)initWithSet:(NSSet *)s { for (id o in s) set_add(self, o); return self; }
- (id)initWithObjects:(id)first, ... {
    va_list ap;
    va_start(ap, first);
    for (id o = first; o; o = va_arg(ap, id)) set_add(self, o);
    va_end(ap);
    return self;
}
- (NSUInteger)count { return CF(self)->count; }
- (id)member:(id)o { long i = o ? dict_find(self, o) : -1; return i < 0 ? nil : SLOTS(self)[i].key; }
- (BOOL)containsObject:(id)o { return [self member:o] != nil; }
- (id)anyObject { EACH_SLOT(self, e) return e->key; return nil; }
- (id)allObjects { id a = [NSMutableArray array]; EACH_SLOT(self, e) [a addObject:e->key]; return a; }
- (NSUInteger)countByEnumeratingWithState:(NSFastEnumerationState *)st objects:(id *)buf count:(NSUInteger)n {
    NSUInteger k = 0;
    st->itemsPtr = buf;
    st->mutationsPtr = (unsigned long *)&CF(self)->count;
    while (k < n && SLOTS(self) && st->state < CF(self)->cap) {
        struct slot *s = &SLOTS(self)[st->state++];
        if (s->hash) buf[k++] = s->key;
    }
    return k;
}
- (void)enumerateObjectsUsingBlock:(void (^)(id, BOOL *))b {
    BOOL stop = NO;
    EACH_SLOT(self, e) { b(e->key, &stop); if (stop) break; }
}
- (BOOL)isSubsetOfSet:(NSSet *)o { EACH_SLOT(self, e) if (![o containsObject:e->key]) return NO; return YES; }
- (BOOL)intersectsSet:(NSSet *)o { EACH_SLOT(self, e) if ([o containsObject:e->key]) return YES; return NO; }
- (BOOL)isEqualToSet:(NSSet *)o { return o && [o count] == CF(self)->count && [self isSubsetOfSet:o]; }
- (BOOL)isEqual:(id)o { return o == self || (cf_is(o, objc_getClass("NSSet")) && [self isEqualToSet:o]); }
- (NSUInteger)hash { return CF(self)->count; }
- (id)setByAddingObject:(id)o { id s = set_copy(self, CF_MUTABLE); set_add(s, o); CF(s)->flags &= ~CF_MUTABLE; return [s autorelease]; }
- (id)setByAddingObjectsFromSet:(NSSet *)o { id s = set_copy(self, CF_MUTABLE); for (id x in o) set_add(s, x); CF(s)->flags &= ~CF_MUTABLE; return [s autorelease]; }
- (id)setByAddingObjectsFromArray:(NSArray *)o { id s = set_copy(self, CF_MUTABLE); for (id x in o) set_add(s, x); CF(s)->flags &= ~CF_MUTABLE; return [s autorelease]; }
- (void)makeObjectsPerformSelector:(SEL)sel { EACH_SLOT(self, e) ((void (*)(id, SEL))objc_msgSend)(e->key, sel); }
- (id)copyWithZone:(NSZone *)z { (void)z; return CF(self)->flags & CF_MUTABLE ? set_copy(self, 0) : [self retain]; }
- (id)mutableCopyWithZone:(NSZone *)z { (void)z; return set_copy(self, CF_MUTABLE); }
- (id)descriptionWithLocale:(id)l indent:(NSUInteger)depth {
    (void)l;
    id s = [NSMutableString string];
    [s appendString:_prism_cf_string("{(\n")];
    NSUInteger i = 0;
    EACH_SLOT(self, e) {
        indent(s, depth + 1);
        describe_into(s, e->key, depth + 1);
        [s appendString:_prism_cf_string(++i < CF(self)->count ? ",\n" : "\n")];
    }
    indent(s, depth);
    [s appendString:_prism_cf_string(")}")];
    return s;
}
- (id)description { return [self descriptionWithLocale:nil indent:0]; }
- (CFTypeID)_cfTypeID { return T_SET; }
- (void)dealloc {
    EACH_SLOT(self, e) el_release(self, e->key);
    if (CF(self)->data) free(CF(self)->data);
    [super dealloc];
}
@end

@implementation NSMutableSet
+ (id)setWithCapacity:(NSUInteger)n { (void)n; return [[self new] autorelease]; }
- (id)initWithCapacity:(NSUInteger)n { (void)n; return self; }
- (void)addObject:(id)o {
    if (!o) _prism_raise("NSInvalidArgumentException", "*** -[%s addObject:]: object cannot be nil", object_getClassName(self));
    set_add(self, o);
}
- (void)removeObject:(id)o { if (o) set_remove(self, o); }
- (void)removeAllObjects {
    EACH_SLOT(self, e) el_release(self, e->key);
    __prism_cf_dict_clear(self);
}
- (void)addObjectsFromArray:(NSArray *)a { for (id o in a) set_add(self, o); }
- (void)unionSet:(NSSet *)o { for (id x in o) set_add(self, x); }
- (void)minusSet:(NSSet *)o { for (id x in [o allObjects]) set_remove(self, x); }
- (void)intersectSet:(NSSet *)o { for (id x in [self allObjects]) if (![o containsObject:x]) set_remove(self, x); }
- (void)setSet:(NSSet *)o { [self removeAllObjects]; [self unionSet:o]; }
@end
@implementation __NSCFSet @end

CFTypeID CFSetGetTypeID(void) { return T_SET; }
const void *CFSetCreate(CFAllocatorRef a, const void **v, CFIndex n, const void *cb) { (void)a; return set_with((id const *)v, n, cb ? 0 : CF_UNRETAINED); }
const void *CFSetCreateCopy(CFAllocatorRef a, const void *s) { (void)a; return set_copy((id)s, 0); }
const void *CFSetCreateMutable(CFAllocatorRef a, CFIndex cap, const void *cb) { (void)a; (void)cap; return make(SET_CLASS, CF_SET | CF_MUTABLE | (cb ? 0 : CF_UNRETAINED)); }
const void *CFSetCreateMutableCopy(CFAllocatorRef a, CFIndex cap, const void *s) { (void)a; (void)cap; return set_copy((id)s, CF_MUTABLE); }
CFIndex CFSetGetCount(const void *s) { return [(id)s count]; }
CFIndex CFSetGetCountOfValue(const void *s, const void *v) { return dict_find((id)s, (id)v) >= 0; }
Boolean CFSetContainsValue(const void *s, const void *v) { return dict_find((id)s, (id)v) >= 0; }
const void *CFSetGetValue(const void *s, const void *v) { long i = dict_find((id)s, (id)v); return i < 0 ? 0 : SLOTS(s)[i].key; }
Boolean CFSetGetValueIfPresent(const void *s, const void *v, const void **out) {
    long i = dict_find((id)s, (id)v);
    if (i >= 0 && out) *out = SLOTS(s)[i].key;
    return i >= 0;
}
void CFSetGetValues(const void *s, const void **out) { NSUInteger k = 0; EACH_SLOT(s, e) out[k++] = e->key; }
void CFSetAddValue(const void *s, const void *v) { set_add((id)s, (id)v); }
void CFSetSetValue(const void *s, const void *v) { set_remove((id)s, (id)v); set_add((id)s, (id)v); }
void CFSetReplaceValue(const void *s, const void *v) { if (dict_find((id)s, (id)v) >= 0) CFSetSetValue(s, v); }
void CFSetRemoveValue(const void *s, const void *v) { set_remove((id)s, (id)v); }
void CFSetRemoveAllValues(const void *s) { [(id)s removeAllObjects]; }
void CFSetApplyFunction(const void *s, void (*f)(const void *, void *), void *ctx) { EACH_SLOT(s, e) f(e->key, ctx); }

/* ------------------------------------------------------------------ NSDate */

/* A date keeps its CFAbsoluteTime (seconds since 2001-01-01 UTC) as double bits in `data`. */
@interface __NSCFDate : NSDate { @public uint64_t _flags; double _t; uint64_t _count, _cap; }
@end
static Class date_class;
#define DATE_CLASS concrete("__NSCFDate", &date_class)
#define T_DATE 42
#define EPOCH_2001 978307200.0

double CFAbsoluteTimeGetCurrent(void) {
    struct timeval tv;
    gettimeofday(&tv, 0);
    return tv.tv_sec - EPOCH_2001 + tv.tv_usec / 1e6;
}
static id date_at(double t) {
    id d = make(DATE_CLASS, 0);
    ((__NSCFDate *)d)->_t = t;
    return d;
}

@implementation NSDate
+ (id)allocWithZone:(NSZone *)z { (void)z; return self == objc_getClass("NSDate") ? date_at(0) : class_createInstance(self, 0); }
+ (id)alloc { return [self allocWithZone:0]; }
+ (id)date { return [date_at(CFAbsoluteTimeGetCurrent()) autorelease]; }
+ (id)dateWithTimeIntervalSinceNow:(double)s { return [date_at(CFAbsoluteTimeGetCurrent() + s) autorelease]; }
+ (id)dateWithTimeIntervalSinceReferenceDate:(double)s { return [date_at(s) autorelease]; }
+ (id)dateWithTimeIntervalSince1970:(double)s { return [date_at(s - EPOCH_2001) autorelease]; }
+ (id)distantFuture { return [date_at(63113904000.0) autorelease]; }
+ (id)distantPast { return [date_at(-63114076800.0) autorelease]; }
+ (double)timeIntervalSinceReferenceDate { return CFAbsoluteTimeGetCurrent(); }
- (id)init { ((__NSCFDate *)self)->_t = CFAbsoluteTimeGetCurrent(); return self; }
- (id)initWithTimeIntervalSinceNow:(double)s { ((__NSCFDate *)self)->_t = CFAbsoluteTimeGetCurrent() + s; return self; }
- (id)initWithTimeIntervalSinceReferenceDate:(double)s { ((__NSCFDate *)self)->_t = s; return self; }
- (id)initWithTimeIntervalSince1970:(double)s { ((__NSCFDate *)self)->_t = s - EPOCH_2001; return self; }
- (double)timeIntervalSinceReferenceDate { return ((__NSCFDate *)self)->_t; }
- (double)timeIntervalSince1970 { return [self timeIntervalSinceReferenceDate] + EPOCH_2001; }
- (double)timeIntervalSinceNow { return [self timeIntervalSinceReferenceDate] - CFAbsoluteTimeGetCurrent(); }
- (double)timeIntervalSinceDate:(NSDate *)o { return [self timeIntervalSinceReferenceDate] - [o timeIntervalSinceReferenceDate]; }
- (id)dateByAddingTimeInterval:(double)s { return [date_at([self timeIntervalSinceReferenceDate] + s) autorelease]; }
- (NSInteger)compare:(NSDate *)o {
    double a = [self timeIntervalSinceReferenceDate], b = [o timeIntervalSinceReferenceDate];
    return a < b ? -1 : a > b;
}
- (id)earlierDate:(NSDate *)o { return [self compare:o] <= 0 ? self : o; }
- (id)laterDate:(NSDate *)o { return [self compare:o] >= 0 ? self : o; }
- (BOOL)isEqualToDate:(NSDate *)o { return o && [self timeIntervalSinceReferenceDate] == [o timeIntervalSinceReferenceDate]; }
- (BOOL)isEqual:(id)o { return o == self || (cf_is(o, objc_getClass("NSDate")) && [self isEqualToDate:o]); }
- (NSUInteger)hash { return (NSUInteger)(long)[self timeIntervalSinceReferenceDate]; }
- (id)copyWithZone:(NSZone *)z { (void)z; return [self retain]; }
- (id)description {
    time_t t = (time_t)floor_([self timeIntervalSince1970]);
    struct tm tm;
    gmtime_r(&t, &tm);
    char b[40];
    strftime(b, sizeof b, "%Y-%m-%d %H:%M:%S +0000", &tm);
    return _prism_cf_string(b);
}
- (CFTypeID)_cfTypeID { return T_DATE; }
@end
@implementation __NSCFDate @end

CFTypeID CFDateGetTypeID(void) { return T_DATE; }
const void *CFDateCreate(CFAllocatorRef a, double t) { (void)a; return date_at(t); }
double CFDateGetAbsoluteTime(const void *d) { return [(id)d timeIntervalSinceReferenceDate]; }
double CFDateGetTimeIntervalSinceDate(const void *d, const void *o) { return [(id)d timeIntervalSinceDate:(id)o]; }
CFIndex CFDateCompare(const void *a, const void *b, void *ctx) { (void)ctx; return [(id)a compare:(id)b]; }

/* ------------------------------------------------------------------ NSEnumerator */


@implementation NSEnumerator
- (id)nextObject { return nil; }
- (id)allObjects { id a = [NSMutableArray array]; for (id o; (o = [self nextObject]);) [a addObject:o]; return a; }
- (NSUInteger)countByEnumeratingWithState:(NSFastEnumerationState *)st objects:(id *)buf count:(NSUInteger)n {
    (void)n;
    id o = [self nextObject];
    if (!o) return 0;
    st->state = 1;
    st->itemsPtr = buf;
    st->mutationsPtr = (unsigned long *)self;
    buf[0] = o;
    return 1;
}
@end
/* Walks a snapshot array, forwards or backwards. */
@interface __NSArrayEnumerator : NSEnumerator { @public id _items; NSUInteger _i; BOOL _back; }
@end
@implementation __NSArrayEnumerator
- (id)nextObject {
    NSUInteger n = [_items count];
    if (_i >= n) return nil;
    return [_items objectAtIndex:_back ? n - 1 - _i++ : _i++];
}
- (void)dealloc { [_items release]; [super dealloc]; }
@end
id _prism_enumerator(id items, BOOL back) {
    __NSArrayEnumerator *e = [[__NSArrayEnumerator new] autorelease];
    e->_items = [items copy];
    e->_back = back;
    return e;
}
@implementation NSArray (Enumerators)
- (id)objectEnumerator { return _prism_enumerator(self, NO); }
- (id)reverseObjectEnumerator { return _prism_enumerator(self, YES); }
@end
@implementation NSDictionary (Enumerators)
- (id)keyEnumerator { return _prism_enumerator([self allKeys], NO); }
- (id)objectEnumerator { return _prism_enumerator([self allValues], NO); }
@end
@implementation NSSet (Enumerators)
- (id)objectEnumerator { return _prism_enumerator([self allObjects], NO); }
@end
