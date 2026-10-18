#include "ldr.h"
#include "../apisets/apiset.h"
#include <string.h>

#define MAX_FORWARD_DEPTH 16

static char dummy_exiting;   /* non-NULL "reserved" value signalling process exit */

/* ---------- small helpers ---------- */
static void set_err(ldr_error *e, ldr_status s, const char *mod, const char *sym)
{
    if (!e) return;
    e->status = s;
    e->module[0] = e->symbol[0] = 0;
    if (mod) { strncpy(e->module, mod, LDR_NAME_MAX - 1); e->module[LDR_NAME_MAX - 1] = 0; }
    if (sym) { strncpy(e->symbol, sym, LDR_NAME_MAX - 1); e->symbol[LDR_NAME_MAX - 1] = 0; }
}

static void fmt_ordinal(unsigned v, char *out)
{
    char t[8]; int n = 0, i = 0;
    do { t[n++] = (char)('0' + v % 10); v /= 10; } while (v);
    out[i++] = '#';
    while (n) out[i++] = t[--n];
    out[i] = 0;
}

/* Reduce a user-supplied module name to its base name: strip directories,
 * lowercase, append ".dll" when there is no extension (a trailing '.' means
 * "no extension" and is dropped). Returns 0 if empty/too long. */
static int norm_name(const char *in, char out[LDR_NAME_MAX], int *had_path)
{
    const char *base = in, *p;
    size_t n = 0, i;
    int dot = 0;

    for (p = in; *p; p++) if (*p == '\\' || *p == '/') base = p + 1;
    if (had_path) *had_path = base != in;
    for (p = base; *p; p++) {
        if (n + 5 >= LDR_NAME_MAX) return 0;
        out[n++] = (*p >= 'A' && *p <= 'Z') ? (char)(*p + 32) : *p;
    }
    if (!n) return 0;
    for (i = 0; i < n; i++) if (out[i] == '.') dot = 1;
    if (out[n - 1] == '.') n--;
    else if (!dot) { memcpy(out + n, ".dll", 4); n += 4; }
    out[n] = 0;
    return n > 0;
}

/* normalised name -> host module name (API-set aware). */
static ldr_status to_host(const char *norm, char host[LDR_NAME_MAX])
{
    if (apiset_is_apiset(norm)) {
        if (apiset_resolve(norm, host, LDR_NAME_MAX) != APISET_OK) return LDR_ERR_DLL_NOT_FOUND;
        return LDR_OK;
    }
    strcpy(host, norm);
    return LDR_OK;
}

static int find_loaded(ldr_context *c, const char *host)
{
    int i;
    for (i = 0; i < LDR_MAX_MODULES; i++)
        if (c->mod[i].state != LDR_FREE && !strcmp(c->mod[i].name, host)) return i;
    return -1;
}

static ldr_status add_dep(ldr_context *c, int from, int to)
{
    ldr_module *m = &c->mod[from];
    unsigned i;
    for (i = 0; i < m->ndeps; i++) if (m->deps[i] == to) return LDR_OK;
    if (m->ndeps >= LDR_MAX_DEPS) return LDR_ERR_TOO_MANY_MODULES;
    m->deps[m->ndeps++] = (unsigned short)to;
    return LDR_OK;
}

static void wr32(uint8_t *p, uint32_t v) { p[0] = (uint8_t)v; p[1] = (uint8_t)(v >> 8); p[2] = (uint8_t)(v >> 16); p[3] = (uint8_t)(v >> 24); }

/* ---------- export lookup ---------- */
static ldr_status load_module(ldr_context *c, const char *name, int is_exe, int *idx, ldr_error *err);

static ldr_status get_proc_internal(ldr_context *c, int idx, const char *name, uint16_t ordinal,
                                    uint64_t *addr, ldr_error *err, int depth)
{
    ldr_module *m = &c->mod[idx];
    pe_export ex;
    pe_status ps = pe_find_export(&m->img, name, ordinal, &ex);
    char sym[LDR_NAME_MAX];

    if (name) { strncpy(sym, name, sizeof sym - 1); sym[sizeof sym - 1] = 0; }
    else fmt_ordinal(ordinal, sym);

    if (ps == PE_ERR_NOT_FOUND) {
        set_err(err, name ? LDR_ERR_PROC_NOT_FOUND : LDR_ERR_ORDINAL_NOT_FOUND, m->name, sym);
        return err ? err->status : LDR_ERR_PROC_NOT_FOUND;
    }
    if (ps) { set_err(err, LDR_ERR_BAD_IMAGE, m->name, NULL); if (err) err->pe = ps; return LDR_ERR_BAD_IMAGE; }

    if (ex.forwarder[0]) {
        char *dot = strrchr(ex.forwarder, '.');
        char dllpart[LDR_NAME_MAX];
        const char *fsym;
        uint16_t ford = 0;
        int ti;
        ldr_status st;

        if (depth >= MAX_FORWARD_DEPTH || !dot || dot == ex.forwarder || !dot[1] ||
            (size_t)(dot - ex.forwarder) >= sizeof dllpart) {
            set_err(err, LDR_ERR_PROC_NOT_FOUND, m->name, sym);
            return LDR_ERR_PROC_NOT_FOUND;
        }
        memcpy(dllpart, ex.forwarder, (size_t)(dot - ex.forwarder));
        dllpart[dot - ex.forwarder] = 0;
        fsym = dot + 1;
        if (*fsym == '#') {
            unsigned long v = 0; const char *q;
            for (q = fsym + 1; *q >= '0' && *q <= '9' && v <= 65535; q++) v = v * 10 + (unsigned long)(*q - '0');
            if (q == fsym + 1 || *q || v > 65535) { set_err(err, LDR_ERR_PROC_NOT_FOUND, m->name, sym); return LDR_ERR_PROC_NOT_FOUND; }
            ford = (uint16_t)v; fsym = NULL;
        }
        if ((st = load_module(c, dllpart, 0, &ti, err)) != LDR_OK) return st;
        if ((st = add_dep(c, idx, ti)) != LDR_OK) { set_err(err, st, dllpart, NULL); return st; }
        return get_proc_internal(c, ti, fsym, ford, addr, err, depth + 1);
    }
    *addr = m->addr + ex.rva;
    return LDR_OK;
}

/* ---------- mapping + binding ---------- */
struct bind_ctx { ldr_context *c; int idx; ldr_error *err; };

static int import_resolver(void *vctx, const pe_import_ref *ref, uint64_t *addr)
{
    struct bind_ctx *b = vctx;
    char rdll[LDR_NAME_MAX], rsym[LDR_NAME_MAX];
    const char *host = ref->host, *name = ref->name;
    uint16_t ordinal = ref->ordinal;
    int di;
    ldr_status st;

    if (b->c->be.redirect_import &&
        b->c->be.redirect_import(b->c->be.ctx, b->c->app, ref->host, ref->name, rdll, sizeof rdll, rsym, sizeof rsym)) {
        host = rdll;
        if (rsym[0]) { name = rsym; ordinal = 0; }
    }
    if ((st = load_module(b->c, host, 0, &di, b->err)) != LDR_OK) return 1;
    if ((st = add_dep(b->c, b->idx, di)) != LDR_OK) { set_err(b->err, st, host, NULL); return 1; }
    if ((st = get_proc_internal(b->c, di, name, ordinal, addr, b->err, 0)) != LDR_OK) return 1;
    return 0;
}

static void free_slot(ldr_context *c, int idx)
{
    ldr_module *m = &c->mod[idx];
    if (m->mem) c->be.free_image(c->be.ctx, m->mem, m->addr, m->size);
    memset(m, 0, sizeof *m);
}

static ldr_status load_module(ldr_context *c, const char *name, int is_exe, int *out_idx, ldr_error *err)
{
    char norm[LDR_NAME_MAX], host[LDR_NAME_MAX];
    int had_path = 0, idx, i;
    ldr_status st;
    ldr_module *m;
    char path_scratch[260];
    const void *data = NULL;
    size_t fsize = 0;
    void *cookie = NULL;
    pe_image file;
    pe_status ps;
    int fixed;
    void *mem = NULL;
    uint64_t addr = 0;

    if (!norm_name(name, norm, &had_path)) { set_err(err, LDR_ERR_INVALID_PARAMETER, name, NULL); return LDR_ERR_INVALID_PARAMETER; }
    if ((st = to_host(norm, host)) != LDR_OK) { set_err(err, st, norm, NULL); return st; }
    if ((idx = find_loaded(c, host)) >= 0) { *out_idx = idx; return LDR_OK; }

    for (idx = -1, i = 0; i < LDR_MAX_MODULES; i++) if (c->mod[i].state == LDR_FREE) { idx = i; break; }
    if (idx < 0) { set_err(err, LDR_ERR_TOO_MANY_MODULES, host, NULL); return LDR_ERR_TOO_MANY_MODULES; }

    if (!c->be.open_module(c->be.ctx, host, had_path ? name : NULL, &data, &fsize, &cookie)) {
        set_err(err, LDR_ERR_DLL_NOT_FOUND, host, NULL);
        return LDR_ERR_DLL_NOT_FOUND;
    }

    {   /* remember where it came from, before the file view goes away */
        const char *mp = c->be.module_path ? c->be.module_path(c->be.ctx, cookie) : NULL;
        size_t pl = mp ? strlen(mp) : 0;
        if (pl >= sizeof c->mod[0].path) pl = 0;
        memcpy(path_scratch, mp ? mp : "", pl); path_scratch[pl] = 0;
    }
    /* pe_open never writes in file layout, so dropping const here is safe. */
    ps = pe_open(&file, (void *)data, fsize, PE_LAYOUT_FILE);
    if (ps) { set_err(err, LDR_ERR_BAD_IMAGE, host, NULL); if (err) err->pe = ps; c->be.release_file(c->be.ctx, cookie); return LDR_ERR_BAD_IMAGE; }
    if (file.is64 != c->is64) { set_err(err, LDR_ERR_MACHINE_MISMATCH, host, NULL); c->be.release_file(c->be.ctx, cookie); return LDR_ERR_MACHINE_MISMATCH; }
    if (!!(file.file_characteristics & PE_FILE_DLL) == !!is_exe) { set_err(err, LDR_ERR_BAD_IMAGE, host, NULL); c->be.release_file(c->be.ctx, cookie); return LDR_ERR_BAD_IMAGE; }

    fixed = (file.file_characteristics & PE_FILE_RELOCS_STRIPPED) != 0;      /* no reloc table alone does not pin an image */
    if (!c->be.alloc_image(c->be.ctx, file.image_base, file.size_of_image, fixed, &mem, &addr)) {
        c->be.release_file(c->be.ctx, cookie);
        st = fixed ? LDR_ERR_CONFLICTING_ADDRESSES : LDR_ERR_NO_MEMORY;
        set_err(err, st, host, NULL);
        return st;
    }

    m = &c->mod[idx];
    memset(m, 0, sizeof *m);
    m->mem = mem; m->addr = addr; m->size = file.size_of_image;
    strcpy(m->name, host);
    strcpy(m->path, path_scratch);
    m->is_exe = is_exe;

    ps = pe_map(&file, mem, m->size, &m->img);
    c->be.release_file(c->be.ctx, cookie);
    if (ps) { set_err(err, LDR_ERR_BAD_IMAGE, host, NULL); if (err) err->pe = ps; free_slot(c, idx); return LDR_ERR_BAD_IMAGE; }
    if (addr != m->img.image_base) {
        ps = pe_relocate(&m->img, addr);
        if (ps) {
            st = ps == PE_ERR_NO_RELOCS ? LDR_ERR_CONFLICTING_ADDRESSES : LDR_ERR_BAD_IMAGE;
            set_err(err, st, host, NULL); if (err) err->pe = ps;
            free_slot(c, idx); return st;
        }
    }
    m->state = LDR_MAPPED;                     /* visible to import cycles from here on */

    {
        pe_tls t;
        ps = pe_get_tls(&m->img, &t);
        if (ps == PE_OK) {
            uint8_t *slot = pe_rva_ptr(&m->img, t.index_rva, 4);
            if (!slot) { set_err(err, LDR_ERR_BAD_IMAGE, host, NULL); free_slot(c, idx); return LDR_ERR_BAD_IMAGE; }
            m->has_tls = 1;
            m->tls_index = c->next_tls_index++;
            wr32(slot, m->tls_index);
        } else if (ps != PE_ERR_NOT_FOUND) {
            set_err(err, LDR_ERR_BAD_IMAGE, host, NULL); if (err) err->pe = ps;
            free_slot(c, idx); return LDR_ERR_BAD_IMAGE;
        }
    }

    {
        struct bind_ctx bc = { c, idx, err };
        pe_bind_error be;
        ps = pe_bind_imports(&m->img, import_resolver, &bc, &be);
        if (ps) {
            if (ps == PE_ERR_APISET) set_err(err, LDR_ERR_DLL_NOT_FOUND, be.dll, NULL);
            else if (ps != PE_ERR_UNRESOLVED) { set_err(err, LDR_ERR_BAD_IMAGE, host, NULL); if (err) err->pe = ps; }
            /* PE_ERR_UNRESOLVED: import_resolver already filled err */
            st = (err && err->status) ? err->status : LDR_ERR_BAD_IMAGE;
            free_slot(c, idx);
            return st;
        }
    }
    m->state = LDR_BOUND;
    *out_idx = idx;
    return LDR_OK;
}

/* ---------- entry points ---------- */
static void tls_callbacks(ldr_context *c, ldr_module *m, uint32_t reason, void *reserved)
{
    pe_tls t;
    unsigned i;
    if (!m->has_tls || pe_get_tls(&m->img, &t)) return;
    for (i = 0; i < t.ncallbacks; i++)
        c->be.call_entry(c->be.ctx, m->addr + t.callback_rva[i], m->addr, reason, reserved);
}

static int has_entry(const ldr_module *m) { return !m->is_exe && m->img.entry_rva; }

static ldr_status init_module(ldr_context *c, int idx, ldr_error *err)
{
    ldr_module *m = &c->mod[idx];
    unsigned i;

    if (m->state == LDR_READY || m->state == LDR_INITIALIZING) return LDR_OK;   /* cycles: DFS stops */
    m->state = LDR_INITIALIZING;
    for (i = 0; i < m->ndeps; i++) {
        ldr_status st = init_module(c, m->deps[i], err);
        if (st != LDR_OK) { m->state = LDR_BOUND; return st; }
    }
    tls_callbacks(c, m, LDR_REASON_PROCESS_ATTACH, NULL);
    if (has_entry(m) &&
        !c->be.call_entry(c->be.ctx, m->addr + m->img.entry_rva, m->addr, LDR_REASON_PROCESS_ATTACH, NULL)) {
        set_err(err, LDR_ERR_INIT_FAILED, m->name, NULL);
        m->state = LDR_BOUND;                  /* never attached, so never detached */
        return LDR_ERR_INIT_FAILED;
    }
    m->state = LDR_READY;
    c->init_order[c->ninit++] = (unsigned short)idx;
    return LDR_OK;
}

static ldr_status init_all_bound(ldr_context *c, ldr_error *err)
{
    int i;
    for (i = 0; i < LDR_MAX_MODULES; i++)
        if (c->mod[i].state == LDR_BOUND) {
            ldr_status st = init_module(c, i, err);
            if (st != LDR_OK) return st;
        }
    return LDR_OK;
}

/* ---------- liveness ---------- */
static void mark(ldr_context *c, int idx, unsigned char *live)
{
    ldr_module *m = &c->mod[idx];
    unsigned i;
    if (live[idx]) return;
    live[idx] = 1;
    for (i = 0; i < m->ndeps; i++) mark(c, m->deps[i], live);
}

static void sweep(ldr_context *c)
{
    unsigned char live[LDR_MAX_MODULES] = {0};
    int i, k;

    for (i = 0; i < LDR_MAX_MODULES; i++)
        if (c->mod[i].state != LDR_FREE && c->mod[i].explicit_refs) mark(c, i, live);

    /* detach garbage in reverse initialisation order */
    for (k = (int)c->ninit - 1; k >= 0; k--) {
        int idx = c->init_order[k];
        ldr_module *m = &c->mod[idx];
        unsigned j;
        if (live[idx]) continue;
        if (has_entry(m)) c->be.call_entry(c->be.ctx, m->addr + m->img.entry_rva, m->addr, LDR_REASON_PROCESS_DETACH, NULL);
        tls_callbacks(c, m, LDR_REASON_PROCESS_DETACH, NULL);
        for (j = (unsigned)k; j + 1 < c->ninit; j++) c->init_order[j] = c->init_order[j + 1];
        c->ninit--;
    }
    for (i = 0; i < LDR_MAX_MODULES; i++)
        if (c->mod[i].state != LDR_FREE && !live[i]) free_slot(c, i);
}

/* ---------- public API ---------- */
void ldr_init(ldr_context *c, const ldr_backend *be, int is64)
{
    memset(c, 0, sizeof *c);
    c->be = *be;
    c->is64 = is64;
}

static ldr_status load_top(ldr_context *c, const char *name, int is_exe, ldr_module **out, ldr_error *err)
{
    int idx;
    ldr_status st;
    ldr_error local;

    if (!err) err = &local;                    /* internals always report through err */
    set_err(err, LDR_OK, NULL, NULL);
    if (!name || !*name) { set_err(err, LDR_ERR_INVALID_PARAMETER, NULL, NULL); return LDR_ERR_INVALID_PARAMETER; }
    if (is_exe) {                              /* the redirect hook needs the app identity before imports bind */
        strncpy(c->app, name, sizeof c->app - 1);
        c->app[sizeof c->app - 1] = 0;
    }
    st = load_module(c, name, is_exe, &idx, err);
    if (st == LDR_OK && is_exe && c->be.exe_mapped) c->be.exe_mapped(c->be.ctx, &c->mod[idx]);
    if (st == LDR_OK) st = init_all_bound(c, err);
    if (st != LDR_OK) { sweep(c); return st; }
    c->mod[idx].explicit_refs++;
    if (out) *out = &c->mod[idx];
    return LDR_OK;
}

ldr_status ldr_load_exe(ldr_context *c, const char *name, ldr_module **out, ldr_error *err)
{ return load_top(c, name, 1, out, err); }

ldr_status ldr_load_dll(ldr_context *c, const char *name, ldr_module **out, ldr_error *err)
{ return load_top(c, name, 0, out, err); }

ldr_status ldr_unload_dll(ldr_context *c, ldr_module *m)
{
    if (!m || m < c->mod || m >= c->mod + LDR_MAX_MODULES || m->state != LDR_READY ||
        m->is_exe || !m->explicit_refs) return LDR_ERR_INVALID_PARAMETER;
    m->explicit_refs--;
    sweep(c);
    return LDR_OK;
}

ldr_module *ldr_get_module(ldr_context *c, const char *name)
{
    char norm[LDR_NAME_MAX], host[LDR_NAME_MAX];
    int idx;
    if (!name || !norm_name(name, norm, NULL) || to_host(norm, host) != LDR_OK) return NULL;
    idx = find_loaded(c, host);
    return idx < 0 ? NULL : &c->mod[idx];
}

ldr_status ldr_get_proc(ldr_context *c, ldr_module *m, const char *name, uint16_t ordinal,
                        uint64_t *addr, ldr_error *err)
{
    ldr_status st;
    ldr_error local;
    if (!err) err = &local;
    set_err(err, LDR_OK, NULL, NULL);
    if (!m || m < c->mod || m >= c->mod + LDR_MAX_MODULES || m->state != LDR_READY || !addr) {
        set_err(err, LDR_ERR_INVALID_PARAMETER, NULL, NULL);
        return LDR_ERR_INVALID_PARAMETER;
    }
    st = get_proc_internal(c, (int)(m - c->mod), name, ordinal, addr, err, 0);
    if (st == LDR_OK || st == LDR_ERR_PROC_NOT_FOUND || st == LDR_ERR_ORDINAL_NOT_FOUND) {
        /* forwarder targets may have been loaded on the way, even on a final miss */
        ldr_error ie;
        ldr_status st2 = init_all_bound(c, &ie);
        if (st2 != LDR_OK) { *err = ie; sweep(c); return st2; }
        return st;
    }
    sweep(c);
    return st;
}

void ldr_thread_notify(ldr_context *c, uint32_t reason)
{
    unsigned k;
    int attach = reason == LDR_REASON_THREAD_ATTACH;
    for (k = 0; k < c->ninit; k++) {
        ldr_module *m = &c->mod[c->init_order[attach ? k : c->ninit - 1 - k]];
        if (attach) tls_callbacks(c, m, reason, NULL);
        if (has_entry(m)) c->be.call_entry(c->be.ctx, m->addr + m->img.entry_rva, m->addr, reason, NULL);
        if (!attach) tls_callbacks(c, m, reason, NULL);
    }
}

void ldr_shutdown(ldr_context *c)
{
    int k;
    for (k = (int)c->ninit - 1; k >= 0; k--) {
        ldr_module *m = &c->mod[c->init_order[k]];
        if (has_entry(m)) c->be.call_entry(c->be.ctx, m->addr + m->img.entry_rva, m->addr, LDR_REASON_PROCESS_DETACH, &dummy_exiting);
        tls_callbacks(c, m, LDR_REASON_PROCESS_DETACH, &dummy_exiting);
    }
}

uint64_t ldr_entry_point(const ldr_module *m) { return m->addr + m->img.entry_rva; }

ldr_status ldr_get_manifest(const ldr_module *m, const char **xml, size_t *len)
{
    uint32_t rva, size;
    uint8_t *p;
    if (!m || m->state == LDR_FREE) return LDR_ERR_INVALID_PARAMETER;
    if (pe_find_resource(&m->img, PE_RT_MANIFEST, 1, &rva, &size) != PE_OK || !size || !(p = pe_rva_ptr(&m->img, rva, size)))
        return LDR_ERR_PROC_NOT_FOUND;                   /* no manifest */
    *xml = (const char *)p; *len = size;
    return LDR_OK;
}

uint32_t ldr_status_to_ntstatus(ldr_status s)
{
    switch (s) {
    case LDR_OK: return 0x00000000u;
    case LDR_ERR_DLL_NOT_FOUND: return 0xC0000135u;
    case LDR_ERR_BAD_IMAGE: return 0xC000007Bu;
    case LDR_ERR_MACHINE_MISMATCH: return 0xC000007Cu;
    case LDR_ERR_PROC_NOT_FOUND: return 0xC000007Au;
    case LDR_ERR_ORDINAL_NOT_FOUND: return 0xC0000138u;
    case LDR_ERR_INIT_FAILED: return 0xC0000142u;
    case LDR_ERR_CONFLICTING_ADDRESSES: return 0xC0000018u;
    case LDR_ERR_NO_MEMORY: return 0xC0000017u;
    case LDR_ERR_TOO_MANY_MODULES: return 0xC000009Au;
    case LDR_ERR_INVALID_PARAMETER: return 0xC000000Du;
    }
    return 0xC0000001u;
}
