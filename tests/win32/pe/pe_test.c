#include "pe.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)
#define CHECK_ST(e, want) do { pe_status s_ = (e); if (s_ != (want)) { printf("FAIL %s:%d: %s = %s, want %s\n", __FILE__, __LINE__, #e, pe_status_str(s_), pe_status_str(want)); fails++; } } while (0)

static void w16(uint8_t *p, unsigned v) { p[0] = v; p[1] = v >> 8; }
static void w32(uint8_t *p, uint32_t v) { w16(p, v & 0xffff); w16(p + 2, v >> 16); }
static void w64(uint8_t *p, uint64_t v) { w32(p, (uint32_t)v); w32(p + 4, (uint32_t)(v >> 32)); }
static uint32_t r32(const uint8_t *p) { return p[0] | p[1] << 8 | p[2] << 16 | (uint32_t)p[3] << 24; }
static uint64_t r64(const uint8_t *p) { return r32(p) | (uint64_t)r32(p + 4) << 32; }

/* ---------- synthetic image builder ----------
 * Layout (file): headers 0x400 | .text raw 0x400..0x600 (rva 0x1000) | .rdata raw 0x600..0xc00 (rva 0x2000)
 * .rdata: import dir @0x2000, ILT @0x2100, IAT @0x2180, hint/names @0x2200, dll names @0x2300,
 *         export dir @0x2400 (+ tables), reloc block in .reloc (rva 0x3000).
 */
#define IMG_SIZE 0x4000
static size_t build_pe(uint8_t *f, int is64, int with_bad_dll)
{
    const unsigned es = is64 ? 8 : 4;
    unsigned opt = 0x40 + 4 + 20, optsz = is64 ? 240 : 224;
    uint8_t *o, *sec;
    memset(f, 0, 0xe00);
    w16(f, 0x5a4d); w32(f + 0x3c, 0x40);
    w32(f + 0x40, 0x4550);
    w16(f + 0x44, is64 ? 0x8664 : 0x14c); w16(f + 0x46, 3);
    w16(f + 0x54, optsz); w16(f + 0x56, 0x2000 /*DLL*/);
    o = f + opt;
    w16(o, is64 ? 0x20b : 0x10b);
    w32(o + 16, 0x1000);
    if (is64) w64(o + 24, 0x180000000ull); else w32(o + 28, 0x10000000);
    w32(o + 32, 0x1000); w32(o + 36, 0x200);
    w32(o + 56, IMG_SIZE); w32(o + 60, 0x400);
    w32(o + (is64 ? 108 : 92), 16);
    {
        uint8_t *dd = o + (is64 ? 112 : 96);
        w32(dd + 0, 0x2400); w32(dd + 4, 0x100);       /* export */
        w32(dd + 8, 0x2000); w32(dd + 12, 0x80);       /* import */
        w32(dd + 5 * 8, 0x3000); w32(dd + 5 * 8 + 4, 0x10); /* reloc */
        w32(dd + 9 * 8, 0x2540);  w32(dd + 9 * 8 + 4, is64 ? 0x28 : 0x18);  /* TLS */
        w32(dd + 11 * 8, 0x25c0); w32(dd + 11 * 8 + 4, 0x20);               /* bound imports */
        w32(dd + 13 * 8, 0x2500); w32(dd + 13 * 8 + 4, 0x40);               /* delay imports */
    }
    sec = o + optsz;
    memcpy(sec, ".text", 5);   w32(sec + 8, 0x200); w32(sec + 12, 0x1000); w32(sec + 16, 0x200); w32(sec + 20, 0x400); w32(sec + 36, 0x60000020);
    sec += 40;
    memcpy(sec, ".rdata", 6);  w32(sec + 8, 0x600); w32(sec + 12, 0x2000); w32(sec + 16, 0x600); w32(sec + 20, 0x600); w32(sec + 36, 0xc0000040);
    sec += 40;
    memcpy(sec, ".reloc", 6);  w32(sec + 8, 0x10);  w32(sec + 12, 0x3000); w32(sec + 16, 0x200); w32(sec + 20, 0xc00); w32(sec + 36, 0x42000040);

    {   /* file offset = rva - 0x2000 + 0x600 for .rdata */
        uint8_t *rd = f + 0x600 - 0x2000;
        /* import descriptors: api-ms kernel, ordinal-only user DLL, terminator */
        w32(rd + 0x2000 + 0, 0x2100); w32(rd + 0x2000 + 12, 0x2300); w32(rd + 0x2000 + 16, 0x2180);
        w32(rd + 0x2014 + 0, 0x2140); w32(rd + 0x2014 + 12, with_bad_dll ? 0x2380 : 0x2340); w32(rd + 0x2014 + 16, 0x21c0);
        /* ILT/IAT #1: CreateFileW (by name), GetLastError (by name), end */
        if (is64) { w64(rd + 0x2100, 0x2200); w64(rd + 0x2108, 0x2220); w64(rd + 0x2180, 0x2200); w64(rd + 0x2188, 0x2220); }
        else      { w32(rd + 0x2100, 0x2200); w32(rd + 0x2104, 0x2220); w32(rd + 0x2180, 0x2200); w32(rd + 0x2184, 0x2220); }
        /* ILT/IAT #2: ordinal 7, end */
        if (is64) { w64(rd + 0x2140, 0x8000000000000007ull); w64(rd + 0x21c0, 0x8000000000000007ull); }
        else      { w32(rd + 0x2140, 0x80000007u);            w32(rd + 0x21c0, 0x80000007u); }
        w16(rd + 0x2200, 5); strcpy((char *)rd + 0x2202, "CreateFileW");
        w16(rd + 0x2220, 9); strcpy((char *)rd + 0x2222, "GetLastError");
        strcpy((char *)rd + 0x2300, "API-MS-Win-Core-File-L1-2-1.DLL");
        strcpy((char *)rd + 0x2340, "USER32.dll");
        strcpy((char *)rd + 0x2380, "api-ms-win-bogus-l1-1-0.dll");
        (void)es;
        /* exports: base 1, 2 funcs "Alpha"(rva 0x1010), "Beta"(forwarder) */
        w32(rd + 0x2400 + 12, 0x2480); w32(rd + 0x2400 + 16, 1); w32(rd + 0x2400 + 20, 2); w32(rd + 0x2400 + 24, 2);
        w32(rd + 0x2400 + 28, 0x2440); w32(rd + 0x2400 + 32, 0x2450); w32(rd + 0x2400 + 36, 0x2460);
        w32(rd + 0x2440, 0x1010); w32(rd + 0x2444, 0x24a0);
        w32(rd + 0x2450, 0x2470); w32(rd + 0x2454, 0x2476);
        w16(rd + 0x2460, 0); w16(rd + 0x2462, 1);
        strcpy((char *)rd + 0x2470, "Alpha"); strcpy((char *)rd + 0x2476, "Beta");
        strcpy((char *)rd + 0x2480, "t.dll"); strcpy((char *)rd + 0x24a0, "NTDLL.RtlBeta");
    }
    {   /* delay import (rva-based), TLS, bound imports */
        uint8_t *rd = f + 0x600 - 0x2000;
        uint64_t b = is64 ? 0x180000000ull : 0x10000000ull;
        w32(rd + 0x2500, 1); w32(rd + 0x2504, 0x25b0); w32(rd + 0x250c, 0x2580); w32(rd + 0x2510, 0x2590);
        if (is64) { w64(rd + 0x2590, 0x25a0); w64(rd + 0x2580, 0x25a0); }
        else      { w32(rd + 0x2590, 0x25a0); w32(rd + 0x2580, 0x25a0); }
        w16(rd + 0x25a0, 3); strcpy((char *)rd + 0x25a2, "DwmFoo");
        strcpy((char *)rd + 0x25b0, "dwmapi.dll");
        w32(rd + 0x25c0, 0x5555); w16(rd + 0x25c4, 0x10); w16(rd + 0x25c6, 0);   /* kernel32, ts 0x5555 */
        strcpy((char *)rd + 0x25d0, "kernel32.dll");
        memcpy(f + 0x500, "TLSDATA!", 8);
        if (is64) {
            w64(rd + 0x2540, b + 0x1100); w64(rd + 0x2548, b + 0x1108); w64(rd + 0x2550, b + 0x1110);
            w64(rd + 0x2558, b + 0x1120); w32(rd + 0x2560, 0x10); w32(rd + 0x2564, 4u << 20);
            w64(f + 0x520, b + 0x1030); w64(f + 0x528, b + 0x1040);
        } else {
            w32(rd + 0x2540, b + 0x1100); w32(rd + 0x2544, b + 0x1108); w32(rd + 0x2548, b + 0x1110);
            w32(rd + 0x254c, b + 0x1120); w32(rd + 0x2550, 0x10); w32(rd + 0x2554, 4u << 20);
            w32(f + 0x520, b + 0x1030); w32(f + 0x524, b + 0x1040);
        }
    }
    /* .text holds a pointer needing relocation at rva 0x1008 */
    if (is64) w64(f + 0x400 + 8, 0x180000000ull + 0x1010); else w32(f + 0x400 + 8, 0x10000000 + 0x1010);
    /* reloc block: page 0x1000, one entry at +8 */
    w32(f + 0xc00, 0x1000); w32(f + 0xc04, 12); w16(f + 0xc08, ((is64 ? 10 : 3) << 12) | 8); w16(f + 0xc0a, 0);
    return 0xe00;
}

struct rctx { int calls; char seen[8][96]; };
static int fake_resolve(void *vctx, const pe_import_ref *r, uint64_t *addr)
{
    struct rctx *c = vctx;
    if (c->calls < 8) snprintf(c->seen[c->calls], sizeof c->seen[0], "%s|%s|%s|%u", r->dll, r->host, r->name ? r->name : "-", r->ordinal);
    *addr = 0xdead0000u + (uint64_t)++c->calls;
    if (r->name && !strcmp(r->name, "NopeFunc")) return 1;
    return 0;
}

static unsigned bound_seen; static char bound_nm[48];
static int bound_cb(void *c, const char *dll, uint32_t ts, unsigned nf)
{ (void)c; bound_seen++; snprintf(bound_nm, sizeof bound_nm, "%s/%x/%u", dll, ts, nf); return 0; }

static void test_synthetic(int is64)
{
    static uint8_t file[0xe00], mem[IMG_SIZE];
    pe_image f, m;
    struct rctx c = {0};
    pe_bind_error e;
    pe_export ex;
    size_t sz = build_pe(file, is64, 0);
    const unsigned es = is64 ? 8 : 4;

    CHECK_ST(pe_open(&f, file, sz, PE_LAYOUT_FILE), PE_OK);
    CHECK(f.is64 == is64 && f.nsections == 3 && f.entry_rva == 0x1000);
    CHECK_ST(pe_map(&f, mem, sizeof mem, &m), PE_OK);

    CHECK_ST(pe_bind_imports(&m, fake_resolve, &c, &e), PE_OK);
    CHECK(c.calls == 3);
    CHECK(!strcmp(c.seen[0], "API-MS-Win-Core-File-L1-2-1.DLL|kernelbase.dll|CreateFileW|5"));
    CHECK(!strcmp(c.seen[1], "API-MS-Win-Core-File-L1-2-1.DLL|kernelbase.dll|GetLastError|9"));
    CHECK(!strcmp(c.seen[2], "USER32.dll|user32.dll|-|7"));
    CHECK((es == 8 ? r64(mem + 0x2180) : r32(mem + 0x2180)) == 0xdead0001u);
    CHECK((es == 8 ? r64(mem + 0x2180 + es) : r32(mem + 0x2180 + es)) == 0xdead0002u);
    CHECK((es == 8 ? r64(mem + 0x21c0) : r32(mem + 0x21c0)) == 0xdead0003u);
    CHECK(f.layout == PE_LAYOUT_FILE && pe_bind_imports(&f, fake_resolve, &c, NULL) == PE_ERR_WRONG_LAYOUT);

    /* delay imports: enumerate then bind into the delay IAT */
    {
        struct rctx dc = {0};
        CHECK_ST(pe_bind_delay_imports(&m, fake_resolve, &dc, &e), PE_OK);
        CHECK(dc.calls == 1 && !strcmp(dc.seen[0], "dwmapi.dll|dwmapi.dll|DwmFoo|3"));
        CHECK((es == 8 ? r64(mem + 0x2580) : r32(mem + 0x2580)) == 0xdead0001u);
    }
    /* bound imports */
    {
        bound_seen = 0; bound_nm[0] = 0;
        CHECK_ST(pe_enum_bound_imports(&m, bound_cb, NULL), PE_OK);
        CHECK(bound_seen == 1 && !strcmp(bound_nm, "kernel32.dll/5555/0"));
    }
    /* TLS */
    {
        pe_tls t; uint8_t blk[64];
        CHECK_ST(pe_get_tls(&m, &t), PE_OK);
        CHECK(t.raw_rva == 0x1100 && t.raw_size == 8 && t.zero_fill == 0x10 && t.index_rva == 0x1110);
        CHECK(t.alignment == 8 && t.ncallbacks == 2 && t.callback_rva[0] == 0x1030 && t.callback_rva[1] == 0x1040);
        memset(blk, 0xaa, sizeof blk);
        CHECK_ST(pe_tls_init_block(&m, &t, blk, sizeof blk), PE_OK);
        CHECK(!memcmp(blk, "TLSDATA!", 8) && blk[8] == 0 && blk[23] == 0 && blk[24] == 0xaa);
        CHECK_ST(pe_tls_init_block(&m, &t, blk, 20), PE_ERR_BUFFER_TOO_SMALL);
        CHECK_ST(pe_get_tls(&f, &t), PE_OK);               /* file layout agrees */
        CHECK(t.raw_rva == 0x1100 && t.ncallbacks == 2);
    }

    /* rebase */
    {
        uint64_t nb = is64 ? 0x7ff600000000ull : 0x00400000;
        uint64_t old = is64 ? 0x180000000ull : 0x10000000;
        CHECK_ST(pe_relocate(&m, nb), PE_OK);
        CHECK((is64 ? r64(mem + 0x1008) : r32(mem + 0x1008)) == nb + 0x1010);
        CHECK(m.image_base == nb);
        CHECK_ST(pe_relocate(&m, old), PE_OK);             /* and back */
        CHECK((is64 ? r64(mem + 0x1008) : r32(mem + 0x1008)) == old + 0x1010);
        if (!is64) CHECK_ST(pe_relocate(&m, 0x100000000ull), PE_ERR_BAD_RELOC);
    }

    /* exports */
    CHECK_ST(pe_find_export(&m, "Alpha", 0, &ex), PE_OK);
    CHECK(ex.rva == 0x1010 && !ex.forwarder[0]);
    CHECK_ST(pe_find_export(&m, "Beta", 0, &ex), PE_OK);
    CHECK(ex.rva == 0 && !strcmp(ex.forwarder, "NTDLL.RtlBeta"));
    CHECK_ST(pe_find_export(&m, NULL, 1, &ex), PE_OK);
    CHECK(ex.rva == 0x1010);
    CHECK_ST(pe_find_export(&m, "Gamma", 0, &ex), PE_ERR_NOT_FOUND);
    CHECK_ST(pe_find_export(&m, NULL, 3, &ex), PE_ERR_NOT_FOUND);

    /* resolver failure reports which symbol */
    build_pe(file, is64, 0);
    memcpy(file + 0x600 + 0x202, "NopeFunc\0\0\0", 11);
    pe_open(&f, file, sz, PE_LAYOUT_FILE); pe_map(&f, mem, sizeof mem, &m);
    CHECK_ST(pe_bind_imports(&m, fake_resolve, &(struct rctx){0}, &e), PE_ERR_UNRESOLVED);
    CHECK(!strcmp(e.symbol, "NopeFunc"));

    /* unknown api set */
    build_pe(file, is64, 1);
    pe_open(&f, file, sz, PE_LAYOUT_FILE); pe_map(&f, mem, sizeof mem, &m);
    CHECK_ST(pe_bind_imports(&m, fake_resolve, &(struct rctx){0}, &e), PE_ERR_APISET);
    CHECK(!strcmp(e.dll, "api-ms-win-bogus-l1-1-0.dll"));
}

static void test_rejects(void)
{
    static uint8_t file[0xe00], mem[IMG_SIZE];
    pe_image f;
    size_t sz = build_pe(file, 1, 0), i;
    uint8_t save;

    for (i = 0; i < 0x200; i++) { /* every truncation inside headers must fail cleanly */
        pe_status s = pe_open(&f, file, i, PE_LAYOUT_FILE);
        CHECK(s != PE_OK);
    }
    CHECK_ST(pe_open(&f, file, sz - 1, PE_LAYOUT_FILE), PE_ERR_TRUNCATED);   /* .reloc raw cut */

    save = file[0]; file[0] = 'X'; CHECK_ST(pe_open(&f, file, sz, PE_LAYOUT_FILE), PE_ERR_BAD_MAGIC); file[0] = save;
    w16(file + 0x44, 0x1c0); CHECK_ST(pe_open(&f, file, sz, PE_LAYOUT_FILE), PE_ERR_UNSUPPORTED); w16(file + 0x44, 0x8664);
    w32(file + 0x40 + 24 + 32, 0x1800); CHECK_ST(pe_open(&f, file, sz, PE_LAYOUT_FILE), PE_ERR_BAD_FORMAT); /* align vs size_of_image */
    w32(file + 0x40 + 24 + 32, 0x1000);
    w32(file + 0x40 + 24 + 56, 0x2000); CHECK(pe_open(&f, file, sz, PE_LAYOUT_FILE) != PE_OK);              /* sections past image */
    w32(file + 0x40 + 24 + 56, IMG_SIZE);
    w32(file + 0x3c, 0xfffffff0u); CHECK_ST(pe_open(&f, file, sz, PE_LAYOUT_FILE), PE_ERR_TRUNCATED);
    w32(file + 0x3c, 0x40);

    CHECK_ST(pe_open(&f, file, sz, PE_LAYOUT_FILE), PE_OK);
    CHECK_ST(pe_map(&f, mem, 0x100, &(pe_image){0}), PE_ERR_BUFFER_TOO_SMALL);
    CHECK(pe_rva_ptr(&f, 0x1000, 0x201) == NULL);          /* past .text raw */
    CHECK(pe_rva_ptr(&f, 0x1ff0, 0x20) == NULL);           /* gap */
    CHECK(pe_rva_ptr(&f, 0xfffffff0u, 0x20) == NULL);      /* overflow */
}

/* ---------- real binaries ---------- */
static int count_cb(void *ctx, const pe_import_ref *r) { (void)r; (*(int *)ctx)++; return 0; }

static uint8_t *slurp(const char *p, size_t *n)
{
    FILE *fp = fopen(p, "rb"); uint8_t *b; long l;
    if (!fp) return NULL;
    fseek(fp, 0, SEEK_END); l = ftell(fp); fseek(fp, 0, SEEK_SET);
    b = malloc(l); if (b && fread(b, 1, l, fp) != (size_t)l) { free(b); b = NULL; }
    fclose(fp); *n = l; return b;
}

static void test_real(const char *path)
{
    size_t n; uint8_t *file = slurp(path, &n), *mem;
    pe_image f, m;
    int imports = 0, found = 0, total = 0;
    pe_status s;

    if (!file) { printf("skip %s (not readable)\n", path); return; }
    s = pe_open(&f, file, n, PE_LAYOUT_FILE);
    CHECK_ST(s, PE_OK);
    if (s) { free(file); return; }
    mem = malloc(f.size_of_image);
    CHECK_ST(pe_map(&f, mem, f.size_of_image, &m), PE_OK);
    CHECK_ST(pe_enum_imports(&m, count_cb, &imports), PE_OK);
    CHECK_ST(pe_enum_imports(&f, count_cb, &(int){0}), PE_OK);          /* file layout agrees */
    {
        int delayed = 0; pe_tls t; pe_status ts;
        CHECK_ST(pe_enum_delay_imports(&m, count_cb, &delayed), PE_OK);
        CHECK_ST(pe_enum_bound_imports(&m, NULL, NULL), PE_OK);
        ts = pe_get_tls(&m, &t);
        CHECK(ts == PE_OK || ts == PE_ERR_NOT_FOUND);
        printf("  %s: %d delay imports, tls %s\n", path, delayed, ts == PE_OK ? "present" : "none");
    }

    /* every named export must be findable via the sorted-table binary search */
    {
        pe_dir d = m.dir[PE_DIR_EXPORT];
        if (d.rva) {
            const uint8_t *ed = pe_rva_ptr(&m, d.rva, 40);
            uint32_t nn = r32(ed + 24), i;
            const uint8_t *names = pe_rva_ptr(&m, r32(ed + 32), nn * 4ul);
            for (i = 0; i < nn; i++) {
                char nm[256]; pe_export ex;
                if (pe_rva_string(&m, r32(names + i * 4), nm, sizeof nm)) { CHECK(0); continue; }
                total++;
                if (pe_find_export(&m, nm, 0, &ex) == PE_OK) found++;
            }
        }
    }
    CHECK(found == total);
    printf("  %s: %s, %u sections, %d imports, %d/%d exports found\n", path, f.is64 ? "PE32+" : "PE32", f.nsections, imports, found, total);

    /* rebase to an arbitrary base and back must restore the image bytes */
    {
        uint8_t *copy = malloc(f.size_of_image);
        uint64_t orig = m.image_base;
        memcpy(copy, mem, f.size_of_image);
        s = pe_relocate(&m, f.is64 ? 0x7ffd12340000ull : 0x6a000000);
        CHECK(s == PE_OK || s == PE_ERR_NO_RELOCS);
        if (s == PE_OK) {
            CHECK_ST(pe_relocate(&m, orig), PE_OK);
            CHECK(!memcmp(copy, mem, f.size_of_image));
        }
        free(copy);
    }
    free(mem); free(file);
}

/* ---------- hostile input: deterministic mutation fuzz ---------- */
static uint32_t rng = 0x12345678;
static uint32_t rnd(void) { rng ^= rng << 13; rng ^= rng >> 17; rng ^= rng << 5; return rng; }

static int sink(void *c, const pe_import_ref *r) { (void)c; (void)r; return 0; }
static int rsink(void *c, const pe_import_ref *r, uint64_t *a) { (void)c; (void)r; *a = 1; return 0; }

static void fuzz_one(uint8_t *buf, size_t n)
{
    pe_image f, m;
    uint8_t *mem;
    pe_export ex;
    if (pe_open(&f, buf, n, PE_LAYOUT_FILE)) return;
    mem = f.size_of_image <= (1u << 24) ? malloc(f.size_of_image) : NULL;   /* mutated SizeOfImage can be 4GB */
    if (mem && !pe_map(&f, mem, f.size_of_image, &m)) {
        pe_enum_imports(&m, sink, NULL);
        pe_bind_imports(&m, rsink, NULL, NULL);
        { uint32_t rr, rs; pe_find_resource(&m, PE_RT_MANIFEST, 1, &rr, &rs); }
        pe_enum_delay_imports(&m, sink, NULL);
        pe_bind_delay_imports(&m, rsink, NULL, NULL);
        pe_enum_bound_imports(&m, NULL, NULL);
        { pe_tls t; if (!pe_get_tls(&m, &t)) { uint8_t blk[256]; pe_tls_init_block(&m, &t, blk, sizeof blk); } }
        pe_find_export(&m, "Alpha", 0, &ex);
        pe_find_export(&m, NULL, 1, &ex);
        pe_relocate(&m, m.is64 ? 0x140000000ull : 0x20000000);
    }
    pe_enum_imports(&f, sink, NULL);
    pe_enum_delay_imports(&f, sink, NULL);
    { pe_tls t; pe_get_tls(&f, &t); }
    pe_find_export(&f, "x", 0, &ex);
    free(mem);
}

static void test_fuzz(void)
{
    static uint8_t base[0xe00], work[0xe00];
    int iter, is64;
    for (is64 = 0; is64 < 2; is64++) {
        size_t sz = build_pe(base, is64, 0);
        for (iter = 0; iter < 20000; iter++) {
            int k, nflip = 1 + rnd() % 6;
            memcpy(work, base, sz);
            for (k = 0; k < nflip; k++) {
                /* bias flips toward the header/table bytes */
                size_t off = (rnd() & 3) ? rnd() % 0x2a0 : 0x600 + rnd() % 0x700;
                work[off] = (uint8_t)rnd();
            }
            fuzz_one(work, (rnd() & 7) ? sz : rnd() % sz);
        }
    }
}

int main(int argc, char **argv)
{
    int i;
    test_synthetic(0);
    test_synthetic(1);
    test_rejects();
    test_fuzz();
    for (i = 1; i < argc; i++) test_real(argv[i]);
    if (fails) printf("pe: %d FAILED\n", fails); else printf("pe: all passed\n");
    return fails != 0;
}
