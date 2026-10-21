#include "apt.h"
#include "../security/crypto.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

static int lc(int c) { return c >= 'A' && c <= 'Z' ? c + 32 : c; }
static bool ieq(const char *a, const char *b) { for (; *a && lc((unsigned char)*a) == lc((unsigned char)*b); a++, b++) { } return !*a && !*b; }
static void cp(char *dst, size_t cap, const char *s, size_t n) { if (n >= cap) n = cap - 1; memcpy(dst, s, n); dst[n] = 0; }

/* ---- sources ---- */
static void apt_source_defaults(apt_source *s) { memset(s, 0, sizeof *s); s->check_date = true; s->enabled = true; }

static bool parse_option(apt_source *s, const char *k, size_t kl, const char *v, size_t vl)
{
    char key[32], val[160];
    cp(key, sizeof key, k, kl); cp(val, sizeof val, v, vl);
    if (!strcmp(key, "arch")) { for (char *p = val; *p;) { char *c = strchr(p, ','); size_t l = c ? (size_t)(c - p) : strlen(p); if (s->narch < 4 && l) { cp(s->arch[s->narch++], 16, p, l); } p += l; if (*p == ',') p++; } return true; }
    if (!strcmp(key, "signed-by")) { strncpy(s->signed_by, val, sizeof s->signed_by - 1); return true; }
    if (!strcmp(key, "trusted")) { s->trusted_override = !strcmp(val, "yes"); return true; }
    if (!strcmp(key, "check-date")) { s->check_date = strcmp(val, "no") != 0 && strcmp(val, "false") != 0; return true; }
    if (!strcmp(key, "by-hash")) { s->by_hash = strcmp(val, "no") != 0; return true; }
    return true;                                                                                    /* unknown options are ignored like apt does */
}

int apt_parse_sources_list(const char *text, apt_source *out, int max)
{
    int n = 0;
    while (*text && n < max) {
        size_t ll = strcspn(text, "\n");
        char line[512]; cp(line, sizeof line, text, ll);
        text += ll; if (*text) text++;
        char *hash = strchr(line, '#'); if (hash) *hash = 0;
        char *tok[24]; int nt = 0;
        for (char *p = line; *p;) {
            while (*p == ' ' || *p == '\t' || *p == '\r') p++;
            if (!*p) break;
            if (nt >= 24) break;
            tok[nt++] = p;
            if (*p == '[') { while (*p && *p != ']') p++; if (*p) p++; } else while (*p && *p != ' ' && *p != '\t' && *p != '\r') p++;
            if (*p) *p++ = 0;
        }
        if (nt < 3) continue;
        apt_source s; apt_source_defaults(&s);
        if (!strcmp(tok[0], "deb")) s.is_src = false; else if (!strcmp(tok[0], "deb-src")) s.is_src = true; else continue;
        int i = 1;
        if (tok[i][0] == '[') {
            char *p = tok[i] + 1; char *end = strchr(p, ']'); if (end) *end = 0;
            while (*p) {
                while (*p == ' ') p++;
                char *eq = strchr(p, '='); char *sp = strchr(p, ' ');
                if (!eq || (sp && sp < eq)) break;
                size_t kl = (size_t)(eq - p); char *ve = sp ? sp : p + strlen(p);
                parse_option(&s, p, kl, eq + 1, (size_t)(ve - eq - 1));
                p = sp ? sp + 1 : ve;
            }
            i++;
        }
        if (i + 1 >= nt) continue;
        cp(s.uri, sizeof s.uri, tok[i], strlen(tok[i])); i++;
        size_t ul = strlen(s.uri); while (ul > 1 && s.uri[ul - 1] == '/') s.uri[--ul] = 0;
        cp(s.suite, sizeof s.suite, tok[i], strlen(tok[i])); i++;
        for (; i < nt && s.ncomp < 8; i++) cp(s.comp[s.ncomp++], 32, tok[i], strlen(tok[i]));
        if (s.ncomp == 0 && s.suite[strlen(s.suite) - 1] != '/') continue;                     /* a dists-style entry needs components */
        out[n++] = s;
    }
    return n;
}

int apt_parse_deb822(const char *text, apt_source *out, int max)
{
    int n = 0; size_t pos = 0, len = strlen(text); deb_stanza st;
    while (deb_stanza_next(text, len, &pos, &st) > 0) {
        const char *types = deb_stanza_get(&st, "Types"), *uris = deb_stanza_get(&st, "URIs"), *suites = deb_stanza_get(&st, "Suites"), *comps = deb_stanza_get(&st, "Components");
        if (types && uris && suites) {
            apt_source base; apt_source_defaults(&base);
            const char *v;
            if ((v = deb_stanza_get(&st, "Enabled"))) base.enabled = !ieq(v, "no");
            if ((v = deb_stanza_get(&st, "Signed-By"))) { char tmp[160]; size_t k = 0; for (; *v && k < sizeof tmp - 1; v++) if (*v != '\n') tmp[k++] = *v; tmp[k] = 0; strncpy(base.signed_by, tmp, sizeof base.signed_by - 1); }
            if ((v = deb_stanza_get(&st, "Trusted"))) base.trusted_override = ieq(v, "yes");
            if ((v = deb_stanza_get(&st, "Check-Date"))) base.check_date = !ieq(v, "no");
            if ((v = deb_stanza_get(&st, "By-Hash"))) base.by_hash = !ieq(v, "no");
            if ((v = deb_stanza_get(&st, "Architectures"))) for (const char *p = v; *p;) { while (*p == ' ' || *p == '\n') p++; size_t l = strcspn(p, " \n"); if (l && base.narch < 4) cp(base.arch[base.narch++], 16, p, l); p += l; }
            if (comps) for (const char *p = comps; *p;) { while (*p == ' ' || *p == '\n') p++; size_t l = strcspn(p, " \n"); if (l && base.ncomp < 8) cp(base.comp[base.ncomp++], 32, p, l); p += l; }
            /* the cross product of types x uris x suites */
            for (const char *t = types; *t;) {
                while (*t == ' ') t++; size_t tl = strcspn(t, " \n"); if (!tl) break;
                bool src = tl == 7 && !strncmp(t, "deb-src", 7);
                for (const char *u = uris; *u;) {
                    while (*u == ' ' || *u == '\n') u++; size_t ul = strcspn(u, " \n"); if (!ul) break;
                    for (const char *su = suites; *su;) {
                        while (*su == ' ' || *su == '\n') su++; size_t sl = strcspn(su, " \n"); if (!sl) break;
                        if (n < max) { out[n] = base; out[n].is_src = src; cp(out[n].uri, sizeof out[n].uri, u, ul); size_t q = strlen(out[n].uri); while (q > 1 && out[n].uri[q - 1] == '/') out[n].uri[--q] = 0; cp(out[n].suite, sizeof out[n].suite, su, sl); n++; }
                        su += sl;
                    }
                    u += ul;
                }
                t += tl;
            }
        }
        deb_stanza_free(&st);
    }
    return n;
}

static bool host_is_private(const char *uri)
{
    const char *p = strstr(uri, "://"); if (!p) return false;
    p += 3;
    if (!strncmp(p, "localhost", 9) || !strncmp(p, "127.", 4) || !strncmp(p, "10.", 3) || !strncmp(p, "192.168.", 8) || !strncmp(p, "[::1]", 5)) return true;
    if (!strncmp(p, "172.", 4)) { int o = atoi(p + 4); return o >= 16 && o <= 31; }
    size_t hl = strcspn(p, "/:"); return hl > 6 && !strncmp(p + hl - 6, ".local", 6);
}
bool apt_source_acceptable(const apt_source *s, const apt_policy *pol, char *why, size_t cap)
{
    #define NO(m) do { if (why) snprintf(why, cap, "%s", m); return false; } while (0)
    if (!s->enabled) NO("disabled");
    if (s->trusted_override && !(pol && pol->allow_unsigned)) NO("[trusted=yes] is not allowed: every repository must be signature-verified");
    if (!strncmp(s->uri, "https://", 8)) return true;
    if (!strncmp(s->uri, "http://", 7)) {
        if (pol && pol->allow_http) return true;
        if (pol && pol->allow_http_local && host_is_private(s->uri)) return true;
        NO("plain http is not allowed for this host: use https (or a mirror on your own network)");
    }
    if (!strncmp(s->uri, "file:/", 6) || !strncmp(s->uri, "/", 1)) return true;
    NO("unsupported URI scheme");
    #undef NO
}

size_t apt_release_url(const apt_source *s, const char *file, char *out, size_t cap)
{
    bool flat = s->suite[strlen(s->suite) - 1] == '/';
    int n = flat ? snprintf(out, cap, "%s/%s%s", s->uri, s->suite, file) : snprintf(out, cap, "%s/dists/%s/%s", s->uri, s->suite, file);
    return n < 0 ? 0 : (size_t)n;
}
size_t apt_index_url(const apt_source *s, const char *comp, const char *arch, const char *name, char *out, size_t cap)
{
    bool flat = s->suite[strlen(s->suite) - 1] == '/';
    int n = flat ? snprintf(out, cap, "%s/%s%s", s->uri, s->suite, name) : snprintf(out, cap, "%s/dists/%s/%s/binary-%s/%s", s->uri, s->suite, comp, arch, name);
    return n < 0 ? 0 : (size_t)n;
}
size_t apt_pool_url(const apt_source *s, const char *filename, char *out, size_t cap)
{
    if (strstr(filename, "..") || filename[0] == '/') { if (cap) out[0] = 0; return 0; }              /* the pool path is untrusted metadata */
    int n = snprintf(out, cap, "%s/%s", s->uri, filename);
    return n < 0 ? 0 : (size_t)n;
}

/* ---- Release ---- */
time_t apt_parse_date(const char *s)
{
    static const char *const mon[] = { "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec" };
    const char *c = strchr(s, ',');
    c = c ? c + 1 : s;
    int d, y, h, mi, se; char m[4];
    if (sscanf(c, " %d %3s %d %d:%d:%d", &d, m, &y, &h, &mi, &se) != 6) return (time_t)-1;
    int mo = -1; for (int i = 0; i < 12; i++) if (!strcmp(m, mon[i])) mo = i;
    if (mo < 0 || d < 1 || d > 31 || h > 23 || mi > 59 || se > 60) return (time_t)-1;
    const char *tz = strrchr(c, ' ');
    long off = 0;
    if (tz && (tz[1] == '+' || tz[1] == '-') && strlen(tz + 1) == 5) { off = (atol(tz + 2) / 100) * 3600 + (atol(tz + 2) % 100) * 60; if (tz[1] == '-') off = -off; }
    long long yy = y - (mo < 2), era = (yy >= 0 ? yy : yy - 399) / 400;
    unsigned yoe = (unsigned)(yy - era * 400), doy = (153u * (unsigned)(mo + (mo > 1 ? -2 : 10)) + 2) / 5 + (unsigned)d - 1, doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    long long days = era * 146097 + (long long)doe - 719468;
    return (time_t)(days * 86400 + h * 3600 + mi * 60 + se - off);
}

static void split_words(const char *v, char out[][32], int *n, int max) { *n = 0; for (const char *p = v; *p;) { while (*p == ' ') p++; size_t l = strcspn(p, " "); if (l && *n < max) { cp(out[(*n)++], 32, p, l); } p += l; } }

bool apt_release_parse(const char *text, size_t len, apt_release *r)
{
    memset(r, 0, sizeof *r);
    size_t pos = 0; deb_stanza st;
    if (deb_stanza_next(text, len, &pos, &st) <= 0) return false;
    const char *v;
    if ((v = deb_stanza_get(&st, "Origin"))) strncpy(r->origin, v, sizeof r->origin - 1);
    if ((v = deb_stanza_get(&st, "Label"))) strncpy(r->label, v, sizeof r->label - 1);
    if ((v = deb_stanza_get(&st, "Suite"))) strncpy(r->suite, v, sizeof r->suite - 1);
    if ((v = deb_stanza_get(&st, "Codename"))) strncpy(r->codename, v, sizeof r->codename - 1);
    if ((v = deb_stanza_get(&st, "Version"))) strncpy(r->version, v, sizeof r->version - 1);
    if ((v = deb_stanza_get(&st, "Date"))) { strncpy(r->date, v, sizeof r->date - 1); r->date_t = apt_parse_date(v); }
    if ((v = deb_stanza_get(&st, "Valid-Until"))) { strncpy(r->valid_until, v, sizeof r->valid_until - 1); r->valid_until_t = apt_parse_date(v); }
    r->by_hash = (v = deb_stanza_get(&st, "Acquire-By-Hash")) && ieq(v, "yes");
    char tmp[16][32]; int n;
    if ((v = deb_stanza_get(&st, "Architectures"))) { split_words(v, tmp, &n, 16); for (int i = 0; i < n; i++) { strncpy(r->archs[i], tmp[i], 15); r->narch++; } }
    if ((v = deb_stanza_get(&st, "Components"))) { split_words(v, r->comps, &r->ncomp, 16); }
    const char *h = deb_stanza_get(&st, "SHA256");
    int cap = 0;
    for (; h && *h;) {
        size_t ll = strcspn(h, "\n");
        char line[320], hash[80], path[192]; unsigned long long size;
        cp(line, sizeof line, h, ll);                                                              /* one entry per line: never let sscanf run into the next one */
        if (sscanf(line, " %79s %llu %191s", hash, &size, path) == 3 && strlen(hash) == 64) {
            if (r->nf == cap) { cap = cap ? cap * 2 : 128; r->f = realloc(r->f, (size_t)cap * sizeof *r->f); if (!r->f) abort(); }
            strcpy(r->f[r->nf].sha256, hash); strcpy(r->f[r->nf].path, path); r->f[r->nf].size = size; r->nf++;
        }
        h += ll; if (*h) h++;
    }
    deb_stanza_free(&st);
    return true;
}
void apt_release_free(apt_release *r) { free(r->f); memset(r, 0, sizeof *r); }
const apt_release_file *apt_release_find(const apt_release *r, const char *path) { for (int i = 0; i < r->nf; i++) if (!strcmp(r->f[i].path, path)) return &r->f[i]; return NULL; }

apt_release_status apt_release_check(const apt_release *r, const apt_source *s, time_t now, long skew)
{
    if (!r->nf) return APT_REL_NO_HASHES;
    if (s->suite[0] && s->suite[strlen(s->suite) - 1] != '/' && strcmp(s->suite, r->suite) && strcmp(s->suite, r->codename)) return APT_REL_WRONG_SUITE;     /* a valid release for another suite must not be accepted */
    if (s->check_date) {
        if (r->date_t != (time_t)-1 && r->date_t > now + skew) return APT_REL_FROM_FUTURE;
        if (r->valid_until[0] && (r->valid_until_t == (time_t)-1 || r->valid_until_t < now)) return APT_REL_EXPIRED;
    }
    return APT_REL_OK;
}

static void hex(const uint8_t *b, size_t n, char *out) { static const char d[] = "0123456789abcdef"; for (size_t i = 0; i < n; i++) { out[2 * i] = d[b[i] >> 4]; out[2 * i + 1] = d[b[i] & 15]; } out[2 * n] = 0; }
bool apt_verify_file(const apt_release_file *e, const void *data, size_t len)
{
    if (e->size != len) return false;
    uint8_t d[32]; char h[65];
    loos_sha256(data, len, d); hex(d, 32, h);
    char want[65]; for (int i = 0; i < 64; i++) want[i] = (char)lc((unsigned char)e->sha256[i]); want[64] = 0;
    return loos_constant_time_equal(h, want, 64);
}
bool apt_verify_deb_hash(const deb_pkg *p, const void *data, size_t len)
{
    if (!p->sha256 || strlen(p->sha256) != 64 || (p->size && p->size != len)) return false;       /* an unhashed pool file is never accepted */
    apt_release_file e; memset(&e, 0, sizeof e); strcpy(e.sha256, p->sha256); e.size = len;
    return apt_verify_file(&e, data, len);
}

size_t apt_best_index(const apt_release *r, const char *comp, const char *arch, char *name, size_t cap, char *path_out, size_t pcap)
{
    static const char *const ext[] = { ".xz", ".gz", "" };
    for (int i = 0; i < 3; i++) {
        char path[192]; snprintf(path, sizeof path, "%s/binary-%s/Packages%s", comp, arch, ext[i]);
        if (apt_release_find(r, path)) {
            if (name) snprintf(name, cap, "Packages%s", ext[i]);
            if (path_out) snprintf(path_out, pcap, "%s", path);
            return strlen(path);
        }
    }
    return 0;
}

/* ---- preferences / pinning ---- */
static bool glob(const char *p, const char *t)
{
    if (!*p) return !*t;
    if (*p == '*') { for (;; t++) { if (glob(p + 1, t)) return true; if (!*t) return false; } }
    return *t && (*p == '?' || *p == *t) && glob(p + 1, t + 1);
}
bool apt_prefs_parse(const char *text, size_t len, apt_prefs *out)
{
    memset(out, 0, sizeof *out);
    size_t pos = 0; deb_stanza st;
    for (;;) {
        int rc = deb_stanza_next(text, len, &pos, &st);
        if (rc < 0) return false;
        if (rc == 0) return true;
        const char *pk = deb_stanza_get(&st, "Package"), *pin = deb_stanza_get(&st, "Pin"), *pr = deb_stanza_get(&st, "Pin-Priority");
        if (pin && pr && out->n < 32) {
            apt_pin *p = &out->pin[out->n];
            memset(p, 0, sizeof *p);
            strncpy(p->pattern, pk ? pk : "*", sizeof p->pattern - 1);
            if (!strncmp(pin, "release ", 8)) strncpy(p->release, pin + 8, sizeof p->release - 1);
            else if (!strncmp(pin, "version ", 8)) strncpy(p->version, pin + 8, sizeof p->version - 1);
            else { deb_stanza_free(&st); continue; }
            p->priority = atoi(pr);
            out->n++;
        }
        deb_stanza_free(&st);
    }
}
static bool release_matches(const char *spec, const apt_release *r)
{
    /* comma separated "a=stable", "n=jammy", "o=Ubuntu", "l=Label", "v=22.04" (a = suite, n = codename) */
    if (!r) return false;
    for (const char *p = spec; *p;) {
        while (*p == ' ' || *p == ',') p++;
        if (!*p) break;
        char k = *p; if (p[1] != '=') return false;
        size_t l = strcspn(p + 2, ", "); char val[64]; cp(val, sizeof val, p + 2, l);
        const char *have = k == 'a' ? r->suite : k == 'n' ? r->codename : k == 'o' ? r->origin : k == 'l' ? r->label : k == 'v' ? r->version : NULL;
        if (!have || !glob(val, have)) return false;
        p += 2 + l;
    }
    return true;
}
int apt_prefs_priority(const apt_prefs *p, const char *pkg, const char *version, const apt_release *rel, bool installed)
{
    for (int i = 0; i < p->n; i++) {
        const apt_pin *pin = &p->pin[i];
        if (!glob(pin->pattern, pkg)) continue;
        if (pin->release[0] && release_matches(pin->release, rel)) return pin->priority;
        if (pin->version[0] && glob(pin->version, version)) return pin->priority;
    }
    return installed ? 100 : 500;
}
