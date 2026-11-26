/* The URL loading system for plain HTTP: NSURLRequest / NSMutableURLRequest, NSURLResponse /
 * NSHTTPURLResponse, NSURLSessionConfiguration, NSURLSession and its data, upload and download
 * tasks (completion handlers or a delegate), plus -[NSData dataWithContentsOfURL:] and friends for
 * http URLs. HTTP/1.1 over Prism's BSD sockets and getaddrinfo: Content-Length, chunked and
 * read-to-close bodies, redirects (301/302/303/307/308, at most 16), one connection per request.
 * https fails with NSURLErrorSecureConnectionFailed: Prism has no TLS yet. */
#include "foundation.h"

void *malloc(size_t);
void *realloc(void *, size_t);
void free(void *);
int snprintf(char *, size_t, const char *, ...);
long long strtoll(const char *, char **, int);
int strncasecmp(const char *, const char *, size_t);
int getpid(void);
int unlink(const char *);
int strcasecmp(const char *, const char *);
size_t strcspn(const char *, const char *);
char *strstr(const char *, const char *);
void *memchr(const void *, int, size_t);

struct sockaddr { unsigned char len, family; char data[14]; };
struct addrinfo { int flags, family, socktype, protocol; unsigned addrlen; char *canonname; struct sockaddr *addr; struct addrinfo *next; };
int getaddrinfo(const char *, const char *, const struct addrinfo *, struct addrinfo **);
void freeaddrinfo(struct addrinfo *);
int socket(int, int, int);
int connect(int, const struct sockaddr *, unsigned);
long send(int, const void *, size_t, int);
long recv(int, void *, size_t, int);
int shutdown(int, int);
int setsockopt(int, int, int, const void *, unsigned);
#define AF_UNSPEC 0
#define SOCK_STREAM 1
#define SOL_SOCKET 0xffff
#define SO_RCVTIMEO 0x1006
#define SO_NOSIGPIPE 0x1022

void dispatch_async_f(void *, void *, void (*)(void *));
void *dispatch_get_global_queue(long, unsigned long);
void *dispatch_queue_create(const char *, const void *);
void *_Block_copy(const void *);
void _Block_release(const void *);
extern id const NSLocalizedDescriptionKey;
id const NSURLErrorDomain = CFSTR("NSURLErrorDomain");
id const NSURLErrorFailingURLErrorKey = CFSTR("NSErrorFailingURLKey");
id const NSURLErrorFailingURLStringErrorKey = CFSTR("NSErrorFailingURLStringKey");
id const NSURLSessionTransferSizeUnknown = (id)-1;

@interface NSObject (PrismURLUses)
+ (id)dictionary;
+ (id)stringWithUTF8String:(const char *)s;
+ (id)dictionaryWithObjectsAndKeys:(id)first, ...;
- (id)objectForKey:(id)k;
- (id)allKeys;
- (NSUInteger)count;
- (id)objectAtIndex:(NSUInteger)i;
- (id)lowercaseString;
- (NSUInteger)length;
- (const void *)bytes;
- (BOOL)writeToFile:(id)p atomically:(BOOL)a;
- (void)addOperationWithBlock:(void (^)(void))b;
- (void)setMaxConcurrentOperationCount:(NSInteger)n;
@end

/* ------------------------------------------------------------------ requests and responses */

@interface NSURLRequest : NSObject {
@public
    id _url, _method, _headers, _body, _mainDoc;
    double _timeout;
    NSUInteger _cache;
    BOOL _handleCookies, _pipelining, _cellular;
}
- (id)_copyAs:(Class)c;
- (id)initWithURL:(id)u cachePolicy:(NSUInteger)c timeoutInterval:(double)t;
@end
@interface NSMutableURLRequest : NSURLRequest
- (void)setValue:(id)v forHTTPHeaderField:(id)f;
@end

@implementation NSURLRequest
+ (id)requestWithURL:(id)u { return [[[self alloc] initWithURL:u] autorelease]; }
+ (id)requestWithURL:(id)u cachePolicy:(NSUInteger)c timeoutInterval:(double)t { return [[[self alloc] initWithURL:u cachePolicy:c timeoutInterval:t] autorelease]; }
+ (BOOL)supportsSecureCoding { return YES; }
- (id)initWithURL:(id)u { return [self initWithURL:u cachePolicy:0 timeoutInterval:60]; }
- (id)initWithURL:(id)u cachePolicy:(NSUInteger)c timeoutInterval:(double)t {
    _url = [u retain];
    _method = [CFSTR("GET") retain];
    _headers = [NSMutableDictionary new];
    _cache = c;
    _timeout = t;
    _handleCookies = YES;
    return self;
}
- (id)URL { return _url; }
- (id)HTTPMethod { return _method; }
- (id)allHTTPHeaderFields { return [[_headers copy] autorelease]; }
- (id)HTTPBody { return _body; }
- (id)HTTPBodyStream { return nil; }
- (double)timeoutInterval { return _timeout; }
- (NSUInteger)cachePolicy { return _cache; }
- (id)mainDocumentURL { return _mainDoc; }
- (BOOL)HTTPShouldHandleCookies { return _handleCookies; }
- (BOOL)HTTPShouldUsePipelining { return _pipelining; }
- (BOOL)allowsCellularAccess { return YES; }
- (NSUInteger)networkServiceType { return 0; }
/* Header names are case-insensitive. */
static id header_key(id headers, id name) {
    id keys = [headers allKeys];
    for (NSUInteger i = 0, n = [keys count]; i < n; i++) {
        id k = [keys objectAtIndex:i];
        if ([k caseInsensitiveCompare:name] == 0) return k;
    }
    return nil;
}
- (id)valueForHTTPHeaderField:(id)f { id k = header_key(_headers, f); return k ? [_headers objectForKey:k] : nil; }
- (id)copyWithZone:(NSZone *)z { (void)z; return [self _copyAs:[NSURLRequest class]]; }
- (id)mutableCopyWithZone:(NSZone *)z { (void)z; return [self _copyAs:[NSMutableURLRequest class]]; }
- (id)_copyAs:(Class)c {
    NSURLRequest *r = [[c alloc] initWithURL:_url cachePolicy:_cache timeoutInterval:_timeout];
    [r->_method release]; r->_method = [_method copy];
    [r->_headers release]; r->_headers = [_headers mutableCopy];
    r->_body = [_body copy];
    r->_mainDoc = [_mainDoc retain];
    r->_handleCookies = _handleCookies;
    return r;
}
- (BOOL)isEqual:(id)o { return o == self || (cf_is(o, [NSURLRequest class]) && [[_url absoluteString] isEqual:[((NSURLRequest *)o)->_url absoluteString]] && [_method isEqual:((NSURLRequest *)o)->_method]); }
- (NSUInteger)hash { return [[_url absoluteString] hash]; }
- (id)description { return [NSString stringWithFormat:CFSTR("<%s: %p> { URL: %@ }"), object_getClassName(self), self, [_url absoluteString]]; }
- (void)dealloc { [_url release]; [_method release]; [_headers release]; [_body release]; [_mainDoc release]; [super dealloc]; }
@end

@implementation NSMutableURLRequest
- (void)setURL:(id)u { id o = _url; _url = [u retain]; [o release]; }
- (void)setHTTPMethod:(id)m { id o = _method; _method = [m copy]; [o release]; }
- (void)setHTTPBody:(id)b { id o = _body; _body = [b copy]; [o release]; }
- (void)setHTTPBodyStream:(id)s { (void)s; }
- (void)setTimeoutInterval:(double)t { _timeout = t; }
- (void)setCachePolicy:(NSUInteger)c { _cache = c; }
- (void)setMainDocumentURL:(id)u { id o = _mainDoc; _mainDoc = [u retain]; [o release]; }
- (void)setHTTPShouldHandleCookies:(BOOL)b { _handleCookies = b; }
- (void)setHTTPShouldUsePipelining:(BOOL)b { _pipelining = b; }
- (void)setAllowsCellularAccess:(BOOL)b { (void)b; }
- (void)setNetworkServiceType:(NSUInteger)t { (void)t; }
- (void)setAllHTTPHeaderFields:(id)h {
    id keys = [h allKeys];
    for (NSUInteger i = 0, n = [keys count]; i < n; i++) [self setValue:[h objectForKey:[keys objectAtIndex:i]] forHTTPHeaderField:[keys objectAtIndex:i]];
}
- (void)setValue:(id)v forHTTPHeaderField:(id)f {
    id k = header_key(_headers, f);
    if (k) [_headers removeObjectForKey:k];
    if (v) [_headers setObject:v forKey:f];
}
- (void)addValue:(id)v forHTTPHeaderField:(id)f {
    id old = [self valueForHTTPHeaderField:f];
    [self setValue:old ? [NSString stringWithFormat:CFSTR("%@,%@"), old, v] : v forHTTPHeaderField:f];
}
@end

@interface NSURLResponse : NSObject { @public id _url, _mime, _encoding; long long _length; }
@end
@implementation NSURLResponse
- (id)initWithURL:(id)u MIMEType:(id)m expectedContentLength:(NSInteger)l textEncodingName:(id)e {
    _url = [u retain]; _mime = [m copy]; _length = l; _encoding = [e copy];
    return self;
}
- (id)URL { return _url; }
- (id)MIMEType { return _mime; }
- (long long)expectedContentLength { return _length; }
- (id)textEncodingName { return _encoding; }
- (id)suggestedFilename { id l = [_url lastPathComponent]; return l && [l length] && ![l isEqual:CFSTR("/")] ? l : CFSTR("Unknown"); }
- (id)copyWithZone:(NSZone *)z { (void)z; return [self retain]; }
- (void)dealloc { [_url release]; [_mime release]; [_encoding release]; [super dealloc]; }
@end

@interface NSHTTPURLResponse : NSURLResponse { @public NSInteger _status; id _headers, _version; }
@end
@implementation NSHTTPURLResponse
- (id)initWithURL:(id)u statusCode:(NSInteger)s HTTPVersion:(id)v headerFields:(id)h {
    _url = [u retain];
    _status = s;
    _version = [v copy];
    _headers = [h copy];
    _length = -1;
    id ct = [self valueForHTTPHeaderField:CFSTR("Content-Type")];
    if (ct) {
        const char *c = [ct UTF8String], *semi = strchr(c, ';');
        size_t ml = semi ? (size_t)(semi - c) : strlen(c);
        while (ml && c[ml - 1] == ' ') ml--;
        _mime = [[[NSString alloc] initWithBytes:c length:ml encoding:4] lowercaseString];
        [_mime retain];
        const char *cs = semi ? strstr(semi, "charset=") : 0;
        if (cs) {
            cs += 8;
            if (*cs == '"') cs++;
            size_t el = strcspn(cs, "\"; ");
            _encoding = [[[NSString alloc] initWithBytes:cs length:el encoding:4] lowercaseString];
            [_encoding retain];
        }
    }
    id cl = [self valueForHTTPHeaderField:CFSTR("Content-Length")];
    if (cl) _length = [cl longLongValue];
    return self;
}
- (NSInteger)statusCode { return _status; }
- (id)allHeaderFields { return _headers; }
- (id)valueForHTTPHeaderField:(id)f { id k = header_key(_headers, f); return k ? [_headers objectForKey:k] : nil; }
+ (id)localizedStringForStatusCode:(NSInteger)s {
    switch (s) {
    case 200: return CFSTR("no error");
    case 201: return CFSTR("created");
    case 204: return CFSTR("no content");
    case 301: return CFSTR("moved permanently");
    case 302: return CFSTR("found");
    case 304: return CFSTR("not modified");
    case 400: return CFSTR("bad request");
    case 401: return CFSTR("unauthorized");
    case 403: return CFSTR("forbidden");
    case 404: return CFSTR("not found");
    case 500: return CFSTR("internal server error");
    case 503: return CFSTR("service unavailable");
    default: return s < 200 ? CFSTR("informational") : s < 300 ? CFSTR("success") : s < 400 ? CFSTR("redirected") : s < 500 ? CFSTR("client error") : CFSTR("server error");
    }
}
- (void)dealloc { [_headers release]; [_version release]; [super dealloc]; }
@end

/* ------------------------------------------------------------------ HTTP/1.1 client */

struct bytes { char *p; size_t n, cap; };
static void add(struct bytes *b, const void *s, size_t n) {
    if (b->n + n + 1 > b->cap) { b->cap = (b->n + n + 1) * 2; b->p = realloc(b->p, b->cap); }
    memcpy(b->p + b->n, s, n);
    b->n += n;
    b->p[b->n] = 0;
}
static void adds(struct bytes *b, const char *s) { add(b, s, strlen(s)); }

static id url_error(NSInteger code, id url, const char *why) {
    id info = [NSMutableDictionary dictionary];
    if (why) [info setObject:[NSString stringWithUTF8String:why] forKey:NSLocalizedDescriptionKey];
    if (url) { [info setObject:url forKey:NSURLErrorFailingURLErrorKey]; [info setObject:[url absoluteString] forKey:NSURLErrorFailingURLStringErrorKey]; }
    return [NSError errorWithDomain:NSURLErrorDomain code:code userInfo:info];
}

/* Split an http URL into host, port and request target (path + query, as written). */
static int split_url(id url, char *host, size_t hn, char *port, size_t pn, struct bytes *target, int *tls) {
    const char *s = [[url absoluteString] UTF8String];
    if (!s) return 0;
    const char *sep = strstr(s, "://");
    if (!sep) return 0;
    size_t sl = (size_t)(sep - s);
    *tls = sl == 5 && !strncasecmp(s, "https", 5);
    if (!*tls && !(sl == 4 && !strncasecmp(s, "http", 4))) return 0;
    const char *a = sep + 3, *end = a + strcspn(a, "/?#");
    const char *at = memchr(a, '@', (size_t)(end - a));
    if (at) a = at + 1; /* user info: not sent (no auth support yet) */
    const char *colon = 0;
    if (*a == '[') { const char *rb = memchr(a, ']', (size_t)(end - a)); if (!rb) return 0; snprintf(host, hn, "%.*s", (int)(rb - a - 1), a + 1); colon = rb + 1 < end && rb[1] == ':' ? rb + 1 : 0; }
    else {
        colon = memchr(a, ':', (size_t)(end - a));
        snprintf(host, hn, "%.*s", (int)((colon ? colon : end) - a), a);
    }
    if (colon && colon + 1 < end) snprintf(port, pn, "%.*s", (int)(end - colon - 1), colon + 1);
    else snprintf(port, pn, "%s", *tls ? "443" : "80");
    if (!*host) return 0;
    const char *frag = strchr(end, '#');
    size_t tl = frag ? (size_t)(frag - end) : strlen(end);
    if (!tl || *end != '/') adds(target, "/");
    add(target, end, tl);
    return 1;
}

struct fetch { id url; id method; id headers; id body; double timeout; BOOL head_only; volatile int *cancel; };
struct result { NSInteger status; id headers; struct bytes body; id final_url; id error; id version; };

static long send_all(int fd, const char *p, size_t n) {
    while (n) {
        long w = send(fd, p, n, 0);
        if (w <= 0) return -1;
        p += w; n -= (size_t)w;
    }
    return 0;
}

/* One request/response exchange; fills r (status, headers, body) or r->error. */
static void exchange(struct fetch *f, id url, struct result *r) {
    char host[256], port[16];
    struct bytes target = {0};
    int tls = 0;
    if (!split_url(url, host, sizeof host, port, sizeof port, &target, &tls)) { free(target.p); r->error = url_error(-1002, url, "unsupported URL"); return; }
    if (tls) { free(target.p); r->error = url_error(-1200, url, "An SSL error has occurred and a secure connection to the server cannot be made (Prism has no TLS yet)."); return; }
    struct addrinfo hints = {0}, *ai = 0;
    hints.family = AF_UNSPEC;
    hints.socktype = SOCK_STREAM;
    if (getaddrinfo(host, port, &hints, &ai) || !ai) { free(target.p); r->error = url_error(-1003, url, "A server with the specified hostname could not be found."); return; }
    int fd = -1;
    for (struct addrinfo *p = ai; p; p = p->next) {
        fd = socket(p->family, p->socktype, p->protocol);
        if (fd < 0) continue;
        if (connect(fd, p->addr, p->addrlen) == 0) break;
        close(fd);
        fd = -1;
    }
    freeaddrinfo(ai);
    if (fd < 0) { free(target.p); r->error = url_error(-1004, url, "Could not connect to the server."); return; }
    int one = 1;
    setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &one, sizeof one);
    if (f->timeout > 0) {
        struct timeval tv = {(time_t)f->timeout, (int)((f->timeout - (double)(time_t)f->timeout) * 1e6)};
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv);
    }

    struct bytes req = {0};
    adds(&req, [f->method UTF8String]);
    adds(&req, " ");
    add(&req, target.p, target.n);
    adds(&req, " HTTP/1.1\r\n");
    free(target.p);
    char line[512];
    BOOL defport = !strcmp(port, "80");
    snprintf(line, sizeof line, strchr(host, ':') ? "Host: [%s]%s%s\r\n" : "Host: %s%s%s\r\n", host, defport ? "" : ":", defport ? "" : port);
    adds(&req, line);
    BOOL have_ua = NO, have_accept = NO, have_ct = NO;
    id keys = [f->headers allKeys];
    for (NSUInteger i = 0, n = [keys count]; i < n; i++) {
        id k = [keys objectAtIndex:i];
        const char *ks = [k UTF8String];
        if (!strcasecmp(ks, "host") || !strcasecmp(ks, "content-length") || !strcasecmp(ks, "connection")) continue;
        have_ua |= !strcasecmp(ks, "user-agent");
        have_accept |= !strcasecmp(ks, "accept");
        have_ct |= !strcasecmp(ks, "content-type");
        adds(&req, ks); adds(&req, ": "); adds(&req, [[[f->headers objectForKey:k] description] UTF8String]); adds(&req, "\r\n");
    }
    if (!have_ua) adds(&req, "User-Agent: Prism CFNetwork/1.0 Darwin/20.0.0\r\n");
    if (!have_accept) adds(&req, "Accept: */*\r\n");
    NSUInteger blen = f->body ? [f->body length] : 0;
    if (f->body && !have_ct && blen) adds(&req, "Content-Type: application/x-www-form-urlencoded\r\n");
    if (f->body || [f->method isEqual:CFSTR("POST")] || [f->method isEqual:CFSTR("PUT")]) { snprintf(line, sizeof line, "Content-Length: %lu\r\n", (unsigned long)blen); adds(&req, line); }
    adds(&req, "Accept-Encoding: identity\r\nConnection: close\r\n\r\n");
    if (blen) add(&req, [f->body bytes], blen);
    long sent = send_all(fd, req.p, req.n);
    free(req.p);
    if (sent < 0) { close(fd); r->error = url_error(-1005, url, "The network connection was lost."); return; }

    /* Read everything (Connection: close), then parse. */
    struct bytes in = {0};
    char buf[16384];
    for (;;) {
        if (f->cancel && *f->cancel) { close(fd); free(in.p); r->error = url_error(-999, url, "cancelled"); return; }
        long got = recv(fd, buf, sizeof buf, 0);
        if (got < 0) { close(fd); free(in.p); r->error = url_error(-1001, url, "The request timed out."); return; }
        if (got == 0) break;
        add(&in, buf, (size_t)got);
        /* Stop early once a Content-Length body is complete (servers may keep the socket open). */
        char *he = in.p ? strstr(in.p, "\r\n\r\n") : 0;
        if (he) {
            const char *cl = 0;
            for (char *h = in.p; h < he; h = strstr(h, "\r\n") + 2) if (!strncasecmp(h, "content-length:", 15)) { cl = h + 15; break; }
            if (cl && in.n - (size_t)(he + 4 - in.p) >= (size_t)strtoll(cl, 0, 10)) break;
            if (f->head_only) break;
        }
    }
    close(fd);
    if (!in.p) { r->error = url_error(-1005, url, "The network connection was lost."); return; }
    char *he = strstr(in.p, "\r\n\r\n");
    if (!he || strncmp(in.p, "HTTP/", 5)) { free(in.p); r->error = url_error(-1011, url, "The server returned a malformed response."); return; }
    *he = 0;
    char *sp = strchr(in.p, ' ');
    r->status = sp ? (NSInteger)strtoll(sp + 1, 0, 10) : 0;
    r->version = [[NSString alloc] initWithBytes:in.p length:(NSUInteger)(sp ? sp - in.p : 8) encoding:4];
    r->headers = [NSMutableDictionary new];
    BOOL chunked = NO;
    long long clen = -1;
    for (char *h = strstr(in.p, "\r\n"); h; ) {
        h += 2;
        char *eol = strstr(h, "\r\n");
        size_t hl = eol ? (size_t)(eol - h) : strlen(h);
        char *colon = memchr(h, ':', hl);
        if (colon) {
            char *v = colon + 1;
            while (*v == ' ' || *v == '\t') v++;
            id k = [[[NSString alloc] initWithBytes:h length:(NSUInteger)(colon - h) encoding:4] autorelease];
            id val = [[[NSString alloc] initWithBytes:v length:(NSUInteger)(h + hl - v) encoding:4] autorelease];
            id prev = [r->headers objectForKey:k];
            [r->headers setObject:prev ? [NSString stringWithFormat:CFSTR("%@, %@"), prev, val] : val forKey:k];
            if (!strncasecmp(h, "transfer-encoding", 17) && strstr(v, "chunked")) chunked = YES;
            if (!strncasecmp(h, "content-length", 14)) clen = strtoll(v, 0, 10);
        }
        h = eol;
    }
    const char *b = he + 4, *e = in.p + in.n;
    if (f->head_only || r->status == 204 || r->status == 304 || (r->status >= 100 && r->status < 200)) {}
    else if (chunked) {
        while (b < e) {
            long long sz = strtoll(b, 0, 16);
            const char *crlf = strstr(b, "\r\n");
            if (!crlf || sz <= 0) break;
            b = crlf + 2;
            if (b + sz > e) sz = e - b;
            add(&r->body, b, (size_t)sz);
            b += sz + 2;
        }
    } else {
        size_t n = (size_t)(e - b);
        if (clen >= 0 && (size_t)clen < n) n = (size_t)clen;
        add(&r->body, b, n);
    }
    free(in.p);
}

/* Follow redirects. On return, r has the final response or an error. */
static void fetch(struct fetch *f, struct result *r) {
    id url = [f->url retain];
    for (int hops = 0;; hops++) {
        memset(r, 0, sizeof *r);
        @autoreleasepool { exchange(f, url, r); [r->error retain]; }
        if (r->error || r->status < 300 || r->status >= 400 || r->status == 304) break;
        id loc = [r->headers objectForKey:header_key(r->headers, CFSTR("Location"))];
        if (!loc) break;
        if (hops == 16) { r->error = [url_error(-1007, url, "too many HTTP redirects") retain]; break; }
        id next = [[NSURL URLWithString:loc relativeToURL:url] absoluteURL];
        if (!next) break;
        if (r->status == 303 || ((r->status == 301 || r->status == 302) && [f->method isEqual:CFSTR("POST")])) { f->method = CFSTR("GET"); f->body = nil; }
        [r->headers release]; [r->version release]; free(r->body.p);
        [url release];
        url = [next retain];
    }
    r->final_url = url; /* retained */
}

/* ------------------------------------------------------------------ sessions and tasks */

@interface NSURLSessionConfiguration : NSObject { @public id _headers, _id; double _timeoutReq, _timeoutRes; NSInteger _maxConn; BOOL _cookies; }
@end
@implementation NSURLSessionConfiguration
+ (id)defaultSessionConfiguration { NSURLSessionConfiguration *c = [[self new] autorelease]; c->_timeoutReq = 60; c->_timeoutRes = 604800; c->_maxConn = 6; c->_cookies = YES; return c; }
+ (id)ephemeralSessionConfiguration { return [self defaultSessionConfiguration]; }
+ (id)backgroundSessionConfigurationWithIdentifier:(id)i { NSURLSessionConfiguration *c = [self defaultSessionConfiguration]; c->_id = [i copy]; return c; }
- (id)identifier { return _id; }
- (id)HTTPAdditionalHeaders { return _headers; }
- (void)setHTTPAdditionalHeaders:(id)h { id o = _headers; _headers = [h copy]; [o release]; }
- (double)timeoutIntervalForRequest { return _timeoutReq; }
- (void)setTimeoutIntervalForRequest:(double)t { _timeoutReq = t; }
- (double)timeoutIntervalForResource { return _timeoutRes; }
- (void)setTimeoutIntervalForResource:(double)t { _timeoutRes = t; }
- (NSInteger)HTTPMaximumConnectionsPerHost { return _maxConn; }
- (void)setHTTPMaximumConnectionsPerHost:(NSInteger)n { _maxConn = n; }
- (BOOL)HTTPShouldSetCookies { return _cookies; }
- (void)setHTTPShouldSetCookies:(BOOL)b { _cookies = b; }
- (void)setRequestCachePolicy:(NSUInteger)p { (void)p; }
- (void)setURLCache:(id)c { (void)c; }
- (void)setHTTPCookieStorage:(id)s { (void)s; }
- (void)setWaitsForConnectivity:(BOOL)w { (void)w; }
- (void)setAllowsCellularAccess:(BOOL)a { (void)a; }
- (void)setTLSMinimumSupportedProtocolVersion:(NSUInteger)v { (void)v; }
- (id)copyWithZone:(NSZone *)z {
    (void)z;
    NSURLSessionConfiguration *c = [NSURLSessionConfiguration new];
    c->_headers = [_headers copy]; c->_id = [_id copy];
    c->_timeoutReq = _timeoutReq; c->_timeoutRes = _timeoutRes; c->_maxConn = _maxConn; c->_cookies = _cookies;
    return c;
}
- (void)dealloc { [_headers release]; [_id release]; [super dealloc]; }
@end

@class NSURLSession;
enum { K_DATA, K_UPLOAD, K_DOWNLOAD };
enum { S_RUNNING, S_SUSPENDED, S_CANCELING, S_COMPLETED };
@interface NSURLSessionTask : NSObject {
@public
    NSURLSession *_session;
    id _orig, _current, _response, _error, _desc;
    void *_handler; /* completion block */
    NSUInteger _ident;
    volatile NSInteger _state;
    volatile int _cancel;
    int _kind;
    BOOL _started;
    long long _received, _sent, _expected;
    double _priority;
}
@end
@interface NSURLSessionDataTask : NSURLSessionTask @end
@interface NSURLSessionUploadTask : NSURLSessionDataTask @end
@interface NSURLSessionDownloadTask : NSURLSessionTask @end

@interface NSURLSession : NSObject { @public id _config, _delegate, _queue, _desc; BOOL _invalid; NSUInteger _next; }
- (id)_initWithConfig:(id)c delegate:(id)d queue:(id)q;
@end

static void run_task(void *p);

@implementation NSURLSessionTask
- (NSUInteger)taskIdentifier { return _ident; }
- (id)originalRequest { return _orig; }
- (id)currentRequest { return _current; }
- (id)response { return _response; }
- (id)error { return _error; }
- (NSInteger)state { return _state; }
- (id)taskDescription { return _desc; }
- (void)setTaskDescription:(id)d { id o = _desc; _desc = [d copy]; [o release]; }
- (long long)countOfBytesReceived { return _received; }
- (long long)countOfBytesSent { return _sent; }
- (long long)countOfBytesExpectedToReceive { return _expected; }
- (long long)countOfBytesExpectedToSend { return [[_orig HTTPBody] length]; }
- (double)priority { return _priority; }
- (void)setPriority:(double)p { _priority = p; }
- (id)progress { return nil; }
- (void)resume {
    if (_started || _state != S_SUSPENDED) { if (_state == S_SUSPENDED) _state = S_RUNNING; return; }
    _started = YES;
    _state = S_RUNNING;
    [self retain];
    dispatch_async_f(dispatch_get_global_queue(0, 0), self, run_task);
}
- (void)suspend { if (_state == S_RUNNING && !_started) _state = S_SUSPENDED; }
- (void)cancel {
    if (_state == S_COMPLETED) return;
    _cancel = 1;
    if (!_started) { /* never resumed: complete right away with NSURLErrorCancelled */
        _started = YES;
        _state = S_CANCELING;
        [self retain];
        dispatch_async_f(dispatch_get_global_queue(0, 0), self, run_task);
    } else _state = S_CANCELING;
}
- (void)dealloc { [_orig release]; [_current release]; [_response release]; [_error release]; [_desc release]; if (_handler) _Block_release(_handler); [_session release]; [super dealloc]; }
@end
@implementation NSURLSessionDataTask @end
@implementation NSURLSessionUploadTask @end
@implementation NSURLSessionDownloadTask
- (void)cancelByProducingResumeData:(void (^)(id))h { [self cancel]; if (h) h(nil); }
@end

/* Deliver results on the session's delegate queue (an NSOperationQueue). */
struct delivery { NSURLSessionTask *task; id data; id location; };
static void deliver(struct delivery *d) {
    NSURLSessionTask *t = d->task;
    NSURLSession *s = t->_session;
    id data = d->data, loc = d->location;
    void (^work)(void) = ^{
        if (t->_handler) {
            if (t->_kind == K_DOWNLOAD) ((void (^)(id, id, id))t->_handler)(loc, t->_response, t->_error);
            else ((void (^)(id, id, id))t->_handler)(data, t->_response, t->_error);
        } else if (s->_delegate) {
            id del = s->_delegate;
            if (t->_response && [del respondsToSelector:@selector(URLSession:dataTask:didReceiveResponse:completionHandler:)])
                ((void (*)(id, SEL, id, id, id, void (^)(NSInteger)))objc_msgSend)(del, @selector(URLSession:dataTask:didReceiveResponse:completionHandler:), s, t, t->_response, ^(NSInteger disp) { (void)disp; });
            if (t->_kind != K_DOWNLOAD && [data length] && [del respondsToSelector:@selector(URLSession:dataTask:didReceiveData:)])
                ((void (*)(id, SEL, id, id, id))objc_msgSend)(del, @selector(URLSession:dataTask:didReceiveData:), s, t, data);
            if (t->_kind == K_DOWNLOAD && loc && [del respondsToSelector:@selector(URLSession:downloadTask:didFinishDownloadingToURL:)])
                ((void (*)(id, SEL, id, id, id))objc_msgSend)(del, @selector(URLSession:downloadTask:didFinishDownloadingToURL:), s, t, loc);
            if ([del respondsToSelector:@selector(URLSession:task:didCompleteWithError:)])
                ((void (*)(id, SEL, id, id, id))objc_msgSend)(del, @selector(URLSession:task:didCompleteWithError:), s, t, t->_error);
        }
        if (loc) unlink([[loc path] UTF8String]);
        [data release];
        [loc release];
        [t release];
    };
    [s->_queue addOperationWithBlock:work];
}

static void run_task(void *p) {
    NSURLSessionTask *t = p;
    @autoreleasepool {
        struct delivery d = {t, nil, nil};
        if (t->_cancel) {
            t->_error = [url_error(-999, [t->_orig URL], "cancelled") retain];
        } else {
            NSURLRequest *req = t->_orig;
            id headers = [NSMutableDictionary dictionary];
            id extra = [t->_session->_config HTTPAdditionalHeaders];
            if (extra) [headers addEntriesFromDictionary:extra];
            [headers addEntriesFromDictionary:[req allHTTPHeaderFields]];
            struct fetch f = {[req URL], [req HTTPMethod], headers, [req HTTPBody], [req timeoutInterval], [[req HTTPMethod] isEqual:CFSTR("HEAD")], &t->_cancel};
            if (t->_kind == K_UPLOAD && t->_current) f.body = t->_current; /* upload data */
            t->_sent = f.body ? (long long)[f.body length] : 0;
            struct result r;
            fetch(&f, &r);
            if (r.error) t->_error = r.error; /* retained */
            else {
                t->_response = [[NSHTTPURLResponse alloc] initWithURL:r.final_url statusCode:r.status HTTPVersion:r.version headerFields:r.headers];
                t->_expected = ((NSURLResponse *)t->_response)->_length;
                t->_received = (long long)r.body.n;
                d.data = [[NSData alloc] initWithBytes:r.body.p length:r.body.n];
                if (t->_kind == K_DOWNLOAD) {
                    static volatile long seq;
                    char path[96];
                    snprintf(path, sizeof path, "/tmp/CFNetworkDownload_%d_%ld.tmp", getpid(), __atomic_add_fetch(&seq, 1, __ATOMIC_RELAXED));
                    id ps = [NSString stringWithUTF8String:path];
                    if ([d.data writeToFile:ps atomically:NO]) d.location = [[NSURL fileURLWithPath:ps] retain];
                    else t->_error = [url_error(-3000, [req URL], "cannot create the download file") retain];
                    [d.data release];
                    d.data = nil;
                }
            }
            if (t->_kind == K_UPLOAD) { [t->_current release]; t->_current = [req retain]; }
            [r.headers release]; [r.version release]; [r.final_url release];
            free(r.body.p);
        }
        t->_state = S_COMPLETED;
        deliver(&d);
    }
}

static NSURLSession *shared_session;
@implementation NSURLSession
+ (id)sharedSession {
    if (!shared_session) shared_session = [[NSURLSession alloc] _initWithConfig:[NSURLSessionConfiguration defaultSessionConfiguration] delegate:nil queue:nil];
    return shared_session;
}
+ (id)sessionWithConfiguration:(id)c { return [[[self alloc] _initWithConfig:c delegate:nil queue:nil] autorelease]; }
+ (id)sessionWithConfiguration:(id)c delegate:(id)d delegateQueue:(id)q { return [[[self alloc] _initWithConfig:c delegate:d queue:q] autorelease]; }
- (id)_initWithConfig:(id)c delegate:(id)d queue:(id)q {
    _config = [c copy];
    _delegate = [d retain]; /* a session keeps its delegate until invalidated, as on macOS */
    if (q) _queue = [q retain];
    else { _queue = [(id)objc_getClass("NSOperationQueue") new]; [_queue setMaxConcurrentOperationCount:1]; }
    return self;
}
- (id)configuration { return [[_config copy] autorelease]; }
- (id)delegate { return _delegate; }
- (id)delegateQueue { return _queue; }
- (id)sessionDescription { return _desc; }
- (void)setSessionDescription:(id)d { id o = _desc; _desc = [d copy]; [o release]; }
- (id)_task:(Class)cls request:(id)req kind:(int)kind handler:(void *)h {
    if (_invalid) _prism_raise("NSGenericException", "Task created in a session that has been invalidated");
    if (cf_is(req, [NSURL class])) req = [NSURLRequest requestWithURL:req];
    NSURLSessionTask *t = [cls new];
    t->_session = [self retain];
    t->_orig = [req copy];
    t->_current = kind == K_UPLOAD ? nil : [req copy];
    t->_kind = kind;
    t->_ident = __atomic_add_fetch(&_next, 1, __ATOMIC_RELAXED);
    t->_state = S_SUSPENDED;
    t->_expected = -1;
    if (h) t->_handler = _Block_copy(h);
    return [t autorelease];
}
- (id)dataTaskWithURL:(id)u { return [self _task:[NSURLSessionDataTask class] request:u kind:K_DATA handler:0]; }
- (id)dataTaskWithRequest:(id)r { return [self _task:[NSURLSessionDataTask class] request:r kind:K_DATA handler:0]; }
- (id)dataTaskWithURL:(id)u completionHandler:(void (^)(id, id, id))h { return [self _task:[NSURLSessionDataTask class] request:u kind:K_DATA handler:h]; }
- (id)dataTaskWithRequest:(id)r completionHandler:(void (^)(id, id, id))h { return [self _task:[NSURLSessionDataTask class] request:r kind:K_DATA handler:h]; }
- (id)uploadTaskWithRequest:(id)r fromData:(id)body completionHandler:(void (^)(id, id, id))h {
    NSURLSessionTask *t = [self _task:[NSURLSessionUploadTask class] request:r kind:K_UPLOAD handler:h];
    t->_current = [body copy]; /* carried to run_task, then replaced by the request */
    return t;
}
- (id)uploadTaskWithRequest:(id)r fromData:(id)body { return [self uploadTaskWithRequest:r fromData:body completionHandler:nil]; }
- (id)uploadTaskWithRequest:(id)r fromFile:(id)file completionHandler:(void (^)(id, id, id))h {
    return [self uploadTaskWithRequest:r fromData:[NSData dataWithContentsOfFile:[file path]] completionHandler:h];
}
- (id)downloadTaskWithURL:(id)u { return [self _task:[NSURLSessionDownloadTask class] request:u kind:K_DOWNLOAD handler:0]; }
- (id)downloadTaskWithRequest:(id)r { return [self _task:[NSURLSessionDownloadTask class] request:r kind:K_DOWNLOAD handler:0]; }
- (id)downloadTaskWithURL:(id)u completionHandler:(void (^)(id, id, id))h { return [self _task:[NSURLSessionDownloadTask class] request:u kind:K_DOWNLOAD handler:h]; }
- (id)downloadTaskWithRequest:(id)r completionHandler:(void (^)(id, id, id))h { return [self _task:[NSURLSessionDownloadTask class] request:r kind:K_DOWNLOAD handler:h]; }
- (void)finishTasksAndInvalidate { if (self != shared_session) _invalid = YES; }
- (void)invalidateAndCancel { if (self != shared_session) _invalid = YES; }
- (void)resetWithCompletionHandler:(void (^)(void))h { if (h) [_queue addOperationWithBlock:h]; }
- (void)flushWithCompletionHandler:(void (^)(void))h { if (h) [_queue addOperationWithBlock:h]; }
- (void)getAllTasksWithCompletionHandler:(void (^)(id))h { if (h) h([NSArray array]); }
- (void)dealloc { [_config release]; [_delegate release]; [_queue release]; [_desc release]; [super dealloc]; }
@end

/* Synchronous GET for -[NSData dataWithContentsOfURL:] and -[NSString stringWithContentsOfURL:]. */
id _prism_http_get(id url, id *error) {
    id headers = [NSMutableDictionary dictionary];
    struct fetch f = {url, CFSTR("GET"), headers, nil, 60, NO, 0};
    struct result r;
    fetch(&f, &r);
    id data = nil;
    if (r.error) { if (error) *error = [r.error autorelease]; else [r.error release]; }
    else if (r.status >= 400) { if (error) *error = [NSError errorWithDomain:CFSTR("NSCocoaErrorDomain") code:256 userInfo:nil]; }
    else data = [NSData dataWithBytes:r.body.p length:r.body.n];
    [r.headers release]; [r.version release]; [r.final_url release];
    free(r.body.p);
    return data;
}
