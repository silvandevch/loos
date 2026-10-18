#include "compat.h"
#include <string.h>

/* ---------- version table ---------- */
static const compat_version versions[COMPAT_ERA_COUNT] = {
    { COMPAT_ERA_WIN7,  "win7",  6,  1, 7601,  1, 0, 0x0100, 1, 2, "Service Pack 1" },
    { COMPAT_ERA_WIN8,  "win8",  6,  2, 9200,  0, 0, 0x0100, 1, 2, "" },
    { COMPAT_ERA_WIN81, "win8.1",6,  3, 9600,  0, 0, 0x0100, 1, 2, "" },
    { COMPAT_ERA_WIN10, "win10", 10, 0, 19045, 0, 0, 0x0100, 1, 2, "" },
    { COMPAT_ERA_WIN11, "win11", 10, 0, 22631, 0, 0, 0x0100, 1, 2, "" },
};

const compat_version *compat_version_of(compat_era e)
{
    return (unsigned)e < COMPAT_ERA_COUNT ? &versions[e] : &versions[COMPAT_NATIVE_ERA];
}

static char lc(char c) { return (c >= 'A' && c <= 'Z') ? (char)(c + 32) : c; }
static int ieq(const char *a, const char *b) { for (; *a && *b; a++, b++) if (lc(*a) != lc(*b)) return 0; return *a == *b; }

int compat_era_parse(const char *s, compat_era *out)
{
    char t[24]; size_t n = 0;
    if (!s) return 0;
    for (; *s && n + 1 < sizeof t; s++) if (*s != ' ' && *s != '.' && *s != '_' && *s != '-') t[n++] = lc(*s);
    t[n] = 0;
    if (!strcmp(t, "7") || !strcmp(t, "win7") || !strcmp(t, "windows7")) { *out = COMPAT_ERA_WIN7; return 1; }
    if (!strcmp(t, "8") || !strcmp(t, "win8") || !strcmp(t, "windows8")) { *out = COMPAT_ERA_WIN8; return 1; }
    if (!strcmp(t, "81") || !strcmp(t, "win81") || !strcmp(t, "windows81")) { *out = COMPAT_ERA_WIN81; return 1; }
    if (!strcmp(t, "10") || !strcmp(t, "win10") || !strcmp(t, "windows10")) { *out = COMPAT_ERA_WIN10; return 1; }
    if (!strcmp(t, "11") || !strcmp(t, "win11") || !strcmp(t, "windows11")) { *out = COMPAT_ERA_WIN11; return 1; }
    return 0;
}

/* ---------- names ---------- */
/* Key for an app: full paths (containing a separator) keep the path with '/' -> '\', lowercased;
 * bare names are lowercased. Returns 0 if empty/too long. */
static int app_key(const char *in, char *out, size_t cap)
{
    size_t n = 0;
    if (!in || !*in) return 0;
    for (; *in; in++) {
        if (n + 1 >= cap) return 0;
        out[n++] = *in == '/' ? '\\' : lc(*in);
    }
    out[n] = 0;
    return 1;
}
static const char *base_of(const char *k) { const char *b = k, *p; for (p = k; *p; p++) if (*p == '\\') b = p + 1; return b; }
static int has_dir(const char *k) { return strchr(k, '\\') != NULL; }

/* ---------- db ---------- */
void compat_db_init(compat_db *db)
{
    memset(db, 0, sizeof *db);
    db->default_era = COMPAT_NATIVE_ERA;
}

static int find_app(const compat_db *db, const char *key)
{
    unsigned i;
    for (i = 0; i < db->napps; i++) if (!strcmp(db->apps[i].name, key)) return (int)i;
    return -1;
}

compat_status compat_db_set_claim(compat_db *db, const char *app, compat_era era)
{
    char key[COMPAT_NAME_MAX]; int i;
    if ((unsigned)era >= COMPAT_ERA_COUNT) return COMPAT_ERR_ERA;
    if (!app_key(app, key, sizeof key)) return COMPAT_ERR_NAME;
    if ((i = find_app(db, key)) < 0) {
        if (db->napps >= COMPAT_MAX_APPS) return COMPAT_ERR_FULL;
        i = (int)db->napps++;
        strcpy(db->apps[i].name, key);
    }
    db->apps[i].era = era;
    return COMPAT_OK;
}

void compat_db_clear_claim(compat_db *db, const char *app)
{
    char key[COMPAT_NAME_MAX]; int i;
    if (!app_key(app, key, sizeof key) || (i = find_app(db, key)) < 0) return;
    db->apps[i] = db->apps[--db->napps];
}

/* path entries beat base-name entries */
const compat_app *compat_db_get_app(const compat_db *db, const char *app)
{
    char key[COMPAT_NAME_MAX]; int i;
    if (!app_key(app, key, sizeof key)) return NULL;
    if (has_dir(key) && (i = find_app(db, key)) >= 0) return &db->apps[i];
    if ((i = find_app(db, base_of(key))) >= 0) return &db->apps[i];
    return NULL;
}

static int sane_dll(const char *s) { return *s && strlen(s) < COMPAT_SYM_MAX && !strchr(s, '\\') && !strchr(s, '/'); }

compat_status compat_db_add_redirect(compat_db *db, const char *app, const char *from_dll, const char *from_sym,
                                     const char *to_dll, const char *to_sym)
{
    compat_redirect *r; char key[COMPAT_NAME_MAX]; unsigned i;
    if (!strcmp(app, "*")) strcpy(key, "*");
    else if (!app_key(app, key, sizeof key)) return COMPAT_ERR_NAME;
    if (!sane_dll(from_dll) || !sane_dll(to_dll) || strlen(from_sym) >= COMPAT_SYM_MAX || strlen(to_sym) >= COMPAT_SYM_MAX) return COMPAT_ERR_NAME;
    if ((from_sym[0] == 0) != (to_sym[0] == 0)) return COMPAT_ERR_SYNTAX;     /* both whole-DLL or both symbol */
    if (db->nredirects >= COMPAT_MAX_REDIRECTS) return COMPAT_ERR_FULL;
    r = &db->redirects[db->nredirects];
    strcpy(r->app, key);
    for (i = 0; from_dll[i]; i++) r->from_dll[i] = lc(from_dll[i]);
    r->from_dll[i] = 0;
    for (i = 0; to_dll[i]; i++) r->to_dll[i] = lc(to_dll[i]);
    r->to_dll[i] = 0;
    strcpy(r->from_sym, from_sym); strcpy(r->to_sym, to_sym);
    db->nredirects++;
    return COMPAT_OK;
}

/* ---- text store ---- */
/* tokenise a line: whitespace separated, double quotes group. Returns token count (max `max`). */
static int tokenise(char *line, char **tok, int max)
{
    int n = 0;
    char *p = line;
    while (*p) {
        while (*p == ' ' || *p == '\t' || *p == '\r') p++;
        if (!*p || *p == '#') break;
        if (n >= max) return -1;
        if (*p == '"') {
            tok[n++] = ++p;
            while (*p && *p != '"') p++;
            if (*p != '"') return -1;
            *p++ = 0;
        } else {
            tok[n++] = p;
            while (*p && *p != ' ' && *p != '\t' && *p != '\r') p++;
            if (*p) *p++ = 0;
        }
    }
    return n;
}

static void split_sym(char *tok, char **dll, char **sym)
{
    char *bang = strchr(tok, '!');
    *dll = tok;
    if (bang) { *bang = 0; *sym = bang + 1; } else *sym = tok + strlen(tok);
}

compat_status compat_db_parse(compat_db *db, const char *text, size_t len, unsigned *err_line)
{
    compat_db tmp = *db;                   /* all-or-nothing */
    size_t pos = 0;
    unsigned lineno = 0;

    while (pos < len) {
        char line[512], *tok[4];
        size_t n = 0;
        int nt;
        compat_status st = COMPAT_OK;
        compat_era era;

        lineno++;
        while (pos < len && text[pos] != '\n') { if (n + 1 >= sizeof line) { if (err_line) *err_line = lineno; return COMPAT_ERR_SYNTAX; } line[n++] = text[pos++]; }
        if (pos < len) pos++;
        line[n] = 0;
        nt = tokenise(line, tok, 4);
        if (nt < 0) { if (err_line) *err_line = lineno; return COMPAT_ERR_SYNTAX; }
        if (nt == 0) continue;

        if (!strcmp(tok[0], "default") && nt == 2) {
            if (!compat_era_parse(tok[1], &era)) st = COMPAT_ERR_ERA; else tmp.default_era = era;
        } else if (!strcmp(tok[0], "app") && nt == 3) {
            if (!compat_era_parse(tok[2], &era)) st = COMPAT_ERR_ERA; else st = compat_db_set_claim(&tmp, tok[1], era);
        } else if (!strcmp(tok[0], "redirect") && nt == 4) {
            char *fd, *fs, *td, *ts;
            split_sym(tok[2], &fd, &fs); split_sym(tok[3], &td, &ts);
            st = compat_db_add_redirect(&tmp, tok[1], fd, fs, td, ts);
        } else st = COMPAT_ERR_SYNTAX;
        if (st != COMPAT_OK) { if (err_line) *err_line = lineno; return st; }
    }
    *db = tmp;
    return COMPAT_OK;
}

typedef struct { char *out; size_t cap, n; } sink;
static void put(sink *s, const char *str) { size_t l = strlen(str); if (s->n + l < s->cap) memcpy(s->out + s->n, str, l); s->n += l; }
static void put_name(sink *s, const char *name)
{
    int q = strchr(name, ' ') != NULL || strchr(name, '\t') != NULL || strchr(name, '#') != NULL;
    if (q) put(s, "\"");
    put(s, name);
    if (q) put(s, "\"");
}

size_t compat_db_dump(const compat_db *db, char *out, size_t cap)
{
    sink s = { out, cap, 0 };
    unsigned i;
    put(&s, "# LoOS compatibility profiles (local store)\n");
    put(&s, "default "); put(&s, versions[db->default_era].name); put(&s, "\n");
    for (i = 0; i < db->napps; i++) {
        put(&s, "app "); put_name(&s, db->apps[i].name); put(&s, " "); put(&s, versions[db->apps[i].era].name); put(&s, "\n");
    }
    for (i = 0; i < db->nredirects; i++) {
        const compat_redirect *r = &db->redirects[i];
        put(&s, "redirect "); put_name(&s, r->app); put(&s, " ");
        put(&s, r->from_dll); if (r->from_sym[0]) { put(&s, "!"); put(&s, r->from_sym); }
        put(&s, " ");
        put(&s, r->to_dll); if (r->to_sym[0]) { put(&s, "!"); put(&s, r->to_sym); }
        put(&s, "\n");
    }
    if (s.cap) s.out[s.n < s.cap ? s.n : s.cap - 1] = 0;
    return s.n;
}

/* ---------- manifest ---------- */
static const struct { const char *guid; compat_era era; int rank; } os_guids[] = {
    { "e2011457-1546-43c5-a5fe-008deee3d3f0", COMPAT_ERA_WIN7,  0 },   /* Vista: no Vista era, floor at Win7 */
    { "35138b9a-5d96-4fbd-8e2d-a2440225f93a", COMPAT_ERA_WIN7,  1 },
    { "4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38", COMPAT_ERA_WIN8,  2 },
    { "1f676c76-80e1-4239-95bb-83d0f6d0da78", COMPAT_ERA_WIN81, 3 },
    { "8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a", COMPAT_NATIVE_ERA, 4 }, /* Windows 10 and 11 share this id */
};

int compat_manifest_era(const char *xml, size_t len, compat_era *out)
{
    size_t i = 0;
    int best = -1;
    compat_era best_era = COMPAT_ERA_WIN7;
    static const char tag[] = "supportedos";

    while (i + sizeof tag <= len) {
        size_t k;
        for (k = 0; k < sizeof tag - 1; k++) if (lc(xml[i + k]) != tag[k]) break;
        if (k == sizeof tag - 1) {
            size_t j = i + k;                              /* scan this element's attributes for Id="{guid}" */
            while (j < len && xml[j] != '>') {
                if ((xml[j] == 'I' || xml[j] == 'i') && (xml[j + 1] == 'd' || xml[j + 1] == 'D') && j + 2 < len) {
                    size_t q = j + 2;
                    while (q < len && (xml[q] == ' ' || xml[q] == '=')) q++;
                    if (q < len && (xml[q] == '"' || xml[q] == '\'')) {
                        char g[40]; size_t gn = 0, e = q + 1; unsigned x;
                        if (e < len && xml[e] == '{') e++;
                        while (e < len && xml[e] != '"' && xml[e] != '\'' && xml[e] != '}' && gn + 1 < sizeof g) g[gn++] = lc(xml[e++]);
                        g[gn] = 0;
                        for (x = 0; x < sizeof os_guids / sizeof os_guids[0]; x++)
                            if (!strcmp(g, os_guids[x].guid) && os_guids[x].rank > best) { best = os_guids[x].rank; best_era = os_guids[x].era; }
                    }
                }
                j++;
            }
            i = j;
        } else i++;
    }
    if (best < 0) return 0;
    *out = best_era;
    return 1;
}

/* ---------- resolution ---------- */
static void set_reason(compat_claim *c, const char *a, const char *b)
{
    size_t n = 0;
    while (*a && n + 1 < sizeof c->reason) c->reason[n++] = *a++;
    while (b && *b && n + 1 < sizeof c->reason) c->reason[n++] = *b++;
    c->reason[n] = 0;
}

void compat_resolve(const compat_db *db, const char *exe_path, const char *manifest, size_t mlen, compat_claim *out)
{
    const compat_app *a = exe_path ? compat_db_get_app(db, exe_path) : NULL;
    compat_era e;
    if (a) {
        out->era = a->era; out->source = COMPAT_SRC_PROFILE;
        set_reason(out, "user profile for ", a->name);
    } else if (manifest && compat_manifest_era(manifest, mlen, &e)) {
        out->era = e; out->source = COMPAT_SRC_MANIFEST;
        set_reason(out, "highest supportedOS in manifest: ", versions[e].name);
    } else {
        out->era = db->default_era; out->source = COMPAT_SRC_DEFAULT;
        set_reason(out, "no profile or manifest; default ", versions[db->default_era].name);
    }
}

/* ---------- import redirects ---------- */
int compat_redirect_import(const compat_db *db, const char *app, const char *dll, const char *sym,
                           char *to_dll, size_t to_dll_sz, char *to_sym, size_t to_sym_sz)
{
    char key[COMPAT_NAME_MAX];
    unsigned i;
    int have_key = app_key(app, key, sizeof key);

    for (i = 0; i < db->nredirects; i++) {
        const compat_redirect *r = &db->redirects[i];
        const char *src;
        if (strcmp(r->app, "*")) {
            if (!have_key) continue;
            if (has_dir(r->app) ? strcmp(r->app, key) : strcmp(r->app, base_of(key))) continue;
        }
        if (!ieq(r->from_dll, dll)) continue;
        if (r->from_sym[0]) { if (!sym || strcmp(r->from_sym, sym)) continue; src = r->to_sym; }
        else src = sym ? sym : "";
        if (strlen(r->to_dll) + 1 > to_dll_sz || strlen(src) + 1 > to_sym_sz) continue;
        strcpy(to_dll, r->to_dll);
        strcpy(to_sym, src);
        return 1;
    }
    return 0;
}
