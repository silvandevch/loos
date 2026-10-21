#include "solver.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

/* ---- repository ---- */
void repo_init(deb_repo *r, const char *native) { memset(r, 0, sizeof *r); strncpy(r->native, native, sizeof r->native - 1); }
void repo_free(deb_repo *r) { for (int i = 0; i < r->n; i++) deb_pkg_free(&r->a[i].p); free(r->a); memset(r, 0, sizeof *r); }
int repo_add_packages(deb_repo *r, const char *text, size_t len, int source, int priority)
{
    size_t pos = 0; int count = 0; deb_stanza s;
    for (;;) {
        int rc = deb_stanza_next(text, len, &pos, &s);
        if (rc < 0) return -1;
        if (rc == 0) return count;
        if (r->n == r->cap) { r->cap = r->cap ? r->cap * 2 : 256; r->a = realloc(r->a, (size_t)r->cap * sizeof *r->a); if (!r->a) abort(); }
        repo_pkg *rp = &r->a[r->n];
        if (deb_pkg_from_stanza(&s, r->native, &rp->p)) { rp->source = source; rp->priority = priority; r->n++; count++; }
        deb_stanza_free(&s);
    }
}
static bool better(const repo_pkg *a, const repo_pkg *b) { return a->priority != b->priority ? a->priority > b->priority : deb_version_compare(a->p.version, b->p.version) > 0; }
const repo_pkg *repo_best(const deb_repo *r, const char *name)
{
    const repo_pkg *best = NULL;
    for (int i = 0; i < r->n; i++) if (!strcmp(r->a[i].p.name, name) && (!best || better(&r->a[i], best))) best = &r->a[i];
    return best;
}

bool solver_parse_target(const char *spec, solver_target *t, char *buf, size_t cap)
{
    memset(t, 0, sizeof *t);
    size_t n = strcspn(spec, "= (");
    if (!n || n >= cap) return false;
    memcpy(buf, spec, n); buf[n] = 0; t->name = buf; t->rel = DREL_NONE;
    const char *p = spec + n;
    while (*p == ' ') p++;
    if (!*p) return true;
    static char vbuf[128];
    if (*p == '=') { t->rel = DREL_EQ; p++; }
    else if (*p == '(') {
        p++; while (*p == ' ') p++;
        if (p[0] == '>' && p[1] == '=') { t->rel = DREL_GE; p += 2; } else if (p[0] == '<' && p[1] == '=') { t->rel = DREL_LE; p += 2; }
        else if (p[0] == '>' && p[1] == '>') { t->rel = DREL_GT; p += 2; } else if (p[0] == '<' && p[1] == '<') { t->rel = DREL_LT; p += 2; } else if (p[0] == '=') { t->rel = DREL_EQ; p++; } else return false;
    } else return false;
    while (*p == ' ') p++;
    size_t vl = strcspn(p, " )");
    if (!vl || vl >= sizeof vbuf) return false;
    memcpy(vbuf, p, vl); vbuf[vl] = 0; t->version = vbuf;
    return deb_version_valid(vbuf);
}

/* ---- universe ---- */
typedef struct { const deb_pkg *p; bool installed; int prio; int slot; } uitem;
typedef struct { const char *name, *arch; int *items; int nitems, cap; int installed_item; int next_same_name; } uslot;
typedef struct { const char *k; int first; } smap_e;
typedef struct { smap_e *t; size_t cap, used; } smap;
typedef struct { int item; const char *version; int next; } uprov;

typedef struct {
    const deb_repo *repo; const dpkg_db *db; const solver_request *req; char native[16];
    uitem *it; int nit, capit;
    uslot *sl; int nsl, capsl;
    smap by_name, by_prov; uprov *prov; int nprov, capprov;
    int *chosen;                 /* per slot: item or -1 */
    int *orig;                   /* initial choice */
    uint8_t *locked;             /* 0 free, 1 locked to current, 2 locked absent */
    uint8_t *changes;
    uint8_t *requested;          /* slot named by an install request */
    struct { int slot, old; } *trail; int ntrail, captrail;
    struct cp { int trail_len; struct { int slot, item; } *opt; int nopt, idx; } *cp; int ncp, capcp;
    int steps, best_depth; char err[512];
    bool allow_remove;           /* effective policy for the current search (soft passes turn it off) */
    uint8_t skip_req[32];        /* requests whose upgrade could not be done: the installed version stays */
} ctx;

static size_t shash(const char *s) { size_t h = 1469598103934665603ull; for (; *s; s++) { h ^= (unsigned char)*s; h *= 1099511628211ull; } return h; }
static smap_e *sm_get(smap *m, const char *k, bool create)
{
    if (!m->cap) { m->cap = 1024; m->t = calloc(m->cap, sizeof *m->t); if (!m->t) abort(); }
    size_t i = shash(k) & (m->cap - 1);
    while (m->t[i].k) { if (!strcmp(m->t[i].k, k)) return &m->t[i]; i = (i + 1) & (m->cap - 1); }
    if (!create) return NULL;
    if ((m->used + 1) * 2 > m->cap) {                                                                         /* grow and rehash */
        smap_e *old = m->t; size_t oc = m->cap; m->cap *= 2; m->t = calloc(m->cap, sizeof *m->t); if (!m->t) abort();
        for (size_t j = 0; j < oc; j++) if (old[j].k) { size_t h = shash(old[j].k) & (m->cap - 1); while (m->t[h].k) h = (h + 1) & (m->cap - 1); m->t[h] = old[j]; }
        free(old);
        return sm_get(m, k, true);
    }
    m->t[i].k = k; m->t[i].first = -1; m->used++;
    return &m->t[i];
}

static int slot_for(ctx *c, const deb_pkg *p)
{
    smap_e *e = sm_get(&c->by_name, p->name, true);
    for (int s = e->first; s >= 0; s = c->sl[s].next_same_name) if (!strcmp(c->sl[s].arch, p->arch)) return s;
    if (c->nsl == c->capsl) { c->capsl = c->capsl ? c->capsl * 2 : 256; c->sl = realloc(c->sl, (size_t)c->capsl * sizeof *c->sl); if (!c->sl) abort(); }
    int s = c->nsl++;
    memset(&c->sl[s], 0, sizeof c->sl[s]);
    c->sl[s].name = p->name; c->sl[s].arch = p->arch; c->sl[s].installed_item = -1; c->sl[s].next_same_name = e->first;
    e->first = s;
    return s;
}
static int add_item(ctx *c, const deb_pkg *p, bool installed, int prio)
{
    if (c->nit == c->capit) { c->capit = c->capit ? c->capit * 2 : 512; c->it = realloc(c->it, (size_t)c->capit * sizeof *c->it); if (!c->it) abort(); }
    int id = c->nit++;
    int s = slot_for(c, p);
    c->it[id] = (uitem){ p, installed, prio, s };
    uslot *sl = &c->sl[s];
    if (sl->nitems == sl->cap) { sl->cap = sl->cap ? sl->cap * 2 : 4; sl->items = realloc(sl->items, (size_t)sl->cap * sizeof *sl->items); if (!sl->items) abort(); }
    sl->items[sl->nitems++] = id;
    if (installed) sl->installed_item = id;
    for (int i = 0; i < p->nprovides; i++) {
        if (c->nprov == c->capprov) { c->capprov = c->capprov ? c->capprov * 2 : 256; c->prov = realloc(c->prov, (size_t)c->capprov * sizeof *c->prov); if (!c->prov) abort(); }
        smap_e *e = sm_get(&c->by_prov, p->provides[i].name, true);
        c->prov[c->nprov] = (uprov){ id, p->provides[i].version, e->first };
        e->first = c->nprov++;
    }
    return id;
}

static void ctx_free(ctx *c)
{
    for (int i = 0; i < c->nsl; i++) free(c->sl[i].items);
    free(c->it); free(c->sl); free(c->by_name.t); free(c->by_prov.t); free(c->prov); free(c->chosen); free(c->orig); free(c->locked); free(c->changes); free(c->requested); free(c->trail);
    for (int i = 0; i < c->ncp; i++) free(c->cp[i].opt);
    free(c->cp);
}

/* ---- matching ---- */
static bool arch_visible(const ctx *c, const char *atom_arch, const char *item_arch, const char *depender_arch)
{
    if (!atom_arch[0]) return !strcmp(item_arch, c->native) || !strcmp(item_arch, "all") || (depender_arch && !strcmp(item_arch, depender_arch));
    if (!strcmp(atom_arch, "any")) return true;
    if (!strcmp(atom_arch, "native")) return !strcmp(item_arch, c->native) || !strcmp(item_arch, "all");
    return !strcmp(item_arch, atom_arch);
}
/* does item `id` satisfy atom (a package of that name at an acceptable version, or a provider of the name)? */
static bool item_matches(const ctx *c, int id, const deb_atom *a, const char *depender_arch, bool allow_virtual)
{
    const deb_pkg *p = c->it[id].p;
    if (!strcmp(p->name, a->name)) return arch_visible(c, a->arch, p->arch, depender_arch) && deb_rel_satisfied(a->rel, a->version ? a->version : "", p->version);
    if (!allow_virtual) return false;
    for (int i = 0; i < p->nprovides; i++)
        if (!strcmp(p->provides[i].name, a->name)) {
            if (a->rel == DREL_NONE) return arch_visible(c, a->arch, p->arch, depender_arch);
            if (p->provides[i].version && deb_rel_satisfied(a->rel, a->version, p->provides[i].version)) return arch_visible(c, a->arch, p->arch, depender_arch);
        }
    return false;
}
/* all items (any slot) that could satisfy the atom */
static int candidates(const ctx *c, const deb_atom *a, const char *darch, int *out, int max, bool virtual_too)
{
    int n = 0;
    smap_e *e = sm_get((smap *)&c->by_name, a->name, false);
    if (e) for (int s = e->first; s >= 0; s = c->sl[s].next_same_name) for (int k = 0; k < c->sl[s].nitems && n < max; k++) if (item_matches(c, c->sl[s].items[k], a, darch, false)) out[n++] = c->sl[s].items[k];
    if (virtual_too) {
        smap_e *pe = sm_get((smap *)&c->by_prov, a->name, false);
        if (pe) for (int pi = pe->first; pi >= 0 && n < max; pi = c->prov[pi].next) if (item_matches(c, c->prov[pi].item, a, darch, true)) { bool dup = false; for (int k = 0; k < n; k++) if (out[k] == c->prov[pi].item) dup = true; if (!dup) out[n++] = c->prov[pi].item; }
    }
    return n;
}
static bool satisfied_by_chosen(const ctx *c, const deb_atom *a, const char *darch, bool virtual_too)
{
    smap_e *e = sm_get((smap *)&c->by_name, a->name, false);
    if (e) for (int s = e->first; s >= 0; s = c->sl[s].next_same_name) if (c->chosen[s] >= 0 && item_matches(c, c->chosen[s], a, darch, false)) return true;
    if (virtual_too) {
        smap_e *pe = sm_get((smap *)&c->by_prov, a->name, false);
        if (pe) for (int pi = pe->first; pi >= 0; pi = c->prov[pi].next) { int it = c->prov[pi].item; if (c->chosen[c->it[it].slot] == it && item_matches(c, it, a, darch, true)) return true; }
    }
    return false;
}

/* ---- state changes with undo ---- */
static void apply(ctx *c, int slot, int item)
{
    if (c->ntrail == c->captrail) { c->captrail = c->captrail ? c->captrail * 2 : 256; c->trail = realloc(c->trail, (size_t)c->captrail * sizeof *c->trail); if (!c->trail) abort(); }
    c->trail[c->ntrail].slot = slot; c->trail[c->ntrail].old = c->chosen[slot]; c->ntrail++;
    c->chosen[slot] = item;
    if (c->changes[slot] < 255) c->changes[slot]++;
}
static void apply_skip(ctx *c, int req)
{
    if (c->ntrail == c->captrail) { c->captrail = c->captrail ? c->captrail * 2 : 256; c->trail = realloc(c->trail, (size_t)c->captrail * sizeof *c->trail); if (!c->trail) abort(); }
    c->trail[c->ntrail].slot = -1 - req; c->trail[c->ntrail].old = 0; c->ntrail++;
    c->skip_req[req] = 1;
}
static void undo_to(ctx *c, int len)
{
    while (c->ntrail > len) {
        c->ntrail--;
        int sl = c->trail[c->ntrail].slot;
        if (sl < 0) { c->skip_req[-1 - sl] = 0; continue; }
        c->chosen[sl] = c->trail[c->ntrail].old;
        if (c->changes[sl]) c->changes[sl]--;
    }
}

/* ---- violations ---- */
typedef enum { V_NONE, V_REQUEST, V_UPGRADE_REQ, V_REMOVE_REQ, V_DEP, V_CONFLICT } vkind;
typedef struct { vkind k; int slot, item, g; const deb_deps *list; int target; int other_item; const deb_atom *atom; bool is_breaks; } violation;

static int cmp_item_pref(const ctx *c, int a, int b)                     /* <0 if a should be tried before b */
{
    if (c->it[a].installed != c->it[b].installed) return c->it[a].installed ? -1 : 1;
    if (c->it[a].prio != c->it[b].prio) return c->it[a].prio > c->it[b].prio ? -1 : 1;
    int v = deb_version_compare(c->it[a].p->version, c->it[b].p->version);
    return v > 0 ? -1 : v < 0 ? 1 : 0;
}
static void sort_items(const ctx *c, int *v, int n) { for (int i = 1; i < n; i++) { int x = v[i], j = i; while (j > 0 && cmp_item_pref(c, v[j - 1], x) > 0) { v[j] = v[j - 1]; j--; } v[j] = x; } }

static bool conflicts_with(const ctx *c, int x, const deb_deps *list, bool breaks, int *y_out, const deb_atom **atom_out)
{
    const deb_pkg *px = c->it[x].p;
    for (int g = 0; g < list->n; g++) for (int a = 0; a < list->grp[g].nalt; a++) {
        const deb_atom *at = &list->grp[g].alt[a];
        smap_e *e = sm_get((smap *)&c->by_name, at->name, false);
        if (e) for (int s = e->first; s >= 0; s = c->sl[s].next_same_name) {
            int y = c->chosen[s];
            if (y < 0 || y == x) continue;
            if (item_matches(c, y, at, px->arch, false) && strcmp(c->it[y].p->name, px->name)) { *y_out = y; *atom_out = at; return true; }
            if (item_matches(c, y, at, px->arch, false) && !strcmp(c->it[y].p->name, px->name)) continue;     /* never conflicts with another arch of itself */
        }
        if (!breaks) {                                                                                   /* Conflicts also applies to virtual names */
            smap_e *pe = sm_get((smap *)&c->by_prov, at->name, false);
            if (pe) for (int pi = pe->first; pi >= 0; pi = c->prov[pi].next) {
                int y = c->prov[pi].item;
                if (y == x || c->chosen[c->it[y].slot] != y || !strcmp(c->it[y].p->name, px->name)) continue;
                if (item_matches(c, y, at, px->arch, true)) { *y_out = y; *atom_out = at; return true; }
            }
        }
    }
    return false;
}

static bool find_violation(ctx *c, violation *v)
{
    const solver_request *rq = c->req;
    for (int i = 0; i < rq->nremove; i++) {                                                          /* removals asked for */
        smap_e *e = sm_get(&c->by_name, rq->remove[i], false);
        if (e) for (int s = e->first; s >= 0; s = c->sl[s].next_same_name) if (c->chosen[s] >= 0) { *v = (violation){ V_REMOVE_REQ, s, c->chosen[s], 0, NULL, i, -1, NULL, false }; return true; }
    }
    for (int i = 0; i < rq->ninstall; i++) {                                                         /* installs asked for */
        deb_atom at = { (char *)rq->install[i].name, (char *)"any", rq->install[i].rel, (char *)rq->install[i].version };      /* any architecture satisfies a request once chosen */
        if (!satisfied_by_chosen(c, &at, NULL, true)) { *v = (violation){ V_REQUEST, -1, -1, 0, NULL, i, -1, NULL, false }; return true; }
        if (!c->skip_req[i] && !rq->install[i].version) {                                            /* installed but a newer candidate exists: "install" upgrades it */
            smap_e *e = sm_get(&c->by_name, rq->install[i].name, false);
            if (e) for (int s = e->first; s >= 0; s = c->sl[s].next_same_name) {
                int cur = c->chosen[s];
                if (cur < 0 || !c->it[cur].installed || c->locked[s]) continue;
                for (int k = 0; k < c->sl[s].nitems; k++) { int it = c->sl[s].items[k]; if (!c->it[it].installed && deb_version_compare(c->it[it].p->version, c->it[cur].p->version) > 0 && c->it[it].prio >= 500) { *v = (violation){ V_UPGRADE_REQ, s, cur, 0, NULL, i, -1, NULL, false }; return true; } }
            }
        }
    }
    for (int s = 0; s < c->nsl; s++) {
        int x = c->chosen[s];
        if (x < 0) continue;
        const deb_pkg *p = c->it[x].p;
        const deb_deps *lists[2] = { &p->pre_depends, &p->depends };
        for (int l = 0; l < 2; l++) {
            for (int g = 0; g < lists[l]->n; g++) {
                bool ok = false;
                for (int a = 0; a < lists[l]->grp[g].nalt && !ok; a++) ok = satisfied_by_chosen(c, &lists[l]->grp[g].alt[a], p->arch, true);
                if (!ok) { *v = (violation){ V_DEP, s, x, g, lists[l], -1, -1, NULL, false }; return true; }
            }
        }
        int y; const deb_atom *at;
        if (conflicts_with(c, x, &p->conflicts, false, &y, &at)) { *v = (violation){ V_CONFLICT, s, x, 0, NULL, -1, y, at, false }; return true; }
        if (conflicts_with(c, x, &p->breaks, true, &y, &at)) { *v = (violation){ V_CONFLICT, s, x, 0, NULL, -1, y, at, true }; return true; }
    }
    return false;
}

/* ---- options ---- */
typedef struct { int slot, item; } opt_t;
static bool slot_changeable(const ctx *c, int slot, int new_item)
{
    if (c->locked[slot] == 1) return c->chosen[slot] == new_item;                                    /* held: only "stay" */
    if (c->locked[slot] == 2) return new_item < 0;
    if (c->changes[slot] >= 2) return false;                                                          /* oscillation guard */
    return true;
}
static int add_opt(opt_t *o, int n, int max, int slot, int item) { if (n < max) { o[n].slot = slot; o[n].item = item; return n + 1; } return n; }
static bool removable(const ctx *c, int slot)
{
    int x = c->chosen[slot];
    if (x < 0 || c->locked[slot] == 1) return false;
    if (c->it[x].p->essential) return false;
    return c->allow_remove && c->it[x].installed && c->orig[slot] == x;
}

/* does keeping x and y chosen together violate a Conflicts/Breaks of either one? */
static bool pair_conflict(const ctx *c, int x, int y)
{
    const deb_pkg *px = c->it[x].p, *py = c->it[y].p;
    if (!strcmp(px->name, py->name)) return false;
    const struct { const deb_deps *d; bool breaks; } lx[2] = { { &px->conflicts, false }, { &px->breaks, true } };
    for (int dir = 0; dir < 2; dir++) {
        const deb_pkg *a = dir ? py : px, *b = dir ? px : py; int bi = dir ? x : y;
        const deb_deps *ls[2] = { &a->conflicts, &a->breaks };
        for (int l = 0; l < 2; l++) for (int g = 0; g < ls[l]->n; g++) for (int k = 0; k < ls[l]->grp[g].nalt; k++)
            if (item_matches(c, bi, &ls[l]->grp[g].alt[k], a->arch, l == 0)) return true;
        (void)b;
    }
    (void)lx;
    return false;
}

static int newer_first(const ctx *c, int a, int b)                       /* ordering for explicit requests: highest pin, then newest, installed last */
{
    if (c->it[a].prio != c->it[b].prio) return c->it[a].prio > c->it[b].prio ? -1 : 1;
    int v = deb_version_compare(c->it[a].p->version, c->it[b].p->version);
    return v > 0 ? -1 : v < 0 ? 1 : 0;
}
static void sort_newer(const ctx *c, int *v, int n) { for (int i = 1; i < n; i++) { int x = v[i], j = i; while (j > 0 && newer_first(c, v[j - 1], x) > 0) { v[j] = v[j - 1]; j--; } v[j] = x; } }

static int conflict_side_options(ctx *c, int slot, int chosen_item, int other_item, opt_t *out, int n, int max)
{
    /* ways to change `slot` so that it stops conflicting with other_item: another version, then (if allowed) removal */
    int cand[512], k = 0;
    for (int q = 0; q < c->sl[slot].nitems && k < 512; q++) cand[k++] = c->sl[slot].items[q];
    sort_items(c, cand, k);
    for (int i = 0; i < k; i++) {
        int it = cand[i];
        if (it == chosen_item || !slot_changeable(c, slot, it)) continue;
        if (c->it[it].installed && c->chosen[slot] != it) continue;
        if (pair_conflict(c, it, other_item)) continue;
        if (deb_version_compare(c->it[it].p->version, c->it[chosen_item].p->version) < 0 && !c->req->allow_downgrade) continue;
        n = add_opt(out, n, max, slot, it);
    }
    return n;
}

static int gen_options(ctx *c, const violation *v, opt_t *out, int max)
{
    int n = 0, cand[512];
    switch (v->k) {
    case V_REMOVE_REQ:
        if (c->it[v->item].p->essential) return 0;
        return add_opt(out, 0, max, v->slot, -1);
    case V_REQUEST: case V_UPGRADE_REQ: {
        const solver_target *t = &c->req->install[v->target];
        deb_atom at = { (char *)t->name, (char *)"", t->rel, (char *)t->version };
        int k = candidates(c, &at, NULL, cand, 512, true);
        if (v->k == V_REQUEST) {
            bool any_native = false;                                                                   /* only a foreign build exists: allow it for an explicit request */
            for (int i = 0; i < k; i++) if (!strcmp(c->it[cand[i]].p->arch, c->native) || !strcmp(c->it[cand[i]].p->arch, "all")) any_native = true;
            if (!any_native) { at.arch = (char *)"any"; k = candidates(c, &at, NULL, cand, 512, true); }
        }
        sort_newer(c, cand, k);
        for (int i = 0; i < k; i++) {
            int s = c->it[cand[i]].slot;
            if (!slot_changeable(c, s, cand[i])) continue;
            if (c->it[cand[i]].installed && c->chosen[s] != cand[i]) continue;
            if (c->chosen[s] == cand[i]) continue;
            const deb_pkg *np = c->it[cand[i]].p; const deb_pkg *cur = c->chosen[s] >= 0 ? c->it[c->chosen[s]].p : NULL;
            if (cur && deb_version_compare(np->version, cur->version) < 0 && !c->req->allow_downgrade) continue;
            if (v->k == V_UPGRADE_REQ && (s != v->slot || deb_version_compare(np->version, cur->version) <= 0)) continue;
            n = add_opt(out, n, max, s, cand[i]);
        }
        if (v->k == V_UPGRADE_REQ) n = add_opt(out, n, max, -1 - v->target, 0);                        /* last resort: leave the installed version alone */
        return n;
    }
    case V_DEP: {
        const deb_or *grp = &v->list->grp[v->g];
        const deb_pkg *dp = c->it[v->item].p;
        for (int a = 0; a < grp->nalt; a++) {
            int k = candidates(c, &grp->alt[a], dp->arch, cand, 512, true);
            sort_items(c, cand, k);
            for (int i = 0; i < k; i++) {
                int s = c->it[cand[i]].slot;
                if (c->chosen[s] == cand[i] || !slot_changeable(c, s, cand[i])) continue;
                if (c->chosen[s] >= 0 && !c->it[c->chosen[s]].installed && c->changes[s] >= 1) continue;     /* already picked a version for this slot in this path */
                const deb_pkg *np = c->it[cand[i]].p; const deb_pkg *cur = c->chosen[s] >= 0 ? c->it[c->chosen[s]].p : NULL;
                if (cur && deb_version_compare(np->version, cur->version) < 0 && !c->req->allow_downgrade) continue;
                if (c->it[cand[i]].installed && c->chosen[s] != cand[i]) continue;
                n = add_opt(out, n, max, s, cand[i]);
            }
        }
        if (removable(c, v->slot)) n = add_opt(out, n, max, v->slot, -1);                              /* last resort: drop the dependent itself */
        return n;
    }
    case V_CONFLICT: {
        int xs = v->slot, ys = c->it[v->other_item].slot;
        /* prefer to move the package that is already on the system, then the newcomer; versions before removals */
        bool y_installed = c->it[v->other_item].installed && c->orig[ys] == v->other_item;
        int first_slot = y_installed ? ys : xs, first_item = y_installed ? v->other_item : v->item;
        int second_slot = y_installed ? xs : ys, second_item = y_installed ? v->item : v->other_item;
        n = conflict_side_options(c, first_slot, first_item, second_item, out, n, max);
        n = conflict_side_options(c, second_slot, second_item, first_item, out, n, max);
        if (removable(c, first_slot)) n = add_opt(out, n, max, first_slot, -1);
        if (removable(c, second_slot)) n = add_opt(out, n, max, second_slot, -1);
        return n;
    }
    default: return 0;
    }
}

static void describe(const ctx *c, const violation *v, char *out, size_t cap)
{
    char tmp[256];
    switch (v->k) {
    case V_REMOVE_REQ: snprintf(out, cap, "cannot remove essential package %s", c->it[v->item].p->name); break;
    case V_UPGRADE_REQ: out[0] = 0; break;
    case V_REQUEST: {
        const solver_target *t = &c->req->install[v->target];
        smap_e *e = sm_get((smap *)&c->by_name, t->name, false);
        smap_e *pe = sm_get((smap *)&c->by_prov, t->name, false);
        if (!e && !pe) snprintf(out, cap, "unable to locate package %s", t->name);
        else if (t->version) snprintf(out, cap, "no available version of %s matches %s %s (or it is held/blocked by another package)", t->name, t->rel == DREL_EQ ? "=" : "constraint", t->version);
        else snprintf(out, cap, "package %s cannot be installed: it conflicts with something that has to stay", t->name);
        break;
    }
    case V_DEP: {
        const deb_or *g = &v->list->grp[v->g];
        deb_deps one = { (deb_or *)g, 1 };
        deb_deps_format(&one, tmp, sizeof tmp);
        snprintf(out, cap, "%s depends on something nothing installable satisfies", c->it[v->item].p->name);
        size_t l = strlen(out);
        snprintf(out + l, cap - l, ": %s", tmp);
        break;
    }
    case V_CONFLICT: snprintf(out, cap, "%s %s %s which cannot be removed or changed", c->it[v->item].p->name, v->is_breaks ? "breaks" : "conflicts with", c->it[v->other_item].p->name); break;
    default: out[0] = 0;
    }
}

/* ---- search ---- */
static void cp_free_top(ctx *c) { free(c->cp[c->ncp - 1].opt); c->ncp--; }

static bool backtrack(ctx *c)
{
    while (c->ncp > 0) {
        struct cp *t = &c->cp[c->ncp - 1];
        undo_to(c, t->trail_len);
        t->idx++;
        if (t->idx < t->nopt) { if (t->opt[t->idx].slot < 0) apply_skip(c, -1 - t->opt[t->idx].slot); else apply(c, t->opt[t->idx].slot, t->opt[t->idx].item); return true; }
        cp_free_top(c);
    }
    return false;
}

static bool search(ctx *c)
{
    violation v; opt_t opts[1536];
    for (;;) {
        if (++c->steps > 200000) { snprintf(c->err, sizeof c->err, "the problem is too complex to solve (search limit reached)"); return false; }
        if (!find_violation(c, &v)) return true;
        int n = gen_options(c, &v, opts, 1536);
        if (n == 0) {
            if (c->ncp >= c->best_depth) { c->best_depth = c->ncp; describe(c, &v, c->err, sizeof c->err); }
            if (!backtrack(c)) return false;
            continue;
        }
        if (c->ncp == c->capcp) { c->capcp = c->capcp ? c->capcp * 2 : 64; c->cp = realloc(c->cp, (size_t)c->capcp * sizeof *c->cp); if (!c->cp) abort(); }
        struct cp *t = &c->cp[c->ncp++];
        t->trail_len = c->ntrail; t->nopt = n; t->idx = 0;
        t->opt = malloc((size_t)n * sizeof *t->opt); if (!t->opt) abort();
        for (int i = 0; i < n; i++) { t->opt[i].slot = opts[i].slot; t->opt[i].item = opts[i].item; }
        if (t->opt[0].slot < 0) apply_skip(c, -1 - t->opt[0].slot); else apply(c, t->opt[0].slot, t->opt[0].item);
    }
}

/* ---- plan construction ---- */
static void act_list_add(solver_plan *p, solver_act a, const deb_pkg *pkg, const deb_pkg *old, bool automatic, const char *why)
{
    p->act = realloc(p->act, (size_t)(p->n + 1) * sizeof *p->act); if (!p->act) abort();
    solver_action *s = &p->act[p->n++];
    s->act = a; s->pkg = pkg; s->old = old; s->automatic = automatic;
    snprintf(s->why, sizeof s->why, "%s", why);
}
void solver_plan_free(solver_plan *p) { free(p->act); memset(p, 0, sizeof *p); }

static bool depends_on(const deb_pkg *a, const deb_pkg *b)
{
    const deb_deps *l[2] = { &a->pre_depends, &a->depends };
    for (int k = 0; k < 2; k++) for (int g = 0; g < l[k]->n; g++) for (int i = 0; i < l[k]->grp[g].nalt; i++) {
        const deb_atom *at = &l[k]->grp[g].alt[i];
        if (!strcmp(at->name, b->name)) return true;
        for (int p = 0; p < b->nprovides; p++) if (!strcmp(at->name, b->provides[p].name)) return true;
    }
    return false;
}

static void topo_order(solver_plan *p, int lo, int hi, bool reverse)
{
    int n = hi - lo; if (n < 2) return;
    solver_action *tmp = malloc((size_t)n * sizeof *tmp); uint8_t *state = calloc((size_t)n, 1);
    int cnt = 0;
    int *stack_i = malloc((size_t)n * sizeof(int)), *stack_k = malloc((size_t)n * sizeof(int));
    for (int root = 0; root < n; root++) {
        if (state[root]) continue;
        int sp = 0; stack_i[sp] = root; stack_k[sp] = 0; state[root] = 1; sp++;
        while (sp) {
            int i = stack_i[sp - 1], k = stack_k[sp - 1]++;
            if (k < n) {
                if (k == i || state[k]) continue;
                const deb_pkg *a = p->act[lo + i].pkg, *b = p->act[lo + k].pkg;
                bool edge = reverse ? depends_on(b, a) : depends_on(a, b);                              /* removals: dependents go first */
                if (edge) { state[k] = 1; stack_i[sp] = k; stack_k[sp] = 0; sp++; }
            } else { tmp[cnt++] = p->act[lo + i]; state[i] = 2; sp--; }
        }
    }
    memcpy(p->act + lo, tmp, (size_t)n * sizeof *tmp);
    free(tmp); free(state); free(stack_i); free(stack_k);
}

static bool build_plan(ctx *c, solver_plan *plan)
{
    const dpkg_db *db = c->db;
    (void)db;
    for (int s = 0; s < c->nsl; s++) {                                                               /* removals first */
        int ins = c->sl[s].installed_item;
        if (ins >= 0 && c->chosen[s] < 0) act_list_add(plan, c->req->purge ? ACT_PURGE : ACT_REMOVE, c->it[ins].p, NULL, false, "requested or required by the new selection");
    }
    int rem_end = plan->n;
    topo_order(plan, 0, rem_end, true);
    for (int s = 0; s < c->nsl; s++) {
        int ch = c->chosen[s], ins = c->sl[s].installed_item;
        if (ch < 0 || ch == ins) continue;
        bool requested = false;
        for (int i = 0; i < c->req->ninstall; i++) if (!strcmp(c->req->install[i].name, c->sl[s].name)) requested = true;
        const deb_pkg *np = c->it[ch].p;
        if (ins < 0) {
            act_list_add(plan, ACT_INSTALL, np, NULL, !requested, requested ? "requested" : "dependency");
            plan->download_size += np->size; plan->install_size_delta_kb += np->installed_size / 1024;
        } else {
            int cmp = deb_version_compare(np->version, c->it[ins].p->version);
            act_list_add(plan, cmp >= 0 ? ACT_UPGRADE : ACT_DOWNGRADE, np, c->it[ins].p, false, requested ? "requested" : "required by another package");
            plan->download_size += np->size;
            plan->install_size_delta_kb += (np->installed_size / 1024) > (c->it[ins].p->installed_size / 1024) ? (np->installed_size - c->it[ins].p->installed_size) / 1024 : 0;
        }
    }
    topo_order(plan, rem_end, plan->n, false);
    return true;
}

/* Try to move slot s to item `it` without removing anything; keeps the change only if the whole system stays consistent. */
static bool try_soft(ctx *c, int s, int it)
{
    int *snap = malloc((size_t)c->nsl * sizeof(int)); uint8_t *chsnap = malloc((size_t)c->nsl);
    memcpy(snap, c->chosen, (size_t)c->nsl * sizeof(int)); memcpy(chsnap, c->changes, (size_t)c->nsl);
    int tl = c->ntrail; bool prev = c->allow_remove, ok;
    c->allow_remove = false;
    apply(c, s, it);
    c->best_depth = -1; c->err[0] = 0;
    ok = search(c);
    c->allow_remove = prev;
    while (c->ncp) cp_free_top(c);
    if (!ok) { memcpy(c->chosen, snap, (size_t)c->nsl * sizeof(int)); memcpy(c->changes, chsnap, (size_t)c->nsl); c->ntrail = tl; }
    free(snap); free(chsnap);
    return ok;
}

static int best_item_of_slot_newer(const ctx *c, int s)
{
    int ins = c->sl[s].installed_item, best = -1;
    for (int k = 0; k < c->sl[s].nitems; k++) {
        int it = c->sl[s].items[k];
        if (!c->it[it].installed && (best < 0 || cmp_item_pref(c, it, best) < 0) && deb_version_compare(c->it[it].p->version, c->it[ins].p->version) > 0) best = it;
    }
    return best;
}

bool solver_resolve(const deb_repo *repo, const dpkg_db *db, const solver_request *req, solver_plan *plan)
{
    ctx c; memset(&c, 0, sizeof c);
    memset(plan, 0, sizeof *plan);
    c.repo = repo; c.db = db; c.req = req; c.allow_remove = req->allow_remove; strncpy(c.native, repo->native[0] ? repo->native : db->native, sizeof c.native - 1);
    for (int i = 0; i < db->n; i++) if (dpkg_entry_installed(&db->e[i])) add_item(&c, &db->e[i].pkg, true, 100);
    for (int i = 0; i < repo->n; i++) {
        const deb_pkg *p = &repo->a[i].p;
        bool dup = false;
        smap_e *e = sm_get(&c.by_name, p->name, false);
        if (e) for (int s = e->first; s >= 0 && !dup; s = c.sl[s].next_same_name)
            if (!strcmp(c.sl[s].arch, p->arch) && c.sl[s].installed_item >= 0 && !strcmp(c.it[c.sl[s].installed_item].p->version, p->version)) { dup = true; if (repo->a[i].priority > c.it[c.sl[s].installed_item].prio) c.it[c.sl[s].installed_item].prio = repo->a[i].priority; }
        if (!dup) add_item(&c, p, false, repo->a[i].priority);
    }
    int ns = c.nsl ? c.nsl : 1;
    c.chosen = malloc((size_t)ns * sizeof(int)); c.orig = malloc((size_t)ns * sizeof(int));
    c.locked = calloc((size_t)ns, 1); c.changes = calloc((size_t)ns, 1); c.requested = calloc((size_t)ns, 1);
    for (int s = 0; s < c.nsl; s++) c.chosen[s] = c.orig[s] = c.sl[s].installed_item;
    for (int i = 0; i < req->nhold; i++) { smap_e *e = sm_get(&c.by_name, req->hold[i], false); if (e) for (int s = e->first; s >= 0; s = c.sl[s].next_same_name) if (c.chosen[s] >= 0) c.locked[s] = 1; }
    for (int i = 0; i < db->n; i++) if (dpkg_entry_installed(&db->e[i]) && db->e[i].want == WANT_HOLD) { smap_e *e = sm_get(&c.by_name, db->e[i].pkg.name, false); if (e) for (int s = e->first; s >= 0; s = c.sl[s].next_same_name) if (c.chosen[s] >= 0) c.locked[s] = 1; }
    for (int i = 0; i < req->nremove; i++) {
        smap_e *e = sm_get(&c.by_name, req->remove[i], false);
        if (e) for (int s = e->first; s >= 0; s = c.sl[s].next_same_name) {
            if (c.locked[s] == 1) { snprintf(plan->error, sizeof plan->error, "%s is held and cannot be removed", req->remove[i]); ctx_free(&c); return false; }
            if (c.chosen[s] >= 0 && c.it[c.chosen[s]].p->essential) { snprintf(plan->error, sizeof plan->error, "refusing to remove essential package %s", req->remove[i]); ctx_free(&c); return false; }
            c.locked[s] = 2;
        }
    }
    c.best_depth = -1;
    bool ok = search(&c);
    while (c.ncp) cp_free_top(&c);
    if (!ok) {
        snprintf(plan->error, sizeof plan->error, "%s", c.err[0] ? c.err : "no consistent set of packages exists for this request");
        plan->steps = c.steps;
        ctx_free(&c);
        return false;
    }
    if (req->upgrade_all) {
        for (int s = 0; s < c.nsl; s++) {
            int ins = c.sl[s].installed_item;
            if (ins < 0 || c.chosen[s] != ins || c.locked[s]) continue;
            int best = best_item_of_slot_newer(&c, s);
            if (best < 0) continue;
            if (!try_soft(&c, s, best) && plan->nkept < 16) snprintf(plan->kept_back[plan->nkept++], 64, "%s", c.sl[s].name);
        }
    }
    if (req->install_recommends) {
        for (int round = 0; round < 8; round++) {
            bool changed = false;
            for (int s = 0; s < c.nsl; s++) {
                int x = c.chosen[s];
                if (x < 0 || c.it[x].installed) continue;
                const deb_deps *rec = &c.it[x].p->recommends;
                for (int g = 0; g < rec->n; g++) {
                    bool sat = false;
                    for (int a = 0; a < rec->grp[g].nalt && !sat; a++) sat = satisfied_by_chosen(&c, &rec->grp[g].alt[a], c.it[x].p->arch, true);
                    if (sat) continue;
                    for (int a = 0; a < rec->grp[g].nalt && !sat; a++) {
                        int cand[256], k = candidates(&c, &rec->grp[g].alt[a], c.it[x].p->arch, cand, 256, true);
                        sort_items(&c, cand, k);
                        for (int i = 0; i < k && !sat; i++) {
                            int cs = c.it[cand[i]].slot;
                            if (c.locked[cs] || c.chosen[cs] >= 0 || c.it[cand[i]].installed) continue;
                            if (try_soft(&c, cs, cand[i])) { sat = true; changed = true; }
                        }
                    }
                }
            }
            if (!changed) break;
        }
    }
    plan->steps = c.steps;
    build_plan(&c, plan);
    ctx_free(&c);
    return true;
}

/* ---- autoremove ---- */
int solver_autoremove(const dpkg_db *db, bool keep_recommends, char out[][64], int max)
{
    int n = db->n; if (!n) return 0;
    uint8_t *needed = calloc((size_t)n, 1); int *stack = malloc((size_t)n * sizeof(int)); int sp = 0;
    for (int i = 0; i < n; i++) if (dpkg_entry_installed(&db->e[i]) && (!db->e[i].auto_installed || db->e[i].pkg.essential || db->e[i].want == WANT_HOLD)) { needed[i] = 1; stack[sp++] = i; }
    while (sp) {
        const deb_pkg *p = &db->e[stack[--sp]].pkg;
        const deb_deps *lists[3] = { &p->pre_depends, &p->depends, keep_recommends ? &p->recommends : NULL };
        for (int l = 0; l < 3; l++) {
            if (!lists[l]) continue;
            for (int g = 0; g < lists[l]->n; g++) {
                bool done = false;                                                                   /* an installed alternative that is already needed satisfies the group */
                for (int a = 0; a < lists[l]->grp[g].nalt && !done; a++) {
                    const deb_atom *at = &lists[l]->grp[g].alt[a];
                    for (int j = 0; j < n && !done; j++) {
                        if (!dpkg_entry_installed(&db->e[j]) || !needed[j]) continue;
                        const deb_pkg *q = &db->e[j].pkg;
                        bool m = !strcmp(q->name, at->name) && deb_rel_satisfied(at->rel, at->version ? at->version : "", q->version);
                        for (int pv = 0; pv < q->nprovides && !m; pv++) m = !strcmp(q->provides[pv].name, at->name);
                        if (m) done = true;
                    }
                }
                if (done) continue;
                for (int a = 0; a < lists[l]->grp[g].nalt && !done; a++) {                            /* otherwise keep the first installed alternative */
                    const deb_atom *at = &lists[l]->grp[g].alt[a];
                    for (int j = 0; j < n && !done; j++) {
                        if (!dpkg_entry_installed(&db->e[j])) continue;
                        const deb_pkg *q = &db->e[j].pkg;
                        bool m = !strcmp(q->name, at->name) && deb_rel_satisfied(at->rel, at->version ? at->version : "", q->version);
                        for (int pv = 0; pv < q->nprovides && !m; pv++) m = !strcmp(q->provides[pv].name, at->name);
                        if (m) { if (!needed[j]) { needed[j] = 1; stack[sp++] = j; } done = true; }
                    }
                }
            }
        }
    }
    int cnt = 0;
    for (int i = 0; i < n; i++) if (dpkg_entry_installed(&db->e[i]) && !needed[i]) { if (cnt < max) snprintf(out[cnt], 64, "%s", db->e[i].pkg.name); cnt++; }
    free(needed); free(stack);
    return cnt;
}
