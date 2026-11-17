/* NSFileManager, NSDirectoryEnumerator and the path functions (NSHomeDirectory,
 * NSTemporaryDirectory, NSSearchPathForDirectoriesInDomains), on libSystem's file calls. */
#include "foundation.h"

/* struct stat (64-bit inodes) and struct dirent, macOS layouts. */
struct mstat {
    int32_t dev; uint16_t mode, nlink; uint64_t ino; uint32_t uid, gid; int32_t rdev;
    struct timespec atime, mtime, ctime, birthtime;
    int64_t size, blocks; int32_t blksize; uint32_t flags, gen; int32_t spare; int64_t qspare[2];
};
struct mdirent { uint64_t ino, seekoff; uint16_t reclen, namlen; uint8_t type; char name[1024]; };
int stat_(const char *, struct mstat *) __asm__("_stat$INODE64");
int lstat_(const char *, struct mstat *) __asm__("_lstat$INODE64");
void *opendir(const char *) __asm__("_opendir$INODE64");
struct mdirent *readdir(void *) __asm__("_readdir$INODE64");
int closedir(void *);
int mkdir(const char *, unsigned short);
int rmdir(const char *);
int unlink(const char *);
int chdir(const char *);
int chmod(const char *, unsigned short);
int symlink(const char *, const char *);
long readlink(const char *, char *, size_t);
char *getenv(const char *);
struct passwd { char *pw_name, *pw_passwd; uint32_t pw_uid, pw_gid; long pw_change; char *pw_class, *pw_gecos, *pw_dir, *pw_shell; long pw_expire; };
struct passwd *getpwnam(const char *);
struct passwd *getpwuid(uint32_t);
uint32_t getuid(void);

#define S_IFMT 0170000
#define S_IFDIR 0040000
#define S_IFREG 0100000
#define S_IFLNK 0120000

id const NSFileType = CFSTR("NSFileType");
id const NSFileTypeDirectory = CFSTR("NSFileTypeDirectory");
id const NSFileTypeRegular = CFSTR("NSFileTypeRegular");
id const NSFileTypeSymbolicLink = CFSTR("NSFileTypeSymbolicLink");
id const NSFileTypeCharacterSpecial = CFSTR("NSFileTypeCharacterSpecial");
id const NSFileTypeBlockSpecial = CFSTR("NSFileTypeBlockSpecial");
id const NSFileTypeSocket = CFSTR("NSFileTypeSocket");
id const NSFileTypeUnknown = CFSTR("NSFileTypeUnknown");
id const NSFileSize = CFSTR("NSFileSize");
id const NSFileModificationDate = CFSTR("NSFileModificationDate");
id const NSFileCreationDate = CFSTR("NSFileCreationDate");
id const NSFilePosixPermissions = CFSTR("NSFilePosixPermissions");
id const NSFileOwnerAccountID = CFSTR("NSFileOwnerAccountID");
id const NSFileGroupOwnerAccountID = CFSTR("NSFileGroupOwnerAccountID");
id const NSFileReferenceCount = CFSTR("NSFileReferenceCount");
id const NSFileSystemNumber = CFSTR("NSFileSystemNumber");
id const NSFileSystemFileNumber = CFSTR("NSFileSystemFileNumber");

static BOOL fail(id *error, id path) {
    if (error) *error = _prism_file_error(errno, path);
    return NO;
}
static const char *fs(id path) { return [path fileSystemRepresentation]; }

/* ------------------------------------------------------------------ paths */

id NSUserName(void) {
    struct passwd *p = getpwuid(getuid());
    return _prism_cf_string(p && p->pw_name ? p->pw_name : "user");
}
id NSHomeDirectory(void) {
    const char *h = getenv("HOME");
    if (!h || !*h) { struct passwd *p = getpwuid(getuid()); h = p && p->pw_dir ? p->pw_dir : "/"; }
    return _prism_cf_string(h);
}
id NSHomeDirectoryForUser(id user) {
    struct passwd *p = user ? getpwnam([user UTF8String]) : getpwuid(getuid());
    return p && p->pw_dir ? _prism_cf_string(p->pw_dir) : nil;
}
id NSTemporaryDirectory(void) {
    const char *t = getenv("TMPDIR");
    id s = _prism_cf_string(t && *t ? t : "/tmp/");
    return [s hasSuffix:CFSTR("/")] ? s : [s stringByAppendingString:CFSTR("/")];
}
id NSOpenStepRootDirectory(void) { return CFSTR("/"); }
id NSFullUserName(void) {
    struct passwd *p = getpwuid(getuid());
    return _prism_cf_string(p && p->pw_gecos && *p->pw_gecos ? p->pw_gecos : p && p->pw_name ? p->pw_name : "");
}

/* NSSearchPathDirectory values and the folder each names under a domain's root. */
static const char *directory_name(NSUInteger d) {
    switch (d) {
    case 1: return "Applications";
    case 4: return "Developer";
    case 5: return "Library";
    case 9: return "Documents";
    case 12: return "Desktop";
    case 13: return "Library/Caches";
    case 14: return "Library/Application Support";
    case 15: return "Downloads";
    case 17: return "Movies";
    case 18: return "Music";
    case 19: return "Pictures";
    case 21: return "Public";
    case 7: return "Library/Logs"; /* NSAdminApplicationDirectory is 4 in some SDKs; logs are Library/Logs */
    default: return 0;
    }
}
id NSSearchPathForDirectoriesInDomains(NSUInteger dir, NSUInteger domains, BOOL expand) {
    id out = [NSMutableArray array];
    const char *name = directory_name(dir);
    if (!name) return out;
    /* NSUserDomainMask 1, NSLocalDomainMask 2, NSNetworkDomainMask 4, NSSystemDomainMask 8 */
    if (domains & 1) {
        id base = expand ? NSHomeDirectory() : CFSTR("~");
        [out addObject:[base stringByAppendingPathComponent:_prism_cf_string(name)]];
    }
    BOOL user_only = dir == 9 || dir == 12 || dir == 15 || dir == 17 || dir == 18 || dir == 19 || dir == 21;
    if ((domains & 2) && !user_only) [out addObject:[CFSTR("/") stringByAppendingPathComponent:_prism_cf_string(name)]];
    if ((domains & 8) && !user_only && dir != 1 && dir != 14)
        [out addObject:[CFSTR("/System") stringByAppendingPathComponent:_prism_cf_string(name)]];
    return out;
}

/* ------------------------------------------------------------------ NSDirectoryEnumerator */

@interface NSDirectoryEnumerator : NSEnumerator {
@public
    id _root, _stack, _attrs; /* _stack: arrays of remaining relative paths, deepest last */
    BOOL _skip;
}
@end

static id list_dir(id path, id *error) {
    void *d = opendir(fs(path));
    if (!d) { if (error) *error = _prism_file_error(errno, path); return nil; }
    id out = [NSMutableArray array];
    for (struct mdirent *e; (e = readdir(d));) {
        if (!strcmp(e->name, ".") || !strcmp(e->name, "..")) continue;
        [out addObject:_prism_cf_string(e->name)];
    }
    closedir(d);
    return [out sortedArrayUsingSelector:@selector(compare:)];
}
static id attributes(id path, BOOL follow, id *error);

@implementation NSDirectoryEnumerator
- (id)nextObject {
    [_attrs release];
    _attrs = nil;
    for (;;) {
        id top = [_stack lastObject];
        if (!top) return nil;
        if (![top count]) { [_stack removeLastObject]; continue; }
        id rel = [[[top objectAtIndex:0] retain] autorelease];
        [top removeObjectAtIndex:0];
        id full = [_root stringByAppendingPathComponent:rel];
        struct mstat st;
        if (lstat_(fs(full), &st) == 0 && (st.mode & S_IFMT) == S_IFDIR) {
            id kids = [NSMutableArray array];
            for (id k in list_dir(full, 0)) [kids addObject:[rel stringByAppendingPathComponent:k]];
            [_stack addObject:kids];
        }
        _attrs = [attributes(full, NO, 0) retain];
        return rel;
    }
}
- (void)skipDescendants { if ([_stack count] > 1) [_stack removeLastObject]; }
- (void)skipDescendents { [self skipDescendants]; }
- (id)fileAttributes { return _attrs; }
- (NSUInteger)level { return [_stack count]; }
- (void)dealloc { [_root release]; [_stack release]; [_attrs release]; [super dealloc]; }
@end

/* ------------------------------------------------------------------ NSFileManager */

static id attributes(id path, BOOL follow, id *error) {
    struct mstat st;
    if ((follow ? stat_(fs(path), &st) : lstat_(fs(path), &st)) != 0) { fail(error, path); return nil; }
    id type;
    switch (st.mode & S_IFMT) {
    case S_IFDIR: type = NSFileTypeDirectory; break;
    case S_IFREG: type = NSFileTypeRegular; break;
    case S_IFLNK: type = NSFileTypeSymbolicLink; break;
    case 0020000: type = NSFileTypeCharacterSpecial; break;
    case 0060000: type = NSFileTypeBlockSpecial; break;
    case 0140000: type = NSFileTypeSocket; break;
    default: type = NSFileTypeUnknown;
    }
    double mt = st.mtime.tv_sec - 978307200.0 + st.mtime.tv_nsec / 1e9;
    double bt = st.birthtime.tv_sec ? st.birthtime.tv_sec - 978307200.0 + st.birthtime.tv_nsec / 1e9 : mt;
    return [NSDictionary dictionaryWithObjectsAndKeys:
        type, NSFileType,
        [NSNumber numberWithUnsignedLongLong:st.size], NSFileSize,
        [NSDate dateWithTimeIntervalSinceReferenceDate:mt], NSFileModificationDate,
        [NSDate dateWithTimeIntervalSinceReferenceDate:bt], NSFileCreationDate,
        [NSNumber numberWithUnsignedShort:st.mode & 07777], NSFilePosixPermissions,
        [NSNumber numberWithUnsignedInt:st.uid], NSFileOwnerAccountID,
        [NSNumber numberWithUnsignedInt:st.gid], NSFileGroupOwnerAccountID,
        [NSNumber numberWithUnsignedShort:st.nlink], NSFileReferenceCount,
        [NSNumber numberWithInt:st.dev], NSFileSystemNumber,
        [NSNumber numberWithUnsignedLongLong:st.ino], NSFileSystemFileNumber,
        nil];
}

static BOOL copy_file(id from, id to, id *error) {
    int in = open(fs(from), 0);
    if (in < 0) return fail(error, from);
    struct mstat st;
    stat_(fs(from), &st);
    int out = open(fs(to), 0xa01 /* O_WRONLY|O_CREAT|O_EXCL */, st.mode & 07777);
    if (out < 0) { close(in); return fail(error, to); }
    char buf[16384];
    long n;
    BOOL ok = YES;
    while ((n = read(in, buf, sizeof buf)) > 0)
        for (char *p = buf; n > 0;) {
            long w = write(out, p, n);
            if (w <= 0) { ok = NO; break; }
            p += w;
            n -= w;
        }
    if (n < 0) ok = NO;
    close(in);
    close(out);
    return ok ? YES : fail(error, to);
}

static NSFileManager *default_manager;

@implementation NSFileManager
+ (id)defaultManager {
    if (!default_manager) default_manager = [NSFileManager new];
    return default_manager;
}
- (id)delegate { return _delegate; }
- (void)setDelegate:(id)d { _delegate = d; }

- (BOOL)fileExistsAtPath:(id)p { struct mstat st; return p && stat_(fs(p), &st) == 0; }
- (BOOL)fileExistsAtPath:(id)p isDirectory:(BOOL *)dir {
    struct mstat st;
    if (!p || stat_(fs(p), &st) != 0) return NO;
    if (dir) *dir = (st.mode & S_IFMT) == S_IFDIR;
    return YES;
}
- (BOOL)isReadableFileAtPath:(id)p { return access(fs(p), 4) == 0; }
- (BOOL)isWritableFileAtPath:(id)p { return access(fs(p), 2) == 0; }
- (BOOL)isExecutableFileAtPath:(id)p { return access(fs(p), 1) == 0; }
- (BOOL)isDeletableFileAtPath:(id)p { return access(fs([p stringByDeletingLastPathComponent]), 2) == 0; }

- (id)currentDirectoryPath { char b[1024]; return getcwd(b, sizeof b) ? _prism_cf_string(b) : nil; }
- (BOOL)changeCurrentDirectoryPath:(id)p { return chdir(fs(p)) == 0; }

- (BOOL)createDirectoryAtPath:(id)p withIntermediateDirectories:(BOOL)inter attributes:(id)attrs error:(id *)error {
    unsigned short mode = attrs && [attrs objectForKey:NSFilePosixPermissions] ? [[attrs objectForKey:NSFilePosixPermissions] unsignedShortValue] : 0777;
    if (inter) {
        BOOL dir;
        if ([self fileExistsAtPath:p isDirectory:&dir]) {
            if (dir) return YES;
            errno = 17;
            return fail(error, p);
        }
        id parent = [p stringByDeletingLastPathComponent];
        if ([parent length] && ![parent isEqual:p] && ![self createDirectoryAtPath:parent withIntermediateDirectories:YES attributes:attrs error:error]) return NO;
    }
    if (mkdir(fs(p), mode) != 0) return fail(error, p);
    return YES;
}
- (BOOL)createDirectoryAtURL:(id)u withIntermediateDirectories:(BOOL)i attributes:(id)a error:(id *)e { return [self createDirectoryAtPath:[u path] withIntermediateDirectories:i attributes:a error:e]; }
- (BOOL)createFileAtPath:(id)p contents:(id)data attributes:(id)attrs {
    if (!_prism_write_file(p, data ? data : [NSData data], YES)) return NO;
    id perm = [attrs objectForKey:NSFilePosixPermissions];
    if (perm) chmod(fs(p), [perm unsignedShortValue]);
    return YES;
}
- (id)contentsAtPath:(id)p { return [NSData dataWithContentsOfFile:p]; }
- (BOOL)contentsEqualAtPath:(id)a andPath:(id)b {
    id x = [NSData dataWithContentsOfFile:a], y = [NSData dataWithContentsOfFile:b];
    return x && y && [x isEqual:y];
}

- (id)contentsOfDirectoryAtPath:(id)p error:(id *)error { return list_dir(p, error); }
- (id)contentsOfDirectoryAtURL:(id)u includingPropertiesForKeys:(id)keys options:(NSUInteger)opts error:(id *)error {
    (void)keys;
    id names = list_dir([u path], error);
    if (!names) return nil;
    id out = [NSMutableArray array];
    for (id n in names) {
        if ((opts & 4) && [n hasPrefix:CFSTR(".")]) continue; /* NSDirectoryEnumerationSkipsHiddenFiles */
        [out addObject:[u URLByAppendingPathComponent:n]];
    }
    return out;
}
- (id)enumeratorAtPath:(id)p {
    id top = list_dir(p, 0);
    if (!top) return nil;
    NSDirectoryEnumerator *e = [[NSDirectoryEnumerator new] autorelease];
    e->_root = [p copy];
    e->_stack = [[NSMutableArray alloc] initWithObjects:[[top mutableCopy] autorelease], nil];
    return e;
}
- (id)subpathsOfDirectoryAtPath:(id)p error:(id *)error {
    id e = [self enumeratorAtPath:p];
    if (!e) { list_dir(p, error); return nil; }
    return [e allObjects];
}
- (id)subpathsAtPath:(id)p { return [self subpathsOfDirectoryAtPath:p error:0]; }

- (id)attributesOfItemAtPath:(id)p error:(id *)error { return attributes(p, NO, error); }
- (id)fileAttributesAtPath:(id)p traverseLink:(BOOL)t { return attributes(p, t, 0); }
- (BOOL)setAttributes:(id)attrs ofItemAtPath:(id)p error:(id *)error {
    id perm = [attrs objectForKey:NSFilePosixPermissions];
    if (perm && chmod(fs(p), [perm unsignedShortValue]) != 0) return fail(error, p);
    return YES;
}
- (id)attributesOfFileSystemForPath:(id)p error:(id *)error {
    struct mstat st;
    if (stat_(fs(p), &st) != 0) { fail(error, p); return nil; }
    return [NSDictionary dictionaryWithObject:[NSNumber numberWithInt:st.dev] forKey:NSFileSystemNumber];
}

- (BOOL)removeItemAtPath:(id)p error:(id *)error {
    struct mstat st;
    if (lstat_(fs(p), &st) != 0) return fail(error, p);
    if ((st.mode & S_IFMT) == S_IFDIR) {
        for (id k in list_dir(p, 0))
            if (![self removeItemAtPath:[p stringByAppendingPathComponent:k] error:error]) return NO;
        if (rmdir(fs(p)) != 0) return fail(error, p);
    } else if (unlink(fs(p)) != 0) {
        return fail(error, p);
    }
    return YES;
}
- (BOOL)removeItemAtURL:(id)u error:(id *)e { return [self removeItemAtPath:[u path] error:e]; }
- (BOOL)moveItemAtPath:(id)from toPath:(id)to error:(id *)error {
    if ([self fileExistsAtPath:to]) { errno = 17; return fail(error, to); }
    if (rename(fs(from), fs(to)) == 0) return YES;
    if (errno != 18 /* EXDEV */) return fail(error, from);
    return [self copyItemAtPath:from toPath:to error:error] && [self removeItemAtPath:from error:error];
}
- (BOOL)moveItemAtURL:(id)a toURL:(id)b error:(id *)e { return [self moveItemAtPath:[a path] toPath:[b path] error:e]; }
- (BOOL)copyItemAtPath:(id)from toPath:(id)to error:(id *)error {
    struct mstat st;
    if (lstat_(fs(from), &st) != 0) return fail(error, from);
    if ([self fileExistsAtPath:to]) { errno = 17; return fail(error, to); }
    switch (st.mode & S_IFMT) {
    case S_IFDIR:
        if (mkdir(fs(to), st.mode & 07777) != 0) return fail(error, to);
        for (id k in list_dir(from, 0))
            if (![self copyItemAtPath:[from stringByAppendingPathComponent:k] toPath:[to stringByAppendingPathComponent:k] error:error]) return NO;
        return YES;
    case S_IFLNK: {
        id dest = [self destinationOfSymbolicLinkAtPath:from error:error];
        return dest && [self createSymbolicLinkAtPath:to withDestinationPath:dest error:error];
    }
    default:
        return copy_file(from, to, error);
    }
}
- (BOOL)copyItemAtURL:(id)a toURL:(id)b error:(id *)e { return [self copyItemAtPath:[a path] toPath:[b path] error:e]; }
- (BOOL)linkItemAtPath:(id)from toPath:(id)to error:(id *)error {
    extern int link(const char *, const char *);
    return link(fs(from), fs(to)) == 0 ? YES : fail(error, to);
}
- (BOOL)createSymbolicLinkAtPath:(id)p withDestinationPath:(id)dest error:(id *)error { return symlink(fs(dest), fs(p)) == 0 ? YES : fail(error, p); }
- (id)destinationOfSymbolicLinkAtPath:(id)p error:(id *)error {
    char b[1024];
    long n = readlink(fs(p), b, sizeof b - 1);
    if (n < 0) { fail(error, p); return nil; }
    b[n] = 0;
    return _prism_cf_string(b);
}

- (id)URLsForDirectory:(NSUInteger)dir inDomains:(NSUInteger)domains {
    id out = [NSMutableArray array];
    for (id p in NSSearchPathForDirectoriesInDomains(dir, domains, YES)) [out addObject:[NSURL fileURLWithPath:p isDirectory:YES]];
    return out;
}
- (id)URLForDirectory:(NSUInteger)dir inDomain:(NSUInteger)domain appropriateForURL:(id)u create:(BOOL)create error:(id *)error {
    (void)u;
    id p = [NSSearchPathForDirectoriesInDomains(dir, domain, YES) firstObject];
    if (!p) { errno = 2; fail(error, nil); return nil; }
    if (create && ![self createDirectoryAtPath:p withIntermediateDirectories:YES attributes:nil error:error]) return nil;
    return [NSURL fileURLWithPath:p isDirectory:YES];
}
- (id)temporaryDirectory { return [NSURL fileURLWithPath:NSTemporaryDirectory() isDirectory:YES]; }
- (id)homeDirectoryForCurrentUser { return [NSURL fileURLWithPath:NSHomeDirectory() isDirectory:YES]; }
- (id)displayNameAtPath:(id)p { return [p lastPathComponent]; }
- (const char *)fileSystemRepresentationWithPath:(id)p { return fs(p); }
- (id)stringWithFileSystemRepresentation:(const char *)s length:(NSUInteger)n { return [[[NSString alloc] initWithBytes:s length:n encoding:4] autorelease]; }
@end
