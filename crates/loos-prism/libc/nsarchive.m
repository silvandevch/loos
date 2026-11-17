/* NSCoder, NSKeyedArchiver and NSKeyedUnarchiver, writing and reading Apple's archive format: a
 * binary property list {$archiver, $version, $top, $objects} whose objects refer to one another
 * by UID. Strings, numbers and data sit in $objects as they are; other objects are dictionaries of
 * their encoded keys plus a $class reference to {$classname, $classes}. Also the coding of the
 * built-in classes. */
#include "foundation.h"

extern id const NSCocoaErrorDomain;
id const NSKeyedArchiveRootObjectKey = CFSTR("root");
id const NSInvalidArchiveOperationException = CFSTR("NSInvalidArchiveOperationException");
id const NSInvalidUnarchiveOperationException = CFSTR("NSInvalidUnarchiveOperationException");

@interface NSCoder : NSObject @end
@implementation NSCoder
- (BOOL)allowsKeyedCoding { return NO; }
- (BOOL)requiresSecureCoding { return NO; }
- (NSInteger)versionForClassName:(id)n { (void)n; return 0; }
- (unsigned)systemVersion { return 1000; }
- (void)failWithError:(id)e { (void)e; }
- (id)error { return nil; }
@end

/* The class an object is archived as: the public class of Prism's concrete ones. */
@implementation NSObject (Archiving)
- (Class)classForCoder { return object_getClass(self); }
- (Class)classForKeyedArchiver { return [self classForCoder]; }
- (id)replacementObjectForCoder:(id)c { (void)c; return self; }
- (id)replacementObjectForKeyedArchiver:(id)c { return [self replacementObjectForCoder:c]; }
- (id)awakeAfterUsingCoder:(id)c { (void)c; return self; }
+ (id)classFallbacksForKeyedArchiver { return [NSArray array]; }
+ (NSInteger)version { return 0; }
+ (void)setVersion:(NSInteger)v { (void)v; }
@end
static BOOL is_mutable(id o) { return (CF(o)->flags & CF_MUTABLE) != 0; }
@implementation NSString (Archiving)
- (Class)classForCoder { return objc_getClass(cf_is(self, objc_getClass("__NSCFString")) && is_mutable(self) ? "NSMutableString" : "NSString"); }
@end
@implementation NSArray (Archiving)
- (Class)classForCoder { return objc_getClass(cf_is(self, objc_getClass("__NSCFArray")) && is_mutable(self) ? "NSMutableArray" : "NSArray"); }
@end
@implementation NSDictionary (Archiving)
- (Class)classForCoder { return objc_getClass(cf_is(self, objc_getClass("__NSCFDictionary")) && is_mutable(self) ? "NSMutableDictionary" : "NSDictionary"); }
@end
@implementation NSSet (Archiving)
- (Class)classForCoder { return objc_getClass(cf_is(self, objc_getClass("__NSCFSet")) && is_mutable(self) ? "NSMutableSet" : "NSSet"); }
@end
@implementation NSData (Archiving)
- (Class)classForCoder { return objc_getClass(cf_is(self, objc_getClass("__NSCFData")) && is_mutable(self) ? "NSMutableData" : "NSData"); }
@end
@implementation NSNumber (Archiving)
- (Class)classForCoder { return objc_getClass("NSNumber"); }
@end
@implementation NSDate (Archiving)
- (Class)classForCoder { return objc_getClass("NSDate"); }
@end

/* ------------------------------------------------------------------ NSKeyedArchiver */

@interface NSKeyedArchiver : NSCoder {
@public
    id _objects;   /* $objects */
    id _ids;       /* object address (NSNumber) → UID number */
    id _classes;   /* class name → UID number */
    id _stack;     /* the dictionaries being filled, innermost last */
    id _top;
    id _data;
    BOOL _secure, _finished;
    NSUInteger _format;
    id _delegate;
}
- (id)_uidFor:(id)o;
@end

@implementation NSKeyedArchiver
- (id)initRequiringSecureCoding:(BOOL)secure {
    _objects = [[NSMutableArray alloc] initWithObjects:CFSTR("$null"), nil];
    _ids = [NSMutableDictionary new];
    _classes = [NSMutableDictionary new];
    _stack = [NSMutableArray new];
    _top = [NSMutableDictionary new];
    [_stack addObject:_top];
    _secure = secure;
    _format = 200;
    return self;
}
- (id)init { return [self initRequiringSecureCoding:NO]; }
- (id)initForWritingWithMutableData:(id)data {
    [self initRequiringSecureCoding:NO];
    _data = [data retain];
    return self;
}
+ (id)archivedDataWithRootObject:(id)root requiringSecureCoding:(BOOL)secure error:(id *)error {
    NSKeyedArchiver *a = [[[self alloc] initRequiringSecureCoding:secure] autorelease];
    @try {
        [a encodeObject:root forKey:NSKeyedArchiveRootObjectKey];
    } @catch (id e) {
        if (error) *error = [NSError errorWithDomain:NSCocoaErrorDomain code:4866 userInfo:[NSDictionary dictionaryWithObject:[e reason] forKey:CFSTR("NSDebugDescription")]];
        return nil;
    }
    if (error) *error = nil;
    return [a encodedData];
}
+ (id)archivedDataWithRootObject:(id)root { return [self archivedDataWithRootObject:root requiringSecureCoding:NO error:0]; }
+ (BOOL)archiveRootObject:(id)root toFile:(id)path {
    id d = [self archivedDataWithRootObject:root];
    return d && [d writeToFile:path atomically:YES];
}
- (BOOL)allowsKeyedCoding { return YES; }
- (BOOL)requiresSecureCoding { return _secure; }
- (void)setRequiresSecureCoding:(BOOL)s { _secure = s; }
- (NSUInteger)outputFormat { return _format; }
- (void)setOutputFormat:(NSUInteger)f { _format = f; }
- (id)delegate { return _delegate; }
- (void)setDelegate:(id)d { _delegate = d; }

/* The UID of {$classname, $classes} for `c`. */
- (id)_classUID:(Class)c {
    id name = _prism_cf_string(class_getName(c));
    id n = [_classes objectForKey:name];
    if (!n) {
        id chain = [NSMutableArray array];
        for (Class k = c; k; k = class_getSuperclass(k)) [chain addObject:_prism_cf_string(class_getName(k))];
        n = [NSNumber numberWithUnsignedInteger:[_objects count]];
        [_objects addObject:[NSDictionary dictionaryWithObjectsAndKeys:name, CFSTR("$classname"), chain, CFSTR("$classes"), nil]];
        [_classes setObject:n forKey:name];
    }
    return _prism_uid([n unsignedIntValue]);
}
/* Encode `o` (once) and return its UID. */
- (id)_uidFor:(id)o {
    if (!o) return _prism_uid(0);
    o = [o replacementObjectForKeyedArchiver:self];
    if (!o) return _prism_uid(0);
    id key = [NSNumber numberWithUnsignedLongLong:(uintptr_t)o];
    id seen = [_ids objectForKey:key];
    if (seen) return _prism_uid([seen unsignedIntValue]);
    NSUInteger me = [_objects count];
    [_ids setObject:[NSNumber numberWithUnsignedInteger:me] forKey:key];
    Class cls = [o classForKeyedArchiver];
    CFTypeID t = [o _cfTypeID];
    /* plain values are stored as they are */
    if ((t == 7 && cls == objc_getClass("NSString")) || t == 22 || t == 21 || (t == 20 && cls == objc_getClass("NSData"))) {
        [_objects addObject:t == 7 ? [[o copy] autorelease] : o];
        return _prism_uid(me);
    }
    if (_secure && ![[o class] respondsToSelector:@selector(supportsSecureCoding)])
        _prism_raise("NSInvalidArgumentException", "This coder requires that objects conform to NSSecureCoding; %s does not", class_getName(cls));
    if (![o respondsToSelector:@selector(encodeWithCoder:)])
        _prism_raise("NSInvalidArgumentException", "-[%s encodeWithCoder:]: unrecognized selector sent to instance %p", object_getClassName(o), (void *)o);
    id dict = [NSMutableDictionary dictionary];
    [_objects addObject:dict];
    [_stack addObject:dict];
    [o encodeWithCoder:self];
    [_stack removeLastObject];
    [dict setObject:[self _classUID:cls] forKey:CFSTR("$class")];
    return _prism_uid(me);
}
- (void)_set:(id)v forKey:(id)k {
    if (_finished) _prism_raise("NSInvalidArgumentException", "*** -[NSKeyedArchiver encode...]: archive already finished, cannot encode anything more");
    [[_stack lastObject] setObject:v forKey:k];
}
- (void)encodeObject:(id)o forKey:(id)k { [self _set:[self _uidFor:o] forKey:k]; }
- (void)encodeConditionalObject:(id)o forKey:(id)k { [self encodeObject:o forKey:k]; }
- (void)encodeRootObject:(id)o { [self encodeObject:o forKey:NSKeyedArchiveRootObjectKey]; }
- (void)encodeBool:(BOOL)v forKey:(id)k { [self _set:[NSNumber numberWithBool:v] forKey:k]; }
- (void)encodeInt:(int)v forKey:(id)k { [self _set:[NSNumber numberWithLongLong:v] forKey:k]; }
- (void)encodeInt32:(int)v forKey:(id)k { [self _set:[NSNumber numberWithLongLong:v] forKey:k]; }
- (void)encodeInt64:(long long)v forKey:(id)k { [self _set:[NSNumber numberWithLongLong:v] forKey:k]; }
- (void)encodeInteger:(NSInteger)v forKey:(id)k { [self _set:[NSNumber numberWithLongLong:v] forKey:k]; }
- (void)encodeFloat:(float)v forKey:(id)k { [self _set:[NSNumber numberWithDouble:v] forKey:k]; }
- (void)encodeDouble:(double)v forKey:(id)k { [self _set:[NSNumber numberWithDouble:v] forKey:k]; }
- (void)encodeBytes:(const void *)b length:(NSUInteger)n forKey:(id)k { [self _set:[NSData dataWithBytes:b length:n] forKey:k]; }
/* An array of references, as collections store their members. */
- (void)_encodeObjects:(id)items forKey:(id)k {
    id uids = [NSMutableArray array];
    for (id o in items) [uids addObject:[self _uidFor:o]];
    [self _set:uids forKey:k];
}
- (void)finishEncoding {
    if (_finished) return;
    _finished = YES;
    id archive = [NSDictionary dictionaryWithObjectsAndKeys:CFSTR("NSKeyedArchiver"), CFSTR("$archiver"), [NSNumber numberWithInt:100000],
        CFSTR("$version"), _top, CFSTR("$top"), _objects, CFSTR("$objects"), nil];
    id bytes = _prism_plist_data(archive, _format == 100 ? 100 : 200);
    if (!_data) _data = [NSMutableData new];
    [_data setData:bytes];
}
- (id)encodedData { [self finishEncoding]; return [[_data copy] autorelease]; }
- (void)dealloc {
    [_objects release]; [_ids release]; [_classes release]; [_stack release]; [_top release]; [_data release];
    [super dealloc];
}
@end

/* ------------------------------------------------------------------ NSKeyedUnarchiver */

@interface NSKeyedUnarchiver : NSCoder {
@public
    id _objects, _top, _stack, _decoded; /* _decoded: UID number → object */
    id _allowed;                       /* secure decoding: the allowed classes, or nil */
    id _names;                         /* class name substitutions */
    BOOL _secure;
    id _error;
}
- (id)_decodeUID:(long)n;
@end

static id archive_error(NSInteger code, const char *why) {
    return [NSError errorWithDomain:NSCocoaErrorDomain code:code userInfo:[NSDictionary dictionaryWithObject:_prism_cf_string(why) forKey:CFSTR("NSDebugDescription")]];
}

@implementation NSKeyedUnarchiver
- (id)initForReadingFromData:(id)data error:(id *)error {
    id plist = [NSPropertyListSerialization propertyListWithData:data options:0 format:0 error:0];
    id objects = cf_is(plist, objc_getClass("NSDictionary")) ? [plist objectForKey:CFSTR("$objects")] : nil;
    id top = [plist objectForKey:CFSTR("$top")];
    if (!cf_is(objects, objc_getClass("NSArray")) || !cf_is(top, objc_getClass("NSDictionary"))) {
        if (error) *error = archive_error(4864, "The data isn’t in the correct format.");
        [self release];
        return nil;
    }
    _objects = [objects retain];
    _top = [top retain];
    _stack = [[NSMutableArray alloc] initWithObjects:top, nil];
    _decoded = [NSMutableDictionary new];
    _names = [NSMutableDictionary new];
    _secure = YES;
    if (error) *error = nil;
    return self;
}
- (id)initForReadingWithData:(id)data {
    self = [self initForReadingFromData:data error:0];
    if (self) _secure = NO;
    else _prism_raise("NSInvalidArgumentException", "*** -[NSKeyedUnarchiver initForReadingWithData:]: incomprehensible archive");
    return self;
}
+ (id)unarchivedObjectOfClasses:(id)classes fromData:(id)data error:(id *)error {
    NSKeyedUnarchiver *u = [[[self alloc] initForReadingFromData:data error:error] autorelease];
    if (!u) return nil;
    u->_allowed = [classes retain];
    id o = nil;
    @try {
        o = [u decodeObjectForKey:NSKeyedArchiveRootObjectKey];
    } @catch (id e) {
        if (error) *error = archive_error(4864, [[e reason] UTF8String]);
        return nil;
    }
    if (u->_error) { if (error) *error = u->_error; return nil; }
    return o;
}
+ (id)unarchivedObjectOfClass:(Class)c fromData:(id)data error:(id *)error {
    return [self unarchivedObjectOfClasses:[NSSet setWithObject:c] fromData:data error:error];
}
+ (id)unarchivedArrayOfObjectsOfClass:(Class)c fromData:(id)data error:(id *)error {
    return [self unarchivedObjectOfClasses:[NSSet setWithObjects:objc_getClass("NSArray"), c, nil] fromData:data error:error];
}
+ (id)unarchivedDictionaryWithKeysOfClass:(Class)k objectsOfClass:(Class)v fromData:(id)data error:(id *)error {
    return [self unarchivedObjectOfClasses:[NSSet setWithObjects:objc_getClass("NSDictionary"), k, v, nil] fromData:data error:error];
}
+ (id)unarchiveTopLevelObjectWithData:(id)data error:(id *)error {
    NSKeyedUnarchiver *u = [[[self alloc] initForReadingFromData:data error:error] autorelease];
    if (!u) return nil;
    u->_secure = NO;
    return [u decodeObjectForKey:NSKeyedArchiveRootObjectKey];
}
+ (id)unarchiveObjectWithData:(id)data {
    NSKeyedUnarchiver *u = [[[self alloc] initForReadingWithData:data] autorelease];
    return [u decodeObjectForKey:NSKeyedArchiveRootObjectKey];
}
+ (id)unarchiveObjectWithFile:(id)path {
    id d = [NSData dataWithContentsOfFile:path];
    return d ? [self unarchiveObjectWithData:d] : nil;
}
- (BOOL)allowsKeyedCoding { return YES; }
- (BOOL)requiresSecureCoding { return _secure; }
- (void)setRequiresSecureCoding:(BOOL)s { _secure = s; }
- (id)error { return _error; }
- (void)failWithError:(id)e { if (!_error) _error = [e retain]; }
- (void)setClass:(Class)c forClassName:(id)n { [_names setObject:_prism_cf_string(class_getName(c)) forKey:n]; }
- (void)finishDecoding {}

- (id)_decodeUID:(long)n {
    if (n <= 0 || (NSUInteger)n >= [_objects count]) return nil;
    id key = [NSNumber numberWithLong:n];
    id done = [_decoded objectForKey:key];
    if (done) return done == (id)kCFNull ? nil : done;
    id entry = [_objects objectAtIndex:n];
    if (!cf_is(entry, objc_getClass("NSDictionary")) || ![entry objectForKey:CFSTR("$class")]) {
        [_decoded setObject:entry forKey:key];
        return entry;
    }
    long cn = _prism_uid_value([entry objectForKey:CFSTR("$class")]);
    id cinfo = cn > 0 && (NSUInteger)cn < [_objects count] ? [_objects objectAtIndex:cn] : nil;
    id name = [cinfo objectForKey:CFSTR("$classname")];
    id sub = [_names objectForKey:name];
    Class cls = objc_getClass([(sub ? sub : name) UTF8String]);
    if (!cls)
        for (id fallback in [cinfo objectForKey:CFSTR("$classes")]) if ((cls = objc_getClass([fallback UTF8String]))) break;
    if (!cls) _prism_raise("NSInvalidUnarchiveOperationException", "*** -[NSKeyedUnarchiver decodeObjectForKey:]: cannot decode object of class (%s)", [[name description] UTF8String]);
    if (_secure && _allowed) {
        BOOL ok = NO;
        for (Class a in _allowed) if ([cls isSubclassOfClass:a] || [a isSubclassOfClass:cls]) ok = YES;
        if (!ok) {
            [self failWithError:archive_error(4864, "value for key 'root' was of unexpected class")];
            return nil;
        }
    }
    [_decoded setObject:(id)kCFNull forKey:key]; /* a cycle back here decodes as nil */
    [_stack addObject:entry];
    id o = [[cls alloc] initWithCoder:self];
    [_stack removeLastObject];
    o = [[o awakeAfterUsingCoder:self] autorelease];
    if (o) [_decoded setObject:o forKey:key];
    return o;
}
- (id)_raw:(id)k { return [[_stack lastObject] objectForKey:k]; }
- (BOOL)containsValueForKey:(id)k { return [self _raw:k] != nil; }
- (id)decodeObjectForKey:(id)k {
    id v = [self _raw:k];
    long n = _prism_uid_value(v);
    return n >= 0 ? [self _decodeUID:n] : v;
}
- (id)decodeObjectOfClass:(Class)c forKey:(id)k { (void)c; return [self decodeObjectForKey:k]; }
- (id)decodeObjectOfClasses:(id)cs forKey:(id)k { (void)cs; return [self decodeObjectForKey:k]; }
- (id)decodeArrayOfObjectsOfClass:(Class)c forKey:(id)k { (void)c; return [self decodeObjectForKey:k]; }
- (id)decodeTopLevelObjectForKey:(id)k error:(id *)e { if (e) *e = nil; return [self decodeObjectForKey:k]; }
- (id)decodeTopLevelObjectOfClass:(Class)c forKey:(id)k error:(id *)e { (void)c; return [self decodeTopLevelObjectForKey:k error:e]; }
- (BOOL)decodeBoolForKey:(id)k { return [[self _raw:k] boolValue]; }
- (int)decodeIntForKey:(id)k { return (int)[[self _raw:k] longLongValue]; }
- (int)decodeInt32ForKey:(id)k { return (int)[[self _raw:k] longLongValue]; }
- (long long)decodeInt64ForKey:(id)k { return [[self _raw:k] longLongValue]; }
- (NSInteger)decodeIntegerForKey:(id)k { return (NSInteger)[[self _raw:k] longLongValue]; }
- (float)decodeFloatForKey:(id)k { return [[self _raw:k] doubleValue]; }
- (double)decodeDoubleForKey:(id)k { return [[self _raw:k] doubleValue]; }
- (const void *)decodeBytesForKey:(id)k returnedLength:(NSUInteger *)n {
    id d = [self _raw:k];
    if (n) *n = [d length];
    return [d bytes];
}
/* The members of a collection: an array of references. */
- (id)_decodeObjectsForKey:(id)k {
    id out = [NSMutableArray array];
    for (id u in [self _raw:k]) {
        id o = [self _decodeUID:_prism_uid_value(u)];
        if (o) [out addObject:o];
    }
    return out;
}
- (void)dealloc {
    [_objects release]; [_top release]; [_stack release]; [_decoded release]; [_allowed release]; [_names release]; [_error release];
    [super dealloc];
}
@end

/* ------------------------------------------------------------------ the built-in classes */

#define SECURE + (BOOL)supportsSecureCoding { return YES; }
@implementation NSArray (Coding)
SECURE
- (void)encodeWithCoder:(id)c { [c _encodeObjects:self forKey:CFSTR("NS.objects")]; }
- (id)initWithCoder:(id)c { for (id o in [c _decodeObjectsForKey:CFSTR("NS.objects")]) [self _prismAppend:o]; return self; }
@end
@implementation NSSet (Coding)
SECURE
- (void)encodeWithCoder:(id)c { [c _encodeObjects:[self allObjects] forKey:CFSTR("NS.objects")]; }
- (id)initWithCoder:(id)c { return [self initWithArray:[c _decodeObjectsForKey:CFSTR("NS.objects")]]; }
@end
@implementation NSDictionary (Coding)
SECURE
- (void)encodeWithCoder:(id)c {
    id keys = [self allKeys];
    id values = [NSMutableArray array];
    for (id k in keys) [values addObject:[self objectForKey:k]];
    [c _encodeObjects:keys forKey:CFSTR("NS.keys")];
    [c _encodeObjects:values forKey:CFSTR("NS.objects")];
}
- (id)initWithCoder:(id)c {
    id keys = [c _decodeObjectsForKey:CFSTR("NS.keys")], values = [c _decodeObjectsForKey:CFSTR("NS.objects")];
    for (NSUInteger i = 0; i < [keys count] && i < [values count]; i++) [(id)self setObject:[values objectAtIndex:i] forKey:[keys objectAtIndex:i]];
    return self;
}
@end
@implementation NSString (Coding)
SECURE
- (void)encodeWithCoder:(id)c { [c encodeObject:[[self copy] autorelease] forKey:CFSTR("NS.string")]; }
- (id)initWithCoder:(id)c { return [self initWithString:[c decodeObjectForKey:CFSTR("NS.string")]]; }
@end
@implementation NSData (Coding)
SECURE
- (void)encodeWithCoder:(id)c { [c encodeObject:[[self copy] autorelease] forKey:CFSTR("NS.data")]; }
- (id)initWithCoder:(id)c { id d = [c decodeObjectForKey:CFSTR("NS.data")]; return [self initWithBytes:[d bytes] length:[d length]]; }
@end
@implementation NSNumber (Coding)
SECURE
@end
@implementation NSDate (Coding)
SECURE
- (void)encodeWithCoder:(id)c { [c encodeDouble:[self timeIntervalSinceReferenceDate] forKey:CFSTR("NS.time")]; }
- (id)initWithCoder:(id)c { return [self initWithTimeIntervalSinceReferenceDate:[c decodeDoubleForKey:CFSTR("NS.time")]]; }
@end
@implementation NSURL (Coding)
SECURE
- (void)encodeWithCoder:(id)c {
    [c encodeObject:_base forKey:CFSTR("NS.base")];
    [c encodeObject:_string forKey:CFSTR("NS.relative")];
}
- (id)initWithCoder:(id)c {
    [self release];
    return [[NSURL URLWithString:[c decodeObjectForKey:CFSTR("NS.relative")] relativeToURL:[c decodeObjectForKey:CFSTR("NS.base")]] retain];
}
@end
@implementation NSNull (Coding)
SECURE
- (void)encodeWithCoder:(id)c { (void)c; }
- (id)initWithCoder:(id)c { (void)c; return (id)kCFNull; }
@end
@implementation NSUUID (Coding)
SECURE
- (void)encodeWithCoder:(id)c {
    unsigned char b[16];
    [self getUUIDBytes:b];
    [c encodeBytes:b length:16 forKey:CFSTR("NS.uuidbytes")];
}
- (id)initWithCoder:(id)c {
    NSUInteger n = 0;
    const unsigned char *b = [c decodeBytesForKey:CFSTR("NS.uuidbytes") returnedLength:&n];
    if (n != 16) { [self release]; return nil; }
    return [self initWithUUIDBytes:b];
}
@end
@implementation NSError (Coding)
SECURE
- (void)encodeWithCoder:(id)c {
    [c encodeObject:[self domain] forKey:CFSTR("NSDomain")];
    [c encodeInteger:[self code] forKey:CFSTR("NSCode")];
    [c encodeObject:[self userInfo] forKey:CFSTR("NSUserInfo")];
}
- (id)initWithCoder:(id)c {
    return [self initWithDomain:[c decodeObjectForKey:CFSTR("NSDomain")] code:[c decodeIntegerForKey:CFSTR("NSCode")] userInfo:[c decodeObjectForKey:CFSTR("NSUserInfo")]];
}
@end
