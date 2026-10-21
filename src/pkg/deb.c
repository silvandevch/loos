#include "deb.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

static char *xstrndup(const char *s, size_t n) { char *p = malloc(n + 1); if (!p) abort(); memcpy(p, s, n); p[n] = 0; return p; }
static char *xstrdup(const char *s) { return xstrndup(s, strlen(s)); }
static int lc(int c) { return c >= 'A' && c <= 'Z' ? c + 32 : c; }
static bool ieq(const char *a, const char *b) { for (; *a && lc((unsigned char)*a) == lc((unsigned char)*b); a++, b++) { } return !*a && !*b; }

/* ---- version comparison: dpkg's verrevcmp ---- */
static int order(int c)
{
    if (c >= '0' && c <= '9') return 0;
    if ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')) return c;
    if (c == '~') return -1;
    if (c) return c + 256;
    return 0;
}
static int verrevcmp(const char *a, const char *b)
{
    while (*a || *b) {
        int first_diff = 0;
        while ((*a && !(*a >= '0' && *a <= '9')) || (*b && !(*b >= '0' && *b <= '9'))) {
            int ac = order((unsigned char)*a), bc = order((unsigned char)*b);
            if (ac != bc) return ac - bc;
            a++; b++;
        }
        while (*a == '0') a++;
        while (*b == '0') b++;
        while ((*a >= '0' && *a <= '9') && (*b >= '0' && *b <= '9')) { if (!first_diff) first_diff = *a - *b; a++; b++; }
        if (*a >= '0' && *a <= '9') return 1;
        if (*b >= '0' && *b <= '9') return -1;
        if (first_diff) return first_diff;
    }
    return 0;
}
static void split_version(const char *v, unsigned long *epoch, char *up, size_t upn, const char **rev)
{
    const char *colon = strchr(v, ':');
    *epoch = 0;
    if (colon) { *epoch = strtoul(v, NULL, 10); v = colon + 1; }
    const char *dash = strrchr(v, '-');
    size_t ul = dash ? (size_t)(dash - v) : strlen(v);
    if (ul >= upn) ul = upn - 1;
    memcpy(up, v, ul); up[ul] = 0;
    *rev = dash ? dash + 1 : "";
}
int deb_version_compare(const char *a, const char *b)
{
    unsigned long ea, eb; char ua[256], ub[256]; const char *ra, *rb;
    split_version(a, &ea, ua, sizeof ua, &ra); split_version(b, &eb, ub, sizeof ub, &rb);
    if (ea != eb) return ea < eb ? -1 : 1;
    int r = verrevcmp(ua, ub);
    if (r) return r < 0 ? -1 : 1;
    r = verrevcmp(ra, rb);
    return r < 0 ? -1 : r > 0;
}
bool deb_version_valid(const char *v)
{
    if (!*v) return false;
    const char *colon = strchr(v, ':');
    if (colon) { if (colon == v) return false; for (const char *c = v; c < colon; c++) if (*c < '0' || *c > '9') return false; v = colon + 1; }
    if (!*v || !((*v >= '0' && *v <= '9'))) return false;                    /* upstream must start with a digit */
    for (; *v; v++) { unsigned char c = (unsigned char)*v; if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || strchr(".+-~:", c))) return false; }
    return true;
}
bool deb_rel_satisfied(deb_rel rel, const char *want, const char *have)
{
    if (rel == DREL_NONE) return true;
    int c = deb_version_compare(have, want);
    switch (rel) { case DREL_LT: return c < 0; case DREL_LE: return c <= 0; case DREL_EQ: return c == 0; case DREL_GE: return c >= 0; case DREL_GT: return c > 0; default: return true; }
}

/* ---- stanzas ---- */
const char *deb_stanza_get(const deb_stanza *s, const char *key) { for (int i = 0; i < s->n; i++) if (ieq(s->f[i].key, key)) return s->f[i].value; return NULL; }
void deb_stanza_set(deb_stanza *s, const char *key, const char *value)
{
    for (int i = 0; i < s->n; i++) if (ieq(s->f[i].key, key)) { free(s->f[i].value); s->f[i].value = xstrdup(value); return; }
    if (s->n == s->cap) { s->cap = s->cap ? s->cap * 2 : 16; s->f = realloc(s->f, (size_t)s->cap * sizeof *s->f); if (!s->f) abort(); }
    s->f[s->n].key = xstrdup(key); s->f[s->n].value = xstrdup(value); s->n++;
}
void deb_stanza_free(deb_stanza *s) { for (int i = 0; i < s->n; i++) { free(s->f[i].key); free(s->f[i].value); } free(s->f); memset(s, 0, sizeof *s); }

int deb_stanza_next(const char *t, size_t len, size_t *pos, deb_stanza *out)
{
    memset(out, 0, sizeof *out);
    size_t p = *pos;
    while (p < len && (t[p] == '\n' || t[p] == '\r')) p++;                      /* blank lines between stanzas */
    if (p >= len) { *pos = p; return 0; }
    char *cur_val = NULL; size_t cur_len = 0, cur_cap = 0; char *cur_key = NULL;
    #define FLUSH() do { if (cur_key) { if (cur_len && cur_val[cur_len - 1] == '\n') cur_len--; if (!cur_val) cur_val = xstrdup(""); cur_val[cur_len] = 0; deb_stanza_set(out, cur_key, cur_val); free(cur_key); free(cur_val); cur_key = cur_val = NULL; cur_len = cur_cap = 0; } } while (0)
    while (p < len) {
        size_t e = p; while (e < len && t[e] != '\n') e++;
        size_t le = e; if (le > p && t[le - 1] == '\r') le--;
        if (le == p) { p = e < len ? e + 1 : e; break; }                         /* blank line ends the stanza */
        if (t[p] == '#') { p = e < len ? e + 1 : e; continue; }
        if (t[p] == ' ' || t[p] == '\t') {                                        /* continuation */
            if (!cur_key) { FLUSH(); deb_stanza_free(out); return -1; }
            const char *c = t + p + 1; size_t cl = le - p - 1;
            while (cl && (c[cl - 1] == ' ' || c[cl - 1] == '\t')) cl--;
            if (cur_len + cl + 2 > cur_cap) { cur_cap = (cur_len + cl + 2) * 2; cur_val = realloc(cur_val, cur_cap); if (!cur_val) abort(); }
            memcpy(cur_val + cur_len, c, cl); cur_len += cl; cur_val[cur_len++] = '\n';
        } else {
            FLUSH();
            const char *colon = memchr(t + p, ':', le - p);
            if (!colon || colon == t + p) { deb_stanza_free(out); return -1; }
            for (const char *c = t + p; c < colon; c++) if ((unsigned char)*c <= 0x20) { deb_stanza_free(out); return -1; }
            cur_key = xstrndup(t + p, (size_t)(colon - (t + p)));
            const char *v = colon + 1; while (v < t + le && (*v == ' ' || *v == '\t')) v++;
            size_t vl = (size_t)(t + le - v);
            while (vl && (v[vl - 1] == ' ' || v[vl - 1] == '\t')) vl--;
            cur_cap = vl + 64; cur_val = malloc(cur_cap); if (!cur_val) abort();
            memcpy(cur_val, v, vl); cur_len = vl; cur_val[cur_len++] = '\n';
        }
        p = e < len ? e + 1 : e;
    }
    FLUSH();
    #undef FLUSH
    *pos = p;
    return out->n ? 1 : 0;
}

size_t deb_stanza_format(const deb_stanza *s, char *out, size_t cap)
{
    size_t n = 0;
    #define PUT(ch) do { if (n + 1 < cap) out[n] = (ch); n++; } while (0)
    for (int i = 0; i < s->n; i++) {
        for (const char *k = s->f[i].key; *k; k++) PUT(*k);
        PUT(':'); PUT(' ');
        for (const char *v = s->f[i].value; *v; v++) { PUT(*v); if (*v == '\n') PUT(' '); }
        PUT('\n');
    }
    PUT('\n');
    #undef PUT
    if (cap) out[n < cap ? n : cap - 1] = 0;
    return n;
}

/* ---- dependency expressions ---- */
void deb_deps_free(deb_deps *d)
{
    for (int i = 0; i < d->n; i++) { for (int k = 0; k < d->grp[i].nalt; k++) { free(d->grp[i].alt[k].name); free(d->grp[i].alt[k].arch); free(d->grp[i].alt[k].version); } free(d->grp[i].alt); }
    free(d->grp); memset(d, 0, sizeof *d);
}
static void skip_ws(const char **p) { while (**p == ' ' || **p == '\t' || **p == '\n') (*p)++; }

bool deb_arch_ok(const char *pa, const char *native) { return !strcmp(pa, "all") || !strcmp(pa, native); }

static bool arch_list_allows(const char *list, size_t n, const char *native)
{
    /* "[amd64 !i386]": a positive list must contain native, a negative list must not */
    bool any_pos = false, hit = false, neg_hit = false;
    const char *p = list, *end = list + n;
    while (p < end) {
        while (p < end && *p == ' ') p++;
        const char *s = p; while (p < end && *p != ' ') p++;
        if (s == p) break;
        bool neg = *s == '!'; if (neg) s++;
        size_t l = (size_t)(p - s);
        bool match = (l == strlen(native) && !strncmp(s, native, l)) || (l == 3 && !strncmp(s, "any", 3)) || (l == 5 && !strncmp(s, "linux", 5));
        if (neg) { if (match) neg_hit = true; } else { any_pos = true; if (match) hit = true; }
    }
    return neg_hit ? false : any_pos ? hit : true;
}

bool deb_deps_parse(const char *text, const char *native, deb_deps *out)
{
    memset(out, 0, sizeof *out);
    const char *p = text;
    int cap = 0;
    skip_ws(&p);
    if (!*p) return true;
    for (;;) {
        deb_or grp = { NULL, 0 }; int acap = 0;
        for (;;) {
            skip_ws(&p);
            const char *s = p;
            while (*p && !strchr(" \t\n,|(:[<", *p)) p++;
            if (p == s) goto fail;
            deb_atom a; memset(&a, 0, sizeof a);
            a.name = xstrndup(s, (size_t)(p - s)); a.arch = xstrdup(""); a.rel = DREL_NONE;
            if (*p == ':') { p++; const char *as = p; while (*p && !strchr(" \t\n,|([<", *p)) p++; free(a.arch); a.arch = xstrndup(as, (size_t)(p - as)); }
            skip_ws(&p);
            if (*p == '(') {
                p++; skip_ws(&p);
                if (p[0] == '<' && p[1] == '<') { a.rel = DREL_LT; p += 2; } else if (p[0] == '<' && p[1] == '=') { a.rel = DREL_LE; p += 2; } else if (p[0] == '>' && p[1] == '>') { a.rel = DREL_GT; p += 2; }
                else if (p[0] == '>' && p[1] == '=') { a.rel = DREL_GE; p += 2; } else if (p[0] == '=') { a.rel = DREL_EQ; p++; } else if (p[0] == '<') { a.rel = DREL_LE; p++; } else if (p[0] == '>') { a.rel = DREL_GE; p++; }
                else { free(a.name); free(a.arch); goto fail; }
                skip_ws(&p);
                const char *vs = p; while (*p && *p != ')' && *p != ' ') p++;
                size_t vl = (size_t)(p - vs);
                skip_ws(&p);
                if (*p != ')' || !vl) { free(a.name); free(a.arch); goto fail; }
                p++;
                a.version = xstrndup(vs, vl);
            }
            skip_ws(&p);
            bool keep = true;
            if (*p == '[') { const char *ls = ++p; while (*p && *p != ']') p++; if (*p != ']') { free(a.name); free(a.arch); free(a.version); goto fail; } keep = arch_list_allows(ls, (size_t)(p - ls), native); p++; skip_ws(&p); }
            while (*p == '<') { while (*p && *p != '>') p++; if (*p == '>') p++; skip_ws(&p); }                  /* build profiles: ignored */
            if (keep) {
                if (grp.nalt == acap) { acap = acap ? acap * 2 : 2; grp.alt = realloc(grp.alt, (size_t)acap * sizeof *grp.alt); if (!grp.alt) abort(); }
                grp.alt[grp.nalt++] = a;
            } else { free(a.name); free(a.arch); free(a.version); }
            skip_ws(&p);
            if (*p == '|') { p++; continue; }
            break;
        }
        if (grp.nalt) {
            if (out->n == cap) { cap = cap ? cap * 2 : 8; out->grp = realloc(out->grp, (size_t)cap * sizeof *out->grp); if (!out->grp) abort(); }
            out->grp[out->n++] = grp;
        } else free(grp.alt);
        skip_ws(&p);
        if (*p == ',') { p++; skip_ws(&p); if (!*p) break; continue; }
        if (!*p) break;
        goto fail;
    }
    return true;
fail:
    deb_deps_free(out);
    return false;
}

size_t deb_deps_format(const deb_deps *d, char *out, size_t cap)
{
    static const char *const rel[] = { "", "<<", "<=", "=", ">=", ">>" };
    size_t n = 0;
    #define PUTS(s) do { for (const char *q_ = (s); *q_; q_++) { if (n + 1 < cap) out[n] = *q_; n++; } } while (0)
    for (int i = 0; i < d->n; i++) {
        if (i) PUTS(", ");
        for (int k = 0; k < d->grp[i].nalt; k++) {
            const deb_atom *a = &d->grp[i].alt[k];
            if (k) PUTS(" | ");
            PUTS(a->name);
            if (a->arch[0]) { PUTS(":"); PUTS(a->arch); }
            if (a->rel != DREL_NONE) { PUTS(" ("); PUTS(rel[a->rel]); PUTS(" "); PUTS(a->version); PUTS(")"); }
        }
    }
    #undef PUTS
    if (cap) out[n < cap ? n : cap - 1] = 0;
    return n;
}

/* ---- package records ---- */
void deb_pkg_free(deb_pkg *p)
{
    free(p->name); free(p->version); free(p->arch); free(p->source); free(p->section); free(p->priority); free(p->maintainer);
    free(p->filename); free(p->sha256); free(p->description); free(p->homepage);
    deb_deps_free(&p->depends); deb_deps_free(&p->pre_depends); deb_deps_free(&p->recommends); deb_deps_free(&p->suggests);
    deb_deps_free(&p->conflicts); deb_deps_free(&p->breaks); deb_deps_free(&p->replaces);
    for (int i = 0; i < p->nprovides; i++) { free(p->provides[i].name); free(p->provides[i].version); }
    free(p->provides);
    memset(p, 0, sizeof *p);
}
static char *opt(const deb_stanza *s, const char *k) { const char *v = deb_stanza_get(s, k); return v ? xstrdup(v) : NULL; }

bool deb_pkg_from_stanza(const deb_stanza *s, const char *native, deb_pkg *p)
{
    memset(p, 0, sizeof *p);
    const char *name = deb_stanza_get(s, "Package"), *ver = deb_stanza_get(s, "Version");
    if (!name || !*name || !ver || !deb_version_valid(ver)) return false;
    p->name = xstrdup(name); p->version = xstrdup(ver);
    const char *arch = deb_stanza_get(s, "Architecture");
    p->arch = xstrdup(arch && *arch ? arch : "all");
    p->source = opt(s, "Source"); p->section = opt(s, "Section"); p->priority = opt(s, "Priority"); p->maintainer = opt(s, "Maintainer");
    p->filename = opt(s, "Filename"); p->sha256 = opt(s, "SHA256"); p->homepage = opt(s, "Homepage");
    const char *d = deb_stanza_get(s, "Description");
    if (d) { const char *nl = strchr(d, '\n'); p->description = xstrndup(d, nl ? (size_t)(nl - d) : strlen(d)); }
    const char *v;
    if ((v = deb_stanza_get(s, "Size"))) p->size = strtoull(v, NULL, 10);
    if ((v = deb_stanza_get(s, "Installed-Size"))) p->installed_size = strtoull(v, NULL, 10) * 1024;
    p->essential = (v = deb_stanza_get(s, "Essential")) && ieq(v, "yes");
    p->multiarch_same = (v = deb_stanza_get(s, "Multi-Arch")) && ieq(v, "same");
    struct { const char *key; deb_deps *dst; } f[] = { { "Depends", &p->depends }, { "Pre-Depends", &p->pre_depends }, { "Recommends", &p->recommends }, { "Suggests", &p->suggests },
                                                       { "Conflicts", &p->conflicts }, { "Breaks", &p->breaks }, { "Replaces", &p->replaces } };
    for (unsigned i = 0; i < sizeof f / sizeof *f; i++) {
        const char *t = deb_stanza_get(s, f[i].key);
        if (t && !deb_deps_parse(t, native, f[i].dst)) { deb_pkg_free(p); return false; }
    }
    if ((v = deb_stanza_get(s, "Provides"))) {
        deb_deps pd;
        if (!deb_deps_parse(v, native, &pd)) { deb_pkg_free(p); return false; }
        for (int i = 0; i < pd.n; i++) for (int k = 0; k < pd.grp[i].nalt; k++) {
            p->provides = realloc(p->provides, (size_t)(p->nprovides + 1) * sizeof *p->provides);
            if (!p->provides) abort();
            p->provides[p->nprovides].name = xstrdup(pd.grp[i].alt[k].name);
            p->provides[p->nprovides].version = pd.grp[i].alt[k].version && pd.grp[i].alt[k].rel == DREL_EQ ? xstrdup(pd.grp[i].alt[k].version) : NULL;
            p->nprovides++;
        }
        deb_deps_free(&pd);
    }
    return true;
}
