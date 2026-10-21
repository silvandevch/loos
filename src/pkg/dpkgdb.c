#include "dpkgdb.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

static char *xstrdup(const char *s) { size_t n = strlen(s); char *p = malloc(n + 1); if (!p) abort(); memcpy(p, s, n + 1); return p; }

void dpkg_db_init(dpkg_db *db, const char *native) { memset(db, 0, sizeof *db); strncpy(db->native, native, sizeof db->native - 1); }
static void entry_free(dpkg_entry *e)
{
    deb_pkg_free(&e->pkg);
    for (int i = 0; i < e->nfiles; i++) free(e->files[i]);
    free(e->files); e->files = NULL; e->nfiles = 0;
}
void dpkg_db_free(dpkg_db *db)
{
    for (int i = 0; i < db->n; i++) entry_free(&db->e[i]);
    free(db->e);
    for (int i = 0; i < db->ndiv; i++) { free(db->diversion[i].path); free(db->diversion[i].from); free(db->diversion[i].to_pkg); }
    free(db->diversion);
    memset(db, 0, sizeof *db);
}

static bool arch_match(const char *have, const char *want) { return !want || !strcmp(have, want); }
dpkg_entry *dpkg_db_find(dpkg_db *db, const char *name, const char *arch)
{
    for (int i = 0; i < db->n; i++) if (!strcmp(db->e[i].pkg.name, name) && arch_match(db->e[i].pkg.arch, arch)) return &db->e[i];
    return NULL;
}
const dpkg_entry *dpkg_db_find_const(const dpkg_db *db, const char *name, const char *arch) { return dpkg_db_find((dpkg_db *)db, name, arch); }
bool dpkg_entry_installed(const dpkg_entry *e) { return e->status >= ST_UNPACKED; }

static dpkg_entry *slot_for(dpkg_db *db, const deb_pkg *pkg)
{
    dpkg_entry *e = dpkg_db_find(db, pkg->name, pkg->arch);
    if (e) { /* replaced below, keep the file list and auto mark */ return e; }
    if (db->n == db->cap) { db->cap = db->cap ? db->cap * 2 : 32; db->e = realloc(db->e, (size_t)db->cap * sizeof *db->e); if (!db->e) abort(); }
    e = &db->e[db->n++];
    memset(e, 0, sizeof *e);
    return e;
}

static bool dup_pkg(deb_pkg *dst, const deb_pkg *src)
{
    /* re-serialise through a stanza: keeps one code path for all fields */
    deb_stanza s = { 0 };
    char buf[16384];
    deb_deps_format(&src->depends, buf, sizeof buf);
    deb_stanza_set(&s, "Package", src->name); deb_stanza_set(&s, "Version", src->version); deb_stanza_set(&s, "Architecture", src->arch);
    if (src->source) deb_stanza_set(&s, "Source", src->source);
    if (src->section) deb_stanza_set(&s, "Section", src->section);
    if (src->priority) deb_stanza_set(&s, "Priority", src->priority);
    if (src->maintainer) deb_stanza_set(&s, "Maintainer", src->maintainer);
    if (src->description) deb_stanza_set(&s, "Description", src->description);
    if (src->filename) deb_stanza_set(&s, "Filename", src->filename);
    if (src->sha256) deb_stanza_set(&s, "SHA256", src->sha256);
    if (src->homepage) deb_stanza_set(&s, "Homepage", src->homepage);
    if (src->essential) deb_stanza_set(&s, "Essential", "yes");
    if (src->multiarch_same) deb_stanza_set(&s, "Multi-Arch", "same");
    char num[32];
    if (src->size) { snprintf(num, sizeof num, "%llu", (unsigned long long)src->size); deb_stanza_set(&s, "Size", num); }
    if (src->installed_size) { snprintf(num, sizeof num, "%llu", (unsigned long long)(src->installed_size / 1024)); deb_stanza_set(&s, "Installed-Size", num); }
    const struct { const char *key; const deb_deps *d; } f[] = { { "Depends", &src->depends }, { "Pre-Depends", &src->pre_depends }, { "Recommends", &src->recommends }, { "Suggests", &src->suggests },
                                                                  { "Conflicts", &src->conflicts }, { "Breaks", &src->breaks }, { "Replaces", &src->replaces } };
    for (unsigned i = 0; i < sizeof f / sizeof *f; i++) if (f[i].d->n) { deb_deps_format(f[i].d, buf, sizeof buf); deb_stanza_set(&s, f[i].key, buf); }
    if (src->nprovides) {
        size_t n = 0; buf[0] = 0;
        for (int i = 0; i < src->nprovides; i++) n += (size_t)snprintf(buf + n, sizeof buf - n, "%s%s%s%s%s", i ? ", " : "", src->provides[i].name, src->provides[i].version ? " (= " : "", src->provides[i].version ? src->provides[i].version : "", src->provides[i].version ? ")" : "");
    }
    if (src->nprovides) deb_stanza_set(&s, "Provides", buf);
    bool ok = deb_pkg_from_stanza(&s, "amd64", dst);                          /* arch lists were already resolved when src was parsed */
    deb_stanza_free(&s);
    return ok;
}

dpkg_entry *dpkg_db_put(dpkg_db *db, const deb_pkg *pkg, dpkg_status st)
{
    dpkg_entry *e = slot_for(db, pkg);
    bool fresh = e->pkg.name == NULL;
    deb_pkg copy;
    if (!dup_pkg(&copy, pkg)) return NULL;
    if (!fresh) deb_pkg_free(&e->pkg);
    e->pkg = copy; e->status = st;
    if (fresh) { e->want = WANT_INSTALL; e->flag = FLAG_OK; }
    return e;
}

bool dpkg_db_remove(dpkg_db *db, const char *name, const char *arch, bool purge)
{
    dpkg_entry *e = dpkg_db_find(db, name, arch);
    if (!e) return false;
    for (int i = 0; i < e->nfiles; i++) free(e->files[i]);
    free(e->files); e->files = NULL; e->nfiles = 0;                           /* the files are gone either way; conffiles stay on disk until purge */
    if (!purge) { e->status = ST_CONFIG_FILES; e->want = WANT_DEINSTALL; return true; }
    entry_free(e);
    *e = db->e[--db->n];
    return true;
}

/* ---- status file ---- */
static const char *const want_s[] = { "unknown", "install", "hold", "deinstall", "purge" };
static const char *const flag_s[] = { "ok", "reinstreq" };
static const char *const stat_s[] = { "not-installed", "config-files", "half-installed", "unpacked", "half-configured", "triggers-awaited", "triggers-pending", "installed" };

static bool parse_status(const char *v, dpkg_entry *e)
{
    char w[24], f[24], s[24];
    if (sscanf(v, "%23s %23s %23s", w, f, s) != 3) return false;
    int a = -1, b = -1, c = -1;
    for (int i = 0; i < 5; i++) if (!strcmp(w, want_s[i])) a = i;
    for (int i = 0; i < 2; i++) if (!strcmp(f, flag_s[i])) b = i;
    for (int i = 0; i < 8; i++) if (!strcmp(s, stat_s[i])) c = i;
    if (a < 0 || b < 0 || c < 0) return false;
    e->want = (dpkg_want)a; e->flag = (dpkg_flag)b; e->status = (dpkg_status)c;
    return true;
}

int dpkg_db_load_status(dpkg_db *db, const char *text, size_t len)
{
    size_t pos = 0; int n = 0; deb_stanza s;
    for (;;) {
        int r = deb_stanza_next(text, len, &pos, &s);
        if (r <= 0) return r < 0 ? -1 : n;
        const char *st = deb_stanza_get(&s, "Status");
        deb_pkg p; dpkg_entry tmp; memset(&tmp, 0, sizeof tmp);
        if (!st || !parse_status(st, &tmp) || !deb_pkg_from_stanza(&s, db->native, &p)) { deb_stanza_free(&s); return -1; }
        dpkg_entry *e = dpkg_db_put(db, &p, tmp.status);
        deb_pkg_free(&p);
        if (e) { e->want = tmp.want; e->flag = tmp.flag; n++; }
        deb_stanza_free(&s);
    }
}

size_t dpkg_db_save_status(const dpkg_db *db, char *out, size_t cap)
{
    size_t n = 0;
    for (int i = 0; i < db->n; i++) {
        const dpkg_entry *e = &db->e[i];
        deb_stanza s = { 0 };
        char buf[16384], tmp[64];
        deb_stanza_set(&s, "Package", e->pkg.name);
        snprintf(buf, sizeof buf, "%s %s %s", want_s[e->want], flag_s[e->flag], stat_s[e->status]);
        deb_stanza_set(&s, "Status", buf);
        if (e->pkg.priority) deb_stanza_set(&s, "Priority", e->pkg.priority);
        if (e->pkg.essential) deb_stanza_set(&s, "Essential", "yes");
        if (e->pkg.section) deb_stanza_set(&s, "Section", e->pkg.section);
        if (e->pkg.installed_size) { snprintf(tmp, sizeof tmp, "%llu", (unsigned long long)(e->pkg.installed_size / 1024)); deb_stanza_set(&s, "Installed-Size", tmp); }
        if (e->pkg.maintainer) deb_stanza_set(&s, "Maintainer", e->pkg.maintainer);
        deb_stanza_set(&s, "Architecture", e->pkg.arch);
        if (e->pkg.multiarch_same) deb_stanza_set(&s, "Multi-Arch", "same");
        if (e->pkg.source) deb_stanza_set(&s, "Source", e->pkg.source);
        deb_stanza_set(&s, "Version", e->pkg.version);
        if (e->pkg.nprovides) { size_t k = 0; buf[0] = 0; for (int j = 0; j < e->pkg.nprovides; j++) k += (size_t)snprintf(buf + k, sizeof buf - k, "%s%s%s%s%s", j ? ", " : "", e->pkg.provides[j].name, e->pkg.provides[j].version ? " (= " : "", e->pkg.provides[j].version ? e->pkg.provides[j].version : "", e->pkg.provides[j].version ? ")" : ""); deb_stanza_set(&s, "Provides", buf); }
        const struct { const char *key; const deb_deps *d; } f[] = { { "Pre-Depends", &e->pkg.pre_depends }, { "Depends", &e->pkg.depends }, { "Recommends", &e->pkg.recommends }, { "Suggests", &e->pkg.suggests },
                                                                      { "Conflicts", &e->pkg.conflicts }, { "Breaks", &e->pkg.breaks }, { "Replaces", &e->pkg.replaces } };
        for (unsigned k = 0; k < sizeof f / sizeof *f; k++) if (f[k].d->n) { deb_deps_format(f[k].d, buf, sizeof buf); deb_stanza_set(&s, f[k].key, buf); }
        if (e->pkg.description) deb_stanza_set(&s, "Description", e->pkg.description);
        if (e->pkg.homepage) deb_stanza_set(&s, "Homepage", e->pkg.homepage);
        size_t w = deb_stanza_format(&s, n < cap ? out + n : NULL, n < cap ? cap - n : 0);
        n += w;
        deb_stanza_free(&s);
    }
    if (cap) out[n < cap ? n : cap - 1] = 0;
    return n;
}

void dpkg_db_load_extended_states(dpkg_db *db, const char *text, size_t len)
{
    size_t pos = 0; deb_stanza s;
    while (deb_stanza_next(text, len, &pos, &s) > 0) {
        const char *name = deb_stanza_get(&s, "Package"), *arch = deb_stanza_get(&s, "Architecture"), *auto_ = deb_stanza_get(&s, "Auto-Installed");
        dpkg_entry *e = name ? dpkg_db_find(db, name, arch) : NULL;
        if (e && auto_) e->auto_installed = auto_[0] == '1';
        deb_stanza_free(&s);
    }
}
size_t dpkg_db_save_extended_states(const dpkg_db *db, char *out, size_t cap)
{
    size_t n = 0;
    for (int i = 0; i < db->n; i++) {
        if (!db->e[i].auto_installed) continue;
        int k = snprintf(out + n, n < cap ? cap - n : 0, "Package: %s\nArchitecture: %s\nAuto-Installed: 1\n\n", db->e[i].pkg.name, db->e[i].pkg.arch);
        if (k > 0) n += (size_t)k;
    }
    return n;
}

/* ---- file ownership ---- */
bool dpkg_db_set_files(dpkg_db *db, const char *name, const char *arch, const char *const *paths, int n)
{
    dpkg_entry *e = dpkg_db_find(db, name, arch);
    if (!e) return false;
    for (int i = 0; i < e->nfiles; i++) free(e->files[i]);
    free(e->files);
    e->files = n ? malloc((size_t)n * sizeof *e->files) : NULL;
    e->nfiles = n;
    for (int i = 0; i < n; i++) e->files[i] = xstrdup(paths[i]);
    return true;
}
int dpkg_db_load_list(dpkg_db *db, const char *name, const char *arch, const char *text)
{
    dpkg_entry *e = dpkg_db_find(db, name, arch);
    if (!e) return -1;
    char **paths = NULL; int n = 0, cap = 0;
    while (*text) {
        size_t l = strcspn(text, "\n");
        if (l && text[0] == '/') { if (n == cap) { cap = cap ? cap * 2 : 16; paths = realloc(paths, (size_t)cap * sizeof *paths); if (!paths) abort(); } paths[n] = malloc(l + 1); memcpy(paths[n], text, l); paths[n][l] = 0; n++; }
        text += l; if (*text) text++;
    }
    dpkg_db_set_files(db, name, arch, (const char *const *)paths, n);
    for (int i = 0; i < n; i++) free(paths[i]);
    free(paths);
    return n;
}
size_t dpkg_db_save_list(const dpkg_db *db, const char *name, const char *arch, char *out, size_t cap)
{
    const dpkg_entry *e = dpkg_db_find_const(db, name, arch);
    size_t n = 0;
    if (e) for (int i = 0; i < e->nfiles; i++) { int k = snprintf(out + n, n < cap ? cap - n : 0, "%s\n", e->files[i]); if (k > 0) n += (size_t)k; }
    if (cap) out[n < cap ? n : cap - 1] = 0;
    return n;
}
const dpkg_entry *dpkg_db_owner(const dpkg_db *db, const char *path)
{
    for (int i = 0; i < db->n; i++) for (int k = 0; k < db->e[i].nfiles; k++) if (!strcmp(db->e[i].files[k], path)) return &db->e[i];
    return NULL;
}
void dpkg_db_add_diversion(dpkg_db *db, const char *path, const char *to, const char *by)
{
    db->diversion = realloc(db->diversion, (size_t)(db->ndiv + 1) * sizeof *db->diversion);
    if (!db->diversion) abort();
    db->diversion[db->ndiv].path = xstrdup(path); db->diversion[db->ndiv].from = xstrdup(to); db->diversion[db->ndiv].to_pkg = xstrdup(by); db->ndiv++;
}

static bool replaces_pkg(const deb_pkg *newp, const dpkg_entry *old)
{
    for (int g = 0; g < newp->replaces.n; g++) for (int a = 0; a < newp->replaces.grp[g].nalt; a++) {
        const deb_atom *at = &newp->replaces.grp[g].alt[a];
        if (!strcmp(at->name, old->pkg.name) && deb_rel_satisfied(at->rel, at->version ? at->version : "", old->pkg.version)) return true;
    }
    return false;
}

int dpkg_db_check_files(const dpkg_db *db, const deb_pkg *pkg, const char *const *paths, int n, dpkg_file_conflict *out, int max)
{
    int c = 0;
    for (int i = 0; i < n; i++) {
        const dpkg_entry *o = dpkg_db_owner(db, paths[i]);
        if (!o) continue;
        if (!strcmp(o->pkg.name, pkg->name) && !strcmp(o->pkg.arch, pkg->arch)) continue;      /* upgrading ourselves */
        if (replaces_pkg(pkg, o)) continue;
        bool diverted = false;                                                                  /* a diversion moves the other package's copy aside */
        for (int d = 0; d < db->ndiv; d++) if (!strcmp(db->diversion[d].path, paths[i]) && !strcmp(db->diversion[d].to_pkg, pkg->name)) diverted = true;
        if (diverted) continue;
        if (pkg->multiarch_same && o->pkg.multiarch_same && !strcmp(o->pkg.name, pkg->name)) continue;   /* M-A: same packages may share identical files */
        if (c < max) { out[c].path = paths[i]; out[c].owner = o->pkg.name; }
        c++;
    }
    return c;
}

/* ---- audit ---- */
static bool provides_satisfy(const dpkg_db *db, const deb_atom *at)
{
    for (int i = 0; i < db->n; i++) {
        const dpkg_entry *e = &db->e[i];
        if (!dpkg_entry_installed(e)) continue;
        if (!strcmp(e->pkg.name, at->name) && deb_rel_satisfied(at->rel, at->version ? at->version : "", e->pkg.version)) return true;
        for (int p = 0; p < e->pkg.nprovides; p++)
            if (!strcmp(e->pkg.provides[p].name, at->name) && (at->rel == DREL_NONE || (e->pkg.provides[p].version && deb_rel_satisfied(at->rel, at->version, e->pkg.provides[p].version)))) return true;
    }
    return false;
}
int dpkg_db_audit(const dpkg_db *db, char out[][160], int max)
{
    int c = 0;
    for (int i = 0; i < db->n; i++) {
        const dpkg_entry *e = &db->e[i];
        if (!dpkg_entry_installed(e)) continue;
        if (e->flag == FLAG_REINSTREQ && c < max) snprintf(out[c++], 160, "%s: reinstallation required", e->pkg.name);
        const deb_deps *ds[2] = { &e->pkg.depends, &e->pkg.pre_depends };
        for (int k = 0; k < 2; k++) for (int g = 0; g < ds[k]->n; g++) {
            bool ok = false;
            for (int a = 0; a < ds[k]->grp[g].nalt && !ok; a++) ok = provides_satisfy(db, &ds[k]->grp[g].alt[a]);
            if (!ok && c < max) snprintf(out[c++], 160, "%s: unmet dependency %s", e->pkg.name, ds[k]->grp[g].alt[0].name);
        }
        const deb_deps *cs[2] = { &e->pkg.conflicts, &e->pkg.breaks };
        for (int k = 0; k < 2; k++) for (int g = 0; g < cs[k]->n; g++) for (int a = 0; a < cs[k]->grp[g].nalt; a++) {
            const deb_atom *at = &cs[k]->grp[g].alt[a];
            for (int j = 0; j < db->n; j++) {
                const dpkg_entry *o = &db->e[j];
                if (j == i || !dpkg_entry_installed(o) || strcmp(o->pkg.name, at->name)) continue;
                if (deb_rel_satisfied(at->rel, at->version ? at->version : "", o->pkg.version) && c < max) snprintf(out[c++], 160, "%s: %s %s", e->pkg.name, k ? "breaks" : "conflicts with", o->pkg.name);
            }
        }
    }
    return c;
}
