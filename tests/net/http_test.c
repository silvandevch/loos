#include "http.h"
#include "test.h"
#include <string.h>
#include <stdlib.h>
#include <stdio.h>

static http_result req(const char *s, http_request *r) { static uint8_t body[4096]; return http_parse_request((const uint8_t *)s, strlen(s), body, sizeof body, r); }
static http_result resp(const char *s, bool head, http_response *r) { static uint8_t body[4096]; return http_parse_response((const uint8_t *)s, strlen(s), head, body, sizeof body, r); }

static void h_hello(const http_request *rq, const char *path, http_reply *rp, void *u)
{
    (void)u; static char buf[256];
    int n = snprintf(buf, sizeof buf, "hello %s %s", rq->method, path);
    rp->body = (const uint8_t *)buf; rp->body_len = (size_t)n;
    rp->h[rp->nh++] = (http_header){ "Content-Type", "text/plain", 12, 10 };
}
static void h_echo(const http_request *rq, const char *path, http_reply *rp, void *u) { (void)path; (void)u; rp->status = 201; rp->body = rq->body; rp->body_len = rq->body_len; }

static int serve_nul(const http_server *s, const uint8_t *in, size_t il, uint8_t *o, size_t cap, size_t *c, bool *cl)
{
    memset(o, 0, cap);                                           /* bytes past the response must not leak into the checks */
    int n = http_server_serve(s, in, il, o, cap, c, cl);
    if (n > 0 && (size_t)n < cap) memset(o + n, 0, cap - (size_t)n);      /* HEAD cuts a longer buffered response short */
    return n;
}

int main(void)
{
    http_request q; http_response r; size_t vl; const char *v;

    /* ---- requests ---- */
    CHECK(req("GET /index.html?x=1 HTTP/1.1\r\nHost: example.com\r\nAccept: */*\r\n\r\n", &q) == HTTP_OK);
    CHECK(!strcmp(q.method, "GET") && !strcmp(q.target, "/index.html?x=1") && q.minor == 1 && q.nh == 2 && q.keep_alive && !q.body_len);
    CHECK((v = http_header_get(q.h, q.nh, "HOST", &vl)) && vl == 11 && !memcmp(v, "example.com", 11) && !http_header_get(q.h, q.nh, "x-none", NULL));
    const char *pipelined = "GET /a HTTP/1.1\r\nHost: h\r\n\r\nGET /b HTTP/1.1\r\nHost: h\r\n\r\n";
    CHECK(req(pipelined, &q) == HTTP_OK && q.consumed == strlen("GET /a HTTP/1.1\r\nHost: h\r\n\r\n") && !strcmp(q.target, "/a"));                                 /* only the first message is consumed */
    CHECK(req("POST /x HTTP/1.1\r\nHost: h\r\nContent-Length: 5\r\n\r\nhello", &q) == HTTP_OK && q.body_len == 5 && !memcmp(q.body, "hello", 5) && q.consumed > 5);
    CHECK(req("POST /x HTTP/1.1\r\nHost: h\r\nContent-Length: 5\r\n\r\nhel", &q) == HTTP_NEED_MORE);
    CHECK(req("GET /x HTTP/1.1\r\nHost: h\r\n", &q) == HTTP_NEED_MORE && req("GET /x HTT", &q) == HTTP_NEED_MORE);
    CHECK(req("POST /c HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\nTrailer: x\r\n\r\n", &q) == HTTP_OK);
    CHECK(q.chunked && q.body_len == 11 && !memcmp(q.body, "hello world", 11));
    CHECK(req("POST /c HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhel", &q) == HTTP_NEED_MORE);
    CHECK(req("POST /c HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\nZ\r\n", &q) == HTTP_BAD_FRAMING);
    CHECK(req("POST /c HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhelloXX0\r\n\r\n", &q) == HTTP_BAD_FRAMING);
    CHECK(req("POST /c HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n100000000\r\n", &q) == HTTP_BAD_FRAMING);
    CHECK(req("GET /x HTTP/1.0\r\n\r\n", &q) == HTTP_OK && !q.keep_alive && q.minor == 0);                                  /* HTTP/1.0 needs no Host */
    CHECK(req("GET /x HTTP/1.0\r\nConnection: keep-alive\r\n\r\n", &q) == HTTP_OK && q.keep_alive);
    CHECK(req("GET /x HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n", &q) == HTTP_OK && !q.keep_alive);
    CHECK(req("GET /x HTTP/1.1\r\nHost: h\r\nConnection: keep-alive, Upgrade\r\n\r\n", &q) == HTTP_OK && q.keep_alive);

    /* strictness: smuggling, injection, malformed */
    CHECK(req("POST /x HTTP/1.1\r\nHost: h\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n", &q) == HTTP_BAD_FRAMING);   /* CL + TE */
    CHECK(req("POST /x HTTP/1.1\r\nHost: h\r\nContent-Length: 4\r\nContent-Length: 5\r\n\r\nabcde", &q) == HTTP_BAD_FRAMING);
    CHECK(req("POST /x HTTP/1.1\r\nHost: h\r\nContent-Length: 4, 4\r\n\r\nabcd", &q) == HTTP_OK && q.body_len == 4);
    CHECK(req("POST /x HTTP/1.1\r\nHost: h\r\nContent-Length: -1\r\n\r\n", &q) == HTTP_BAD_FRAMING && req("POST /x HTTP/1.1\r\nHost: h\r\nContent-Length: 1x\r\n\r\n", &q) == HTTP_BAD_FRAMING);
    CHECK(req("POST /x HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: gzip, chunked\r\n\r\n", &q) == HTTP_BAD_FRAMING && req("POST /x HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: identity\r\n\r\n", &q) == HTTP_BAD_FRAMING);
    CHECK(req("POST /x HTTP/1.1\r\nHost: h\r\nContent-Length: 99999999999\r\n\r\n", &q) == HTTP_TOO_LARGE);
    CHECK(req("POST /x HTTP/1.1\r\nHost: h\r\nContent-Length: 20000000\r\n\r\n", &q) == HTTP_TOO_LARGE);
    CHECK(req("GET /x HTTP/1.1\nHost: h\n\n", &q) == HTTP_BAD_REQUEST);                                                      /* bare LF */
    CHECK(req("GET /x HTTP/1.1\r\nHost: h\rX: y\r\n\r\n", &q) == HTTP_BAD_REQUEST);                                           /* bare CR */
    CHECK(req("GET /x HTTP/1.1\r\nHost: h\r\n X: folded\r\n\r\n", &q) == HTTP_BAD_REQUEST);                                    /* obs-fold */
    CHECK(req("GET /x HTTP/1.1\r\nHost : h\r\n\r\n", &q) == HTTP_BAD_REQUEST);                                                /* space before colon */
    CHECK(req("GET /x HTTP/1.1\r\nHo st: h\r\n\r\n", &q) == HTTP_BAD_REQUEST && req("GET /x HTTP/1.1\r\n: h\r\n\r\n", &q) == HTTP_BAD_REQUEST);
    CHECK(req("GET /x HTTP/1.1\r\nHost: h\r\nX: a\001b\r\n\r\n", &q) == HTTP_BAD_REQUEST);
    uint8_t nul[] = "GET /x HTTP/1.1\r\nHost: h\0x\r\n\r\n"; uint8_t bb[64]; CHECK(http_parse_request(nul, sizeof nul - 1, bb, sizeof bb, &q) == HTTP_BAD_REQUEST);
    CHECK(req("GET /x HTTP/1.1\r\n\r\n", &q) == HTTP_BAD_REQUEST && req("GET /x HTTP/1.1\r\nHost: a\r\nHost: b\r\n\r\n", &q) == HTTP_BAD_REQUEST);   /* Host: exactly once */
    CHECK(req("GET /x HTTP/2.0\r\nHost: h\r\n\r\n", &q) == HTTP_BAD_VERSION && req("GET /x HTTP/1.2\r\nHost: h\r\n\r\n", &q) == HTTP_BAD_VERSION && req("GET /x http/1.1\r\nHost: h\r\n\r\n", &q) == HTTP_BAD_VERSION);
    CHECK(req("GET  /x HTTP/1.1\r\nHost: h\r\n\r\n", &q) == HTTP_BAD_REQUEST && req("GET /x  HTTP/1.1\r\nHost: h\r\n\r\n", &q) == HTTP_BAD_VERSION);
    CHECK(req("G@T /x HTTP/1.1\r\nHost: h\r\n\r\n", &q) == HTTP_BAD_METHOD && req(" /x HTTP/1.1\r\nHost: h\r\n\r\n", &q) == HTTP_BAD_METHOD);
    CHECK(req("GET x HTTP/1.1\r\nHost: h\r\n\r\n", &q) == HTTP_BAD_REQUEST);
    char longreq[4096 + 64]; snprintf(longreq, sizeof longreq, "GET /"); memset(longreq + 5, 'a', 3000); strcpy(longreq + 3005, " HTTP/1.1\r\nHost: h\r\n\r\n");
    CHECK(req(longreq, &q) == HTTP_URI_TOO_LONG);
    char hugehdr[20000]; strcpy(hugehdr, "GET / HTTP/1.1\r\nHost: h\r\nX: "); size_t hl = strlen(hugehdr); memset(hugehdr + hl, 'v', 17000); hugehdr[hl + 17000] = 0;
    CHECK(req(hugehdr, &q) == HTTP_HEADERS_TOO_LARGE);
    char manyh[8192]; strcpy(manyh, "GET / HTTP/1.1\r\nHost: h\r\n"); for (int i = 0; i < 80; i++) strcat(manyh, "X-A: 1\r\n"); strcat(manyh, "\r\n");
    CHECK(req(manyh, &q) == HTTP_HEADERS_TOO_LARGE);
    for (int t = 0; t < 4000; t++) {                                                                                          /* mutation fuzz: never crash */
        char f[256]; strcpy(f, "POST /p HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n");
        unsigned s = (unsigned)t * 2654435761u; for (int k = 0; k < 3; k++) { s = s * 1664525u + 1013904223u; f[(s >> 8) % (unsigned)strlen(f)] = (char)(s >> 24); }
        req(f, &q);
    }

    /* ---- responses ---- */
    CHECK(resp("HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc", false, &r) == HTTP_OK && r.status == 200 && !strcmp(r.reason, "OK") && r.body_len == 3 && r.keep_alive);
    CHECK(resp("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n", false, &r) == HTTP_OK && r.status == 404 && !r.body_len);
    CHECK(resp("HTTP/1.1 204 No Content\r\n\r\n", false, &r) == HTTP_OK && r.consumed == 27);
    CHECK(resp("HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n", true, &r) == HTTP_OK && !r.body_len);                       /* HEAD: headers only */
    CHECK(resp("HTTP/1.1 304 Not Modified\r\nContent-Length: 100\r\n\r\n", false, &r) == HTTP_OK && !r.body_len);
    CHECK(resp("HTTP/1.0 200 OK\r\n\r\nstream until close", false, &r) == HTTP_OK && r.body_until_close && r.body_len == 18 && !r.keep_alive);
    CHECK(resp("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n", false, &r) == HTTP_OK && r.chunked && r.body_len == 3);
    CHECK(resp("HTTP/1.1 200\r\nContent-Length: 0\r\n\r\n", false, &r) == HTTP_OK && !r.reason[0]);                            /* reason phrase may be absent */
    CHECK(resp("HTTP/1.1 20 OK\r\n\r\n", false, &r) != HTTP_OK && resp("HTTP/9.9 200 OK\r\n\r\n", false, &r) == HTTP_BAD_VERSION && resp("HTTP/1.1 099 x\r\n\r\n", false, &r) == HTTP_BAD_REQUEST);
    CHECK(resp("HTTP/1.1 301 Moved\r\nLocation: /new\r\nContent-Length: 0\r\n\r\n", false, &r) == HTTP_OK && (v = http_header_get(r.h, r.nh, "location", &vl)) && vl == 4);
    CHECK(http_header_has_token(r.h, r.nh, "location", "/new") && !http_header_has_token(r.h, r.nh, "location", "/old"));

    /* ---- writers ---- */
    uint8_t out[1024];
    http_header ex[2] = { { "Accept", "text/html", 6, 9 }, { "X-Test", "1", 6, 1 } };
    int n = http_write_request(out, sizeof out, "GET", "example.com", "/p?q=1", ex, 2, NULL, 0);
    CHECK(n > 0 && !strncmp((char *)out, "GET /p?q=1 HTTP/1.1\r\nHost: example.com\r\nAccept: text/html\r\nX-Test: 1\r\n\r\n", (size_t)n) && (size_t)n == strlen("GET /p?q=1 HTTP/1.1\r\nHost: example.com\r\nAccept: text/html\r\nX-Test: 1\r\n\r\n"));
    n = http_write_request(out, sizeof out, "POST", "h", "/u", NULL, 0, "data", 4);
    CHECK(n > 0 && strstr((char *)out, "Content-Length: 4\r\n\r\ndata") != NULL);
    CHECK(http_parse_request(out, (size_t)n, bb, sizeof bb, &q) == HTTP_OK && q.body_len == 4);                              /* writer output parses */
    http_header bad_inj = { "X", "a\r\nInjected: 1", 1, 14 };
    CHECK(http_write_request(out, sizeof out, "GET", "h", "/", &bad_inj, 1, NULL, 0) == -1);
    CHECK(http_write_request(out, sizeof out, "GET\r\nX:", "h", "/", NULL, 0, NULL, 0) == -1 && http_write_request(out, sizeof out, "GET", "h", "/a b", NULL, 0, NULL, 0) == -1 && http_write_request(out, 20, "GET", "h", "/", NULL, 0, NULL, 0) == -1);
    n = http_write_response(out, sizeof out, 200, NULL, ex, 1, "ok", 2, false);
    CHECK(n > 0 && strstr((char *)out, "HTTP/1.1 200 OK\r\n") && strstr((char *)out, "Content-Length: 2\r\n") && strstr((char *)out, "Connection: close\r\n") && !strcmp((char *)out + n - 2, "ok"));
    CHECK(http_parse_response(out, (size_t)n, false, bb, sizeof bb, &r) == HTTP_OK && r.body_len == 2 && !r.keep_alive);
    n = http_write_response(out, sizeof out, 204, NULL, NULL, 0, "x", 1, true);
    CHECK(strstr((char *)out, "Content-Length") == NULL && out[n - 1] == '\n');                                                 /* 204 never carries a body */
    CHECK(http_write_response(out, sizeof out, 99, NULL, NULL, 0, NULL, 0, true) == -1 && http_write_response(out, sizeof out, 200, "bad\nreason", NULL, 0, NULL, 0, true) == -1);
    uint8_t ch[64]; n = http_write_chunk(ch, sizeof ch, "hello", 5); CHECK(n == 10 && !memcmp(ch, "5\r\nhello\r\n", 10));
    n = http_write_chunk(ch, sizeof ch, NULL, 0); CHECK(n == 5 && !memcmp(ch, "0\r\n\r\n", 5));
    CHECK(http_write_chunk(ch, 4, "hello", 5) == -1);

    /* ---- URLs ---- */
    http_url u;
    CHECK(http_url_parse("HTTPS://Example.COM:8443/a/b?x=1&y=2#frag", &u) && u.https && !strcmp(u.host, "example.com") && u.port == 8443 && !strcmp(u.path, "/a/b") && !strcmp(u.query, "x=1&y=2"));
    CHECK(http_url_parse("http://h", &u) && !u.https && u.port == 80 && !strcmp(u.path, "/") && http_url_parse("https://h?q", &u) && u.port == 443 && !strcmp(u.query, "q") && !strcmp(u.path, "/"));
    CHECK(http_url_parse("http://[2001:db8::1]:8080/x", &u) && u.ipv6_literal && !strcmp(u.host, "2001:db8::1") && u.port == 8080);
    char fmt[300]; http_url_format(&u, fmt, sizeof fmt); CHECK(!strcmp(fmt, "http://[2001:db8::1]:8080/x"));
    http_url_parse("https://example.com/p?x=1", &u); http_url_format(&u, fmt, sizeof fmt); CHECK(!strcmp(fmt, "https://example.com/p?x=1"));
    const char *bad_urls[] = { "ftp://x/", "example.com", "http://", "http:///x", "http://user@host/", "http://host:0/", "http://host:99999/", "http://host:/", "http://ho st/", "http://[::1/", "http://[]/", "http://[zz]/", "http://-.x/", "http://.host/", "http://host./", "javascript:alert(1)", "http://a@b@c/" };
    for (unsigned i = 0; i < sizeof bad_urls / sizeof *bad_urls; i++) CHECK(!http_url_parse(bad_urls[i], &u));
    char enc[64]; CHECK(http_percent_encode("a b/c?d=\xc3\xa9", enc, sizeof enc, false) == 22 && !strcmp(enc, "a%20b%2Fc%3Fd%3D%C3%A9"));
    http_percent_encode("a b/c", enc, sizeof enc, true); CHECK(!strcmp(enc, "a%20b/c"));
    http_percent_encode("~-._", enc, sizeof enc, false); CHECK(!strcmp(enc, "~-._"));
    char dec[64]; CHECK(http_percent_decode("a%20b%2Fc", 9, dec, sizeof dec) == 5 && !strcmp(dec, "a b/c") && http_percent_decode("%4", 2, dec, sizeof dec) == -1 && http_percent_decode("%zz", 3, dec, sizeof dec) == -1 && http_percent_decode("a%00b", 5, dec, sizeof dec) == -1);
    CHECK(http_percent_decode("abc", 3, dec, 3) == -1 && http_percent_decode("abc", 3, dec, 4) == 3);
    char norm[128];
    CHECK(http_path_normalize("/a/b/../c/./d", norm, sizeof norm) && !strcmp(norm, "/a/c/d"));
    CHECK(http_path_normalize("/a//b/", norm, sizeof norm) && !strcmp(norm, "/a/b/") && http_path_normalize("/", norm, sizeof norm) && !strcmp(norm, "/") && http_path_normalize("/a/..", norm, sizeof norm) && !strcmp(norm, "/"));
    CHECK(!http_path_normalize("/../etc/passwd", norm, sizeof norm) && !http_path_normalize("/a/../../x", norm, sizeof norm) && !http_path_normalize("/%2e%2e/x", norm, sizeof norm));
    CHECK(!http_path_normalize("/a%2Fb", norm, sizeof norm) && !http_path_normalize("/a%5Cb", norm, sizeof norm) && !http_path_normalize("/a%00", norm, sizeof norm) && !http_path_normalize("x", norm, sizeof norm));
    CHECK(http_path_normalize("/a/%61b", norm, sizeof norm) && !strcmp(norm, "/a/ab"));                                        /* decoded exactly once */
    CHECK(http_path_normalize("/%252e%252e/x", norm, sizeof norm) && !strcmp(norm, "/%2e%2e/x"));                             /* double encoding stays literal */
    char qv[64];
    CHECK(http_query_get("a=1&b=hello+world&c=%41%42&d", "b", qv, sizeof qv) && !strcmp(qv, "hello world") && http_query_get("a=1&b=2&c=%41%42&d", "c", qv, sizeof qv) && !strcmp(qv, "AB"));
    CHECK(http_query_get("a=1&d", "d", qv, sizeof qv) && !qv[0] && !http_query_get("a=1", "z", qv, sizeof qv) && http_query_get("k%65y=v", "key", qv, sizeof qv) && !strcmp(qv, "v"));

    /* ---- redirects ---- */
    http_url base, to; http_url_parse("https://example.com/dir/page?x=1", &base); char s[300];
    CHECK(http_redirect_resolve(&base, "https://other.org/z", &to) && !strcmp(to.host, "other.org"));
    CHECK(http_redirect_resolve(&base, "/abs/path?q=2", &to) && (http_url_format(&to, s, sizeof s), !strcmp(s, "https://example.com/abs/path?q=2")));
    CHECK(http_redirect_resolve(&base, "rel/x", &to) && (http_url_format(&to, s, sizeof s), !strcmp(s, "https://example.com/dir/rel/x")));
    CHECK(http_redirect_resolve(&base, "../up", &to) && (http_url_format(&to, s, sizeof s), !strcmp(s, "https://example.com/up")));
    CHECK(http_redirect_resolve(&base, "//cdn.example.net/a", &to) && to.https && !strcmp(to.host, "cdn.example.net"));
    CHECK(http_redirect_resolve(&base, "?other=1", &to) && (http_url_format(&to, s, sizeof s), !strcmp(s, "https://example.com/dir/page?other=1")));
    CHECK(http_redirect_resolve(&base, "", &to) && !strcmp(to.path, "/dir/page") && !strcmp(to.query, "x=1"));
    CHECK(http_redirect_resolve(&base, "../../../../x", &to) && !strcmp(to.path, "/x"));                                       /* cannot climb above the root */
    http_redirect_policy pol = { 3, false }; bool strip;
    http_url plain; http_url_parse("http://example.com/", &plain);
    CHECK(http_redirect_allowed(&pol, 0, &base, &base, &strip) && !strip);
    CHECK(!http_redirect_allowed(&pol, 0, &base, &plain, &strip));                                                             /* https -> http is a downgrade */
    CHECK(http_redirect_allowed(&pol, 0, &plain, &base, &strip) && strip);                                                     /* upgrade is fine but crosses origin */
    http_url other; http_url_parse("https://evil.test/", &other);
    CHECK(http_redirect_allowed(&pol, 2, &base, &other, &strip) && strip && !http_redirect_allowed(&pol, 3, &base, &other, &strip));
    pol.allow_downgrade = true; CHECK(http_redirect_allowed(&pol, 0, &base, &plain, &strip));

    /* ---- server ---- */
    http_server srv; http_server_init(&srv);
    CHECK(http_server_add(&srv, HTTP_M_GET | HTTP_M_HEAD, "/hello", h_hello, NULL) && http_server_add(&srv, HTTP_M_GET, "/static/*", h_hello, NULL) && http_server_add(&srv, HTTP_M_POST | HTTP_M_PUT, "/echo", h_echo, NULL));
    CHECK(!http_server_add(&srv, HTTP_M_GET, "noslash", h_hello, NULL));
    uint8_t o[2048]; size_t consumed; bool close;
    #define SERVE(str) serve_nul(&srv, (const uint8_t *)(str), strlen(str), o, sizeof o, &consumed, &close)
    n = SERVE("GET /hello HTTP/1.1\r\nHost: h\r\n\r\n");
    CHECK(n > 0 && !strncmp((char *)o, "HTTP/1.1 200 OK", 15) && strstr((char *)o, "hello GET /hello") && !close && consumed == strlen("GET /hello HTTP/1.1\r\nHost: h\r\n\r\n"));
    n = SERVE("HEAD /hello HTTP/1.1\r\nHost: h\r\n\r\n");
    CHECK(n > 0 && strstr((char *)o, "Content-Length: 17") && !strstr((char *)o, "hello HEAD") && o[n - 1] == '\n' && o[n - 4] == '\r');
    n = SERVE("GET /static/a/b/../c.txt HTTP/1.1\r\nHost: h\r\n\r\n"); CHECK(strstr((char *)o, "hello GET /static/a/c.txt") != NULL);
    n = SERVE("GET /static/../../etc/passwd HTTP/1.1\r\nHost: h\r\n\r\n"); CHECK(!strncmp((char *)o, "HTTP/1.1 400", 12));
    n = SERVE("GET /static/%2e%2e/%2e%2e/x HTTP/1.1\r\nHost: h\r\n\r\n"); CHECK(!strncmp((char *)o, "HTTP/1.1 400", 12));
    n = SERVE("GET /nothing HTTP/1.1\r\nHost: h\r\n\r\n"); CHECK(!strncmp((char *)o, "HTTP/1.1 404", 12));
    n = SERVE("DELETE /hello HTTP/1.1\r\nHost: h\r\n\r\n"); CHECK(!strncmp((char *)o, "HTTP/1.1 405", 12) && strstr((char *)o, "Allow: GET, HEAD"));
    n = SERVE("POST /hello HTTP/1.1\r\nHost: h\r\nContent-Length: 0\r\n\r\n"); CHECK(!strncmp((char *)o, "HTTP/1.1 405", 12));
    n = SERVE("BREW /hello HTTP/1.1\r\nHost: h\r\n\r\n"); CHECK(!strncmp((char *)o, "HTTP/1.1 501", 12));
    n = SERVE("POST /echo HTTP/1.1\r\nHost: h\r\nContent-Length: 4\r\n\r\nping"); CHECK(!strncmp((char *)o, "HTTP/1.1 201", 12) && !memcmp(o + n - 4, "ping", 4));
    n = SERVE("POST /echo HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n4\r\npong\r\n0\r\n\r\n"); CHECK(!strncmp((char *)o, "HTTP/1.1 201", 12) && !memcmp(o + n - 4, "pong", 4));
    n = SERVE("OPTIONS /echo HTTP/1.1\r\nHost: h\r\n\r\n"); CHECK(!strncmp((char *)o, "HTTP/1.1 204", 12) && strstr((char *)o, "Allow: POST, PUT, OPTIONS"));
    n = SERVE("OPTIONS * HTTP/1.1\r\nHost: h\r\n\r\n"); CHECK(!strncmp((char *)o, "HTTP/1.1 204", 12));
    n = SERVE("GET http://h/hello?z=1 HTTP/1.1\r\nHost: h\r\n\r\n"); CHECK(strstr((char *)o, "hello GET /hello") != NULL);                        /* absolute-form targets */
    n = SERVE("GET /hello HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n"); CHECK(close && strstr((char *)o, "Connection: close"));
    n = SERVE("GET /hel"); CHECK(n == 0 && consumed == 0);
    n = SERVE("GET /hello HTTP/1.1\r\n\r\n"); CHECK(!strncmp((char *)o, "HTTP/1.1 400", 12) && close);
    n = SERVE("GET /hello HTTP/3\r\nHost: h\r\n\r\n"); CHECK(!strncmp((char *)o, "HTTP/1.1 505", 12));
    n = SERVE("POST /echo HTTP/1.1\r\nHost: h\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n"); CHECK(!strncmp((char *)o, "HTTP/1.1 400", 12) && close);
    n = SERVE("POST /echo HTTP/1.1\r\nHost: h\r\nContent-Length: 6000000\r\n\r\n"); CHECK(n > 0 && close);
    srv.max_body = 8; n = SERVE("POST /echo HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\nA\r\n0123456789\r\n0\r\n\r\n"); CHECK(!strncmp((char *)o, "HTTP/1.1 413", 12));
    DONE("http_test");
}
