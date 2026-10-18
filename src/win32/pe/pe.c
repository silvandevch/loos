#include "pe.h"
#include "../apisets/apiset.h"
#include <string.h>

#define MAX_SECTIONS   96
#define MAX_IMPORT_DLL 4096
#define MAX_THUNKS     65536
#define MAX_NAME       512

/* ---- unaligned little-endian access ---- */
static uint16_t rd16(const uint8_t *p) { return (uint16_t)(p[0] | p[1] << 8); }
static uint32_t rd32(const uint8_t *p) { return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24; }
static uint64_t rd64(const uint8_t *p) { return rd32(p) | (uint64_t)rd32(p + 4) << 32; }
static void wr16(uint8_t *p, uint16_t v) { p[0] = (uint8_t)v; p[1] = (uint8_t)(v >> 8); }
static void wr32(uint8_t *p, uint32_t v) { wr16(p, (uint16_t)v); wr16(p + 2, (uint16_t)(v >> 16)); }
static void wr64(uint8_t *p, uint64_t v) { wr32(p, (uint32_t)v); wr32(p + 4, (uint32_t)(v >> 32)); }

static int pow2(uint32_t v) { return v && !(v & (v - 1)); }
static uint64_t align_up(uint64_t v, uint32_t a) { return (v + a - 1) & ~(uint64_t)(a - 1); }

const char *pe_status_str(pe_status s)
{
    switch (s) {
    case PE_OK: return "ok";
    case PE_ERR_TRUNCATED: return "truncated image";
    case PE_ERR_BAD_MAGIC: return "bad magic";
    case PE_ERR_UNSUPPORTED: return "unsupported image type";
    case PE_ERR_BAD_FORMAT: return "malformed header";
    case PE_ERR_BAD_RVA: return "RVA outside image";
    case PE_ERR_WRONG_LAYOUT: return "wrong buffer layout for operation";
    case PE_ERR_BUFFER_TOO_SMALL: return "buffer too small";
    case PE_ERR_NO_RELOCS: return "image needs relocation but has none";
    case PE_ERR_BAD_RELOC: return "bad relocation entry";
    case PE_ERR_APISET: return "unknown API set";
    case PE_ERR_UNRESOLVED: return "unresolved import";
    case PE_ERR_NOT_FOUND: return "not found";
    }
    return "?";
}

/* ---- open ---- */
pe_status pe_open(pe_image *img, void *buf, size_t size, pe_layout layout)
{
    const uint8_t *b = buf;
    uint32_t lfanew, nrva;
    size_t nt, opt, optsz, i;
    uint16_t magic;
    uint64_t prev_end = 0;

    memset(img, 0, sizeof *img);
    if (!b || size < 0x40) return PE_ERR_TRUNCATED;
    if (rd16(b) != 0x5a4d) return PE_ERR_BAD_MAGIC;
    lfanew = rd32(b + 0x3c);
    if ((uint64_t)lfanew + 24 > size) return PE_ERR_TRUNCATED;
    nt = lfanew;
    if (rd32(b + nt) != 0x00004550) return PE_ERR_BAD_MAGIC;

    img->machine = rd16(b + nt + 4);
    img->nsections = rd16(b + nt + 6);
    optsz = rd16(b + nt + 20);
    img->file_characteristics = rd16(b + nt + 22);
    opt = nt + 24;
    if (img->nsections > MAX_SECTIONS) return PE_ERR_BAD_FORMAT;
    if (opt + optsz > size || optsz < 2) return PE_ERR_TRUNCATED;

    magic = rd16(b + opt);
    if (magic == 0x10b && img->machine == PE_MACHINE_I386) img->is64 = 0;
    else if (magic == 0x20b && img->machine == PE_MACHINE_AMD64) img->is64 = 1;
    else if (magic == 0x10b || magic == 0x20b) return PE_ERR_UNSUPPORTED;
    else return PE_ERR_BAD_MAGIC;

    if (optsz < (img->is64 ? 112u : 96u)) return PE_ERR_TRUNCATED;
    img->entry_rva = rd32(b + opt + 16);
    if (img->is64) {
        img->image_base = rd64(b + opt + 24);
        img->image_base_off = opt + 24;
        nrva = rd32(b + opt + 108);
    } else {
        img->image_base = rd32(b + opt + 28);
        img->image_base_off = opt + 28;
        nrva = rd32(b + opt + 92);
    }
    img->section_align = rd32(b + opt + 32);
    img->file_align = rd32(b + opt + 36);
    img->size_of_image = rd32(b + opt + 56);
    img->size_of_headers = rd32(b + opt + 60);
    img->dll_characteristics = rd16(b + opt + 70);

    if (!pow2(img->section_align) || !pow2(img->file_align)) return PE_ERR_BAD_FORMAT;
    if (img->file_align < 512 || img->file_align > 65536) return PE_ERR_BAD_FORMAT;
    if (img->size_of_image == 0 || (img->size_of_image & (img->section_align - 1))) return PE_ERR_BAD_FORMAT;
    if (img->size_of_headers == 0 || img->size_of_headers > img->size_of_image) return PE_ERR_BAD_FORMAT;
    if (img->entry_rva >= img->size_of_image && img->entry_rva) return PE_ERR_BAD_RVA;

    if (nrva > 16) nrva = 16;
    if ((uint64_t)(img->is64 ? 112 : 96) + (uint64_t)nrva * 8 > optsz) return PE_ERR_BAD_FORMAT;
    {
        size_t dd = opt + (img->is64 ? 112 : 96);
        for (i = 0; i < nrva; i++) {
            img->dir[i].rva = rd32(b + dd + i * 8);
            img->dir[i].size = rd32(b + dd + i * 8 + 4);
        }
    }

    img->sect_off = opt + optsz;
    if ((uint64_t)img->sect_off + (uint64_t)img->nsections * 40 > size) return PE_ERR_TRUNCATED;
    if (img->sect_off + (size_t)img->nsections * 40 > img->size_of_headers) return PE_ERR_BAD_FORMAT;

    img->base = buf;
    img->size = size;
    img->layout = layout;

    if (layout == PE_LAYOUT_FILE) {
        if (img->size_of_headers > size) return PE_ERR_TRUNCATED;
    } else if (size < img->size_of_image) {
        return PE_ERR_TRUNCATED;
    }

    prev_end = img->size_of_headers;
    for (i = 0; i < img->nsections; i++) {
        pe_section s;
        uint64_t vext, end;
        pe_status st = pe_section_at(img, (unsigned)i, &s);
        if (st) return st;
        vext = align_up(s.virtual_size ? s.virtual_size : s.raw_size, img->section_align);
        end = (uint64_t)s.virtual_address + vext;
        if (s.virtual_address & (img->section_align - 1)) return PE_ERR_BAD_FORMAT;
        if (s.virtual_address < prev_end) return PE_ERR_BAD_FORMAT;   /* ascending, no overlap */
        if (end > img->size_of_image) return PE_ERR_BAD_FORMAT;
        if (layout == PE_LAYOUT_FILE && s.raw_size &&
            (uint64_t)s.raw_offset + s.raw_size > size) return PE_ERR_TRUNCATED;
        prev_end = end;
    }
    return PE_OK;
}

pe_status pe_section_at(const pe_image *img, unsigned idx, pe_section *out)
{
    const uint8_t *s;
    if (idx >= img->nsections) return PE_ERR_BAD_FORMAT;
    s = img->base + img->sect_off + (size_t)idx * 40;
    memcpy(out->name, s, 8);
    out->name[8] = 0;
    out->virtual_size = rd32(s + 8);
    out->virtual_address = rd32(s + 12);
    out->raw_size = rd32(s + 16);
    out->raw_offset = rd32(s + 20);
    out->characteristics = rd32(s + 36);
    return PE_OK;
}

uint8_t *pe_rva_ptr(const pe_image *img, uint32_t rva, size_t len)
{
    unsigned i;
    if (!len) return NULL;
    if (img->layout == PE_LAYOUT_MAPPED) {
        if (rva >= img->size_of_image || len > img->size_of_image - rva) return NULL;
        if (len > img->size - rva) return NULL;
        return img->base + rva;
    }
    if (rva < img->size_of_headers) {
        if (len > img->size_of_headers - rva) return NULL;
        return img->base + rva;
    }
    for (i = 0; i < img->nsections; i++) {
        pe_section s;
        if (pe_section_at(img, i, &s)) return NULL;
        if (rva >= s.virtual_address && rva - s.virtual_address < s.raw_size) {
            uint64_t off = rva - s.virtual_address;
            if (off + len > s.raw_size) return NULL;
            if ((uint64_t)s.raw_offset + off + len > img->size) return NULL;
            return img->base + s.raw_offset + off;
        }
    }
    return NULL;
}

/* NUL-terminated string at rva, fully inside the image; returns pointer or NULL. */
static const char *cstr_at(const pe_image *img, uint32_t rva, size_t *len_out)
{
    size_t n;
    const uint8_t *p = pe_rva_ptr(img, rva, 1);
    if (!p) return NULL;
    for (n = 0; n < MAX_NAME; n++) {
        const uint8_t *q = pe_rva_ptr(img, rva + (uint32_t)n, 1);
        if (!q) return NULL;
        if (!*q) { if (len_out) *len_out = n; return (const char *)p; }
    }
    return NULL;
}

pe_status pe_rva_string(const pe_image *img, uint32_t rva, char *out, size_t outsz)
{
    size_t n;
    const char *s = cstr_at(img, rva, &n);
    if (!s) return PE_ERR_BAD_RVA;
    if (n + 1 > outsz) return PE_ERR_BUFFER_TOO_SMALL;
    memcpy(out, s, n + 1);
    return PE_OK;
}

/* ---- map ---- */
pe_status pe_map(const pe_image *file, void *dst, size_t dst_size, pe_image *mapped)
{
    unsigned i;
    if (file->layout != PE_LAYOUT_FILE) return PE_ERR_WRONG_LAYOUT;
    if (dst_size < file->size_of_image) return PE_ERR_BUFFER_TOO_SMALL;
    memset(dst, 0, file->size_of_image);
    memcpy(dst, file->base, file->size_of_headers);
    for (i = 0; i < file->nsections; i++) {
        pe_section s;
        uint32_t extent, n;
        pe_section_at(file, i, &s);
        extent = s.virtual_size ? s.virtual_size : s.raw_size;
        n = s.raw_size < extent ? s.raw_size : extent;
        if (n) memcpy((uint8_t *)dst + s.virtual_address, file->base + s.raw_offset, n);
    }
    return pe_open(mapped, dst, file->size_of_image, PE_LAYOUT_MAPPED);
}

/* ---- relocations ---- */
pe_status pe_relocate(pe_image *img, uint64_t new_base)
{
    pe_dir d = img->dir[PE_DIR_BASERELOC];
    uint64_t delta = new_base - img->image_base;
    uint32_t pos = 0;

    if (img->layout != PE_LAYOUT_MAPPED) return PE_ERR_WRONG_LAYOUT;
    if (!img->is64 && new_base > 0xffffffffull) return PE_ERR_BAD_RELOC;
    if (new_base == img->image_base) return PE_OK;
    if (!d.rva || !d.size) {
        /* No reloc table: fine for a position-independent image (nothing holds an absolute
         * address), fatal only if the linker said the relocations were stripped. */
        if (img->file_characteristics & PE_FILE_RELOCS_STRIPPED) return PE_ERR_NO_RELOCS;
        if (img->is64) wr64(img->base + img->image_base_off, new_base);
        else wr32(img->base + img->image_base_off, (uint32_t)new_base);
        img->image_base = new_base;
        return PE_OK;
    }

    while (pos + 8 <= d.size) {
        const uint8_t *blk = pe_rva_ptr(img, d.rva + pos, 8);
        uint32_t page, bsize, e;
        if (!blk) return PE_ERR_BAD_RVA;
        page = rd32(blk);
        bsize = rd32(blk + 4);
        if (!page && !bsize) break;                       /* trailing padding */
        if (bsize < 8 || (bsize & 1) || bsize > d.size - pos) return PE_ERR_BAD_RELOC;
        for (e = 8; e < bsize; e += 2) {
            const uint8_t *ep = pe_rva_ptr(img, d.rva + pos + e, 2);
            uint16_t ent;
            uint32_t type, rva;
            uint8_t *t;
            if (!ep) return PE_ERR_BAD_RVA;
            ent = rd16(ep);
            type = ent >> 12;
            rva = page + (ent & 0xfff);
            switch (type) {
            case 0: break;
            case 1: if (!(t = pe_rva_ptr(img, rva, 2))) return PE_ERR_BAD_RVA;
                    wr16(t, (uint16_t)(rd16(t) + (uint16_t)(delta >> 16))); break;
            case 2: if (!(t = pe_rva_ptr(img, rva, 2))) return PE_ERR_BAD_RVA;
                    wr16(t, (uint16_t)(rd16(t) + (uint16_t)delta)); break;
            case 3: if (!(t = pe_rva_ptr(img, rva, 4))) return PE_ERR_BAD_RVA;
                    wr32(t, rd32(t) + (uint32_t)delta); break;
            case 10: if (!img->is64) return PE_ERR_BAD_RELOC;
                    if (!(t = pe_rva_ptr(img, rva, 8))) return PE_ERR_BAD_RVA;
                    wr64(t, rd64(t) + delta); break;
            default: return PE_ERR_BAD_RELOC;
            }
        }
        pos += bsize;
    }
    if (img->is64) wr64(img->base + img->image_base_off, new_base);
    else wr32(img->base + img->image_base_off, (uint32_t)new_base);
    img->image_base = new_base;
    return PE_OK;
}

/* ---- exports ---- */
pe_status pe_find_export(const pe_image *img, const char *name, uint16_t ordinal, pe_export *out)
{
    pe_dir d = img->dir[PE_DIR_EXPORT];
    const uint8_t *ed, *funcs, *names, *ords;
    uint32_t base, nfuncs, nnames, idx, frva;

    memset(out, 0, sizeof *out);
    if (!d.rva || d.size < 40) return PE_ERR_NOT_FOUND;
    if (!(ed = pe_rva_ptr(img, d.rva, 40))) return PE_ERR_BAD_RVA;
    base = rd32(ed + 16);
    nfuncs = rd32(ed + 20);
    nnames = rd32(ed + 24);
    if (nfuncs > 0x40000 || nnames > 0x40000) return PE_ERR_BAD_FORMAT;
    if (!nfuncs) return PE_ERR_NOT_FOUND;
    if (!(funcs = pe_rva_ptr(img, rd32(ed + 28), (size_t)nfuncs * 4))) return PE_ERR_BAD_RVA;

    if (name) {
        uint32_t lo = 0, hi = nnames;
        if (!nnames) return PE_ERR_NOT_FOUND;
        if (!(names = pe_rva_ptr(img, rd32(ed + 32), (size_t)nnames * 4))) return PE_ERR_BAD_RVA;
        if (!(ords = pe_rva_ptr(img, rd32(ed + 36), (size_t)nnames * 2))) return PE_ERR_BAD_RVA;
        idx = 0xffffffffu;
        while (lo < hi) {                                    /* name table is sorted */
            uint32_t mid = lo + (hi - lo) / 2;
            const char *s = cstr_at(img, rd32(names + (size_t)mid * 4), NULL);
            int c;
            if (!s) return PE_ERR_BAD_RVA;
            c = strcmp(name, s);
            if (!c) { idx = rd16(ords + (size_t)mid * 2); break; }
            if (c < 0) hi = mid; else lo = mid + 1;
        }
        if (idx == 0xffffffffu) return PE_ERR_NOT_FOUND;
    } else {
        if (ordinal < base || ordinal - base >= nfuncs) return PE_ERR_NOT_FOUND;
        idx = ordinal - base;
    }
    if (idx >= nfuncs) return PE_ERR_BAD_FORMAT;

    frva = rd32(funcs + (size_t)idx * 4);
    if (!frva) return PE_ERR_NOT_FOUND;
    if (frva >= d.rva && frva - d.rva < d.size) {            /* forwarder string */
        if (pe_rva_string(img, frva, out->forwarder, sizeof out->forwarder)) return PE_ERR_BAD_FORMAT;
        return PE_OK;
    }
    out->rva = frva;
    return PE_OK;
}

/* ---- imports ---- */
static void to_lower(char *s) { for (; *s; s++) if (*s >= 'A' && *s <= 'Z') *s += 32; }

static void fmt_symbol(const pe_import_ref *r, char *out, size_t n)
{
    size_t i = 0;
    if (r->name) {
        for (; r->name[i] && i + 1 < n; i++) out[i] = r->name[i];
    } else {
        unsigned v = r->ordinal; char tmp[8]; int t = 0;
        if (n < 8) { out[0] = 0; return; }
        out[i++] = '#';
        do { tmp[t++] = (char)('0' + v % 10); v /= 10; } while (v);
        while (t) out[i++] = tmp[--t];
    }
    out[i] = 0;
}

static int va_to_rva(const pe_image *img, uint64_t va, uint32_t *rva)
{
    if (va < img->image_base || va - img->image_base >= img->size_of_image) return 0;
    *rva = (uint32_t)(va - img->image_base);
    return 1;
}

/* One walker for enumerate and bind, over the normal or delay-load table. bind != NULL => write IAT (mapped only). */
static pe_status walk_imports(pe_image *img, int delay, int (*visit)(void *, const pe_import_ref *), void *vctx,
                              pe_resolver resolve, void *rctx, pe_bind_error *err)
{
    pe_dir d = img->dir[delay ? PE_DIR_DELAY_IMPORT : PE_DIR_IMPORT];
    const unsigned es = img->is64 ? 8 : 4;
    const unsigned dsz = delay ? 32 : 20;
    unsigned di;

    if (err) { err->status = PE_OK; err->dll[0] = err->symbol[0] = 0; }
    if (resolve && img->layout != PE_LAYOUT_MAPPED) return PE_ERR_WRONG_LAYOUT;
    if (!d.rva) return PE_OK;

    for (di = 0; di < MAX_IMPORT_DLL; di++) {
        const uint8_t *desc = pe_rva_ptr(img, d.rva + di * dsz, dsz);
        uint32_t oft, name_rva, ft, k;
        int va_based = 0;
        const char *dll;
        char host[128];
        pe_import_ref ref;
        apiset_status as;

        if (!desc) return PE_ERR_BAD_RVA;
        if (delay) {
            /* Attributes bit0 clear: legacy descriptor holding VAs instead of RVAs */
            va_based = !(rd32(desc) & 1);
            name_rva = rd32(desc + 4); ft = rd32(desc + 12); oft = rd32(desc + 16);
            if (va_based) {
                if (name_rva && !va_to_rva(img, name_rva, &name_rva)) return PE_ERR_BAD_RVA;
                if (ft && !va_to_rva(img, ft, &ft)) return PE_ERR_BAD_RVA;
                if (oft && !va_to_rva(img, oft, &oft)) return PE_ERR_BAD_RVA;
            }
            if (!name_rva && !ft && !oft) return PE_OK;      /* terminator */
        } else {
            oft = rd32(desc); name_rva = rd32(desc + 12); ft = rd32(desc + 16);
            if (!oft && !name_rva && !ft) return PE_OK;      /* terminator */
        }
        if (!name_rva || !ft) return PE_ERR_BAD_FORMAT;
        if (!oft) oft = ft;
        if (!(dll = cstr_at(img, name_rva, NULL))) return PE_ERR_BAD_RVA;

        as = apiset_resolve(dll, host, sizeof host);
        if (as == APISET_NOT_AN_APISET) {
            size_t n = strlen(dll);
            if (n + 1 > sizeof host) return PE_ERR_BAD_FORMAT;
            memcpy(host, dll, n + 1);
            to_lower(host);
        } else if (as != APISET_OK) {
            if (err) { err->status = PE_ERR_APISET; strncpy(err->dll, dll, sizeof err->dll - 1); err->dll[sizeof err->dll - 1] = 0; }
            return PE_ERR_APISET;
        }

        for (k = 0; k < MAX_THUNKS; k++) {
            const uint8_t *t = pe_rva_ptr(img, oft + k * es, es);
            uint64_t v;
            uint64_t addr = 0;
            if (!t) return PE_ERR_BAD_RVA;
            v = es == 8 ? rd64(t) : rd32(t);
            if (!v) break;

            ref.dll = dll; ref.host = host; ref.name = NULL; ref.ordinal = 0;
            if (v >> (es * 8 - 1)) {
                ref.ordinal = (uint16_t)(v & 0xffff);
            } else {
                uint32_t nrva = (uint32_t)v;
                const uint8_t *hn;
                if (va_based && !va_to_rva(img, v, &nrva)) return PE_ERR_BAD_RVA;
                hn = pe_rva_ptr(img, nrva, 2);
                if (!hn || !(ref.name = cstr_at(img, nrva + 2, NULL))) return PE_ERR_BAD_RVA;
                ref.ordinal = rd16(hn);
            }

            if (visit && visit(vctx, &ref)) return PE_OK;
            if (resolve) {
                uint8_t *slot = pe_rva_ptr(img, ft + k * es, es);
                if (!slot) return PE_ERR_BAD_RVA;
                if (resolve(rctx, &ref, &addr)) {
                    if (err) {
                        err->status = PE_ERR_UNRESOLVED;
                        strncpy(err->dll, dll, sizeof err->dll - 1); err->dll[sizeof err->dll - 1] = 0;
                        fmt_symbol(&ref, err->symbol, sizeof err->symbol);
                    }
                    return PE_ERR_UNRESOLVED;
                }
                if (es == 8) wr64(slot, addr); else wr32(slot, (uint32_t)addr);
            }
        }
        if (k == MAX_THUNKS) return PE_ERR_BAD_FORMAT;
    }
    return PE_ERR_BAD_FORMAT;
}

pe_status pe_bind_imports(pe_image *img, pe_resolver resolve, void *ctx, pe_bind_error *err)
{
    if (!resolve) return PE_ERR_UNRESOLVED;
    return walk_imports(img, 0, NULL, NULL, resolve, ctx, err);
}

pe_status pe_enum_imports(const pe_image *img, int (*visit)(void *, const pe_import_ref *), void *ctx)
{
    return walk_imports((pe_image *)img, 0, visit, ctx, NULL, NULL, NULL);
}

pe_status pe_bind_delay_imports(pe_image *img, pe_resolver resolve, void *ctx, pe_bind_error *err)
{
    if (!resolve) return PE_ERR_UNRESOLVED;
    return walk_imports(img, 1, NULL, NULL, resolve, ctx, err);
}

pe_status pe_enum_delay_imports(const pe_image *img, int (*visit)(void *, const pe_import_ref *), void *ctx)
{
    return walk_imports((pe_image *)img, 1, visit, ctx, NULL, NULL, NULL);
}

/* ---- bound imports ---- */
pe_status pe_enum_bound_imports(const pe_image *img,
                                int (*visit)(void *ctx, const char *dll, uint32_t timestamp, unsigned nforwarders),
                                void *ctx)
{
    pe_dir d = img->dir[PE_DIR_BOUND_IMPORT];
    uint32_t pos = 0;
    unsigned count = 0;

    if (!d.rva || !d.size) return PE_OK;
    for (;;) {
        const uint8_t *e;
        char nm[128];
        uint32_t ts;
        unsigned off, nfwd, j;
        if ((uint64_t)pos + 8 > d.size) return PE_ERR_BAD_FORMAT;
        if (!(e = pe_rva_ptr(img, d.rva + pos, 8))) return PE_ERR_BAD_RVA;
        ts = rd32(e); off = rd16(e + 4); nfwd = rd16(e + 6);
        if (!ts && !off && !nfwd) return PE_OK;                 /* terminator */
        if (++count > MAX_IMPORT_DLL || off >= d.size) return PE_ERR_BAD_FORMAT;
        if (pe_rva_string(img, d.rva + off, nm, sizeof nm)) return PE_ERR_BAD_RVA;
        if (visit && visit(ctx, nm, ts, nfwd)) return PE_OK;
        if ((uint64_t)pos + 8 + (uint64_t)nfwd * 8 > d.size) return PE_ERR_BAD_FORMAT;
        for (j = 0; j < nfwd; j++) {                            /* forwarder refs: same 8-byte layout */
            const uint8_t *f = pe_rva_ptr(img, d.rva + pos + 8 + j * 8, 8);
            if (!f) return PE_ERR_BAD_RVA;
            if (rd16(f + 4) >= d.size) return PE_ERR_BAD_FORMAT;
        }
        pos += 8 + nfwd * 8;
    }
}

/* ---- TLS ---- */
pe_status pe_get_tls(const pe_image *img, pe_tls *t)
{
    pe_dir d = img->dir[PE_DIR_TLS];
    const unsigned es = img->is64 ? 8 : 4;
    const uint8_t *p;
    uint64_t start, end, idx, cb, ch;
    uint32_t zero;
    uint32_t start_rva, cb_rva;
    unsigned n;

    memset(t, 0, sizeof *t);
    if (!d.rva || !d.size) return PE_ERR_NOT_FOUND;
    if (!(p = pe_rva_ptr(img, d.rva, es * 4 + 8))) return PE_ERR_BAD_RVA;
    if (es == 8) { start = rd64(p); end = rd64(p + 8); idx = rd64(p + 16); cb = rd64(p + 24); }
    else         { start = rd32(p); end = rd32(p + 4); idx = rd32(p + 8);  cb = rd32(p + 12); }
    zero = rd32(p + es * 4);
    ch = rd32(p + es * 4 + 4);

    if (end < start) return PE_ERR_BAD_FORMAT;
    if (end != start) {
        if (!va_to_rva(img, start, &start_rva)) return PE_ERR_BAD_RVA;
        if (end - start > 0x10000000u) return PE_ERR_BAD_FORMAT;
        if (!pe_rva_ptr(img, start_rva, (size_t)(end - start))) return PE_ERR_BAD_RVA;
        t->raw_rva = start_rva;
        t->raw_size = (uint32_t)(end - start);
    }
    t->zero_fill = zero;
    if ((uint64_t)t->raw_size + zero > 0x10000000u) return PE_ERR_BAD_FORMAT;
    if (!va_to_rva(img, idx, &t->index_rva)) return PE_ERR_BAD_RVA;
    n = (unsigned)((ch >> 20) & 0xf);
    t->alignment = n ? (1u << (n - 1)) : es;

    if (cb) {
        if (!va_to_rva(img, cb, &cb_rva)) return PE_ERR_BAD_RVA;
        for (n = 0;; n++) {
            const uint8_t *c = pe_rva_ptr(img, cb_rva + n * es, es);
            uint64_t v;
            if (!c) return PE_ERR_BAD_RVA;
            v = es == 8 ? rd64(c) : rd32(c);
            if (!v) break;
            if (n >= PE_MAX_TLS_CALLBACKS) return PE_ERR_BAD_FORMAT;
            if (!va_to_rva(img, v, &t->callback_rva[n])) return PE_ERR_BAD_RVA;
        }
        t->ncallbacks = n;
    }
    return PE_OK;
}

pe_status pe_tls_init_block(const pe_image *img, const pe_tls *t, void *dst, size_t dst_size)
{
    uint8_t *raw;
    if ((uint64_t)t->raw_size + t->zero_fill > dst_size) return PE_ERR_BUFFER_TOO_SMALL;
    if (t->raw_size) {
        if (!(raw = pe_rva_ptr(img, t->raw_rva, t->raw_size))) return PE_ERR_BAD_RVA;
        memcpy(dst, raw, t->raw_size);
    }
    memset((uint8_t *)dst + t->raw_size, 0, t->zero_fill);
    return PE_OK;
}

/* ---- resources ---- */
/* Resource tree: directory (16 bytes) of 8-byte entries; entry offset high bit = subdirectory.
 * Level 1 = type, level 2 = name/id, level 3 = language. We take the first language. */
static pe_status res_find_entry(const pe_image *img, uint32_t res_rva, uint32_t res_size, uint32_t dir_off,
                                int by_id, uint32_t id, uint32_t *out_off)
{
    const uint8_t *d;
    unsigned nnamed, nid, i, total;
    if ((uint64_t)dir_off + 16 > res_size) return PE_ERR_BAD_RVA;
    if (!(d = pe_rva_ptr(img, res_rva + dir_off, 16))) return PE_ERR_BAD_RVA;
    nnamed = rd16(d + 12); nid = rd16(d + 14);
    total = nnamed + nid;
    if ((uint64_t)dir_off + 16 + (uint64_t)total * 8 > res_size) return PE_ERR_BAD_RVA;
    for (i = 0; i < total; i++) {
        const uint8_t *e = pe_rva_ptr(img, res_rva + dir_off + 16 + i * 8, 8);
        uint32_t name, off;
        if (!e) return PE_ERR_BAD_RVA;
        name = rd32(e); off = rd32(e + 4);
        if (by_id < 0) { *out_off = off; return PE_OK; }          /* any entry (language level) */
        if (name & 0x80000000u) continue;                          /* named entries are not matched by id */
        if (i < nnamed) continue;
        if (name == id) { *out_off = off; return PE_OK; }
    }
    return PE_ERR_NOT_FOUND;
}

pe_status pe_find_resource(const pe_image *img, uint32_t type, uint32_t id, uint32_t *rva, uint32_t *size)
{
    pe_dir d = img->dir[PE_DIR_RESOURCE];
    uint32_t off, o1, o2, depth;
    const uint8_t *de;
    pe_status st;

    if (!d.rva || !d.size) return PE_ERR_NOT_FOUND;
    if ((st = res_find_entry(img, d.rva, d.size, 0, 1, type, &o1))) return st;
    if (!(o1 & 0x80000000u)) return PE_ERR_BAD_FORMAT;
    if ((st = res_find_entry(img, d.rva, d.size, o1 & 0x7fffffffu, 1, id, &o2))) return st;
    off = o2;
    for (depth = 0; (off & 0x80000000u) && depth < 4; depth++) {          /* descend to the data entry, first language */
        uint32_t next;
        if ((st = res_find_entry(img, d.rva, d.size, off & 0x7fffffffu, -1, 0, &next))) return st;
        off = next;
    }
    if (off & 0x80000000u) return PE_ERR_BAD_FORMAT;
    if ((uint64_t)off + 16 > d.size) return PE_ERR_BAD_RVA;
    if (!(de = pe_rva_ptr(img, d.rva + off, 16))) return PE_ERR_BAD_RVA;
    *rva = rd32(de); *size = rd32(de + 4);
    if (!pe_rva_ptr(img, *rva, *size ? *size : 1)) return PE_ERR_BAD_RVA;
    return PE_OK;
}

/* ---- export enumeration ---- */
pe_status pe_enum_exports(const pe_image *img,
                          int (*visit)(void *ctx, const char *name, uint16_t ordinal, uint32_t rva, const char *forwarder),
                          void *ctx)
{
    pe_dir d = img->dir[PE_DIR_EXPORT];
    const uint8_t *ed, *funcs, *names = NULL, *ords = NULL;
    uint32_t base, nfuncs, nnames, i, j;
    char fwd[128];

    if (!d.rva || d.size < 40) return PE_OK;
    if (!(ed = pe_rva_ptr(img, d.rva, 40))) return PE_ERR_BAD_RVA;
    base = rd32(ed + 16); nfuncs = rd32(ed + 20); nnames = rd32(ed + 24);
    if (nfuncs > 0x40000 || nnames > 0x40000) return PE_ERR_BAD_FORMAT;
    if (!nfuncs) return PE_OK;
    if (!(funcs = pe_rva_ptr(img, rd32(ed + 28), (size_t)nfuncs * 4))) return PE_ERR_BAD_RVA;
    if (nnames) {
        if (!(names = pe_rva_ptr(img, rd32(ed + 32), (size_t)nnames * 4))) return PE_ERR_BAD_RVA;
        if (!(ords = pe_rva_ptr(img, rd32(ed + 36), (size_t)nnames * 2))) return PE_ERR_BAD_RVA;
    }
    for (i = 0; i < nfuncs; i++) {
        uint32_t frva = rd32(funcs + (size_t)i * 4);
        const char *name = NULL;
        char nm[256];
        if (!frva) continue;
        for (j = 0; j < nnames; j++) if (rd16(ords + (size_t)j * 2) == i) {
            if (pe_rva_string(img, rd32(names + (size_t)j * 4), nm, sizeof nm)) return PE_ERR_BAD_RVA;
            name = nm; break;
        }
        fwd[0] = 0;
        if (frva >= d.rva && frva - d.rva < d.size && pe_rva_string(img, frva, fwd, sizeof fwd)) return PE_ERR_BAD_FORMAT;
        if (visit(ctx, name, (uint16_t)(base + i), fwd[0] ? 0 : frva, fwd[0] ? fwd : NULL)) return PE_OK;
    }
    return PE_OK;
}
