#include "http.h"
#include <string.h>
#include <stdio.h>
#include <stdlib.h>

/* tiny local replacements: the network code stays free of libc locale and GNU extensions */
static int strncasecmp_(const char *a, const char *b, size_t n)
{
    for (; n; n--, a++, b++) { int x = *a >= 'A' && *a <= 'Z' ? *a + 32 : *a, y = *b >= 'A' && *b <= 'Z' ? *b + 32 : *b; if (x != y || !x) return x - y; }
    return 0;
}
static const uint8_t *memmem_(const uint8_t *h, size_t hl, const char *n, size_t nl)
{
    for (size_t i = 0; i + nl <= hl; i++) if (!memcmp(h + i, n, nl)) return h + i;
    return NULL;
}

static int lc(int c) { return c >= 'A' && c <= 'Z' ? c + 32 : c; }
static bool ieq(const char *a, size_t an, const char *b) { size_t bn = strlen(b); if (an != bn) return false; for (size_t i = 0; i < an; i++) if (lc((unsigned char)a[i]) != lc((unsigned char)b[i])) return false; return true; }
static bool is_tchar(unsigned char c) { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || strchr("!#$%&'*+-.^_`|~", c) != NULL; }

const char *http_reason(int s)
{
    switch (s) {
    case 100: return "Continue"; case 200: return "OK"; case 201: return "Created"; case 204: return "No Content"; case 206: return "Partial Content";
    case 301: return "Moved Permanently"; case 302: return "Found"; case 303: return "See Other"; case 304: return "Not Modified"; case 307: return "Temporary Redirect"; case 308: return "Permanent Redirect";
    case 400: return "Bad Request"; case 401: return "Unauthorized"; case 403: return "Forbidden"; case 404: return "Not Found"; case 405: return "Method Not Allowed";
    case 408: return "Request Timeout"; case 411: return "Length Required"; case 413: return "Content Too Large"; case 414: return "URI Too Long"; case 431: return "Request Header Fields Too Large";
    case 500: return "Internal Server Error"; case 501: return "Not Implemented"; case 503: return "Service Unavailable"; case 505: return "HTTP Version Not Supported";
    default: return "Status";
    }
}

const char *http_header_get(const http_header *h, int n, const char *name, size_t *vlen)
{
    for (int i = 0; i < n; i++) if (ieq(h[i].name, h[i].nlen, name)) { if (vlen) *vlen = h[i].vlen; return h[i].value; }
    return NULL;
}
bool http_header_has_token(const http_header *h, int n, const char *name, const char *token)
{
    for (int i = 0; i < n; i++) {
        if (!ieq(h[i].name, h[i].nlen, name)) continue;
        const char *p = h[i].value, *end = p + h[i].vlen;
        while (p < end) {
            while (p < end && (*p == ' ' || *p == '\t' || *p == ',')) p++;
            const char *s = p;
            while (p < end && *p != ',') p++;
            const char *e = p;
            while (e > s && (e[-1] == ' ' || e[-1] == '\t')) e--;
            if (e > s && ieq(s, (size_t)(e - s), token)) return true;
        }
    }
    return false;
}

/* locate the blank line ending the header block; strict CRLF only */
static http_result find_head_end(const uint8_t *b, size_t len, size_t *end)
{
    size_t scan = len < HTTP_MAX_HEADER_BYTES + 4 ? len : HTTP_MAX_HEADER_BYTES + 4;
    for (size_t i = 0; i < scan; i++) {
        if (b[i] == 0) return HTTP_BAD_REQUEST;
        if (b[i] == '\n' && (i == 0 || b[i - 1] != '\r')) return HTTP_BAD_REQUEST;                 /* bare LF: smuggling vector */
        if (b[i] == '\r' && (i + 1 >= len ? false : b[i + 1] != '\n')) return HTTP_BAD_REQUEST;     /* bare CR */
        if (i >= 3 && b[i] == '\n' && b[i - 1] == '\r' && b[i - 2] == '\n' && b[i - 3] == '\r') { *end = i + 1; return HTTP_OK; }
    }
    return len > HTTP_MAX_HEADER_BYTES + 4 ? HTTP_HEADERS_TOO_LARGE : HTTP_NEED_MORE;
}

static http_result parse_headers(const char *p, const char *end, http_header *h, int *nh)
{
    *nh = 0;
    while (p < end) {
        const char *eol = memchr(p, '\r', (size_t)(end - p));
        if (!eol) return HTTP_BAD_REQUEST;
        if (eol == p) return HTTP_OK;
        if (*p == ' ' || *p == '\t') return HTTP_BAD_REQUEST;                                       /* obs-fold */
        const char *colon = memchr(p, ':', (size_t)(eol - p));
        if (!colon || colon == p) return HTTP_BAD_REQUEST;
        for (const char *c = p; c < colon; c++) if (!is_tchar((unsigned char)*c)) return HTTP_BAD_REQUEST;     /* also rejects "Name :" */
        const char *v = colon + 1, *ve = eol;
        while (v < ve && (*v == ' ' || *v == '\t')) v++;
        while (ve > v && (ve[-1] == ' ' || ve[-1] == '\t')) ve--;
        for (const char *c = v; c < ve; c++) if ((unsigned char)*c < 0x20 && *c != '\t') return HTTP_BAD_REQUEST;
        if (*nh >= HTTP_MAX_HEADERS) return HTTP_HEADERS_TOO_LARGE;
        h[*nh].name = p; h[*nh].nlen = (size_t)(colon - p); h[*nh].value = v; h[*nh].vlen = (size_t)(ve - v);
        (*nh)++;
        p = eol + 2;
    }
    return HTTP_BAD_REQUEST;
}

/* framing: 0 none, 1 content-length, 2 chunked */
static http_result framing(const http_header *h, int n, int *kind, size_t *cl)
{
    *kind = 0; *cl = 0;
    int ncl = 0, nte = 0; const http_header *te = NULL;
    for (int i = 0; i < n; i++) {
        if (ieq(h[i].name, h[i].nlen, "content-length")) {
            ncl++;
            const char *p = h[i].value, *end = p + h[i].vlen;
            while (p < end) {                                                                        /* "5, 5" is tolerated, "5, 6" is not */
                while (p < end && (*p == ' ' || *p == ',')) p++;
                if (p >= end) break;
                size_t v = 0; int digits = 0;
                while (p < end && *p >= '0' && *p <= '9') { if (v > (SIZE_MAX - 9) / 10) return HTTP_BAD_FRAMING; v = v * 10 + (size_t)(*p++ - '0'); digits++; }
                if (!digits || (p < end && *p != ',' && *p != ' ')) return HTTP_BAD_FRAMING;
                if (*kind == 1 && v != *cl) return HTTP_BAD_FRAMING;
                *kind = 1; *cl = v;
            }
        } else if (ieq(h[i].name, h[i].nlen, "transfer-encoding")) { nte++; te = &h[i]; }
    }
    (void)ncl;
    if (nte) {
        if (*kind == 1) return HTTP_BAD_FRAMING;                                                     /* both: classic smuggling */
        if (nte > 1 || !ieq(te->value, te->vlen, "chunked")) return HTTP_BAD_FRAMING;
        *kind = 2;
    }
    if (*kind == 1 && *cl > HTTP_MAX_BODY) return HTTP_TOO_LARGE;
    return HTTP_OK;
}

static http_result parse_chunked(const uint8_t *b, size_t len, size_t pos, uint8_t *out, size_t cap, size_t *out_len, size_t *end)
{
    size_t total = 0;
    for (;;) {
        size_t v = 0; int digits = 0;
        while (pos < len && ((b[pos] >= '0' && b[pos] <= '9') || ((b[pos] | 32) >= 'a' && (b[pos] | 32) <= 'f'))) {
            int d = b[pos] <= '9' ? b[pos] - '0' : (b[pos] | 32) - 'a' + 10;
            if (++digits > 8) return HTTP_BAD_FRAMING;
            v = v * 16 + (size_t)d; pos++;
        }
        if (pos >= len) return HTTP_NEED_MORE;
        if (!digits) return HTTP_BAD_FRAMING;
        size_t ext = 0;
        while (pos < len && b[pos] != '\r') { if (++ext > 1024) return HTTP_BAD_FRAMING; pos++; }          /* chunk extensions ignored, bounded */
        if (pos + 1 >= len) return HTTP_NEED_MORE;
        if (b[pos + 1] != '\n') return HTTP_BAD_FRAMING;
        pos += 2;
        if (v == 0) {
            /* trailers until a blank line */
            for (int guard = 0; guard < 64; guard++) {
                if (pos + 1 >= len) return HTTP_NEED_MORE;
                if (b[pos] == '\r' && b[pos + 1] == '\n') { *out_len = total; *end = pos + 2; return HTTP_OK; }
                const uint8_t *eol = memchr(b + pos, '\n', len - pos);
                if (!eol) return len - pos > 4096 ? HTTP_HEADERS_TOO_LARGE : HTTP_NEED_MORE;
                pos = (size_t)(eol - b) + 1;
            }
            return HTTP_HEADERS_TOO_LARGE;
        }
        if (v > HTTP_MAX_BODY || total + v > HTTP_MAX_BODY || total + v > cap) return HTTP_TOO_LARGE;
        if (pos + v + 2 > len) return HTTP_NEED_MORE;
        memcpy(out + total, b + pos, v); total += v; pos += v;
        if (b[pos] != '\r' || b[pos + 1] != '\n') return HTTP_BAD_FRAMING;
        pos += 2;
    }
}

static bool conn_keepalive(const http_header *h, int n, int minor)
{
    if (http_header_has_token(h, n, "connection", "close")) return false;
    if (minor == 0) return http_header_has_token(h, n, "connection", "keep-alive");
    return true;
}

http_result http_parse_request(const uint8_t *buf, size_t len, uint8_t *body_buf, size_t body_cap, http_request *r)
{
    size_t head_end; http_result rc = find_head_end(buf, len, &head_end);
    if (rc != HTTP_OK) return rc;
    memset(r, 0, sizeof *r);
    const char *p = (const char *)buf, *eol = memchr(p, '\r', head_end);
    if (!eol) return HTTP_BAD_REQUEST;
    const char *sp1 = memchr(p, ' ', (size_t)(eol - p));
    if (!sp1 || sp1 == p || (size_t)(sp1 - p) >= sizeof r->method) return HTTP_BAD_METHOD;
    for (const char *c = p; c < sp1; c++) if (!is_tchar((unsigned char)*c)) return HTTP_BAD_METHOD;
    memcpy(r->method, p, (size_t)(sp1 - p));
    const char *t = sp1 + 1, *sp2 = memchr(t, ' ', (size_t)(eol - t));
    if (!sp2 || sp2 == t) return HTTP_BAD_REQUEST;
    if ((size_t)(sp2 - t) >= sizeof r->target) return HTTP_URI_TOO_LONG;
    for (const char *c = t; c < sp2; c++) if ((unsigned char)*c <= 0x20 || (unsigned char)*c == 0x7f) return HTTP_BAD_REQUEST;
    memcpy(r->target, t, (size_t)(sp2 - t));
    if (r->target[0] != '/' && strcmp(r->target, "*") && strncmp(r->target, "http://", 7) && strncmp(r->target, "https://", 8)) return HTTP_BAD_REQUEST;
    const char *v = sp2 + 1;
    if ((size_t)(eol - v) != 8 || memcmp(v, "HTTP/1.", 7) || (v[7] != '0' && v[7] != '1')) return HTTP_BAD_VERSION;
    r->minor = v[7] - '0';
    rc = parse_headers(eol + 2, p + head_end, r->h, &r->nh);
    if (rc != HTTP_OK) return rc;
    if (r->minor == 1) {
        int hosts = 0; for (int i = 0; i < r->nh; i++) if (ieq(r->h[i].name, r->h[i].nlen, "host")) hosts++;
        if (hosts != 1) return HTTP_BAD_REQUEST;
    }
    int kind; size_t cl;
    rc = framing(r->h, r->nh, &kind, &cl);
    if (rc != HTTP_OK) return rc;
    r->keep_alive = conn_keepalive(r->h, r->nh, r->minor);
    if (kind == 0) { r->consumed = head_end; return HTTP_OK; }
    if (kind == 1) {
        if (cl > body_cap) return HTTP_TOO_LARGE;                                 /* refuse early, do not wait for a body we would reject */
        if (len - head_end < cl) return HTTP_NEED_MORE;
        r->body = buf + head_end; r->body_len = cl; r->consumed = head_end + cl;
        return HTTP_OK;
    }
    size_t out_len, end;
    rc = parse_chunked(buf, len, head_end, body_buf, body_cap, &out_len, &end);
    if (rc != HTTP_OK) return rc;
    r->chunked = true; r->body = body_buf; r->body_len = out_len; r->consumed = end;
    return HTTP_OK;
}

http_result http_parse_response(const uint8_t *buf, size_t len, bool head_request, uint8_t *body_buf, size_t body_cap, http_response *r)
{
    size_t head_end; http_result rc = find_head_end(buf, len, &head_end);
    if (rc != HTTP_OK) return rc;
    memset(r, 0, sizeof *r);
    const char *p = (const char *)buf, *eol = memchr(p, '\r', head_end);
    if (!eol || (size_t)(eol - p) < 12 || memcmp(p, "HTTP/1.", 7) || (p[7] != '0' && p[7] != '1') || p[8] != ' ') return HTTP_BAD_VERSION;
    r->minor = p[7] - '0';
    for (int i = 9; i < 12; i++) if (p[i] < '0' || p[i] > '9') return HTTP_BAD_REQUEST;
    r->status = (p[9] - '0') * 100 + (p[10] - '0') * 10 + (p[11] - '0');
    if (r->status < 100) return HTTP_BAD_REQUEST;
    if (eol - p > 12) {
        if (p[12] != ' ') return HTTP_BAD_REQUEST;
        size_t rl = (size_t)(eol - p) - 13; if (rl >= sizeof r->reason) rl = sizeof r->reason - 1;
        memcpy(r->reason, p + 13, rl);
    }
    rc = parse_headers(eol + 2, p + head_end, r->h, &r->nh);
    if (rc != HTTP_OK) return rc;
    r->keep_alive = conn_keepalive(r->h, r->nh, r->minor);
    bool no_body = head_request || (r->status >= 100 && r->status < 200) || r->status == 204 || r->status == 304;
    int kind; size_t cl;
    rc = framing(r->h, r->nh, &kind, &cl);
    if (rc != HTTP_OK) return rc;
    if (no_body) { r->consumed = head_end; return HTTP_OK; }
    if (kind == 1) {
        if (len - head_end < cl) return HTTP_NEED_MORE;
        r->body = buf + head_end; r->body_len = cl; r->consumed = head_end + cl;
        return HTTP_OK;
    }
    if (kind == 2) {
        size_t out_len, end;
        rc = parse_chunked(buf, len, head_end, body_buf, body_cap, &out_len, &end);
        if (rc != HTTP_OK) return rc;
        r->chunked = true; r->body = body_buf; r->body_len = out_len; r->consumed = end;
        return HTTP_OK;
    }
    r->body_until_close = true; r->keep_alive = false;
    r->body = buf + head_end; r->body_len = len - head_end; r->consumed = len;
    if (r->body_len > HTTP_MAX_BODY) return HTTP_TOO_LARGE;
    return HTTP_OK;
}

/* ---- writers ---- */
static bool safe_field(const char *s, bool allow_space) { for (; *s; s++) if ((unsigned char)*s < 0x20 || *s == 0x7f || (!allow_space && *s == ' ')) return false; return true; }
#define W(...) do { int k_ = snprintf((char *)buf + n, n < cap ? cap - n : 0, __VA_ARGS__); if (k_ < 0 || n + (size_t)k_ >= cap) return -1; n += (size_t)k_; } while (0)

static int write_headers(uint8_t *buf, size_t cap, size_t n, const http_header *x, int nx)
{
    for (int i = 0; i < nx; i++) {
        char name[64], val[512];
        if (x[i].nlen >= sizeof name || x[i].vlen >= sizeof val) return -1;
        memcpy(name, x[i].name, x[i].nlen); name[x[i].nlen] = 0; memcpy(val, x[i].value, x[i].vlen); val[x[i].vlen] = 0;
        if (!x[i].nlen) return -1;
        for (const char *c = name; *c; c++) if (!is_tchar((unsigned char)*c)) return -1;
        if (!safe_field(val, true)) return -1;                                                       /* no CR/LF injection */
        W("%s: %s\r\n", name, val);
    }
    return (int)n;
}

int http_write_request(uint8_t *buf, size_t cap, const char *method, const char *host, const char *target, const http_header *extra, int nextra, const void *body, size_t body_len)
{
    size_t n = 0;
    if (!safe_field(method, false) || !*method || !safe_field(target, false) || !*target || !safe_field(host, false) || !*host) return -1;
    W("%s %s HTTP/1.1\r\nHost: %s\r\n", method, target, host);
    int r = write_headers(buf, cap, n, extra, nextra);
    if (r < 0) return -1;
    n = (size_t)r;
    if (body_len || !strcmp(method, "POST") || !strcmp(method, "PUT")) W("Content-Length: %zu\r\n", body_len);
    W("\r\n");
    if (n + body_len > cap) return -1;
    if (body_len) memcpy(buf + n, body, body_len);
    return (int)(n + body_len);
}

int http_write_response(uint8_t *buf, size_t cap, int status, const char *reason, const http_header *extra, int nextra, const void *body, size_t body_len, bool keep_alive)
{
    size_t n = 0;
    if (status < 100 || status > 599 || !safe_field(reason ? reason : "", true)) return -1;
    W("HTTP/1.1 %d %s\r\n", status, reason ? reason : http_reason(status));
    int r = write_headers(buf, cap, n, extra, nextra);
    if (r < 0) return -1;
    n = (size_t)r;
    bool bodyless = status < 200 || status == 204 || status == 304;
    if (!bodyless) W("Content-Length: %zu\r\n", body_len);
    if (!keep_alive) W("Connection: close\r\n");
    W("\r\n");
    if (bodyless) return (int)n;
    if (n + body_len > cap) return -1;
    if (body_len) memcpy(buf + n, body, body_len);
    return (int)(n + body_len);
}

int http_write_chunk(uint8_t *buf, size_t cap, const void *data, size_t len)
{
    size_t n = 0;
    W("%zx\r\n", len);
    if (n + len + 2 > cap) return -1;
    if (len) memcpy(buf + n, data, len);
    n += len; buf[n++] = '\r'; buf[n++] = '\n';
    return (int)n;
}

/* ---- URLs ---- */
size_t http_percent_encode(const char *in, char *out, size_t cap, bool keep_slash)
{
    size_t n = 0;
    for (; *in; in++) {
        unsigned char c = (unsigned char)*in;
        bool keep = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '-' || c == '.' || c == '_' || c == '~' || (keep_slash && c == '/');
        if (keep) { if (n + 1 < cap) out[n] = (char)c; n++; }
        else { if (n + 3 < cap) { static const char hx[] = "0123456789ABCDEF"; out[n] = '%'; out[n + 1] = hx[c >> 4]; out[n + 2] = hx[c & 15]; } n += 3; }
    }
    if (cap) out[n < cap ? n : cap - 1] = 0;
    return n;
}
static int hexv(int c) { return c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 'a' + 10 : c >= 'A' && c <= 'F' ? c - 'A' + 10 : -1; }
int http_percent_decode(const char *in, size_t n, char *out, size_t cap)
{
    size_t o = 0;
    for (size_t i = 0; i < n; i++) {
        unsigned char c = (unsigned char)in[i];
        if (c == '%') {
            if (i + 2 >= n) return -1;
            int a = hexv(in[i + 1]), b = hexv(in[i + 2]);
            if (a < 0 || b < 0) return -1;
            c = (unsigned char)(a * 16 + b); i += 2;
            if (c == 0) return -1;
        }
        if (o + 1 >= cap) return -1;
        out[o++] = (char)c;
    }
    if (cap) out[o] = 0;
    return (int)o;
}

bool http_path_normalize(const char *path, char *out, size_t cap)
{
    if (path[0] != '/') return false;
    char segs[64][256]; int ns = 0;
    const char *p = path + 1;
    bool trailing = false;
    while (*p) {
        const char *e = strchr(p, '/'); size_t l = e ? (size_t)(e - p) : strlen(p);
        char dec[256];
        int dl = http_percent_decode(p, l, dec, sizeof dec);
        if (dl < 0 || strchr(dec, '/') || strchr(dec, '\\')) return false;               /* an encoded slash would hide a traversal */
        if (!strcmp(dec, "..")) { if (!ns) return false; ns--; trailing = true; }
        else if (!strcmp(dec, ".") || dl == 0) trailing = true;
        else { if (ns >= 64) return false; strcpy(segs[ns++], dec); trailing = false; }
        p += l; if (*p == '/') { p++; if (!*p) trailing = true; }
    }
    size_t o = 0;
    for (int i = 0; i < ns; i++) { size_t l = strlen(segs[i]); if (o + 1 + l + 1 >= cap) return false; out[o++] = '/'; memcpy(out + o, segs[i], l); o += l; }
    if (!ns || trailing) { if (o + 2 > cap) return false; out[o++] = '/'; }
    if (ns && !trailing) { /* keep as is */ }
    out[o] = 0;
    return true;
}

bool http_query_get(const char *query, const char *key, char *out, size_t cap)
{
    size_t kl = strlen(key);
    const char *p = query;
    while (*p) {
        const char *amp = strchr(p, '&'); size_t pl = amp ? (size_t)(amp - p) : strlen(p);
        const char *eq = memchr(p, '=', pl);
        size_t nl = eq ? (size_t)(eq - p) : pl;
        char name[128]; int d = nl < sizeof name ? http_percent_decode(p, nl, name, sizeof name) : -1;
        if (d >= 0 && (size_t)d == kl && !memcmp(name, key, kl)) {
            char tmp[512]; size_t vl = eq ? pl - nl - 1 : 0;
            if (vl >= sizeof tmp) return false;
            for (size_t i = 0; i < vl; i++) tmp[i] = eq[1 + i] == '+' ? ' ' : eq[1 + i];
            tmp[vl] = 0;
            return http_percent_decode(tmp, vl, out, cap) >= 0;
        }
        p += pl; if (*p == '&') p++;
    }
    return false;
}

bool http_url_parse(const char *s, http_url *u)
{
    memset(u, 0, sizeof *u);
    if (!strncasecmp_(s, "https://", 8)) { u->https = true; s += 8; } else if (!strncasecmp_(s, "http://", 7)) s += 7; else return false;
    const char *auth = s, *end = auth + strcspn(auth, "/?#");
    if (end == auth) return false;
    for (const char *c = auth; c < end; c++) if (*c == '@' || (unsigned char)*c <= 0x20) return false;        /* userinfo is refused: it enables look-alike URLs */
    const char *hostend = end, *portp = NULL;
    if (*auth == '[') {
        const char *rb = memchr(auth, ']', (size_t)(end - auth));
        if (!rb || rb == auth + 1) return false;
        char lit[64]; size_t ll = (size_t)(rb - auth - 1); if (ll >= sizeof lit) return false;
        memcpy(lit, auth + 1, ll); lit[ll] = 0;
        ip6_addr a; if (!ip6_parse(lit, &a)) return false;
        strcpy(u->host, lit); u->ipv6_literal = true;
        hostend = rb + 1;
        if (hostend < end) { if (*hostend != ':') return false; portp = hostend + 1; }
    } else {
        const char *colon = memchr(auth, ':', (size_t)(end - auth));
        hostend = colon ? colon : end;
        if (colon) portp = colon + 1;
        size_t hl = (size_t)(hostend - auth);
        if (!hl || hl > 253) return false;
        for (size_t i = 0; i < hl; i++) {
            unsigned char c = (unsigned char)auth[i];
            bool ok = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '-' || c == '.' || c == '_';
            if (!ok) return false;
            u->host[i] = (char)lc(c);
        }
        if (u->host[0] == '.' || u->host[hl - 1] == '.') return false;
        for (size_t i = 0; i < hl; i++)                                          /* labels may not start or end with a hyphen or be empty */
            if ((u->host[i] == '-' && (i == 0 || u->host[i - 1] == '.' || i + 1 == hl || u->host[i + 1] == '.')) || (u->host[i] == '.' && i + 1 < hl && u->host[i + 1] == '.')) return false;
    }
    u->port = u->https ? 443 : 80;
    if (portp) {
        if (portp == end) return false;
        unsigned v = 0;
        for (const char *c = portp; c < end; c++) { if (*c < '0' || *c > '9') return false; v = v * 10 + (unsigned)(*c - '0'); if (v > 65535) return false; }
        if (v == 0) return false;
        u->port = (uint16_t)v;
    }
    const char *rest = end;
    size_t pl = strcspn(rest, "?#");
    if (pl >= sizeof u->path) return false;
    if (pl) memcpy(u->path, rest, pl); else strcpy(u->path, "/");
    rest += pl;
    if (*rest == '?') { rest++; size_t ql = strcspn(rest, "#"); if (ql >= sizeof u->query) return false; memcpy(u->query, rest, ql); }
    return true;
}

size_t http_url_format(const http_url *u, char *out, size_t cap)
{
    bool dflt = u->port == (u->https ? 443 : 80);
    int n = snprintf(out, cap, "%s://%s%s%s", u->https ? "https" : "http", u->ipv6_literal ? "[" : "", u->host, u->ipv6_literal ? "]" : "");
    if (n < 0) return 0;
    size_t o = (size_t)n;
    if (!dflt) { n = snprintf(out + o, o < cap ? cap - o : 0, ":%u", u->port); o += (size_t)n; }
    n = snprintf(out + o, o < cap ? cap - o : 0, "%s", u->path[0] ? u->path : "/"); o += (size_t)n;
    if (u->query[0]) { n = snprintf(out + o, o < cap ? cap - o : 0, "?%s", u->query); o += (size_t)n; }
    return o;
}

/* RFC 3986 5.2.4 on the raw (still percent-encoded) path */
static void remove_dots(const char *in, char *out, size_t cap)
{
    char tmp[HTTP_MAX_TARGET]; size_t o = 0;
    char buf[HTTP_MAX_TARGET]; strncpy(buf, in, sizeof buf - 1); buf[sizeof buf - 1] = 0;
    char *p = buf;
    while (*p) {
        if (!strncmp(p, "../", 3)) p += 3;
        else if (!strncmp(p, "./", 2)) p += 2;
        else if (!strncmp(p, "/./", 3)) p += 2;
        else if (!strcmp(p, "/.")) { p[1] = 0; }
        else if (!strncmp(p, "/../", 4) || !strcmp(p, "/..")) {
            if (!strcmp(p, "/..")) { p[1] = 0; } else p += 3;
            while (o > 0 && tmp[o - 1] != '/') o--;
            if (o > 0) o--;
        } else if (!strcmp(p, ".") || !strcmp(p, "..")) p += strlen(p);
        else {
            size_t l = strcspn(p + 1, "/") + 1;
            if (o + l >= sizeof tmp) break;
            memcpy(tmp + o, p, l); o += l; p += l;
        }
    }
    tmp[o] = 0;
    strncpy(out, tmp, cap - 1); out[cap - 1] = 0;
}

bool http_redirect_resolve(const http_url *base, const char *loc, http_url *out)
{
    if (!strncasecmp_(loc, "http://", 7) || !strncasecmp_(loc, "https://", 8)) return http_url_parse(loc, out);
    char abs[HTTP_MAX_TARGET + 300], origin[300];
    http_url o = *base; o.path[0] = 0; o.query[0] = 0; strcpy(o.path, "/");
    http_url_format(&o, origin, sizeof origin);
    size_t ol = strlen(origin); if (ol && origin[ol - 1] == '/') origin[ol - 1] = 0;                 /* origin without the trailing slash */
    if (loc[0] == '/' && loc[1] == '/') snprintf(abs, sizeof abs, "%s:%s", base->https ? "https" : "http", loc);
    else if (loc[0] == '/') snprintf(abs, sizeof abs, "%s%s", origin, loc);
    else if (loc[0] == '?') snprintf(abs, sizeof abs, "%s%s%s", origin, base->path, loc);
    else if (!loc[0]) { http_url_format(base, abs, sizeof abs); }
    else {
        char dir[HTTP_MAX_TARGET]; strncpy(dir, base->path, sizeof dir - 1); dir[sizeof dir - 1] = 0;
        char *slash = strrchr(dir, '/'); if (slash) slash[1] = 0; else strcpy(dir, "/");
        snprintf(abs, sizeof abs, "%s%s%s", origin, dir, loc);
    }
    if (!http_url_parse(abs, out)) return false;
    char np[HTTP_MAX_TARGET]; remove_dots(out->path, np, sizeof np);
    strcpy(out->path, np[0] ? np : "/");
    return true;
}

bool http_redirect_allowed(const http_redirect_policy *p, int followed, const http_url *from, const http_url *to, bool *strip)
{
    int max = p && p->max_redirects > 0 ? p->max_redirects : 5;
    if (followed >= max) return false;
    if (from->https && !to->https && !(p && p->allow_downgrade)) return false;
    if (strip) *strip = from->https != to->https || strcmp(from->host, to->host) || from->port != to->port;      /* never leak credentials cross-origin */
    return true;
}

/* ---- server ---- */
void http_server_init(http_server *s) { memset(s, 0, sizeof *s); s->max_body = 64 * 1024; }
bool http_server_add(http_server *s, unsigned methods, const char *pattern, http_handler fn, void *user)
{
    if (s->nroute >= 32 || strlen(pattern) >= 64 || pattern[0] != '/') return false;
    http_route *r = &s->route[s->nroute++];
    r->methods = methods; strcpy(r->pattern, pattern); r->fn = fn; r->user = user;
    return true;
}

static int error_reply(uint8_t *out, size_t cap, int status, const http_header *extra, int nextra, bool keep)
{
    char body[96]; int bl = snprintf(body, sizeof body, "%d %s\n", status, http_reason(status));
    http_header h[3]; int nh = 0;
    h[nh++] = (http_header){ "Content-Type", "text/plain; charset=utf-8", 12, 25 };
    for (int i = 0; i < nextra && nh < 3; i++) h[nh++] = extra[i];
    return http_write_response(out, cap, status, NULL, h, nh, body, (size_t)bl, keep);
}

static bool route_matches(const http_route *r, const char *path)
{
    size_t pl = strlen(r->pattern);
    if (pl >= 2 && r->pattern[pl - 1] == '*' && r->pattern[pl - 2] == '/') return !strncmp(path, r->pattern, pl - 1);
    return !strcmp(path, r->pattern);
}

int http_server_serve(const http_server *s, const uint8_t *in, size_t in_len, uint8_t *out, size_t cap, size_t *consumed, bool *close)
{
    static uint8_t body[64 * 1024];
    http_request rq;
    *consumed = 0; *close = false;
    http_result rc = http_parse_request(in, in_len, body, sizeof body < s->max_body ? sizeof body : s->max_body, &rq);
    if (rc == HTTP_NEED_MORE) return 0;
    if (rc != HTTP_OK) {
        int status = rc == HTTP_TOO_LARGE ? 413 : rc == HTTP_HEADERS_TOO_LARGE ? 431 : rc == HTTP_URI_TOO_LONG ? 414 : rc == HTTP_BAD_VERSION ? 505 : rc == HTTP_BAD_METHOD ? 501 : 400;
        *consumed = in_len; *close = true;                                                       /* a framing error poisons the connection */
        return error_reply(out, cap, status, NULL, 0, false);
    }
    *consumed = rq.consumed; *close = !rq.keep_alive;
    unsigned m = !strcmp(rq.method, "GET") ? HTTP_M_GET : !strcmp(rq.method, "HEAD") ? HTTP_M_HEAD : !strcmp(rq.method, "POST") ? HTTP_M_POST :
                 !strcmp(rq.method, "PUT") ? HTTP_M_PUT : !strcmp(rq.method, "DELETE") ? HTTP_M_DELETE : 0;
    bool options = !strcmp(rq.method, "OPTIONS");
    if (!m && !options) return error_reply(out, cap, 501, NULL, 0, rq.keep_alive);
    char path[HTTP_MAX_TARGET];
    const char *t = rq.target;
    if (!strncmp(t, "http://", 7) || !strncmp(t, "https://", 8)) { t = strchr(t + (t[4] == ':' ? 7 : 8), '/'); if (!t) t = "/"; }
    if (!strcmp(t, "*")) { http_header al = { "Allow", "GET, HEAD, POST, PUT, DELETE, OPTIONS", 5, 37 }; return http_write_response(out, cap, 204, NULL, &al, 1, NULL, 0, rq.keep_alive); }
    size_t plen = strcspn(t, "?#");
    char raw[HTTP_MAX_TARGET]; memcpy(raw, t, plen); raw[plen] = 0;
    if (!http_path_normalize(raw, path, sizeof path)) return error_reply(out, cap, 400, NULL, 0, rq.keep_alive);
    unsigned allowed = 0; const http_route *best = NULL;
    for (int i = 0; i < s->nroute; i++) {
        const http_route *r = &s->route[i];
        if (!route_matches(r, path)) continue;
        allowed |= r->methods;
        if ((r->methods & (m == HTTP_M_HEAD ? (HTTP_M_GET | HTTP_M_HEAD) : m)) && (!best || strlen(r->pattern) > strlen(best->pattern))) best = r;
    }
    if (options && allowed) {
        char al[96] = ""; if (allowed & (HTTP_M_GET | HTTP_M_HEAD)) strcat(al, "GET, HEAD"); if (allowed & HTTP_M_POST) strcat(al, *al ? ", POST" : "POST"); if (allowed & HTTP_M_PUT) strcat(al, *al ? ", PUT" : "PUT"); if (allowed & HTTP_M_DELETE) strcat(al, *al ? ", DELETE" : "DELETE"); strcat(al, ", OPTIONS");
        http_header ah = { "Allow", al, 5, strlen(al) };
        return http_write_response(out, cap, 204, NULL, &ah, 1, NULL, 0, rq.keep_alive);
    }
    if (!best) {
        if (!allowed) return error_reply(out, cap, 404, NULL, 0, rq.keep_alive);
        char al[96] = ""; if (allowed & (HTTP_M_GET | HTTP_M_HEAD)) strcat(al, "GET, HEAD"); if (allowed & HTTP_M_POST) strcat(al, *al ? ", POST" : "POST"); if (allowed & HTTP_M_PUT) strcat(al, *al ? ", PUT" : "PUT"); if (allowed & HTTP_M_DELETE) strcat(al, *al ? ", DELETE" : "DELETE");
        http_header ah = { "Allow", al, 5, strlen(al) };
        return error_reply(out, cap, 405, &ah, 1, rq.keep_alive);
    }
    http_reply rp; memset(&rp, 0, sizeof rp); rp.status = 200;
    best->fn(&rq, path, &rp, best->user);
    int n = http_write_response(out, cap, rp.status, rp.reason, rp.h, rp.nh, rp.body, rp.body_len, rq.keep_alive);
    if (n < 0) return error_reply(out, cap, 500, NULL, 0, rq.keep_alive);
    if (m == HTTP_M_HEAD) {                                                                      /* same headers and Content-Length, no body */
        const uint8_t *e = memmem_(out, (size_t)n, "\r\n\r\n", 4);
        if (e) n = (int)(e - out) + 4;
    }
    return n;
}
