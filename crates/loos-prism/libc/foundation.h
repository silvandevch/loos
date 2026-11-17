/* Declarations shared by the Objective-C part of Prism's runtime: the root class and the
 * CoreFoundation value classes. CF objects are Objective-C objects ("toll-free bridging"): a
 * CFStringRef is an NSString *, and so on. The logic is in Rust (src/cf); these classes are the
 * thin layer that sends messages and reads fields. */
#pragma once
#include "objc.h"

typedef struct _NSZone NSZone;
typedef unsigned long NSUInteger;
typedef long NSInteger;
typedef unsigned short unichar;
typedef struct { NSUInteger location, length; } NSRange;
#define NSNotFound 0x7fffffffffffffffL
typedef long CFIndex;
typedef unsigned long CFTypeID, CFHashCode, CFOptionFlags;
typedef const void *CFTypeRef, *CFAllocatorRef;
typedef unsigned int CFStringEncoding;
typedef unsigned char Boolean, UInt8;
typedef int SInt32;
typedef unsigned int UInt32;
typedef struct { CFIndex location, length; } CFRange;
typedef struct { unsigned long state; id *itemsPtr; unsigned long *mutationsPtr; unsigned long extra[5]; } NSFastEnumerationState;

__attribute__((objc_root_class))
@interface NSObject { Class isa; }
+ (id)alloc;
+ (id)new;
+ (Class)class;
- (id)init;
- (void)dealloc;
- (id)retain;
- (oneway void)release;
- (id)autorelease;
- (NSUInteger)retainCount;
- (Class)class;
- (BOOL)isKindOfClass:(Class)c;
- (BOOL)respondsToSelector:(SEL)s;
- (NSUInteger)hash;
- (BOOL)isEqual:(id)o;
- (id)copy;
- (id)mutableCopy;
- (id)copyWithZone:(NSZone *)z;
- (id)mutableCopyWithZone:(NSZone *)z;
@end
@interface NSObject (CoreFoundation)
- (id)description;
- (CFTypeID)_cfTypeID;
@end

/* libSystem calls the classes use (macOS layouts). */
struct timeval { time_t tv_sec; int tv_usec; };
struct timespec { time_t tv_sec; long tv_nsec; };
int gettimeofday(struct timeval *, void *);
struct tm *gmtime_r(const time_t *, struct tm *);
size_t strftime(char *, size_t, const char *, const struct tm *);
int nanosleep(const struct timespec *, struct timespec *);
long read(int, void *, size_t);
long write(int, const void *, size_t);
int close(int);
int rename(const char *, const char *);
int access(const char *, int);
char *realpath(const char *, char *) __asm__("_realpath$DARWIN_EXTSN");
int _NSGetExecutablePath(char *, uint32_t *);
char *getcwd(char *, size_t);
static inline double floor_(double x) { double t = (double)(long long)x; return t > x ? t - 1 : t; }

/* Every CF value object: isa, then these (src/cf/mod.rs). */
struct cfobj { Class isa; uint64_t flags; void *data; uint64_t count, cap; };
#define CF_UTF16 0x10ul
#define CF_OWNED (1ul << 16)   /* `data` belongs to the object */
#define CF_MUTABLE (1ul << 17)
#define CF_UNRETAINED (1ul << 18) /* a CF collection made without retain callbacks */
#define CF_SET (1ul << 19)
#define CF(o) ((struct cfobj *)(o))
#define CFSTR(s) ((id)__builtin___CFStringMakeConstantString(s))
/* Raise an NSException with a printf-formatted reason (nsexception.m). */
void _prism_raise(const char *name, const char *fmt, ...) __attribute__((noreturn));

@interface NSString : NSObject
+ (id)string;
- (id)initWithBytes:(const void *)b length:(NSUInteger)n encoding:(NSUInteger)e;
- (BOOL)hasPrefix:(id)s;
- (BOOL)hasSuffix:(id)s;
- (id)substringFromIndex:(NSUInteger)i;
- (id)stringByAppendingString:(id)s;
- (id)stringByAppendingFormat:(id)f, ...;
- (NSInteger)caseInsensitiveCompare:(id)s;
- (BOOL)getCString:(char *)b maxLength:(NSUInteger)n encoding:(NSUInteger)e;
- (NSInteger)integerValue;
- (int)intValue;
+ (id)stringWithFormat:(id)f, ...;
- (NSUInteger)length;
- (unichar)characterAtIndex:(NSUInteger)i;
- (void)getCharacters:(unichar *)buf range:(NSRange)r;
- (const char *)UTF8String;
- (BOOL)isEqualToString:(NSString *)s;
@end
@interface NSMutableString : NSString
- (void)appendString:(NSString *)s;
- (NSUInteger)replaceOccurrencesOfString:(NSString *)a withString:(NSString *)b options:(NSUInteger)opts range:(NSRange)r;
@end
@interface NSNumber : NSObject
@end
@interface NSData : NSObject
- (NSUInteger)length;
- (const void *)bytes;
@end
@interface NSMutableData : NSData
@end
@interface NSArray : NSObject
- (NSUInteger)count;
- (id)objectAtIndex:(NSUInteger)i;
- (void)_prismAppend:(id)o;
- (NSUInteger)indexOfObjectIdenticalTo:(id)o;
- (void)getObjects:(id *)out range:(NSRange)r;
@end
@interface NSMutableArray : NSArray
- (void)removeObjectIdenticalTo:(id)o;
- (void)addObject:(id)o;
- (void)addObjectsFromArray:(NSArray *)a;
- (void)insertObject:(id)o atIndex:(NSUInteger)i;
- (void)removeObjectAtIndex:(NSUInteger)i;
- (void)removeAllObjects;
@end
@interface NSDictionary : NSObject
- (NSUInteger)count;
- (id)objectForKey:(id)k;
- (id)allKeys;
- (void)getObjects:(id *)o andKeys:(id *)k count:(NSUInteger)n;
@end
@interface NSSet : NSObject
- (NSUInteger)count;
- (BOOL)containsObject:(id)o;
- (id)allObjects;
@end
@interface NSMutableSet : NSSet
@end
@interface NSDate : NSObject
- (double)timeIntervalSinceReferenceDate;
- (double)timeIntervalSince1970;
- (double)timeIntervalSinceDate:(id)o;
- (NSInteger)compare:(id)o;
+ (id)dateWithTimeIntervalSinceReferenceDate:(double)s;
@end
@interface NSEnumerator : NSObject
@end
@interface NSMutableDictionary : NSDictionary
- (void)setObject:(id)o forKey:(id)k;
@end
@interface NSNumber (PrismCtors)
+ (id)numberWithUnsignedLongLong:(unsigned long long)v;
+ (id)numberWithUnsignedShort:(unsigned short)v;
+ (id)numberWithUnsignedInt:(unsigned int)v;
+ (id)numberWithInt:(int)v;
+ (id)numberWithBool:(BOOL)v;
+ (id)numberWithDouble:(double)v;
+ (id)numberWithLongLong:(long long)v;
@end
@interface NSURL : NSObject { @public id _string; NSURL *_base; }
+ (id)fileURLWithPath:(id)p isDirectory:(BOOL)d;
+ (id)fileURLWithPath:(id)p;
@end
@interface NSData (PrismFiles)
+ (id)dataWithContentsOfFile:(id)p;
@end

/* Methods the classes send one another (declared so the compiler uses the right return types). */
@interface NSObject (PrismMethods)
+ (BOOL)isSubclassOfClass:(Class)c;
- (const char *)objCType;
- (double)doubleValue;
- (long long)longLongValue;
- (const char *)fileSystemRepresentation;
- (const char *)cStringUsingEncoding:(NSUInteger)e;
- (BOOL)isFileURL;
- (id)path;
- (id)absoluteURL;
- (id)name;
- (void)invalidate;
- (void)addTimer:(id)t forMode:(id)m;
- (id)bundleIdentifier;
- (id)infoDictionary;
- (id)objectForInfoDictionaryKey:(id)k;
- (id)bundleURL;
- (id)resourceURL;
- (id)executableURL;
- (id)privateFrameworksPath;
- (id)builtInPlugInsPath;
- (id)URLForResource:(id)n withExtension:(id)e subdirectory:(id)s;
- (id)localizedStringForKey:(id)k value:(id)v table:(id)t;
- (id)URLByAppendingPathComponent:(id)c isDirectory:(BOOL)d;
- (id)URLByAppendingPathComponent:(id)c;
- (id)URLByDeletingLastPathComponent;
- (id)URLByAppendingPathExtension:(id)e;
- (id)URLByDeletingPathExtension;
- (id)scheme;
- (id)host;
- (id)port;
- (id)user;
- (id)password;
- (id)query;
- (id)fragment;
- (id)resourceSpecifier;
- (id)lastPathComponent;
- (id)pathExtension;
- (BOOL)hasDirectoryPath;
- (BOOL)checkResourceIsReachableAndReturnError:(id *)e;
- (id)stringByAppendingPathComponent:(id)c;
- (id)stringByDeletingLastPathComponent;
- (id)stringByAppendingPathExtension:(id)e;
- (id)stringByDeletingPathExtension;
- (id)stringByStandardizingPath;
- (id)stringByExpandingTildeInPath;
- (id)stringByResolvingSymlinksInPath;
- (id)stringByRemovingPercentEncoding;
- (id)pathComponents;
- (BOOL)isAbsolutePath;
- (id)initWithBytes:(const void *)b length:(NSUInteger)n;
- (id)initWithBytesNoCopy:(void *)b length:(NSUInteger)n freeWhenDone:(BOOL)f;
- (void)appendBytes:(const void *)b length:(NSUInteger)n;
- (id)objectEnumerator;
- (void)setObject:(id)o forKey:(id)k;
- (unsigned short)unsignedShortValue;
- (id)sortedArrayUsingSelector:(SEL)s;
- (id)lastObject;
- (id)firstObject;
- (void)removeLastObject;
- (id)allObjects;
- (id)initWithObjects:(id)first, ...;
- (BOOL)fileExistsAtPath:(id)p isDirectory:(BOOL *)d;
- (BOOL)fileExistsAtPath:(id)p;
- (BOOL)createDirectoryAtPath:(id)p withIntermediateDirectories:(BOOL)i attributes:(id)a error:(id *)e;
- (BOOL)copyItemAtPath:(id)a toPath:(id)b error:(id *)e;
- (BOOL)removeItemAtPath:(id)p error:(id *)e;
- (BOOL)createSymbolicLinkAtPath:(id)p withDestinationPath:(id)d error:(id *)e;
- (id)destinationOfSymbolicLinkAtPath:(id)p error:(id *)e;
- (id)nextObject;
- (void)removeObjectForKey:(id)k;
- (void)addEntriesFromDictionary:(id)d;
- (id)propertyList;
- (id)stringValue;
- (BOOL)boolValue;
- (BOOL)synchronize;
- (void)postNotificationName:(id)n object:(id)o;
- (id)stringByAppendingPathExtension:(id)e;
- (id)absoluteString;
+ (id)defaultManager;
+ (id)defaultCenter;
+ (id)mainBundle;
+ (id)processInfo;
+ (id)UUID;
- (id)UUIDString;
- (id)arguments;
- (id)processName;
- (id)_domain:(id)n;
+ (id)URLWithString:(id)s;
+ (id)URLWithString:(id)s relativeToURL:(id)b;
+ (id)dictionaryWithDictionary:(id)d;
- (void)postNotificationName:(id)n object:(id)o userInfo:(id)u;
- (void)postNotification:(id)n;
- (BOOL)containsObject:(id)o;
- (void)unionSet:(id)s;
- (void)removeObject:(id)o;
+ (id)setWithObject:(id)o;
+ (id)setWithArray:(id)a;
+ (id)setWithSet:(id)s;
- (id)initWithObjects:(id)first, ...;
- (id)componentsSeparatedByString:(id)s;
+ (id)numberWithUnsignedInteger:(NSUInteger)v;
+ (id)numberWithLong:(long)v;
- (unsigned int)unsignedIntValue;
- (BOOL)writeToFile:(id)p atomically:(BOOL)a;
- (void)setData:(id)d;
- (void)encodeWithCoder:(id)c;
- (id)initWithCoder:(id)c;
+ (id)setWithObjects:(id)first, ...;
- (id)replacementObjectForKeyedArchiver:(id)c;
- (Class)classForKeyedArchiver;
- (id)awakeAfterUsingCoder:(id)c;
- (void)_encodeObjects:(id)items forKey:(id)k;
- (id)_decodeObjectsForKey:(id)k;
- (void)encodeObject:(id)o forKey:(id)k;
- (id)decodeObjectForKey:(id)k;
- (void)encodeDouble:(double)v forKey:(id)k;
- (double)decodeDoubleForKey:(id)k;
- (void)encodeInteger:(NSInteger)v forKey:(id)k;
- (NSInteger)decodeIntegerForKey:(id)k;
- (void)encodeBytes:(const void *)b length:(NSUInteger)n forKey:(id)k;
- (const void *)decodeBytesForKey:(id)k returnedLength:(NSUInteger *)n;
- (id)initWithString:(id)s;
- (id)initWithArray:(id)a;
- (id)initWithTimeIntervalSinceReferenceDate:(double)t;
- (id)encodedData;
- (id)initRequiringSecureCoding:(BOOL)s;
- (id)initForReadingFromData:(id)d error:(id *)e;
- (id)initForReadingWithData:(id)d;
@end
@interface NSData (PrismMethods)
+ (id)dataWithBytes:(const void *)b length:(NSUInteger)n;
+ (id)data;
@end
@interface NSDictionary (PrismMethods)
+ (id)dictionary;
+ (id)dictionaryWithObject:(id)o forKey:(id)k;
+ (id)dictionaryWithObjectsAndKeys:(id)first, ...;
- (id)allValues;
@end
@interface NSArray (PrismMethods)
+ (id)array;
+ (id)arrayWithObject:(id)o;
+ (id)arrayWithObjects:(id)first, ...;
@end
@interface NSDate (PrismMethods)
+ (id)date;
@end
@interface NSNumber (PrismMethods)
+ (id)numberWithInteger:(NSInteger)v;
@end

id _prism_file_error(int e, id path); /* nsexception.m */
id _prism_enumerator(id items, BOOL back); /* cfbase.m */
id _prism_plist_from_file(id path); /* cfplist.m */
id _prism_plist_data(id plist, unsigned long format); /* cfplist.m: serialized bytes or nil */
BOOL _prism_write_file(id path, id data, BOOL atomically); /* cfplist.m */

@interface NSBundle : NSObject { @public id _path, _resources, _executable, _info; BOOL _app; }
@end
@interface NSFileManager : NSObject { @public id _delegate; }
@end
@interface NSNotificationCenter : NSObject { @public id _observers; NSUInteger _posting; }
@end
Boolean CFPropertyListIsValid(id plist, CFIndex format);
@interface NSException : NSObject { @public id _name, _reason, _userInfo, _stack; }
- (id)reason;
@end
@interface NSError : NSObject { @public id _domain, _userInfo; NSInteger _code; }
+ (id)errorWithDomain:(id)d code:(NSInteger)c userInfo:(id)u;
- (id)initWithDomain:(id)d code:(NSInteger)c userInfo:(id)u;
- (id)domain;
- (NSInteger)code;
- (id)userInfo;
@end
@interface NSNull : NSObject
@end
@interface NSUUID : NSObject { @public unsigned char _b[16]; }
- (void)getUUIDBytes:(unsigned char *)b;
- (id)initWithUUIDBytes:(const unsigned char *)b;
@end
@interface NSPropertyListSerialization : NSObject
+ (id)propertyListWithData:(id)d options:(NSUInteger)o format:(NSUInteger *)f error:(id *)e;
@end
extern id kCFNull;

id _prism_uid(uint32_t v); /* cfplist.m: a keyed-archive UID */
long _prism_uid_value(id o);

/* Is `o` an instance of `c` or a subclass (also through a copied class such as the constant string's)? */
static inline BOOL cf_is(id o, Class c) {
    for (Class k = o ? object_getClass(o) : 0; k; k = class_getSuperclass(k)) if (k == c) return YES;
    return NO;
}
/* A new string with this text / the description of `o` (cfstring.m). */
id _prism_cf_string(const char *utf8);
id _prism_cf_format(id format, va_list ap);
id _prism_cf_describe(id o);
void *_prism_cf_autoreleased_buffer(void *malloced);
const void *CFRetain(const void *);
void CFRelease(const void *);
#define RETAIN(o) ((id)CFRetain(o))

/* src/cf/string.rs */
long __prism_cf_str_init(void *s, const void *bytes, unsigned long n, unsigned long enc);
long __prism_cf_str_init_chars(void *s, const unichar *chars, unsigned long n);
long __prism_cf_str_encode(void *s, unsigned long loc, unsigned long len, unsigned long enc, long loss, void *buf, unsigned long room);
long __prism_cf_str_valid_encoding(unsigned long enc);
long __prism_cf_str_chars(void *s, unsigned long loc, unsigned long len, unichar *buf);
long __prism_cf_str_compare(void *a, unsigned long loc, unsigned long len, void *b, unsigned long opts);
unsigned long __prism_cf_str_hash(void *s);
long __prism_cf_str_find(void *hay, unsigned long loc, unsigned long len, void *needle, unsigned long opts, NSRange *out);
long __prism_cf_str_splice(void *s, unsigned long loc, unsigned long len, void *other);
long __prism_cf_str_sub(void *dst, void *src, unsigned long loc, unsigned long len);
long __prism_cf_str_map(void *dst, void *src, unsigned long mode);
long long __prism_cf_str_i64(void *s);
double __prism_cf_str_f64(void *s);
/* src/cf/number.rs */
unsigned long __prism_cf_num_read(unsigned long type, const void *p);
long __prism_cf_num_write(unsigned long type, unsigned long bits, unsigned long to, void *p);
long __prism_cf_num_compare(unsigned long t1, unsigned long b1, unsigned long t2, unsigned long b2);
unsigned long __prism_cf_num_hash(unsigned long t, unsigned long b);
long long __prism_cf_num_i64(unsigned long t, unsigned long b);
double __prism_cf_num_f64(unsigned long t, unsigned long b);
long __prism_cf_num_str(unsigned long t, unsigned long b, void *string);
/* src/cf/collection.rs */
long __prism_cf_vec_splice(void *o, unsigned long elem, unsigned long loc, unsigned long len, const void *src, unsigned long n);
long __prism_cf_vec_resize(void *o, unsigned long elem, unsigned long n);
unsigned long __prism_cf_bytes_hash(const void *p, unsigned long n);
long __prism_cf_bytes_find(const void *hay, unsigned long hn, const void *needle, unsigned long nn, long backwards);
long __prism_cf_dict_probe(void *o, unsigned long hash, long cursor);
long __prism_cf_dict_insert(void *o, unsigned long hash, const void *key, const void *value);
long __prism_cf_dict_remove(void *o, unsigned long slot);
long __prism_cf_dict_clear(void *o);
