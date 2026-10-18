#include "ldr.h"
#include "../common/pebuild.h"
#include "compat.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)
#define CHECK_EQ(a, b) do { unsigned long long a_ = (unsigned long long)(a), b_ = (unsigned long long)(b); \
    if (a_ != b_) { printf("FAIL %s:%d: %s = %llu, want %llu\n", __FILE__, __LINE__, #a, a_, b_); fails++; } } while (0)
#define CHECK_ST(e, want) do { ldr_status s_ = (e); if (s_ != (want)) { printf("FAIL %s:%d: %s = %d, want %d\n", __FILE__, __LINE__, #e, s_, want); fails++; } } while (0)

/* ---------- fake OS backend ---------- */
#define MAXF 32
#define MAXEV 64
typedef struct { char name[64]; bimage img; } vfile;
typedef struct { uint64_t addr; size_t size; void *mem; } vmap;
typedef struct { uint64_t fn, base; uint32_t reason; int exiting; } event;

typedef struct {
    vfile files[MAXF]; int nfiles;
    vmap maps[64]; int nmaps; int allocs, frees;
    uint64_t next_free;
    event ev[MAXEV]; int nev;
    uint64_t fail_attach_base;        /* DllMain(ATTACH) returns FALSE for this base */
} world;

static world W;

static void add_file(const char *name, const bspec *s)
{
    vfile *v = &W.files[W.nfiles++];
    strcpy(v->name, name);
    bimage_build(s, &v->img);
}

static int be_open(void *c, const char *name, const char *path, const void **d, size_t *n, void **ck)
{
    int i; (void)c; (void)path;
    for (i = 0; i < W.nfiles; i++) if (!strcmp(W.files[i].name, name)) { *d = W.files[i].img.data; *n = W.files[i].img.size; *ck = &W.files[i]; return 1; }
    return 0;
}
static void be_release(void *c, void *ck) { (void)c; (void)ck; }

static int overlaps(uint64_t a, size_t n)
{
    int i;
    for (i = 0; i < W.nmaps; i++)
        if (W.maps[i].mem && a < W.maps[i].addr + W.maps[i].size && W.maps[i].addr < a + n) return 1;
    return 0;
}
static int be_alloc(void *c, uint64_t pref, size_t size, int fixed, void **mem, uint64_t *addr)
{
    uint64_t a = pref; (void)c;
    if (overlaps(a, size)) {
        if (fixed) return 0;
        a = W.next_free; W.next_free += (size + 0xffff) & ~(uint64_t)0xffff;
    }
    *mem = calloc(1, size); *addr = a;
    W.maps[W.nmaps].addr = a; W.maps[W.nmaps].size = size; W.maps[W.nmaps].mem = *mem; W.nmaps++;
    W.allocs++;
    return 1;
}
static void be_free(void *c, void *mem, uint64_t addr, size_t size)
{
    int i; (void)c; (void)size;
    for (i = 0; i < W.nmaps; i++) if (W.maps[i].mem == mem && W.maps[i].addr == addr) { W.maps[i].mem = NULL; break; }
    free(mem); W.frees++;
}
static int be_call(void *c, uint64_t fn, uint64_t base, uint32_t reason, void *reserved)
{
    (void)c;
    if (W.nev < MAXEV) W.ev[W.nev++] = (event){ fn, base, reason, reserved != NULL };
    if (reason == LDR_REASON_PROCESS_ATTACH && base == W.fail_attach_base && fn == base + 0x1000) return 0;
    return 1;
}

static void world_reset(void)
{
    int i;
    for (i = 0; i < W.nfiles; i++) bimage_free(&W.files[i].img);
    for (i = 0; i < W.nmaps; i++) free(W.maps[i].mem);
    memset(&W, 0, sizeof W);
    W.next_free = 0x7ff800000000ull;
}

static ldr_context CTX;
static void start(int is64)
{
    ldr_backend be = { NULL, be_open, be_release, be_alloc, be_free, be_call, NULL, NULL, NULL };
    ldr_init(&CTX, &be, is64);
}

static bspec dll64(uint64_t base)
{
    bspec s; memset(&s, 0, sizeof s);
    s.is64 = 1; s.is_dll = 1; s.entry = 1; s.relocs = 1; s.base = base;
    return s;
}
static void addexp(bspec *s, const char *name, const char *fwd)
{ s->exports[s->nexports].name = name; s->exports[s->nexports].forward = fwd; s->nexports++; }
static void imp(bspec *s, const char *dll, const char *a, const char *b, const char *c)
{
    bimport *im = &s->imports[s->nimports++];
    int n = 0;
    im->dll = dll;
    if (a) im->syms[n++] = a;
    if (b) im->syms[n++] = b;
    if (c) im->syms[n++] = c;
    im->syms[n] = NULL;
}
static uint64_t rd_ptr(ldr_module *m, uint32_t rva, int is64)
{
    uint8_t *p = (uint8_t *)m->mem + rva;
    uint64_t v = p[0] | p[1] << 8 | p[2] << 16 | (uint64_t)p[3] << 24;
    if (is64) v |= (uint64_t)(p[4] | p[5] << 8 | p[6] << 16 | (uint64_t)p[7] << 24) << 32;
    return v;
}
static int count_ev(uint32_t reason, uint64_t base)
{ int i, n = 0; for (i = 0; i < W.nev; i++) if (W.ev[i].reason == reason && W.ev[i].base == base && W.ev[i].fn == base + 0x1000) n++; return n; }
static int first_ev(uint32_t reason, uint64_t base)
{ int i; for (i = 0; i < W.nev; i++) if (W.ev[i].reason == reason && W.ev[i].base == base && W.ev[i].fn == base + 0x1000) return i; return -1; }

/* ---------- tests ---------- */
static void test_graph(void)
{
    bspec ntdll = dll64(0x180000000ull), kb = dll64(0x180100000ull), k32 = dll64(0x180200000ull), app;
    ldr_module *a, *nt, *kbm, *k;
    ldr_error e;
    bimage probe;
    uint64_t addr, expect;

    world_reset(); start(1);
    addexp(&ntdll, "NtFoo", NULL); addexp(&ntdll, "RtlBar", NULL);
    add_file("ntdll.dll", &ntdll);
    addexp(&kb, "CreateFileW", NULL); imp(&kb, "ntdll.dll", "NtFoo", "#2", NULL);
    add_file("kernelbase.dll", &kb);
    addexp(&k32, "CreateFileW", "KERNELBASE.CreateFileW"); addexp(&k32, "ByOrd", "ntdll.#2"); addexp(&k32, "Local", NULL);
    imp(&k32, "ntdll.dll", "NtFoo", NULL, NULL);
    add_file("kernel32.dll", &k32);

    memset(&app, 0, sizeof app); app.is64 = 1; app.relocs = 1; app.base = 0x140000000ull;
    imp(&app, "api-ms-win-core-file-l1-2-1.dll", "CreateFileW", NULL, NULL);
    imp(&app, "KERNEL32.dll", "CreateFileW", "ByOrd", "Local");
    add_file("app.exe", &app);
    bimage_build(&app, &probe);                         /* only for IAT rvas */

    CHECK_ST(ldr_load_exe(&CTX, "C:\\Apps\\APP.EXE", &a, &e), LDR_OK);
    if (!a) { bimage_free(&probe); return; }
    nt = ldr_get_module(&CTX, "ntdll"); kbm = ldr_get_module(&CTX, "KernelBase.dll"); k = ldr_get_module(&CTX, "kernel32.dll");
    CHECK(nt && kbm && k);
    CHECK(ldr_get_module(&CTX, "api-ms-win-core-file-l1-2-0.dll") == kbm);      /* API set -> same module */
    CHECK(ldr_get_module(&CTX, "nonexistent.dll") == NULL);
    if (!(nt && kbm && k)) { bimage_free(&probe); return; }

    /* IAT contents: api-set import and forwarded imports all land on the real function */
    expect = kbm->addr + B_FUNC_RVA(0);
    CHECK(rd_ptr(a, probe.iat_rva[0][0], 1) == expect);
    CHECK(rd_ptr(a, probe.iat_rva[1][0], 1) == expect);                         /* kernel32 -> kernelbase forward */
    CHECK(rd_ptr(a, probe.iat_rva[1][1], 1) == nt->addr + B_FUNC_RVA(1));       /* forward to ntdll ordinal 2 */
    CHECK(rd_ptr(a, probe.iat_rva[1][2], 1) == k->addr + B_FUNC_RVA(2));
    /* kernelbase's own import of ntdll by name and by ordinal */
    {
        bimage kbp; bimage_build(&kb, &kbp);
        CHECK(rd_ptr(kbm, kbp.iat_rva[0][0], 1) == nt->addr + B_FUNC_RVA(0));
        CHECK(rd_ptr(kbm, kbp.iat_rva[0][1], 1) == nt->addr + B_FUNC_RVA(1));
        bimage_free(&kbp);
    }
    /* relocated? all preferred bases were free, so pointers stay */
    CHECK(rd_ptr(nt, B_PTR_RVA, 1) == nt->addr + B_FUNC_RVA(0));

    /* initialisation: each DLL attached exactly once, dependencies first, exe entry not called */
    CHECK(count_ev(1, nt->addr) == 1 && count_ev(1, kbm->addr) == 1 && count_ev(1, k->addr) == 1);
    CHECK(first_ev(1, nt->addr) < first_ev(1, kbm->addr));
    CHECK(first_ev(1, kbm->addr) < first_ev(1, k->addr) || first_ev(1, nt->addr) < first_ev(1, k->addr));
    CHECK(count_ev(1, a->addr) == 0);
    CHECK(ldr_entry_point(a) == a->addr + 0x1000);

    /* GetProcAddress through forwarders, by name and ordinal */
    CHECK_ST(ldr_get_proc(&CTX, k, "CreateFileW", 0, &addr, &e), LDR_OK);
    CHECK(addr == expect);
    CHECK_ST(ldr_get_proc(&CTX, k, NULL, 2, &addr, &e), LDR_OK);
    CHECK(addr == nt->addr + B_FUNC_RVA(1));
    CHECK_ST(ldr_get_proc(&CTX, k, "Nope", 0, &addr, &e), LDR_ERR_PROC_NOT_FOUND);
    CHECK(!strcmp(e.module, "kernel32.dll") && !strcmp(e.symbol, "Nope"));
    CHECK_ST(ldr_get_proc(&CTX, k, NULL, 99, &addr, &e), LDR_ERR_ORDINAL_NOT_FOUND);
    CHECK(!strcmp(e.symbol, "#99"));

    /* LoadLibrary/FreeLibrary counting: dependency edges keep it alive */
    {
        ldr_module *k2;
        CHECK_ST(ldr_load_dll(&CTX, "Kernel32", &k2, &e), LDR_OK);
        CHECK(k2 == k && k->explicit_refs == 1);
        CHECK_ST(ldr_unload_dll(&CTX, k), LDR_OK);
        CHECK(ldr_get_module(&CTX, "kernel32.dll") == k);       /* still imported by the exe */
        CHECK_ST(ldr_unload_dll(&CTX, k), LDR_ERR_INVALID_PARAMETER);   /* no explicit ref left */
        CHECK_ST(ldr_unload_dll(&CTX, a), LDR_ERR_INVALID_PARAMETER);   /* EXE can't be unloaded */
    }
    /* exe/dll mix-ups */
    {
        bspec o; ldr_module *tmp;
        memset(&o, 0, sizeof o); o.is64 = 1; o.relocs = 1; o.base = 0x140100000ull;
        add_file("other.exe", &o);
        { bspec sp = dll64(0x180400000ull); addexp(&sp, "S", NULL); add_file("spare.dll", &sp); }
        CHECK_ST(ldr_load_dll(&CTX, "other.exe", &tmp, &e), LDR_ERR_BAD_IMAGE);
        CHECK_ST(ldr_load_exe(&CTX, "spare.dll", &tmp, &e), LDR_ERR_BAD_IMAGE);
        CHECK(ldr_get_module(&CTX, "other.exe") == NULL && ldr_get_module(&CTX, "spare.dll") == NULL);
    }

    /* standalone DLL: attach on load, detach + free on unload, deps stay */
    {
        bspec ex = dll64(0x180300000ull); ldr_module *x; int before_frees = W.frees;
        uint64_t xbase;
        addexp(&ex, "Do", NULL); imp(&ex, "ntdll.dll", "NtFoo", NULL, NULL);
        add_file("extra.dll", &ex);
        CHECK_ST(ldr_load_dll(&CTX, "extra", &x, &e), LDR_OK);
        xbase = x->addr;
        CHECK(count_ev(1, xbase) == 1);
        CHECK_ST(ldr_unload_dll(&CTX, x), LDR_OK);
        CHECK(count_ev(0, xbase) == 1 && ldr_get_module(&CTX, "extra.dll") == NULL);
        CHECK(W.frees == before_frees + 1);
        CHECK(ldr_get_module(&CTX, "ntdll.dll") == nt);
    }
    bimage_free(&probe);
}

static void test_cycle(void)
{
    bspec a = dll64(0x180000000ull), b = dll64(0x180100000ull);
    ldr_module *ma; ldr_error e; uint64_t abase, bbase;

    world_reset(); start(1);
    addexp(&a, "A", NULL); imp(&a, "b.dll", "B", NULL, NULL);
    addexp(&b, "B", NULL); imp(&b, "a.dll", "A", NULL, NULL);
    add_file("a.dll", &a); add_file("b.dll", &b);
    CHECK_ST(ldr_load_dll(&CTX, "a.dll", &ma, &e), LDR_OK);
    if (!ma) return;
    abase = ma->addr; bbase = ldr_get_module(&CTX, "b.dll")->addr;
    CHECK(count_ev(1, abase) == 1 && count_ev(1, bbase) == 1);
    CHECK_ST(ldr_unload_dll(&CTX, ma), LDR_OK);                         /* the whole cycle must be collected */
    CHECK(ldr_get_module(&CTX, "a.dll") == NULL && ldr_get_module(&CTX, "b.dll") == NULL);
    CHECK(count_ev(0, abase) == 1 && count_ev(0, bbase) == 1);
    CHECK(W.allocs == W.frees);
}

static void test_failures(void)
{
    ldr_module *m; ldr_error e;
    bspec x = dll64(0x180000000ull), y = dll64(0x180100000ull), nt = dll64(0x180200000ull);

    /* missing DLL */
    world_reset(); start(1);
    imp(&x, "nope.dll", "F", NULL, NULL); addexp(&x, "X", NULL);
    add_file("x.dll", &x);
    CHECK_ST(ldr_load_dll(&CTX, "x.dll", &m, &e), LDR_ERR_DLL_NOT_FOUND);
    CHECK(!strcmp(e.module, "nope.dll"));
    CHECK(ldr_get_module(&CTX, "x.dll") == NULL && W.allocs == W.frees);
    CHECK_ST(ldr_load_dll(&CTX, "", &m, &e), LDR_ERR_INVALID_PARAMETER);

    /* missing symbol; the half-loaded dependency chain must not leak */
    world_reset(); start(1);
    addexp(&nt, "Have", NULL); add_file("ntdll.dll", &nt);
    memset(&y, 0, sizeof y); y = dll64(0x180100000ull); addexp(&y, "Y", NULL); imp(&y, "ntdll.dll", "Missing", NULL, NULL);
    add_file("y.dll", &y);
    CHECK_ST(ldr_load_dll(&CTX, "y.dll", &m, &e), LDR_ERR_PROC_NOT_FOUND);
    CHECK(!strcmp(e.module, "ntdll.dll") && !strcmp(e.symbol, "Missing"));
    CHECK(ldr_get_module(&CTX, "ntdll.dll") == NULL && W.allocs == W.frees && W.allocs == 2);

    /* ordinal missing */
    world_reset(); start(1);
    nt = dll64(0x180200000ull); addexp(&nt, "Have", NULL); add_file("ntdll.dll", &nt);
    y = dll64(0x180100000ull); addexp(&y, "Y", NULL); imp(&y, "ntdll.dll", "#9", NULL, NULL); add_file("y.dll", &y);
    CHECK_ST(ldr_load_dll(&CTX, "y.dll", &m, &e), LDR_ERR_ORDINAL_NOT_FOUND);

    /* unknown API set */
    world_reset(); start(1);
    y = dll64(0x180100000ull); addexp(&y, "Y", NULL); imp(&y, "api-ms-win-bogus-l1-1-0.dll", "F", NULL, NULL); add_file("y.dll", &y);
    CHECK_ST(ldr_load_dll(&CTX, "y.dll", &m, &e), LDR_ERR_DLL_NOT_FOUND);
    CHECK(!strcmp(e.module, "api-ms-win-bogus-l1-1-0.dll"));

    /* DllMain(ATTACH) fails: earlier attaches are undone in reverse, everything freed */
    world_reset(); start(1);
    nt = dll64(0x180200000ull); addexp(&nt, "N", NULL); add_file("ntdll.dll", &nt);
    { bspec w = dll64(0x180300000ull); addexp(&w, "W", NULL); imp(&w, "ntdll.dll", "N", NULL, NULL); add_file("w.dll", &w); W.fail_attach_base = 0x180300000ull; }
    y = dll64(0x180100000ull); addexp(&y, "Y", NULL); imp(&y, "w.dll", "W", NULL, NULL); add_file("y.dll", &y);
    CHECK_ST(ldr_load_dll(&CTX, "y.dll", &m, &e), LDR_ERR_INIT_FAILED);
    CHECK(!strcmp(e.module, "w.dll"));
    CHECK(count_ev(1, 0x180200000ull) == 1 && count_ev(0, 0x180200000ull) == 1);   /* ntdll attached then detached */
    CHECK(count_ev(0, 0x180300000ull) == 0 && count_ev(1, 0x180100000ull) == 0);   /* failing DLL: no detach; importer never attached */
    CHECK(W.allocs == W.frees && ldr_get_module(&CTX, "ntdll.dll") == NULL);

    /* forwarder loop terminates */
    world_reset(); start(1);
    { bspec p = dll64(0x180000000ull), q = dll64(0x180100000ull);
      addexp(&p, "F", "q.F"); addexp(&q, "F", "p.F"); add_file("p.dll", &p); add_file("q.dll", &q);
      CHECK_ST(ldr_load_dll(&CTX, "p.dll", &m, &e), LDR_OK);
      { uint64_t a; CHECK_ST(ldr_get_proc(&CTX, m, "F", 0, &a, &e), LDR_ERR_PROC_NOT_FOUND); } }
}

static void test_rebase_and_machine(void)
{
    bspec a = dll64(0x180000000ull), b = dll64(0x180000000ull), c = dll64(0x180000000ull);
    ldr_module *ma, *mb, *mc; ldr_error e;

    world_reset(); start(1);
    addexp(&a, "A", NULL); addexp(&b, "B", NULL); add_file("a.dll", &a); add_file("b.dll", &b);
    CHECK_ST(ldr_load_dll(&CTX, "a.dll", &ma, &e), LDR_OK);
    CHECK_ST(ldr_load_dll(&CTX, "b.dll", &mb, &e), LDR_OK);
    if (!ma || !mb) return;
    CHECK(ma->addr == 0x180000000ull && mb->addr != ma->addr);                 /* b was moved */
    CHECK(rd_ptr(mb, B_PTR_RVA, 1) == mb->addr + B_FUNC_RVA(0));               /* ...and its pointer fixed up */
    CHECK(mb->img.image_base == mb->addr);

    c.relocs = 0; addexp(&c, "C", NULL); add_file("c.dll", &c);                   /* stripped + base taken */
    CHECK_ST(ldr_load_dll(&CTX, "c.dll", &mc, &e), LDR_ERR_CONFLICTING_ADDRESSES);
    { bspec p = dll64(0x180000000ull); p.relocs = 0; p.pic = 1; addexp(&p, "P", NULL); add_file("pic.dll", &p);   /* no table, not stripped: movable */
      CHECK_ST(ldr_load_dll(&CTX, "pic.dll", &mc, &e), LDR_OK);
      CHECK(mc && mc->addr != 0x180000000ull && mc->img.image_base == mc->addr); }

    /* 32-bit image in a 64-bit process */
    { bspec d = dll64(0x10000000); d.is64 = 0; addexp(&d, "D", NULL); add_file("d32.dll", &d); }
    CHECK_ST(ldr_load_dll(&CTX, "d32.dll", &mc, &e), LDR_ERR_MACHINE_MISMATCH);

    /* a full 32-bit process */
    world_reset(); start(0);
    { bspec n = dll64(0x10000000), u = dll64(0x10100000);
      n.is64 = u.is64 = 0; addexp(&n, "N1", NULL); addexp(&n, "N2", NULL);
      addexp(&u, "U", NULL); imp(&u, "ntdll.dll", "N2", "#1", NULL);
      add_file("ntdll.dll", &n); add_file("u.dll", &u);
      CHECK_ST(ldr_load_dll(&CTX, "u.dll", &mc, &e), LDR_OK);
      if (mc) {
          bimage up; ldr_module *nt = ldr_get_module(&CTX, "ntdll.dll");
          bimage_build(&u, &up);
          CHECK(nt && rd_ptr(mc, up.iat_rva[0][0], 0) == nt->addr + B_FUNC_RVA(1));
          CHECK(nt && rd_ptr(mc, up.iat_rva[0][1], 0) == nt->addr + B_FUNC_RVA(0));
          bimage_free(&up);
      } }
}

static void test_tls_and_threads(void)
{
    bspec t = dll64(0x180000000ull), t2 = dll64(0x180100000ull);
    ldr_module *m, *m2; ldr_error e; int i, before;

    world_reset(); start(1);
    t.tls = 1; addexp(&t, "T", NULL); add_file("t.dll", &t);
    t2.tls = 1; addexp(&t2, "T2", NULL); add_file("t2.dll", &t2);
    CHECK_ST(ldr_load_dll(&CTX, "t.dll", &m, &e), LDR_OK);
    CHECK_ST(ldr_load_dll(&CTX, "t2.dll", &m2, &e), LDR_OK);
    if (!m || !m2) return;
    CHECK(m->has_tls && m->tls_index == 0 && m2->tls_index == 1);
    CHECK(rd_ptr(m, B_TLS_INDEX_RVA, 0) == 0 && (rd_ptr(m2, B_TLS_INDEX_RVA, 0) & 0xffffffffu) == 1);
    /* TLS callback (fn = base+0x1010) runs on attach, before DllMain */
    {
        int cb = -1, dm = -1;
        for (i = 0; i < W.nev; i++) {
            if (W.ev[i].base != m->addr || W.ev[i].reason != 1) continue;
            if (W.ev[i].fn == m->addr + 0x1010) cb = i;
            if (W.ev[i].fn == m->addr + 0x1000) dm = i;
        }
        CHECK(cb >= 0 && dm >= 0 && cb < dm);
    }
    before = W.nev;
    ldr_thread_notify(&CTX, LDR_REASON_THREAD_ATTACH);
    ldr_thread_notify(&CTX, LDR_REASON_THREAD_DETACH);
    CHECK(W.nev - before == 8);                          /* 2 modules x (tls cb + DllMain) x 2 reasons */
    before = W.nev;
    ldr_shutdown(&CTX);
    CHECK(W.nev - before == 4);
    for (i = before; i < W.nev; i++) CHECK(W.ev[i].reason == 0 && W.ev[i].exiting);
    CHECK(W.ev[before].base == m2->addr);                /* reverse init order */
    CHECK(ldr_get_module(&CTX, "t.dll") == m);           /* shutdown frees nothing */
}

/* ---------- manifest + compat redirects ---------- */
static compat_db CDB;
static int be_redirect(void *c, const char *app, const char *dll, const char *name, char *td, size_t tds, char *ts, size_t tss)
{ (void)c; return compat_redirect_import(&CDB, app, dll, name, td, tds, ts, tss); }

static void test_compat_hooks(void)
{
    bspec nt = dll64(0x180000000ull), neu = dll64(0x180100000ull), app;
    ldr_module *a; ldr_error e; bimage probe;
    const char *xml; size_t xl; compat_era era;
    static const char manifest[] =
        "<assembly xmlns='urn:schemas-microsoft-com:asm.v1'><compatibility xmlns='urn:schemas-microsoft-com:compatibility.v1'><application>"
        "<supportedOS Id='{35138b9a-5d96-4fbd-8e2d-a2440225f93a}'/><supportedOS Id=\"{4A2F28E3-53B9-4441-BA9C-D69D4A4A6E38}\"/>"
        "</application></compatibility></assembly>";

    world_reset(); start(1);
    /* "old.dll" does not exist; imports of it are redirected: whole-DLL to new.dll, and one symbol renamed */
    addexp(&neu, "NewFunc", NULL); addexp(&neu, "Same", NULL); add_file("new.dll", &neu);
    addexp(&nt, "X", NULL); add_file("ntdll.dll", &nt);
    memset(&app, 0, sizeof app); app.is64 = 1; app.relocs = 1; app.base = 0x140000000ull; app.manifest = manifest;
    imp(&app, "old.dll", "OldFunc", "Same", NULL);
    imp(&app, "ntdll.dll", "X", NULL, NULL);
    add_file("legacy.exe", &app);
    bimage_build(&app, &probe);

    compat_db_init(&CDB);
    CHECK_EQ(compat_db_add_redirect(&CDB, "legacy.exe", "old.dll", "OldFunc", "new.dll", "NewFunc"), COMPAT_OK);
    CHECK_EQ(compat_db_add_redirect(&CDB, "*", "old.dll", "", "new.dll", ""), COMPAT_OK);

    /* without the hook the import fails */
    CHECK_ST(ldr_load_exe(&CTX, "C:\\Apps\\Legacy.EXE", &a, &e), LDR_ERR_DLL_NOT_FOUND);
    CHECK(!strcmp(e.module, "old.dll"));
    start(1); CTX.be.redirect_import = be_redirect;
    CHECK_ST(ldr_load_exe(&CTX, "C:\\Apps\\Legacy.EXE", &a, &e), LDR_OK);
    if (a) {
        ldr_module *nm = ldr_get_module(&CTX, "new.dll");
        CHECK(nm && rd_ptr(a, probe.iat_rva[0][0], 1) == nm->addr + B_FUNC_RVA(0));   /* OldFunc -> NewFunc (export 0) */
        CHECK(nm && rd_ptr(a, probe.iat_rva[0][1], 1) == nm->addr + B_FUNC_RVA(1));   /* Same: DLL-level redirect keeps the name */
        CHECK(ldr_get_module(&CTX, "old.dll") == NULL);
        CHECK_ST(ldr_get_manifest(a, &xml, &xl), LDR_OK);
        CHECK(xl == strlen(manifest) && !memcmp(xml, manifest, xl));
        CHECK(compat_manifest_era(xml, xl, &era) && era == COMPAT_ERA_WIN8);          /* highest of Win7+Win8 */
        { ldr_module *nt_m = ldr_get_module(&CTX, "ntdll.dll"); CHECK_ST(ldr_get_manifest(nt_m, &xml, &xl), LDR_ERR_PROC_NOT_FOUND); }
    }
    bimage_free(&probe);
}

int main(void)
{
    test_compat_hooks();
    test_graph();
    test_cycle();
    test_failures();
    test_rebase_and_machine();
    test_tls_and_threads();
    world_reset();
    CHECK(ldr_status_to_ntstatus(LDR_ERR_DLL_NOT_FOUND) == 0xC0000135u);
    if (fails) printf("ldr: %d FAILED\n", fails); else printf("ldr: all passed\n");
    return fails != 0;
}
