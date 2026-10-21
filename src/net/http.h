/* HTTP/1.1 (RFC 9110/9112) message codec, URL handling, redirect policy and a request router for the local
 * server role. I/O-free: callers hand over bytes and receive bytes, so the same code serves winhttp/http.sys
 * facades and tests. Parsers are strict (request smuggling, header injection and oversize inputs are errors). */
#ifndef LOOS_HTTP_H
#define LOOS_HTTP_H
#include "netutil.h"

#ifdef __cplusplus
extern "C" {
#endif

#define HTTP_MAX_HEADERS 64
#define HTTP_MAX_HEADER_BYTES 16384
#define HTTP_MAX_TARGET 2048
#define HTTP_MAX_BODY (8u * 1024 * 1024)
typedef enum { HTTP_OK = 0, HTTP_NEED_MORE, HTTP_BAD_REQUEST, HTTP_TOO_LARGE, HTTP_HEADERS_TOO_LARGE, HTTP_BAD_VERSION, HTTP_BAD_METHOD, HTTP_URI_TOO_LONG, HTTP_BAD_FRAMING } http_result;

typedef struct { const char *name, *value; size_t nlen, vlen; } http_header;
typedef struct {
    char method[16], target[HTTP_MAX_TARGET]; int minor;
    http_header h[HTTP_MAX_HEADERS]; int nh;
    const uint8_t *body; size_t body_len;
    bool chunked, keep_alive;
    size_t consumed;                       /* bytes of the input this message occupied */
} http_request;
typedef struct {
    int status; char reason[48]; int minor;
    http_header h[HTTP_MAX_HEADERS]; int nh;
    const uint8_t *body; size_t body_len;
    bool chunked, keep_alive;
    bool body_until_close;                 /* no length given: the body is whatever arrives before the connection closes */
    size_t consumed;
} http_response;

/* Parse one complete message from buf. body_buf receives de-chunked bodies (identity bodies point into buf). */
http_result http_parse_request(const uint8_t *buf, size_t len, uint8_t *body_buf, size_t body_cap, http_request *out);
http_result http_parse_response(const uint8_t *buf, size_t len, bool head_request, uint8_t *body_buf, size_t body_cap, http_response *out);
const char *http_header_get(const http_header *h, int n, const char *name, size_t *vlen);     /* case-insensitive; first match */
bool http_header_has_token(const http_header *h, int n, const char *name, const char *token); /* comma-list membership */

/* serialisation: returns bytes written or -1 */
int http_write_request(uint8_t *buf, size_t cap, const char *method, const char *host, const char *target, const http_header *extra, int nextra, const void *body, size_t body_len);
int http_write_response(uint8_t *buf, size_t cap, int status, const char *reason, const http_header *extra, int nextra, const void *body, size_t body_len, bool keep_alive);
int http_write_chunk(uint8_t *buf, size_t cap, const void *data, size_t len);                  /* "len\r\n data \r\n"; len 0 = final chunk */
const char *http_reason(int status);

/* ---- URLs ---- */
typedef struct { bool https; char host[256]; uint16_t port; char path[HTTP_MAX_TARGET]; char query[HTTP_MAX_TARGET]; bool ipv6_literal; } http_url;
bool http_url_parse(const char *s, http_url *u);                       /* absolute http(s) only; userinfo is refused */
size_t http_url_format(const http_url *u, char *out, size_t cap);
size_t http_percent_encode(const char *in, char *out, size_t cap, bool keep_slash);
int  http_percent_decode(const char *in, size_t n, char *out, size_t cap);   /* length or -1 for malformed %XX / NUL */
bool http_path_normalize(const char *path, char *out, size_t cap);     /* decode once, drop "."/".." segments, refuse escapes above the root and NULs */
bool http_query_get(const char *query, const char *key, char *out, size_t cap);
bool http_redirect_resolve(const http_url *base, const char *location, http_url *out);   /* RFC 3986 reference resolution */
typedef struct { int max_redirects; bool allow_downgrade; } http_redirect_policy;
bool http_redirect_allowed(const http_redirect_policy *p, int followed, const http_url *from, const http_url *to, bool *strip_credentials);

/* ---- server-side router ---- */
typedef struct { int status; const char *reason; http_header h[8]; int nh; const uint8_t *body; size_t body_len; } http_reply;
typedef void (*http_handler)(const http_request *req, const char *path, http_reply *reply, void *user);
#define HTTP_M_GET 1
#define HTTP_M_HEAD 2
#define HTTP_M_POST 4
#define HTTP_M_PUT 8
#define HTTP_M_DELETE 16
typedef struct { unsigned methods; char pattern[64]; http_handler fn; void *user; } http_route;
typedef struct { http_route route[32]; int nroute; uint32_t max_body; } http_server;
void http_server_init(http_server *s);
bool http_server_add(http_server *s, unsigned methods, const char *pattern, http_handler fn, void *user);   /* pattern: exact path, or a path ending in slash-star for a prefix */
/* Serves the first complete request in in[]. Returns bytes written to out (0 if more input is needed); *consumed gets the
 * request length and *close tells the caller to close the connection after sending. */
int  http_server_serve(const http_server *s, const uint8_t *in, size_t in_len, uint8_t *out, size_t out_cap, size_t *consumed, bool *close);

#ifdef __cplusplus
}
#endif
#endif
