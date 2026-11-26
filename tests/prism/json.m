/* NSJSONSerialization, NSNumberFormatter, NSDateFormatter, NSISO8601DateFormatter and NSTimeZone
 * through Prism (ARC). No SDK: the interfaces are declared here. */
int printf(const char *, ...);
typedef unsigned long NSUInteger;
typedef long NSInteger;
#if __arm64__
typedef _Bool BOOL;
#else
typedef signed char BOOL;
#endif
#define nil ((void *)0)
#define YES __objc_yes
#define NO __objc_no

__attribute__((objc_root_class))
@interface NSObject
+ (instancetype)alloc;
+ (instancetype)new;
- (instancetype)init;
- (BOOL)isEqual:(id)o;
@end
@interface NSString : NSObject
+ (id)stringWithUTF8String:(const char *)s;
- (const char *)UTF8String;
- (NSUInteger)length;
- (instancetype)initWithData:(id)d encoding:(NSUInteger)e;
- (id)dataUsingEncoding:(NSUInteger)e;
- (id)stringByReplacingOccurrencesOfString:(id)a withString:(id)b;
@end
@interface NSNumber : NSObject
+ (id)numberWithInt:(int)v;
+ (id)numberWithDouble:(double)v;
+ (id)numberWithBool:(BOOL)v;
- (int)intValue;
- (long long)longLongValue;
- (double)doubleValue;
- (BOOL)boolValue;
@end
@interface NSNull : NSObject
+ (id)null;
@end
@interface NSArray : NSObject
+ (id)arrayWithObjects:(const id *)o count:(NSUInteger)n;
- (NSUInteger)count;
- (id)objectAtIndexedSubscript:(NSUInteger)i;
@end
@interface NSDictionary : NSObject
+ (id)dictionaryWithObjects:(const id *)o forKeys:(const id *)k count:(NSUInteger)n;
- (id)objectForKeyedSubscript:(id)k;
@end
@interface NSError : NSObject
- (NSInteger)code;
@end
@interface NSDate : NSObject
+ (id)date;
+ (id)dateWithTimeIntervalSince1970:(double)t;
- (double)timeIntervalSince1970;
@end
@interface NSJSONSerialization : NSObject
+ (id)JSONObjectWithData:(id)d options:(NSUInteger)o error:(id *)e;
+ (id)dataWithJSONObject:(id)o options:(NSUInteger)opts error:(id *)e;
+ (BOOL)isValidJSONObject:(id)o;
@end
@interface NSTimeZone : NSObject
+ (id)timeZoneForSecondsFromGMT:(NSInteger)s;
+ (id)timeZoneWithName:(id)n;
- (NSInteger)secondsFromGMT;
- (id)name;
@end
@interface NSNumberFormatter : NSObject
- (void)setNumberStyle:(NSUInteger)s;
- (void)setMaximumFractionDigits:(NSInteger)n;
- (id)stringFromNumber:(id)n;
- (id)numberFromString:(id)s;
@end
@interface NSDateFormatter : NSObject
- (void)setDateFormat:(id)f;
- (void)setTimeZone:(id)z;
- (void)setDateStyle:(NSUInteger)s;
- (void)setTimeStyle:(NSUInteger)s;
- (id)stringFromDate:(id)d;
- (id)dateFromString:(id)s;
@end
@interface NSISO8601DateFormatter : NSObject
- (void)setFormatOptions:(NSUInteger)o;
- (id)stringFromDate:(id)d;
- (id)dateFromString:(id)s;
@end

static id json(const char *s) { return [@(s) dataUsingEncoding:4]; }
static const char *text(id data) { return [[[NSString alloc] initWithData:data encoding:4] UTF8String]; }

int main(void) {
    @autoreleasepool {
        id err = nil;
        NSDictionary *d = [NSJSONSerialization JSONObjectWithData:json("{\"name\":\"Zo\xc3\xab\",\"n\":42,\"pi\":3.25,\"big\":12345678901234,\"neg\":-1e2,"
                                                                       "\"ok\":true,\"no\":false,\"nil\":null,\"arr\":[1,\"two\",[3]],"
                                                                       "\"esc\":\"a\\\"b\\\\c\\nd\\u00e9\\ud83d\\ude00\"}")
                                                          options:0 error:&err];
        NSArray *arr = d[@"arr"];
        printf("json: err=%d name=%s n=%d pi=%g big=%lld neg=%g ok=%d no=%d null=%d arr=%lu/%s/%lu esc=%lu\n", err != nil, [d[@"name"] UTF8String],
               [d[@"n"] intValue], [d[@"pi"] doubleValue], [d[@"big"] longLongValue], [d[@"neg"] doubleValue], [d[@"ok"] boolValue],
               [d[@"no"] boolValue], d[@"nil"] == [NSNull null], (unsigned long)[arr count], [arr[1] UTF8String], (unsigned long)[arr[2] count],
               (unsigned long)[d[@"esc"] length]);

        id out = [NSJSONSerialization dataWithJSONObject:@{@"s" : @"tab\t\"q\"", @"b" : @[ @1, @2.5, @"x/y" ], @"a" : @{@"z" : @YES, @"q" : [NSNull null]}}
                                                 options:2 error:nil];
        id pretty = [NSJSONSerialization dataWithJSONObject:@{@"k" : @[ @1, @2 ]} options:1 error:nil];
        id bad = nil, frag_err = nil;
        id broken = [NSJSONSerialization JSONObjectWithData:json("[1,2") options:0 error:&bad];
        id frag = [NSJSONSerialization JSONObjectWithData:json("42") options:4 error:nil];
        id nofrag = [NSJSONSerialization JSONObjectWithData:json("42") options:0 error:&frag_err];
        printf("json2: %s | %s | broken=%d/%ld frag=%d nofrag=%d valid=%d/%d\n", text(out),
               [[[[NSString alloc] initWithData:pretty encoding:4] stringByReplacingOccurrencesOfString:@"\n" withString:@"/"] UTF8String],
               broken == nil, (long)[bad code], [frag intValue], nofrag == nil && frag_err != nil,
               [NSJSONSerialization isValidJSONObject:@[ @1, @"a" ]], [NSJSONSerialization isValidJSONObject:@[ [NSDate date] ]]);

        NSNumberFormatter *nf = [NSNumberFormatter new];
        [nf setNumberStyle:1];
        id dec = [nf stringFromNumber:@1234567.891];
        id parsed = [nf numberFromString:@"1,234.5"];
        [nf setNumberStyle:2];
        id cur = [nf stringFromNumber:@1234.5];
        [nf setNumberStyle:3];
        id pct = [nf stringFromNumber:@0.256];
        [nf setNumberStyle:4];
        id sci = [nf stringFromNumber:@12345];
        [nf setNumberStyle:0];
        id none = [nf stringFromNumber:@3.7];
        printf("numfmt: %s %g %s %s %s %s\n", [dec UTF8String], [parsed doubleValue], [cur UTF8String], [pct UTF8String], [sci UTF8String], [none UTF8String]);

        NSDate *when = [NSDate dateWithTimeIntervalSince1970:1136214245];
        NSDateFormatter *df = [NSDateFormatter new];
        [df setDateFormat:@"yyyy-MM-dd HH:mm:ss"];
        id a = [df stringFromDate:when];
        NSDate *p = [df dateFromString:@"1999-12-31 23:59:58"];
        [df setDateFormat:@"EEE, d MMM yyyy h:mm a"];
        id b = [df stringFromDate:when];
        [df setTimeZone:[NSTimeZone timeZoneForSecondsFromGMT:7200]];
        [df setDateFormat:@"yyyy-MM-dd'T'HH:mmXXXXX"];
        id c = [df stringFromDate:when];
        [df setTimeZone:[NSTimeZone timeZoneWithName:@"UTC"]];
        [df setDateStyle:2];
        [df setTimeStyle:1];
        id styled = [df stringFromDate:when];
        NSISO8601DateFormatter *iso = [NSISO8601DateFormatter new];
        id isos = [iso stringFromDate:when];
        NSDate *isop = [iso dateFromString:@"2020-02-29T12:00:00.250+01:00"];
        printf("datefmt: %s | %.0f | %s | %s | %s | %s | %.2f | tz=%ld/%s\n", [a UTF8String], [p timeIntervalSince1970], [b UTF8String], [c UTF8String],
               [styled UTF8String], [isos UTF8String], [isop timeIntervalSince1970], (long)[[NSTimeZone timeZoneWithName:@"GMT+05:30"] secondsFromGMT],
               [[[NSTimeZone timeZoneForSecondsFromGMT:-3600] name] UTF8String]);
    }
    return 0;
}
