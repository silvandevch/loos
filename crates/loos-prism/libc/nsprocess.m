/* NSProcessInfo, NSUUID, NSUserDefaults and CFPreferences. Defaults live where macOS keeps
 * them, ~/Library/Preferences/<domain>.plist, searched in macOS's order: the arguments
 * (`-key value`), the application's domain, NSGlobalDomain, then registered defaults. */
#include "foundation.h"

char ***_NSGetArgv(void);
int *_NSGetArgc(void);
char ***_NSGetEnviron(void);
int getpid(void);
int gethostname(char *, size_t);
int sysctlbyname(const char *, void *, size_t *, void *, size_t);
void arc4random_buf(void *, size_t);
uint64_t mach_absolute_time(void);
id NSHomeDirectory(void);
extern id const NSCocoaErrorDomain;

/* ------------------------------------------------------------------ NSUUID */

@implementation NSUUID
+ (id)UUID { return [[[self alloc] init] autorelease]; }
- (id)init {
    arc4random_buf(_b, 16);
    _b[6] = (_b[6] & 0x0f) | 0x40; /* version 4 */
    _b[8] = (_b[8] & 0x3f) | 0x80; /* RFC 4122 variant */
    return self;
}
- (id)initWithUUIDBytes:(const unsigned char *)b { memcpy(_b, b, 16); return self; }
- (id)initWithUUIDString:(id)s {
    const char *p = [s UTF8String];
    int n = 0;
    for (; *p && n < 32; p++) {
        int v = *p >= '0' && *p <= '9' ? *p - '0' : (*p | 32) >= 'a' && (*p | 32) <= 'f' ? (*p | 32) - 'a' + 10 : -1;
        if (*p == '-') continue;
        if (v < 0) break;
        _b[n / 2] = n % 2 ? _b[n / 2] | v : v << 4;
        n++;
    }
    if (n != 32 || [s length] != 36) { [self release]; return nil; }
    return self;
}
- (void)getUUIDBytes:(unsigned char *)out { memcpy(out, _b, 16); }
- (id)UUIDString {
    char t[37];
    snprintf(t, sizeof t, "%02X%02X%02X%02X-%02X%02X-%02X%02X-%02X%02X-%02X%02X%02X%02X%02X%02X", _b[0], _b[1], _b[2], _b[3], _b[4], _b[5], _b[6], _b[7],
             _b[8], _b[9], _b[10], _b[11], _b[12], _b[13], _b[14], _b[15]);
    return _prism_cf_string(t);
}
- (BOOL)isEqual:(id)o { return o == self || (cf_is(o, objc_getClass("NSUUID")) && !memcmp(_b, ((NSUUID *)o)->_b, 16)); }
- (NSUInteger)hash { NSUInteger h; memcpy(&h, _b, sizeof h); return h; }
- (id)copyWithZone:(NSZone *)z { (void)z; return [self retain]; }
- (id)description { return [NSString stringWithFormat:CFSTR("<__NSConcreteUUID %p> %@"), self, [self UUIDString]]; }
@end

/* ------------------------------------------------------------------ NSProcessInfo */

typedef struct { NSInteger majorVersion, minorVersion, patchVersion; } NSOperatingSystemVersion;

@interface NSProcessInfo : NSObject { @public id _name, _args, _env; }
@end
static NSProcessInfo *the_process;

static id sysctl_string(const char *name, const char *fallback) {
    char b[256];
    size_t n = sizeof b;
    return _prism_cf_string(sysctlbyname(name, b, &n, 0, 0) == 0 ? b : fallback);
}
static unsigned long long sysctl_int(const char *name, unsigned long long fallback) {
    unsigned long long v = 0;
    size_t n = sizeof v;
    return sysctlbyname(name, &v, &n, 0, 0) == 0 ? (n == 4 ? (unsigned)v : v) : fallback;
}

@implementation NSProcessInfo
+ (id)processInfo {
    if (!the_process) the_process = [NSProcessInfo new];
    return the_process;
}
- (id)arguments {
    if (!_args) {
        _args = [NSMutableArray new];
        char **v = *_NSGetArgv();
        for (int i = 0; i < *_NSGetArgc(); i++) [_args addObject:_prism_cf_string(v[i])];
    }
    return _args;
}
- (id)environment {
    if (!_env) {
        _env = [NSMutableDictionary new];
        for (char **e = *_NSGetEnviron(); e && *e; e++) {
            const char *eq = strchr(*e, '=');
            if (!eq) continue;
            id k = [[[NSString alloc] initWithBytes:*e length:eq - *e encoding:4] autorelease];
            [_env setObject:_prism_cf_string(eq + 1) forKey:k];
        }
    }
    return _env;
}
- (id)processName {
    if (!_name) { id a = [[self arguments] firstObject]; _name = [(a ? [a lastPathComponent] : CFSTR("")) copy]; }
    return _name;
}
- (void)setProcessName:(id)n { [_name release]; _name = [n copy]; }
- (int)processIdentifier { return getpid(); }
- (id)hostName { char b[256]; return gethostname(b, sizeof b) == 0 ? _prism_cf_string(b) : CFSTR("localhost"); }
- (id)globallyUniqueString {
    static unsigned long long counter;
    return [NSString stringWithFormat:CFSTR("%@-%d-%016llX"), [[NSUUID UUID] UUIDString], getpid(), ++counter];
}
- (id)operatingSystemVersionString {
    NSOperatingSystemVersion v = [self operatingSystemVersion];
    id build = sysctl_string("kern.osversion", "24G0");
    return v.patchVersion ? [NSString stringWithFormat:CFSTR("Version %ld.%ld.%ld (Build %@)"), v.majorVersion, v.minorVersion, v.patchVersion, build]
                          : [NSString stringWithFormat:CFSTR("Version %ld.%ld (Build %@)"), v.majorVersion, v.minorVersion, build];
}
- (NSOperatingSystemVersion)operatingSystemVersion {
    NSOperatingSystemVersion v = {0, 0, 0};
    const char *s = [sysctl_string("kern.osproductversion", "15.0") UTF8String];
    v.majorVersion = strtol(s, (char **)&s, 10);
    if (*s == '.') v.minorVersion = strtol(s + 1, (char **)&s, 10);
    if (*s == '.') v.patchVersion = strtol(s + 1, (char **)&s, 10);
    return v;
}
- (BOOL)isOperatingSystemAtLeastVersion:(NSOperatingSystemVersion)want {
    NSOperatingSystemVersion v = [self operatingSystemVersion];
    if (v.majorVersion != want.majorVersion) return v.majorVersion > want.majorVersion;
    if (v.minorVersion != want.minorVersion) return v.minorVersion > want.minorVersion;
    return v.patchVersion >= want.patchVersion;
}
- (NSUInteger)operatingSystem { return 5; /* NSMACHOperatingSystem */ }
- (id)operatingSystemName { return CFSTR("NSMACHOperatingSystem"); }
- (NSUInteger)processorCount { return sysctl_int("hw.ncpu", 1); }
- (NSUInteger)activeProcessorCount { return sysctl_int("hw.activecpu", 1); }
- (unsigned long long)physicalMemory { return sysctl_int("hw.memsize", 1ull << 30); }
- (double)systemUptime { return mach_absolute_time() / 1e9; }
- (NSInteger)thermalState { return 0; }
- (BOOL)isLowPowerModeEnabled { return NO; }
- (BOOL)isMacCatalystApp { return NO; }
- (BOOL)isiOSAppOnMac { return NO; }
- (id)userName { extern id NSUserName(void); return NSUserName(); }
- (id)fullUserName { extern id NSFullUserName(void); return NSFullUserName(); }
- (id)beginActivityWithOptions:(unsigned long long)o reason:(id)r { (void)o; (void)r; return [[NSObject new] autorelease]; }
- (void)endActivity:(id)a { (void)a; }
- (void)performActivityWithOptions:(unsigned long long)o reason:(id)r usingBlock:(void (^)(void))b { (void)o; (void)r; b(); }
- (void)disableSuddenTermination {}
- (void)enableSuddenTermination {}
- (void)disableAutomaticTermination:(id)r { (void)r; }
- (void)enableAutomaticTermination:(id)r { (void)r; }
@end

/* ------------------------------------------------------------------ NSUserDefaults */

id const NSGlobalDomain = CFSTR("NSGlobalDomain");
id const NSArgumentDomain = CFSTR("NSArgumentDomain");
id const NSRegistrationDomain = CFSTR("NSRegistrationDomain");
id const NSUserDefaultsDidChangeNotification = CFSTR("NSUserDefaultsDidChangeNotification");

@interface NSUserDefaults : NSObject {
@public
    id _suite;                   /* the persistent domain written to */
    id _domains;                 /* name → NSMutableDictionary, loaded */
    id _args, _registered;
}
@end
static NSUserDefaults *standard;

static id prefs_path(id domain) {
    id name = [domain isEqual:NSGlobalDomain] ? CFSTR(".GlobalPreferences") : domain;
    return [[[NSHomeDirectory() stringByAppendingPathComponent:CFSTR("Library/Preferences")] stringByAppendingPathComponent:name]
        stringByAppendingPathExtension:CFSTR("plist")];
}
static id app_domain(void) {
    id ident = [[NSBundle mainBundle] bundleIdentifier];
    return ident ? ident : [[NSProcessInfo processInfo] processName];
}
/* `-key value` pairs from the command line; the value is read as a property list when it is one. */
static id argument_domain(void) {
    id d = [NSMutableDictionary dictionary];
    id args = [[NSProcessInfo processInfo] arguments];
    for (NSUInteger i = 1; i + 1 < [args count]; i++) {
        id a = [args objectAtIndex:i];
        if (![a hasPrefix:CFSTR("-")] || [a length] < 2) continue;
        id raw = [args objectAtIndex:i + 1];
        id v = nil;
        @try { v = [raw propertyList]; } @catch (id e) { v = nil; }
        /* only (...), {...} and <...> are read as property lists; anything else stays text */
        if (!v || !([raw hasPrefix:CFSTR("(")] || [raw hasPrefix:CFSTR("{")] || [raw hasPrefix:CFSTR("<")])) v = raw;
        [d setObject:v forKey:[a substringFromIndex:1]];
        i++;
    }
    return d;
}

@implementation NSUserDefaults
+ (id)standardUserDefaults {
    if (!standard) standard = [[NSUserDefaults alloc] initWithSuiteName:nil];
    return standard;
}
+ (void)resetStandardUserDefaults {}
- (id)init { return [self initWithSuiteName:nil]; }
- (id)initWithSuiteName:(id)suite {
    if ([suite isEqual:NSGlobalDomain]) { [self release]; return nil; }
    _suite = [(suite ? suite : app_domain()) copy];
    _domains = [NSMutableDictionary new];
    _args = [argument_domain() retain];
    _registered = [NSMutableDictionary new];
    return self;
}
- (id)_domain:(id)name {
    id d = [_domains objectForKey:name];
    if (!d) {
        id f = _prism_plist_from_file(prefs_path(name));
        d = cf_is(f, objc_getClass("NSDictionary")) ? [[f mutableCopy] autorelease] : [NSMutableDictionary dictionary];
        [_domains setObject:d forKey:name];
    }
    return d;
}
- (BOOL)_save:(id)name {
    id path = prefs_path(name);
    [[NSFileManager defaultManager] createDirectoryAtPath:[path stringByDeletingLastPathComponent] withIntermediateDirectories:YES attributes:nil error:0];
    id data = _prism_plist_data([self _domain:name], 200);
    return data && _prism_write_file(path, data, YES);
}
- (void)_changed {
    [[NSNotificationCenter defaultCenter] postNotificationName:NSUserDefaultsDidChangeNotification object:self];
}
- (id)objectForKey:(id)k {
    id v;
    if ((v = [_args objectForKey:k])) return v;
    if ((v = [[self _domain:_suite] objectForKey:k])) return v;
    if ((v = [[self _domain:NSGlobalDomain] objectForKey:k])) return v;
    return [_registered objectForKey:k];
}
- (void)setObject:(id)v forKey:(id)k {
    if (!v) { [self removeObjectForKey:k]; return; }
    if (!CFPropertyListIsValid(v, 200))
        _prism_raise("NSInvalidArgumentException", "Attempt to insert non-property list object %s for key %s", [[v description] UTF8String], [[k description] UTF8String]);
    id copy = [v copy];
    [[self _domain:_suite] setObject:copy forKey:k];
    [copy release];
    [self _save:_suite];
    [self _changed];
}
- (void)removeObjectForKey:(id)k {
    [[self _domain:_suite] removeObjectForKey:k];
    [self _save:_suite];
    [self _changed];
}
- (void)registerDefaults:(id)d { [_registered addEntriesFromDictionary:d]; }
- (id)stringForKey:(id)k {
    id v = [self objectForKey:k];
    if ([v _cfTypeID] == 7) return v;
    if ([v _cfTypeID] == 22 || [v _cfTypeID] == 21) return [v stringValue];
    return nil;
}
- (id)arrayForKey:(id)k { id v = [self objectForKey:k]; return cf_is(v, objc_getClass("NSArray")) ? v : nil; }
- (id)dictionaryForKey:(id)k { id v = [self objectForKey:k]; return cf_is(v, objc_getClass("NSDictionary")) ? v : nil; }
- (id)dataForKey:(id)k { id v = [self objectForKey:k]; return cf_is(v, objc_getClass("NSData")) ? v : nil; }
- (id)stringArrayForKey:(id)k {
    id a = [self arrayForKey:k];
    for (id x in a) if ([x _cfTypeID] != 7) return nil;
    return a;
}
static BOOL text_bool(id s) {
    return [s caseInsensitiveCompare:CFSTR("YES")] == 0 || [s caseInsensitiveCompare:CFSTR("true")] == 0 || [s integerValue] != 0;
}
- (NSInteger)integerForKey:(id)k { id v = [self objectForKey:k]; return [v respondsToSelector:@selector(integerValue)] ? [v integerValue] : 0; }
- (double)doubleForKey:(id)k { id v = [self objectForKey:k]; return [v respondsToSelector:@selector(doubleValue)] ? [v doubleValue] : 0; }
- (float)floatForKey:(id)k { return [self doubleForKey:k]; }
- (BOOL)boolForKey:(id)k {
    id v = [self objectForKey:k];
    if ([v _cfTypeID] == 7) return text_bool(v);
    return [v respondsToSelector:@selector(boolValue)] ? [v boolValue] : NO;
}
- (id)URLForKey:(id)k {
    id v = [self objectForKey:k];
    if ([v _cfTypeID] == 7) return [[v stringByExpandingTildeInPath] isAbsolutePath] ? [NSURL fileURLWithPath:[v stringByExpandingTildeInPath]] : [NSURL URLWithString:v];
    return nil;
}
- (void)setInteger:(NSInteger)v forKey:(id)k { [self setObject:[NSNumber numberWithInteger:v] forKey:k]; }
- (void)setDouble:(double)v forKey:(id)k { [self setObject:[NSNumber numberWithDouble:v] forKey:k]; }
- (void)setFloat:(float)v forKey:(id)k { [self setObject:[NSNumber numberWithDouble:v] forKey:k]; }
- (void)setBool:(BOOL)v forKey:(id)k { [self setObject:[NSNumber numberWithBool:v] forKey:k]; }
- (void)setURL:(id)u forKey:(id)k { [self setObject:[u isFileURL] ? [u path] : [u absoluteString] forKey:k]; }
- (BOOL)synchronize { return YES; /* every change is written when it is made */ }
- (id)dictionaryRepresentation {
    id d = [NSMutableDictionary dictionaryWithDictionary:_registered];
    [d addEntriesFromDictionary:[self _domain:NSGlobalDomain]];
    [d addEntriesFromDictionary:[self _domain:_suite]];
    [d addEntriesFromDictionary:_args];
    return d;
}
- (id)volatileDomainForName:(id)n { return [n isEqual:NSArgumentDomain] ? _args : [n isEqual:NSRegistrationDomain] ? _registered : [NSDictionary dictionary]; }
- (id)persistentDomainForName:(id)n { return [[[self _domain:n] copy] autorelease]; }
- (void)setPersistentDomain:(id)d forName:(id)n {
    [_domains setObject:[[d mutableCopy] autorelease] forKey:n];
    [self _save:n];
    [self _changed];
}
- (void)removePersistentDomainForName:(id)n {
    [_domains setObject:[NSMutableDictionary dictionary] forKey:n];
    [[NSFileManager defaultManager] removeItemAtPath:prefs_path(n) error:0];
    [self _changed];
}
- (void)addSuiteNamed:(id)n { (void)n; }
- (void)removeSuiteNamed:(id)n { (void)n; }
- (BOOL)objectIsForcedForKey:(id)k { (void)k; return NO; }
@end

/* ------------------------------------------------------------------ CFPreferences */

id const kCFPreferencesCurrentApplication = CFSTR("kCFPreferencesCurrentApplication");
id const kCFPreferencesAnyApplication = CFSTR("kCFPreferencesAnyApplication");
id const kCFPreferencesCurrentUser = CFSTR("kCFPreferencesCurrentUser");
id const kCFPreferencesAnyUser = CFSTR("kCFPreferencesAnyUser");
id const kCFPreferencesCurrentHost = CFSTR("kCFPreferencesCurrentHost");
id const kCFPreferencesAnyHost = CFSTR("kCFPreferencesAnyHost");

static NSUserDefaults *for_app(id app) {
    if (!app || [app isEqual:kCFPreferencesCurrentApplication] || [app isEqual:app_domain()]) return [NSUserDefaults standardUserDefaults];
    static id suites;
    if (!suites) suites = [NSMutableDictionary new];
    id name = [app isEqual:kCFPreferencesAnyApplication] ? NSGlobalDomain : app;
    NSUserDefaults *d = [suites objectForKey:name];
    if (!d) {
        d = [[NSUserDefaults alloc] initWithSuiteName:CFSTR("_prism_placeholder")];
        [d->_suite release];
        d->_suite = [name copy];
        [suites setObject:d forKey:name];
        [d release];
    }
    return d;
}
id CFPreferencesCopyAppValue(id key, id app) { return [[for_app(app) objectForKey:key] retain]; }
id CFPreferencesCopyValue(id key, id app, id user, id host) { (void)user; (void)host; return CFPreferencesCopyAppValue(key, app); }
Boolean CFPreferencesGetAppBooleanValue(id key, id app, Boolean *valid) {
    id v = [for_app(app) objectForKey:key];
    if (valid) *valid = v != nil;
    return [for_app(app) boolForKey:key];
}
CFIndex CFPreferencesGetAppIntegerValue(id key, id app, Boolean *valid) {
    id v = [for_app(app) objectForKey:key];
    if (valid) *valid = v != nil;
    return [for_app(app) integerForKey:key];
}
void CFPreferencesSetAppValue(id key, id value, id app) { [for_app(app) setObject:value forKey:key]; }
void CFPreferencesSetValue(id key, id value, id app, id user, id host) { (void)user; (void)host; CFPreferencesSetAppValue(key, value, app); }
Boolean CFPreferencesAppSynchronize(id app) { return [for_app(app) synchronize]; }
Boolean CFPreferencesSynchronize(id app, id user, id host) { (void)user; (void)host; return CFPreferencesAppSynchronize(app); }
id CFPreferencesCopyKeyList(id app, id user, id host) {
    (void)user; (void)host;
    NSUserDefaults *d = for_app(app);
    return [[[d _domain:d->_suite] allKeys] retain];
}
void CFPreferencesAddSuitePreferencesToApp(id app, id suite) { (void)app; (void)suite; }
