/* NSException: what Foundation and Prism's own classes raise (range errors, unrecognized
 * selectors, nil arguments), with the uncaught-exception handler and message of macOS. */
#include "foundation.h"

id const NSGenericException = CFSTR("NSGenericException");
id const NSRangeException = CFSTR("NSRangeException");
id const NSInvalidArgumentException = CFSTR("NSInvalidArgumentException");
id const NSInternalInconsistencyException = CFSTR("NSInternalInconsistencyException");
id const NSMallocException = CFSTR("NSMallocException");
id const NSObjectInaccessibleException = CFSTR("NSObjectInaccessibleException");
id const NSObjectNotAvailableException = CFSTR("NSObjectNotAvailableException");
id const NSDestinationInvalidException = CFSTR("NSDestinationInvalidException");
id const NSPortTimeoutException = CFSTR("NSPortTimeoutException");
id const NSInvalidSendPortException = CFSTR("NSInvalidSendPortException");
id const NSInvalidReceivePortException = CFSTR("NSInvalidReceivePortException");
id const NSPortSendException = CFSTR("NSPortSendException");
id const NSPortReceiveException = CFSTR("NSPortReceiveException");
id const NSOldStyleException = CFSTR("NSOldStyleException");
id const NSCharacterConversionException = CFSTR("NSCharacterConversionException");
id const NSParseErrorException = CFSTR("NSParseErrorException");

@interface NSException : NSObject {
@public
    id _name, _reason, _userInfo, _stack;
}
@end

@implementation NSException
+ (id)exceptionWithName:(id)name reason:(id)reason userInfo:(id)info { return [[[self alloc] initWithName:name reason:reason userInfo:info] autorelease]; }
+ (void)raise:(id)name format:(id)fmt, ... {
    va_list ap;
    va_start(ap, fmt);
    id reason = _prism_cf_format(fmt, ap);
    va_end(ap);
    @throw [self exceptionWithName:name reason:reason userInfo:nil];
}
+ (void)raise:(id)name format:(id)fmt arguments:(va_list)ap { @throw [self exceptionWithName:name reason:_prism_cf_format(fmt, ap) userInfo:nil]; }
- (id)initWithName:(id)name reason:(id)reason userInfo:(id)info {
    _name = [name copy];
    _reason = [reason copy];
    _userInfo = [info retain];
    return self;
}
- (id)name { return _name; }
- (id)reason { return _reason; }
- (id)userInfo { return _userInfo; }
- (id)callStackReturnAddresses { return [NSArray array]; }
- (id)callStackSymbols { return [NSArray array]; }
- (void)raise { @throw self; }
- (id)description { return _reason ? _reason : _name; }
- (id)copyWithZone:(NSZone *)z { (void)z; return [self retain]; }
- (void)dealloc {
    [_name release];
    [_reason release];
    [_userInfo release];
    [super dealloc];
}
@end

/* Raise an NSException named `name` with a printf-formatted reason (the runtime and the CF
 * classes use this). */
void _prism_raise(const char *name, const char *fmt, ...) {
    char buf[512];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(buf, sizeof buf, fmt, ap);
    va_end(ap);
    @throw [NSException exceptionWithName:_prism_cf_string(name) reason:_prism_cf_string(buf) userInfo:nil];
    __builtin_unreachable();
}

typedef void NSUncaughtExceptionHandler(id);
static NSUncaughtExceptionHandler *user_handler;
void NSSetUncaughtExceptionHandler(NSUncaughtExceptionHandler *h) { user_handler = h; }
NSUncaughtExceptionHandler *NSGetUncaughtExceptionHandler(void) { return user_handler; }

/* The runtime's uncaught handler: the program's own, then macOS's message. */
static void uncaught(id e) {
    if (user_handler) user_handler(e);
    if (cf_is(e, objc_getClass("NSException")))
        fprintf(stderr, "*** Terminating app due to uncaught exception '%s', reason: '%s'\n", [[e name] UTF8String],
                [((NSException *)e)->_reason ? ((NSException *)e)->_reason : _prism_cf_string("") UTF8String]);
}
void _prism_ns_exception_init(void) {
    extern void (*objc_setUncaughtExceptionHandler(void (*)(id)))(id);
    objc_setUncaughtExceptionHandler(uncaught);
}
