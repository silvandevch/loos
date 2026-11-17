/* NSURL / CFURL, NSString's path methods, and NSBundle / CFBundle. Parsing, resolution, escapes
 * and path operations are in src/cf/url.rs; this layer keeps the strings and asks. */
#include "foundation.h"

char *__prism_cf_url_part(const char *s, unsigned long n, unsigned long part);
char *__prism_cf_url_resolve(const char *b, unsigned long bn, const char *r, unsigned long rn);
char *__prism_cf_url_unescape(const char *s, unsigned long n);
char *__prism_cf_url_escape(const char *s, unsigned long n, unsigned long kind);
char *__prism_cf_path(const char *s, unsigned long n, unsigned long op, const char *a, unsigned long an);

/* A malloc'ed C string as an autoreleased NSString (nil for NULL). */
static id take(char *p) {
    if (!p) return nil;
    id s = _prism_cf_string(p);
    free(p);
    return s;
}
static id path_op(id s, unsigned long op, id arg) {
    const char *a = s ? [s UTF8String] : "", *b = arg ? [arg UTF8String] : "";
    return take(__prism_cf_path(a, strlen(a), op, b, strlen(b)));
}

/* ------------------------------------------------------------------ NSString paths */

@implementation NSString (Paths)
- (id)lastPathComponent { return path_op(self, 0, nil); }
- (id)pathExtension { return path_op(self, 1, nil); }
- (id)stringByAppendingPathComponent:(id)c { return path_op(self, 2, c); }
- (id)stringByDeletingLastPathComponent { return path_op(self, 3, nil); }
- (id)stringByAppendingPathExtension:(id)e { return path_op(self, 4, e); }
- (id)stringByDeletingPathExtension { return path_op(self, 5, nil); }
- (id)stringByStandardizingPath {
    id s = self;
    if ([s hasPrefix:CFSTR("~")]) s = [self stringByExpandingTildeInPath];
    return path_op(s, 6, nil);
}
- (id)stringByExpandingTildeInPath {
    if (![self hasPrefix:CFSTR("~")]) return self;
    extern char *getenv(const char *);
    const char *home = getenv("HOME");
    id rest = [self substringFromIndex:1];
    return [_prism_cf_string(home ? home : "/") stringByAppendingString:rest];
}
- (id)stringByResolvingSymlinksInPath {
    char *r = realpath([self UTF8String], 0);
    return r ? take(r) : [self stringByStandardizingPath];
}
- (BOOL)isAbsolutePath { return [self hasPrefix:CFSTR("/")] || [self hasPrefix:CFSTR("~")]; }
- (id)pathComponents {
    id a = [NSMutableArray array];
    const char *s = [self UTF8String];
    if (*s == '/') [a addObject:CFSTR("/")];
    while (*s) {
        while (*s == '/') s++;
        const char *e = s;
        while (*e && *e != '/') e++;
        if (e > s) [a addObject:[[[NSString alloc] initWithBytes:s length:e - s encoding:4] autorelease]];
        s = e;
    }
    if ([self length] > 1 && [self hasSuffix:CFSTR("/")]) [a addObject:CFSTR("/")];
    return a;
}
+ (id)pathWithComponents:(id)parts {
    id s = [NSMutableString string];
    for (id p in parts) {
        if ([s length] && ![s hasSuffix:CFSTR("/")]) [s appendString:CFSTR("/")];
        [s appendString:p];
    }
    return s;
}
- (const char *)fileSystemRepresentation { return [self UTF8String]; }
- (BOOL)getFileSystemRepresentation:(char *)buf maxLength:(NSUInteger)n { return [self getCString:buf maxLength:n encoding:4]; }
- (id)stringByRemovingPercentEncoding { const char *s = [self UTF8String]; return take(__prism_cf_url_unescape(s, strlen(s))); }
- (id)stringByAddingPercentEscapesUsingEncoding:(NSUInteger)e { (void)e; const char *s = [self UTF8String]; return take(__prism_cf_url_escape(s, strlen(s), 1)); }
@end

/* ------------------------------------------------------------------ NSURL */

#define T_URL 29
@interface NSURL (Ivars)
@end

static id url_new(Class c, id string, id base) {
    if (!string) return nil;
    NSURL *u = class_createInstance(c ? c : objc_getClass("NSURL"), 0);
    u->_string = [string copy];
    u->_base = [base retain];
    return u;
}
/* The absolute text of `u`. */
static id absolute(NSURL *u) {
    if (!u->_base) return u->_string;
    id b = absolute(u->_base);
    const char *bs = [b UTF8String], *rs = [u->_string UTF8String];
    return take(__prism_cf_url_resolve(bs, strlen(bs), rs, strlen(rs)));
}
static id part(NSURL *u, unsigned long which) {
    const char *s = [absolute(u) UTF8String];
    return take(__prism_cf_url_part(s, strlen(s), which));
}
static id file_url_text(id path, BOOL dir) {
    if (![path isAbsolutePath]) {
        char cwd[1024];
        path = [_prism_cf_string(getcwd(cwd, sizeof cwd) ? cwd : "/") stringByAppendingPathComponent:path];
    }
    path = [path stringByStandardizingPath];
    const char *p = [path UTF8String];
    id esc = take(__prism_cf_url_escape(p, strlen(p), 0));
    if (dir && ![esc hasSuffix:CFSTR("/")]) esc = [esc stringByAppendingString:CFSTR("/")];
    return [CFSTR("file://") stringByAppendingString:esc];
}
static BOOL is_dir(id path) {
    struct { char pad[4]; unsigned short mode; char rest[200]; } st; /* struct stat, macOS layout: st_mode at 4 */
    extern int stat(const char *, void *) __asm__("_stat$INODE64");
    return stat([path UTF8String], &st) == 0 && (st.mode & 0170000) == 0040000;
}

@implementation NSURL
+ (id)URLWithString:(id)s { return [url_new(self, s, nil) autorelease]; }
+ (id)URLWithString:(id)s relativeToURL:(NSURL *)base { return [url_new(self, s, base) autorelease]; }
+ (id)fileURLWithPath:(id)p { return [url_new(self, file_url_text(p, is_dir(p)), nil) autorelease]; }
+ (id)fileURLWithPath:(id)p isDirectory:(BOOL)d { return [url_new(self, file_url_text(p, d), nil) autorelease]; }
+ (id)fileURLWithFileSystemRepresentation:(const char *)p isDirectory:(BOOL)d relativeToURL:(NSURL *)b {
    (void)b;
    return [self fileURLWithPath:_prism_cf_string(p) isDirectory:d];
}
- (id)initWithString:(id)s { [self release]; return url_new(0, s, nil); }
- (id)initWithString:(id)s relativeToURL:(NSURL *)b { [self release]; return url_new(0, s, b); }
- (id)initFileURLWithPath:(id)p { [self release]; return url_new(0, file_url_text(p, is_dir(p)), nil); }
- (id)initFileURLWithPath:(id)p isDirectory:(BOOL)d { [self release]; return url_new(0, file_url_text(p, d), nil); }
- (id)absoluteString { return absolute(self); }
- (id)relativeString { return _string; }
- (id)baseURL { return _base; }
- (id)absoluteURL { return _base ? [[self class] URLWithString:absolute(self)] : self; }
- (id)scheme { return part(self, 0); }
- (id)user { return part(self, 1); }
- (id)password { return part(self, 2); }
- (id)host { return part(self, 3); }
- (id)port { id p = part(self, 4); return p ? [NSNumber numberWithInteger:[p integerValue]] : nil; }
- (id)path { return part(self, 11); }
- (id)relativePath { const char *s = [_string UTF8String]; return take(__prism_cf_url_part(s, strlen(s), 11)); }
- (id)query { return part(self, 6); }
- (id)fragment { return part(self, 7); }
- (id)resourceSpecifier { return part(self, 8); }
- (id)lastPathComponent { return part(self, 9); }
- (id)pathExtension { return part(self, 10); }
- (id)pathComponents { return [[self path] pathComponents]; }
- (BOOL)hasDirectoryPath { id p = part(self, 5); return [p hasSuffix:CFSTR("/")]; }
- (BOOL)isFileURL { id s = [self scheme]; return s && [s caseInsensitiveCompare:CFSTR("file")] == 0; }
- (const char *)fileSystemRepresentation { return [[self path] UTF8String]; }
- (BOOL)getFileSystemRepresentation:(char *)b maxLength:(NSUInteger)n { return [[self path] getCString:b maxLength:n encoding:4]; }
- (id)URLByAppendingPathComponent:(id)c {
    if ([self isFileURL]) return [NSURL fileURLWithPath:[[self path] stringByAppendingPathComponent:c] isDirectory:[c hasSuffix:CFSTR("/")]];
    id s = absolute(self);
    if (![s hasSuffix:CFSTR("/")]) s = [s stringByAppendingString:CFSTR("/")];
    const char *e = [c UTF8String];
    return [NSURL URLWithString:[s stringByAppendingString:take(__prism_cf_url_escape(e, strlen(e), 0))]];
}
- (id)URLByAppendingPathComponent:(id)c isDirectory:(BOOL)d {
    id u = [self URLByAppendingPathComponent:c];
    return d && ![u hasDirectoryPath] ? [NSURL URLWithString:[[u absoluteString] stringByAppendingString:CFSTR("/")]] : u;
}
- (id)URLByDeletingLastPathComponent {
    if ([self isFileURL]) return [NSURL fileURLWithPath:[[self path] stringByDeletingLastPathComponent] isDirectory:YES];
    /* "./" against the URL drops its last segment; a directory URL needs "../" */
    return [[NSURL URLWithString:[self hasDirectoryPath] ? CFSTR("../") : CFSTR("./") relativeToURL:self] absoluteURL];
}
- (id)URLByAppendingPathExtension:(id)e {
    if ([self isFileURL]) return [NSURL fileURLWithPath:[[self path] stringByAppendingPathExtension:e]];
    return [NSURL URLWithString:[absolute(self) stringByAppendingFormat:CFSTR(".%@"), e]];
}
- (id)URLByDeletingPathExtension {
    if ([self isFileURL]) return [NSURL fileURLWithPath:[[self path] stringByDeletingPathExtension]];
    return self;
}
- (id)URLByStandardizingPath { return [self isFileURL] ? [NSURL fileURLWithPath:[[self path] stringByStandardizingPath]] : self; }
- (id)URLByResolvingSymlinksInPath { return [self isFileURL] ? [NSURL fileURLWithPath:[[self path] stringByResolvingSymlinksInPath]] : self; }
- (BOOL)checkResourceIsReachableAndReturnError:(id *)e { if (e) *e = nil; return [self isFileURL] && access([[self path] UTF8String], 0) == 0; }
- (BOOL)isEqual:(id)o { return o == self || (cf_is(o, objc_getClass("NSURL")) && [absolute(self) isEqual:absolute(o)]); }
- (NSUInteger)hash { return [absolute(self) hash]; }
- (id)copyWithZone:(NSZone *)z { (void)z; return [self retain]; }
- (id)description { return _base ? [_string stringByAppendingFormat:CFSTR(" -- %@"), _base] : _string; }
- (CFTypeID)_cfTypeID { return T_URL; }
- (void)dealloc { [_string release]; [_base release]; [super dealloc]; }
@end

/* ------------------------------------------------------------------ CFURL */

enum { kCFURLPOSIXPathStyle = 0 };
CFTypeID CFURLGetTypeID(void) { return T_URL; }
id CFURLCreateWithString(CFAllocatorRef a, id s, id base) { (void)a; return url_new(0, s, base); }
id CFURLCreateAbsoluteURLWithBytes(CFAllocatorRef a, const UInt8 *b, CFIndex n, unsigned enc, id base, Boolean useCompat) {
    (void)a; (void)enc; (void)useCompat;
    id s = [[[NSString alloc] initWithBytes:b length:n encoding:4] autorelease];
    NSURL *u = url_new(0, s, base);
    id abs = [[u absoluteURL] retain];
    [u release];
    return abs;
}
id CFURLCreateWithBytes(CFAllocatorRef a, const UInt8 *b, CFIndex n, unsigned enc, id base) {
    (void)a; (void)enc;
    return url_new(0, [[[NSString alloc] initWithBytes:b length:n encoding:4] autorelease], base);
}
id CFURLCreateWithFileSystemPath(CFAllocatorRef a, id path, CFIndex style, Boolean dir) { (void)a; (void)style; return url_new(0, file_url_text(path, dir), nil); }
id CFURLCreateWithFileSystemPathRelativeToBase(CFAllocatorRef a, id path, CFIndex style, Boolean dir, id base) {
    (void)a; (void)style;
    if ([path isAbsolutePath] || !base) return url_new(0, file_url_text(path, dir), nil);
    return url_new(0, file_url_text([[base path] stringByAppendingPathComponent:path], dir), nil);
}
id CFURLCreateFromFileSystemRepresentation(CFAllocatorRef a, const UInt8 *b, CFIndex n, Boolean dir) {
    (void)a;
    return url_new(0, file_url_text([[[NSString alloc] initWithBytes:b length:n encoding:4] autorelease], dir), nil);
}
Boolean CFURLGetFileSystemRepresentation(id u, Boolean resolve, UInt8 *buf, CFIndex max) { (void)resolve; return [[u path] getCString:(char *)buf maxLength:max encoding:4]; }
id CFURLGetString(id u) { return ((NSURL *)u)->_string; }
id CFURLGetBaseURL(id u) { return ((NSURL *)u)->_base; }
id CFURLCopyAbsoluteURL(id u) { return [[u absoluteURL] retain]; }
id CFURLCopyScheme(id u) { return [[u scheme] retain]; }
id CFURLCopyHostName(id u) { return [[u host] retain]; }
SInt32 CFURLGetPortNumber(id u) { id p = [u port]; return p ? [p intValue] : -1; }
id CFURLCopyUserName(id u) { return [[u user] retain]; }
id CFURLCopyPassword(id u) { return [[u password] retain]; }
id CFURLCopyPath(id u) { return [part(u, 5) retain]; }
id CFURLCopyFileSystemPath(id u, CFIndex style) { (void)style; return [[u path] retain]; }
id CFURLCopyQueryString(id u, id esc) { (void)esc; return [[u query] retain]; }
id CFURLCopyFragment(id u, id esc) { (void)esc; return [[u fragment] retain]; }
id CFURLCopyResourceSpecifier(id u) { return [[u resourceSpecifier] retain]; }
id CFURLCopyLastPathComponent(id u) { return [[u lastPathComponent] retain]; }
id CFURLCopyPathExtension(id u) { id e = [u pathExtension]; return [e length] ? [e retain] : nil; }
Boolean CFURLHasDirectoryPath(id u) { return [u hasDirectoryPath]; }
id CFURLCreateCopyAppendingPathComponent(CFAllocatorRef a, id u, id c, Boolean dir) { (void)a; return [[u URLByAppendingPathComponent:c isDirectory:dir] retain]; }
id CFURLCreateCopyDeletingLastPathComponent(CFAllocatorRef a, id u) { (void)a; return [[u URLByDeletingLastPathComponent] retain]; }
id CFURLCreateCopyAppendingPathExtension(CFAllocatorRef a, id u, id e) { (void)a; return [[u URLByAppendingPathExtension:e] retain]; }
id CFURLCreateCopyDeletingPathExtension(CFAllocatorRef a, id u) { (void)a; return [[u URLByDeletingPathExtension] retain]; }
id CFURLCreateStringByReplacingPercentEscapes(CFAllocatorRef a, id s, id keep) { (void)a; (void)keep; return [[s stringByRemovingPercentEncoding] retain]; }
Boolean CFURLResourceIsReachable(id u, void *err) { (void)err; return [u checkResourceIsReachableAndReturnError:0]; }

/* ------------------------------------------------------------------ NSBundle */

#define T_BUNDLE 31
id _prism_plist_from_file(id path); /* cfplist.m */

static NSBundle *main_bundle;
static id bundles; /* path → bundle */

static NSBundle *bundle_at(id path) {
    if (!path) return nil;
    path = [path stringByStandardizingPath];
    if (!bundles) bundles = [NSMutableDictionary new];
    NSBundle *b = [bundles objectForKey:path];
    if (b) return b;
    if (!is_dir(path)) return nil;
    b = [NSBundle new];
    b->_path = [path copy];
    id contents = [path stringByAppendingPathComponent:CFSTR("Contents")];
    b->_app = is_dir(contents);
    id info_dir = b->_app ? contents : path;
    b->_resources = [(b->_app ? [contents stringByAppendingPathComponent:CFSTR("Resources")] : path) copy];
    id info = _prism_plist_from_file([info_dir stringByAppendingPathComponent:CFSTR("Info.plist")]);
    if (!info && !b->_app) info = _prism_plist_from_file([[path stringByAppendingPathComponent:CFSTR("Resources")] stringByAppendingPathComponent:CFSTR("Info.plist")]);
    b->_info = [(cf_is(info, objc_getClass("NSDictionary")) ? info : [NSDictionary dictionary]) retain];
    id exe = [b->_info objectForKey:CFSTR("CFBundleExecutable")];
    if (exe) b->_executable = [[[contents stringByAppendingPathComponent:CFSTR("MacOS")] stringByAppendingPathComponent:exe] copy];
    [bundles setObject:b forKey:path];
    [b release];
    return b;
}

@implementation NSBundle
+ (id)mainBundle {
    if (main_bundle) return main_bundle;
    char buf[1024];
    uint32_t n = sizeof buf;
    id exe = _NSGetExecutablePath(buf, &n) == 0 ? [_prism_cf_string(buf) stringByResolvingSymlinksInPath] : nil;
    id dir = [exe stringByDeletingLastPathComponent];
    /* An app's executable is <bundle>/Contents/MacOS/<name>. */
    if ([[dir lastPathComponent] isEqual:CFSTR("MacOS")] && [[[dir stringByDeletingLastPathComponent] lastPathComponent] isEqual:CFSTR("Contents")])
        main_bundle = bundle_at([[dir stringByDeletingLastPathComponent] stringByDeletingLastPathComponent]);
    if (!main_bundle) {
        main_bundle = bundle_at(dir);
        main_bundle->_app = NO;
    }
    if (!main_bundle->_executable) main_bundle->_executable = [exe copy];
    [main_bundle retain];
    return main_bundle;
}
+ (id)bundleWithPath:(id)p { return bundle_at(p); }
+ (id)bundleWithURL:(id)u { return bundle_at([u path]); }
- (id)initWithPath:(id)p { [self release]; return [bundle_at(p) retain]; }
+ (id)bundleForClass:(Class)c { (void)c; return [self mainBundle]; }
+ (id)bundleWithIdentifier:(id)ident {
    for (id k in bundles) {
        NSBundle *b = [bundles objectForKey:k];
        if ([[b bundleIdentifier] isEqual:ident]) return b;
    }
    return [[[self mainBundle] bundleIdentifier] isEqual:ident] ? main_bundle : nil;
}
+ (id)allBundles { return [self mainBundle] ? [NSArray arrayWithObject:main_bundle] : [NSArray array]; }
- (id)bundlePath { return _path; }
- (id)bundleURL { return [NSURL fileURLWithPath:_path isDirectory:YES]; }
- (id)resourcePath { return _resources; }
- (id)resourceURL { return [NSURL fileURLWithPath:_resources isDirectory:YES]; }
- (id)executablePath { return _executable; }
- (id)executableURL { return _executable ? [NSURL fileURLWithPath:_executable isDirectory:NO] : nil; }
- (id)privateFrameworksPath { return [_path stringByAppendingPathComponent:_app ? CFSTR("Contents/Frameworks") : CFSTR("Frameworks")]; }
- (id)builtInPlugInsPath { return [_path stringByAppendingPathComponent:_app ? CFSTR("Contents/PlugIns") : CFSTR("PlugIns")]; }
- (id)bundleIdentifier { return [_info objectForKey:CFSTR("CFBundleIdentifier")]; }
- (id)infoDictionary { return _info; }
- (id)localizedInfoDictionary { return _info; }
- (id)objectForInfoDictionaryKey:(id)k { return [_info objectForKey:k]; }
- (BOOL)load { return YES; }
- (BOOL)isLoaded { return YES; }
- (id)pathForResource:(id)name ofType:(id)ext inDirectory:(id)sub {
    id dir = sub ? [_resources stringByAppendingPathComponent:sub] : _resources;
    id file = [ext length] ? [name stringByAppendingPathExtension:ext] : name;
    /* the resources directory, then each .lproj (English first) */
    id p = [dir stringByAppendingPathComponent:file];
    if (access([p UTF8String], 0) == 0) return p;
    for (id l in [NSArray arrayWithObjects:CFSTR("en.lproj"), CFSTR("English.lproj"), CFSTR("Base.lproj"), nil]) {
        p = [[dir stringByAppendingPathComponent:l] stringByAppendingPathComponent:file];
        if (access([p UTF8String], 0) == 0) return p;
    }
    return nil;
}
- (id)pathForResource:(id)name ofType:(id)ext { return [self pathForResource:name ofType:ext inDirectory:nil]; }
- (id)URLForResource:(id)name withExtension:(id)ext subdirectory:(id)sub {
    id p = [self pathForResource:name ofType:ext inDirectory:sub];
    return p ? [NSURL fileURLWithPath:p] : nil;
}
- (id)URLForResource:(id)name withExtension:(id)ext { return [self URLForResource:name withExtension:ext subdirectory:nil]; }
- (id)localizedStringForKey:(id)key value:(id)value table:(id)table {
    id path = [self pathForResource:table ? table : CFSTR("Localizable") ofType:CFSTR("strings")];
    id strings = path ? _prism_plist_from_file(path) : nil;
    id s = cf_is(strings, objc_getClass("NSDictionary")) ? [strings objectForKey:key] : nil;
    return s ? s : [value length] ? value : key;
}
- (id)description { return [NSString stringWithFormat:CFSTR("NSBundle <%@> (loaded)"), _path]; }
- (CFTypeID)_cfTypeID { return T_BUNDLE; }
@end

/* ------------------------------------------------------------------ CFBundle */

CFTypeID CFBundleGetTypeID(void) { return T_BUNDLE; }
id CFBundleGetMainBundle(void) { return [NSBundle mainBundle]; }
id CFBundleCreate(CFAllocatorRef a, id url) { (void)a; return [bundle_at([url path]) retain]; }
id CFBundleGetBundleWithIdentifier(id ident) { return [NSBundle bundleWithIdentifier:ident]; }
id CFBundleCopyBundleURL(id b) { return [[b bundleURL] retain]; }
id CFBundleCopyResourcesDirectoryURL(id b) { return [[b resourceURL] retain]; }
id CFBundleCopyExecutableURL(id b) { return [[b executableURL] retain]; }
id CFBundleCopyPrivateFrameworksURL(id b) { return [[NSURL fileURLWithPath:[b privateFrameworksPath] isDirectory:YES] retain]; }
id CFBundleCopyBuiltInPlugInsURL(id b) { return [[NSURL fileURLWithPath:[b builtInPlugInsPath] isDirectory:YES] retain]; }
id CFBundleGetIdentifier(id b) { return [b bundleIdentifier]; }
id CFBundleGetInfoDictionary(id b) { return [b infoDictionary]; }
id CFBundleGetLocalInfoDictionary(id b) { return [b infoDictionary]; }
id CFBundleGetValueForInfoDictionaryKey(id b, id k) { return [b objectForInfoDictionaryKey:k]; }
id CFBundleCopyResourceURL(id b, id name, id type, id sub) { return [[b URLForResource:name withExtension:type subdirectory:sub] retain]; }
id CFBundleCopyLocalizedString(id b, id key, id value, id table) { return [[b localizedStringForKey:key value:value table:table] retain]; }
UInt32 CFBundleGetVersionNumber(id b) {
    id v = [b objectForInfoDictionaryKey:CFSTR("CFBundleVersion")];
    return v ? (UInt32)[v intValue] << 24 : 0;
}
Boolean CFBundleLoadExecutable(id b) { (void)b; return 1; }
Boolean CFBundleIsExecutableLoaded(id b) { (void)b; return 1; }
void *CFBundleGetFunctionPointerForName(id b, id name) {
    (void)b;
    extern void *dlsym(void *, const char *);
    return dlsym((void *)-2, [name UTF8String]);
}
