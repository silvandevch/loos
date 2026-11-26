/* NSURLSession and friends against an HTTP server running on a thread of this process
 * (127.0.0.1:8480). ARC, no SDK: the interfaces are declared here. */
int printf(const char *, ...);
int snprintf(char *, unsigned long, const char *, ...);
typedef unsigned long NSUInteger, size_t;
typedef long NSInteger;
#if __arm64__
typedef _Bool BOOL;
#else
typedef signed char BOOL;
#endif
#define nil ((void *)0)
#define YES __objc_yes
#define NO __objc_no
char *strstr(const char *, const char *);
size_t strlen(const char *);
int strncmp(const char *, const char *, size_t);
int strncasecmp(const char *, const char *, size_t);
long long strtoll(const char *, char **, int);
void *memcpy(void *, const void *, size_t);
int access(const char *, int);

/* ---- sockets + threads ---- */
struct sockaddr_in { unsigned char len, family; unsigned short port; unsigned int addr; char zero[8]; };
int socket(int, int, int);
int bind(int, const void *, unsigned);
int listen(int, int);
int accept(int, void *, unsigned *);
long send(int, const void *, size_t, int);
long recv(int, void *, size_t, int);
int close(int);
int setsockopt(int, int, int, const void *, unsigned);
typedef struct pthread *pthread_t;
int pthread_create(pthread_t *, const void *, void *(*)(void *), void *);
int pthread_detach(pthread_t);
typedef void *dispatch_semaphore_t;
dispatch_semaphore_t dispatch_semaphore_create(long);
long dispatch_semaphore_wait(dispatch_semaphore_t, unsigned long long);
long dispatch_semaphore_signal(dispatch_semaphore_t);

static int listener;
static void reply(int c, const char *head, const char *body) {
    char out[2048];
    int n = snprintf(out, sizeof out, "%s\r\n\r\n%s", head, body);
    send(c, out, (size_t)n, 0);
}
static void *server(void *arg) {
    (void)arg;
    for (;;) {
        int c = accept(listener, 0, 0);
        if (c < 0) continue;
        char req[4096];
        size_t n = 0;
        for (;;) {
            long got = recv(c, req + n, sizeof req - 1 - n, 0);
            if (got <= 0) break;
            n += (size_t)got;
            req[n] = 0;
            char *he = strstr(req, "\r\n\r\n");
            if (!he) continue;
            char *cl = strstr(req, "Content-Length: ");
            size_t want = cl && cl < he ? (size_t)strtoll(cl + 16, 0, 10) : 0;
            if (n - (size_t)(he + 4 - req) >= want) break;
        }
        req[n] = 0;
        char *he = strstr(req, "\r\n\r\n");
        const char *body = he ? he + 4 : "";
        char head[512];
        if (!strncmp(req, "GET /hello ", 11)) {
            const char *ua = strstr(req, "User-Agent: ") ? "ua" : "noua";
            snprintf(head, sizeof head, "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nX-UA: %s\r\nContent-Length: 11", ua);
            reply(c, head, "hello prism");
        } else if (!strncmp(req, "GET /chunk ", 11)) {
            reply(c, "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked", "3\r\nabc\r\n4\r\ndefg\r\n0\r\n\r\n");
        } else if (!strncmp(req, "GET /redir ", 11)) {
            reply(c, "HTTP/1.1 302 Found\r\nLocation: /hello\r\nContent-Length: 0", "");
        } else if (!strncmp(req, "POST /echo ", 11)) {
            const char *tok = strstr(req, "X-Token: secret") ? "ok" : "missing";
            snprintf(head, sizeof head, "HTTP/1.1 201 Created\r\nX-Method: POST\r\nX-Token: %s\r\nContent-Length: %lu", tok, (unsigned long)strlen(body));
            reply(c, head, body);
        } else if (!strncmp(req, "GET /json ", 10)) {
            reply(c, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15", "{\"a\":1,\"b\":[2]}");
        } else {
            reply(c, "HTTP/1.1 404 Not Found\r\nContent-Length: 4", "nope");
        }
        close(c);
    }
    return 0;
}

/* ---- Foundation ---- */
__attribute__((objc_root_class))
@interface NSObject
+ (instancetype)alloc;
+ (instancetype)new;
- (instancetype)init;
- (BOOL)isEqual:(id)o;
@end
@interface NSString : NSObject
+ (id)stringWithUTF8String:(const char *)s;
+ (id)stringWithContentsOfURL:(id)u encoding:(NSUInteger)e error:(id *)err;
- (const char *)UTF8String;
- (instancetype)initWithData:(id)d encoding:(NSUInteger)e;
- (id)dataUsingEncoding:(NSUInteger)e;
@end
@interface NSData : NSObject
+ (id)dataWithContentsOfURL:(id)u;
- (NSUInteger)length;
@end
@interface NSNumber : NSObject
- (int)intValue;
@end
@interface NSArray : NSObject
- (id)objectAtIndexedSubscript:(NSUInteger)i;
@end
@interface NSDictionary : NSObject
+ (id)dictionaryWithObjects:(const id *)o forKeys:(const id *)k count:(NSUInteger)n;
- (id)objectForKeyedSubscript:(id)k;
@end
@interface NSError : NSObject
- (NSInteger)code;
- (id)domain;
@end
@interface NSURL : NSObject
+ (id)URLWithString:(id)s;
- (id)path;
- (id)absoluteString;
@end
@interface NSURLRequest : NSObject
@end
@interface NSMutableURLRequest : NSURLRequest
+ (id)requestWithURL:(id)u;
- (void)setHTTPMethod:(id)m;
- (void)setHTTPBody:(id)b;
- (void)setValue:(id)v forHTTPHeaderField:(id)f;
@end
@interface NSURLResponse : NSObject
- (id)URL;
- (id)MIMEType;
- (id)textEncodingName;
- (long long)expectedContentLength;
@end
@interface NSHTTPURLResponse : NSURLResponse
- (NSInteger)statusCode;
- (id)valueForHTTPHeaderField:(id)f;
@end
@interface NSURLSessionTask : NSObject
- (void)resume;
- (void)cancel;
- (NSInteger)state;
@end
@interface NSURLSessionConfiguration : NSObject
+ (id)defaultSessionConfiguration;
- (void)setHTTPAdditionalHeaders:(id)h;
@end
@interface NSOperationQueue : NSObject
@end
@interface NSURLSession : NSObject
+ (id)sharedSession;
+ (id)sessionWithConfiguration:(id)c delegate:(id)d delegateQueue:(id)q;
- (NSURLSessionTask *)dataTaskWithURL:(id)u completionHandler:(void (^)(id, id, id))h;
- (NSURLSessionTask *)dataTaskWithRequest:(id)r completionHandler:(void (^)(id, id, id))h;
- (NSURLSessionTask *)dataTaskWithURL:(id)u;
- (NSURLSessionTask *)downloadTaskWithURL:(id)u completionHandler:(void (^)(id, id, id))h;
@end
@interface NSJSONSerialization : NSObject
+ (id)JSONObjectWithData:(id)d options:(NSUInteger)o error:(id *)e;
@end

@interface Collector : NSObject {
@public
    long bytes;
    int done;
    NSInteger code;
    dispatch_semaphore_t sem;
}
@end
@implementation Collector
- (void)URLSession:(id)s dataTask:(id)t didReceiveData:(NSData *)d { bytes += (long)[d length]; }
- (void)URLSession:(id)s task:(id)t didCompleteWithError:(NSError *)e { code = e ? [e code] : 0; done = 1; dispatch_semaphore_signal(sem); }
@end

static NSURL *U(const char *path) {
    char b[128];
    snprintf(b, sizeof b, "http://127.0.0.1:8480%s", path);
    return [NSURL URLWithString:[NSString stringWithUTF8String:b]];
}
static const char *str(id data) { return data ? [[[NSString alloc] initWithData:data encoding:4] UTF8String] : "(nil)"; }

int main(void) {
    listener = socket(2, 1, 0);
    int one = 1;
    setsockopt(listener, 0xffff, 4, &one, sizeof one); /* SO_REUSEADDR */
    struct sockaddr_in sa = {sizeof sa, 2, (unsigned short)((8480 >> 8) | ((8480 & 0xff) << 8)), 0x0100007f, {0}};
    if (bind(listener, &sa, sizeof sa) || listen(listener, 16)) { printf("http: cannot listen\n"); return 1; }
    pthread_t t;
    pthread_create(&t, 0, server, 0);
    pthread_detach(t);

    @autoreleasepool {
        NSURLSession *s = [NSURLSession sharedSession];
        dispatch_semaphore_t sem = dispatch_semaphore_create(0);
        __block NSInteger status = 0;
        __block id body = nil, mime = nil, enc = nil, ua = nil, err = nil;
        __block long long len = 0;
        [[s dataTaskWithURL:U("/hello") completionHandler:^(id d, NSHTTPURLResponse *r, id e) {
            status = [r statusCode]; body = d; mime = [r MIMEType]; enc = [r textEncodingName]; len = [r expectedContentLength];
            ua = [r valueForHTTPHeaderField:@"x-ua"]; err = e;
            dispatch_semaphore_signal(sem);
        }] resume];
        dispatch_semaphore_wait(sem, ~0ull);
        printf("http: status=%ld body=%s mime=%s enc=%s len=%lld ua=%s err=%d\n", (long)status, str(body), [mime UTF8String], [enc UTF8String], len,
               [ua UTF8String], err != nil);

        __block id chunk = nil, redir_path = nil;
        __block NSInteger redir_status = 0;
        [[s dataTaskWithURL:U("/chunk") completionHandler:^(id d, id r, id e) { chunk = d; dispatch_semaphore_signal(sem); }] resume];
        dispatch_semaphore_wait(sem, ~0ull);
        printf("dbg: chunk-now=%s\n", str(chunk));
        [[s dataTaskWithURL:U("/redir") completionHandler:^(id d, NSHTTPURLResponse *r, id e) {
            redir_status = [r statusCode]; redir_path = [[r URL] path]; dispatch_semaphore_signal(sem);
        }] resume];
        dispatch_semaphore_wait(sem, ~0ull);
        NSMutableURLRequest *post = [NSMutableURLRequest requestWithURL:U("/echo")];
        [post setHTTPMethod:@"POST"];
        [post setHTTPBody:[@"name=prism&x=1" dataUsingEncoding:4]];
        [post setValue:@"secret" forHTTPHeaderField:@"X-Token"];
        __block NSInteger post_status = 0;
        __block id echo = nil, method = nil, token = nil;
        [[s dataTaskWithRequest:post completionHandler:^(id d, NSHTTPURLResponse *r, id e) {
            post_status = [r statusCode]; echo = d; method = [r valueForHTTPHeaderField:@"x-method"]; token = [r valueForHTTPHeaderField:@"X-TOKEN"];
            dispatch_semaphore_signal(sem);
        }] resume];
        dispatch_semaphore_wait(sem, ~0ull);
        __block NSInteger missing = 0;
        [[s dataTaskWithURL:U("/missing") completionHandler:^(id d, NSHTTPURLResponse *r, id e) { missing = [r statusCode]; dispatch_semaphore_signal(sem); }] resume];
        dispatch_semaphore_wait(sem, ~0ull);
        printf("http2: chunk=%s redir=%ld%s post=%ld/%s/%s/%s 404=%ld\n", str(chunk), (long)redir_status, [redir_path UTF8String], (long)post_status,
               str(echo), [method UTF8String], [token UTF8String], (long)missing);

        __block NSInteger refused = 0, tls = 0, cancelled = 0;
        [[s dataTaskWithURL:[NSURL URLWithString:@"http://127.0.0.1:1/"] completionHandler:^(id d, id r, NSError *e) { refused = [e code]; dispatch_semaphore_signal(sem); }] resume];
        dispatch_semaphore_wait(sem, ~0ull);
        [[s dataTaskWithURL:[NSURL URLWithString:@"https://127.0.0.1:8480/"] completionHandler:^(id d, id r, NSError *e) { tls = [e code]; dispatch_semaphore_signal(sem); }] resume];
        dispatch_semaphore_wait(sem, ~0ull);
        NSURLSessionTask *ct = [s dataTaskWithURL:U("/hello") completionHandler:^(id d, id r, NSError *e) { cancelled = [e code]; dispatch_semaphore_signal(sem); }];
        [ct cancel];
        dispatch_semaphore_wait(sem, ~0ull);
        __block int existed = 0;
        __block id dl = nil;
        [[s downloadTaskWithURL:U("/hello") completionHandler:^(NSURL *loc, id r, id e) {
            existed = access([[loc path] UTF8String], 0) == 0;
            dl = [NSString stringWithContentsOfURL:loc encoding:4 error:nil];
            dispatch_semaphore_signal(sem);
        }] resume];
        dispatch_semaphore_wait(sem, ~0ull);
        Collector *col = [Collector new];
        col->sem = sem;
        NSURLSessionConfiguration *cfg = [NSURLSessionConfiguration defaultSessionConfiguration];
        [cfg setHTTPAdditionalHeaders:@{@"X-Extra" : @"1"}];
        NSURLSession *ds = [NSURLSession sessionWithConfiguration:cfg delegate:col delegateQueue:nil];
        [[ds dataTaskWithURL:U("/json")] resume];
        dispatch_semaphore_wait(sem, ~0ull);
        NSDictionary *j = [NSJSONSerialization JSONObjectWithData:[NSData dataWithContentsOfURL:U("/json")] options:0 error:nil];
        id text = [NSString stringWithContentsOfURL:U("/hello") encoding:4 error:nil];
        printf("http3: refused=%ld tls=%ld cancel=%ld download=%d/%s delegate=%ld/%d/%ld json=%d/%d text=%s\n", (long)refused, (long)tls, (long)cancelled,
               existed, [dl UTF8String], col->bytes, col->done, (long)col->code, [j[@"a"] intValue], [j[@"b"][0] intValue], [text UTF8String]);
    }
    return 0;
}
