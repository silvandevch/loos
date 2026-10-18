#include "pebuild.h"
#include <stdlib.h>
#include <string.h>

#define FILE_SIZE 0x2800
#define IMG_SIZE  0x5000

static void w16(uint8_t *p, unsigned v) { p[0] = (uint8_t)v; p[1] = (uint8_t)(v >> 8); }
static void w32(uint8_t *p, uint32_t v) { w16(p, v & 0xffff); w16(p + 2, v >> 16); }
static void w64(uint8_t *p, uint64_t v) { w32(p, (uint32_t)v); w32(p + 4, (uint32_t)(v >> 32)); }

typedef struct { uint8_t *f; unsigned es; int is64; uint32_t ro; uint32_t relocs[64]; int nrel; } B;

static uint8_t *at_rva(B *b, uint32_t rva)           /* file pointer for an rva in .text/.rdata */
{
    if (rva >= 0x2000) return b->f + 0x600 + (rva - 0x2000);
    return b->f + 0x400 + (rva - 0x1000);
}
static uint32_t rdalloc(B *b, uint32_t n, uint32_t align) { b->ro = (b->ro + align - 1) & ~(align - 1); { uint32_t r = 0x2000 + b->ro; b->ro += n; return r; } }
static uint32_t rdstr(B *b, const char *s) { uint32_t r = rdalloc(b, (uint32_t)strlen(s) + 1, 1); strcpy((char *)at_rva(b, r), s); return r; }
static void put_ptr(B *b, uint32_t rva, uint64_t v, int reloc)
{
    if (b->is64) w64(at_rva(b, rva), v); else w32(at_rva(b, rva), (uint32_t)v);
    if (reloc) b->relocs[b->nrel++] = rva;
}
static void put_rva_ptr(B *b, uint32_t rva, uint64_t v)       /* thunk entries: plain RVAs / flags */
{ if (b->is64) w64(at_rva(b, rva), v); else w32(at_rva(b, rva), (uint32_t)v); }

static int cmp_name(const void *a, const void *b2)
{ return strcmp(((const bexport *)a)->name, ((const bexport *)b2)->name); }

void bimage_build(const bspec *s, bimage *out)
{
    B b = {0};
    uint8_t *f = calloc(1, FILE_SIZE);
    unsigned optsz = s->is64 ? 240 : 224, opt = 0x40 + 24, i, j;
    uint8_t *o, *sec, *dd;
    uint32_t imp_rva = 0, exp_rva = 0, exp_size = 0, tls_rva = 0;
    uint64_t base = s->base;

    b.f = f; b.is64 = s->is64; b.es = s->is64 ? 8 : 4;
    memset(out, 0, sizeof *out);

    w16(f, 0x5a4d); w32(f + 0x3c, 0x40); w32(f + 0x40, 0x4550);
    w16(f + 0x44, s->is64 ? 0x8664 : 0x14c); w16(f + 0x46, 3);
    w16(f + 0x54, optsz);
    w16(f + 0x56, (s->is_dll ? 0x2000 : 0) | 0x0002 | ((s->relocs || s->pic) ? 0 : 0x0001));
    o = f + opt;
    w16(o, s->is64 ? 0x20b : 0x10b);
    w32(o + 16, (s->entry || !s->is_dll) ? 0x1000 : 0);
    if (s->is64) w64(o + 24, base); else w32(o + 28, (uint32_t)base);
    w32(o + 32, 0x1000); w32(o + 36, 0x200); w32(o + 56, IMG_SIZE); w32(o + 60, 0x400);
    w16(o + 68, 3);
    w32(o + (s->is64 ? 108 : 92), 16);
    dd = o + (s->is64 ? 112 : 96);

    /* ---- imports ---- */
    if (s->nimports) {
        imp_rva = rdalloc(&b, (uint32_t)(s->nimports + 1) * 20, 4);
        for (i = 0; i < (unsigned)s->nimports; i++) {
            const bimport *im = &s->imports[i];
            unsigned n = 0;
            uint32_t ilt, iat, desc = imp_rva + i * 20;
            while (im->syms[n]) n++;
            ilt = rdalloc(&b, (n + 1) * b.es, 8);
            iat = rdalloc(&b, (n + 1) * b.es, 8);
            for (j = 0; j < n; j++) {
                const char *sym = im->syms[j];
                if (sym[0] == '#') {
                    uint64_t flag = s->is64 ? 0x8000000000000000ull : 0x80000000ull;
                    uint64_t v = flag | (uint64_t)atoi(sym + 1);
                    put_rva_ptr(&b, ilt + j * b.es, v); put_rva_ptr(&b, iat + j * b.es, v);
                } else {
                    uint32_t hn = rdalloc(&b, 2 + (uint32_t)strlen(sym) + 1, 2);
                    w16(at_rva(&b, hn), j);
                    strcpy((char *)at_rva(&b, hn) + 2, sym);
                    put_rva_ptr(&b, ilt + j * b.es, hn); put_rva_ptr(&b, iat + j * b.es, hn);
                }
                out->iat_rva[i][j] = iat + j * b.es;
            }
            w32(at_rva(&b, desc), ilt);
            w32(at_rva(&b, desc + 12), rdstr(&b, im->dll));
            w32(at_rva(&b, desc + 16), iat);
        }
    }

    /* ---- exports ---- */
    if (s->nexports) {
        bexport sorted[BMAX_EXP];
        int ord[BMAX_EXP];
        uint32_t funcs, names, ords, dllname, ed;
        uint32_t start = b.ro;
        memcpy(sorted, s->exports, sizeof(bexport) * (size_t)s->nexports);
        qsort(sorted, (size_t)s->nexports, sizeof(bexport), cmp_name);
        for (i = 0; i < (unsigned)s->nexports; i++)
            for (j = 0; j < (unsigned)s->nexports; j++)
                if (s->exports[j].name == sorted[i].name) ord[i] = (int)j;
        ed = rdalloc(&b, 40, 4);
        funcs = rdalloc(&b, (uint32_t)s->nexports * 4, 4);
        names = rdalloc(&b, (uint32_t)s->nexports * 4, 4);
        ords = rdalloc(&b, (uint32_t)s->nexports * 2, 2);
        dllname = rdstr(&b, "test.dll");
        for (i = 0; i < (unsigned)s->nexports; i++) {
            w32(at_rva(&b, names + i * 4), rdstr(&b, sorted[i].name));
            w16(at_rva(&b, ords + i * 2), (unsigned)ord[i]);
        }
        for (i = 0; i < (unsigned)s->nexports; i++)
            w32(at_rva(&b, funcs + i * 4), s->exports[i].forward ? rdstr(&b, s->exports[i].forward) : B_FUNC_RVA(i));
        w32(at_rva(&b, ed + 12), dllname); w32(at_rva(&b, ed + 16), 1);
        w32(at_rva(&b, ed + 20), (uint32_t)s->nexports); w32(at_rva(&b, ed + 24), (uint32_t)s->nexports);
        w32(at_rva(&b, ed + 28), funcs); w32(at_rva(&b, ed + 32), names); w32(at_rva(&b, ed + 36), ords);
        exp_rva = ed;
        exp_size = 0x2000 + b.ro - ed;       /* directory span covers tables + strings so forwarders are inside it */
        (void)start;
    }

    /* ---- manifest resource: type 24 / id 1 / lang 1033 ---- */
    if (s->manifest) {
        size_t ml = strlen(s->manifest);
        uint32_t blob = rdalloc(&b, (uint32_t)ml, 4), rsrc = rdalloc(&b, 88, 4);
        uint8_t *r = at_rva(&b, rsrc);
        memcpy(at_rva(&b, blob), s->manifest, ml);
        w16(r + 14, 1); w32(r + 16, 24);   w32(r + 20, 0x80000000u | 24);      /* root -> type 24 */
        w16(r + 24 + 14, 1); w32(r + 24 + 16, 1); w32(r + 24 + 20, 0x80000000u | 48);   /* type -> id 1 */
        w16(r + 48 + 14, 1); w32(r + 48 + 16, 1033); w32(r + 48 + 20, 72);      /* id -> lang -> data entry */
        w32(r + 72, blob); w32(r + 76, (uint32_t)ml); w32(r + 80, 0);
        w32(dd + 2 * 8, rsrc); w32(dd + 2 * 8 + 4, 88);
    }

    /* ---- .text data: relocated pointer, TLS template, callback array ---- */
    put_ptr(&b, B_PTR_RVA, base + B_FUNC_RVA(0), s->relocs);
    if (s->tls) {
        uint32_t cb = 0x11a0;
        memcpy(at_rva(&b, 0x1180), "TLSDATA!", 8);
        put_ptr(&b, cb, base + 0x1010, s->relocs);
        tls_rva = rdalloc(&b, s->is64 ? 0x28 : 0x18, 8);
        if (s->is64) {
            put_ptr(&b, tls_rva + 0, base + 0x1180, s->relocs); put_ptr(&b, tls_rva + 8, base + 0x1188, s->relocs);
            put_ptr(&b, tls_rva + 16, base + B_TLS_INDEX_RVA, s->relocs); put_ptr(&b, tls_rva + 24, base + cb, s->relocs);
            w32(at_rva(&b, tls_rva + 32), 0x10);
        } else {
            put_ptr(&b, tls_rva + 0, base + 0x1180, s->relocs); put_ptr(&b, tls_rva + 4, base + 0x1188, s->relocs);
            put_ptr(&b, tls_rva + 8, base + B_TLS_INDEX_RVA, s->relocs); put_ptr(&b, tls_rva + 12, base + cb, s->relocs);
            w32(at_rva(&b, tls_rva + 16), 0x10);
        }
    }

    /* ---- data directories ---- */
    if (exp_rva) { w32(dd + 0, exp_rva); w32(dd + 4, exp_size); }
    if (imp_rva) { w32(dd + 8, imp_rva); w32(dd + 12, (uint32_t)(s->nimports + 1) * 20); }
    if (tls_rva) { w32(dd + 9 * 8, tls_rva); w32(dd + 9 * 8 + 4, s->is64 ? 0x28 : 0x18); }

    /* ---- relocation blocks (grouped by 4K page) ---- */
    if (s->relocs && b.nrel) {
        uint8_t *r = f + 0x2600;
        uint32_t used = 0;
        int k = 0;
        while (k < b.nrel) {
            uint32_t page = b.relocs[k] & ~0xfffu;
            int cnt = 0, m = k;
            while (m < b.nrel && (b.relocs[m] & ~0xfffu) == page) { cnt++; m++; }
            {
                uint32_t bsize = 8 + (uint32_t)cnt * 2;
                if (bsize & 3) bsize += 2;
                w32(r + used, page); w32(r + used + 4, bsize);
                for (j = 0; j < (unsigned)cnt; j++)
                    w16(r + used + 8 + j * 2, ((s->is64 ? 10u : 3u) << 12) | (b.relocs[k + (int)j] & 0xfff));
                used += bsize;
            }
            k = m;
        }
        w32(dd + 5 * 8, 0x4000); w32(dd + 5 * 8 + 4, used);
    }

    /* ---- section table ---- */
    sec = o + optsz;
    memcpy(sec, ".text", 5);  w32(sec + 8, 0x200);  w32(sec + 12, 0x1000); w32(sec + 16, 0x200);  w32(sec + 20, 0x400);  w32(sec + 36, 0x60000020);
    sec += 40;
    memcpy(sec, ".rdata", 6); w32(sec + 8, 0x2000); w32(sec + 12, 0x2000); w32(sec + 16, 0x2000); w32(sec + 20, 0x600);  w32(sec + 36, 0xc0000040);
    sec += 40;
    memcpy(sec, ".reloc", 6); w32(sec + 8, 0x200);  w32(sec + 12, 0x4000); w32(sec + 16, 0x200);  w32(sec + 20, 0x2600); w32(sec + 36, 0x42000040);

    out->data = f; out->size = FILE_SIZE;
}

void bimage_free(bimage *b) { free(b->data); b->data = NULL; }
