/* Loads the *real* linked ntdll.dll / kernel32.dll (scripts/win32/build-pe.sh) with the LoOS
 * loader on the host, forcing relocation, then calls pure code inside the loaded ntdll through
 * the Windows x64 ABI. Proves: linker output parses, relocations apply, kernel32's imports are
 * satisfied by ntdll's exports, and the generated machine code runs. */
#include "ldr.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)
#define CHECK_ST(e, want) do { ldr_status s_ = (e); if (s_ != (want)) { printf("FAIL %s:%d: %s = %d, want %d\n", __FILE__, __LINE__, #e, s_, want); fails++; } } while (0)

static const char *dir;
static int image_bytes_executable = 1;

static int be_open(void *c, const char *name, const char *path, const void **d, size_t *n, void **ck)
{
    char p[512]; FILE *f; long l; void *b;
    (void)c; (void)path;
    snprintf(p, sizeof p, "%s/%s", dir, name);
    if (!(f = fopen(p, "rb"))) return 0;
    fseek(f, 0, SEEK_END); l = ftell(f); fseek(f, 0, SEEK_SET);
    b = malloc((size_t)l);
    if (!b || fread(b, 1, (size_t)l, f) != (size_t)l) { fclose(f); free(b); return 0; }
    fclose(f);
    *d = b; *n = (size_t)l; *ck = b;
    return 1;
}
static void be_release(void *c, void *ck) { (void)c; free(ck); }
static int be_alloc(void *c, uint64_t pref, size_t size, int fixed, void **mem, uint64_t *addr)
{
    void *p;
    (void)c; (void)pref;
    if (fixed) return 0;                                    /* never the preferred base: always exercises relocation */
    p = mmap(NULL, size, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) { image_bytes_executable = 0; p = calloc(1, size); if (!p) return 0; }
    *mem = p; *addr = (uint64_t)(uintptr_t)p;
    return 1;
}
static void be_free(void *c, void *mem, uint64_t addr, size_t size) { (void)c; (void)addr; if (image_bytes_executable) munmap(mem, size); else free(mem); }
static int be_call(void *c, uint64_t fn, uint64_t b, uint32_t r, void *x) { (void)c; (void)fn; (void)b; (void)r; (void)x; return 1; }

/* Windows x64 ABI types for calling into the loaded image */
#define MS __attribute__((ms_abi))
typedef struct { uint32_t size, major, minor, build, platform; uint16_t csd[128]; uint16_t spmaj, spmin, suite; uint8_t ptype, res; } OSVEX;
typedef uint32_t (MS *fn_status_to_dos)(int32_t);
typedef int32_t (MS *fn_getver)(OSVEX *);
typedef int32_t (MS *fn_verify)(OSVEX *, uint32_t, uint64_t);
typedef uint64_t (MS *fn_setmask)(uint64_t, uint32_t, uint8_t);

int main(int argc, char **argv)
{
    ldr_context c; ldr_backend be;
    ldr_module *k32, *nt, *kb;
    ldr_error e;
    uint64_t a;
    unsigned i;

    if (argc < 2) { printf("usage: artifacts_test <dir with ntdll.dll kernel32.dll>\n"); return 2; }
    dir = argv[1];
    memset(&be, 0, sizeof be);
    be.open_module = be_open; be.release_file = be_release; be.alloc_image = be_alloc; be.free_image = be_free; be.call_entry = be_call;
    ldr_init(&c, &be, 1);

    CHECK_ST(ldr_load_dll(&c, "kernel32.dll", &k32, &e), LDR_OK);
    if (!k32) { printf("  load failed: %s %s\n", e.module, e.symbol); printf("artifacts: FAILED\n"); return 1; }
    CHECK(ldr_get_module(&c, "kernelbase.dll") == NULL);                  /* kernel32 is pure forwarders: nothing loaded yet */
    /* following a forwarder pulls in kernelbase, which pulls in ntdll */
    CHECK_ST(ldr_get_proc(&c, k32, "CreateFileW", 0, &a, &e), LDR_OK);
    kb = ldr_get_module(&c, "kernelbase.dll");
    nt = ldr_get_module(&c, "ntdll.dll");
    CHECK(kb != NULL && nt != NULL);
    if (!kb || !nt) { printf("artifacts: FAILED\n"); return 1; }
    CHECK(a > kb->addr && a < kb->addr + kb->size);                       /* resolved inside kernelbase */
    CHECK(kb->img.image_base == kb->addr && nt->img.image_base == nt->addr);   /* relocated to the mmap address */
    k32 = kb;                                                             /* the checks below inspect the implementation */

    /* every export resolves to an address inside its own image */
    CHECK_ST(ldr_get_proc(&c, k32, "CreateFileW", 0, &a, &e), LDR_OK);
    CHECK(a > k32->addr && a < k32->addr + k32->size);
    CHECK_ST(ldr_get_proc(&c, k32, "GetProcAddress", 0, &a, &e), LDR_OK);
    CHECK_ST(ldr_get_proc(&c, k32, "NoSuchApi", 0, &a, &e), LDR_ERR_PROC_NOT_FOUND);
    CHECK_ST(ldr_get_proc(&c, nt, "NtCreateFile", 0, &a, &e), LDR_OK);
    {   /* the syscall stub bytes: mov r10,rcx ; mov eax,N ; syscall ; ret */
        const uint8_t *p = (const uint8_t *)(uintptr_t)a;
        CHECK(p[0] == 0x49 && p[1] == 0x89 && p[2] == 0xca && p[3] == 0xb8 && p[8] == 0x0f && p[9] == 0x05 && p[10] == 0xc3);
    }
    {   /* kernel32's imports are only ntdll's exports: every Nt, Rtl and Ldr function it declares must exist */
        pe_dir d = k32->img.dir[PE_DIR_IMPORT];
        CHECK(d.rva != 0);
        for (i = 0; ; i++) {
            const uint8_t *desc = pe_rva_ptr(&k32->img, d.rva + i * 20, 20);
            char nm[64];
            if (!desc || !(desc[0] | desc[1] | desc[2] | desc[3] | desc[12] | desc[13] | desc[14] | desc[15])) break;
            pe_rva_string(&k32->img, desc[12] | desc[13] << 8 | desc[14] << 16 | (uint32_t)desc[15] << 24, nm, sizeof nm);
            CHECK(!strcasecmp(nm, "ntdll.dll"));
        }
        CHECK(i == 1);
    }

    if (image_bytes_executable) {
        fn_setmask setmask; fn_getver getver; fn_verify verify; fn_status_to_dos s2d;
        OSVEX v, want;
        uint64_t m;
        ldr_get_proc(&c, nt, "VerSetConditionMask", 0, &a, &e); setmask = (fn_setmask)(uintptr_t)a;
        ldr_get_proc(&c, nt, "RtlGetVersion", 0, &a, &e); getver = (fn_getver)(uintptr_t)a;
        ldr_get_proc(&c, nt, "RtlVerifyVersionInfo", 0, &a, &e); verify = (fn_verify)(uintptr_t)a;
        ldr_get_proc(&c, nt, "RtlNtStatusToDosError", 0, &a, &e); s2d = (fn_status_to_dos)(uintptr_t)a;

        CHECK(s2d((int32_t)0xC0000034u) == 2);                  /* OBJECT_NAME_NOT_FOUND -> FILE_NOT_FOUND */
        CHECK(s2d(0) == 0);
        memset(&v, 0, sizeof v); v.size = sizeof v;
        CHECK(getver(&v) == 0);
        CHECK(v.major == 10 && v.build >= 22000 && v.platform == 2);                    /* native era: Windows 11 */
        memset(&want, 0, sizeof want); want.size = sizeof want; want.major = 10;
        m = setmask(0, 2 /*VER_MAJORVERSION*/, 3 /*GREATER_EQUAL*/);
        CHECK(verify(&want, 2, m) == 0);
        want.major = 11; CHECK((uint32_t)verify(&want, 2, m) == 0xC0000059u);           /* REVISION_MISMATCH */
        {   /* more of the freshly linked ntdll: C runtime exports, counted strings, bitmaps */
            typedef int (MS *f_wcsicmp)(const uint16_t *, const uint16_t *);
            typedef int (MS *f_towupper)(unsigned);
            typedef size_t (MS *f_strlen)(const char *);
            typedef void *(MS *f_memcpy)(void *, const void *, size_t);
            typedef struct { uint32_t bits; uint32_t *buf; } BM;
            typedef void (MS *f_initbm)(BM *, uint32_t *, uint32_t);
            typedef uint32_t (MS *f_findset)(BM *, uint32_t, uint32_t);
            typedef uint32_t (MS *f_count)(BM *);
            static const uint16_t a1[] = { 'H', 0xE9, 'Y', 0 }, a2[] = { 'h', 0xC9, 'y', 0 }, a3[] = { 'h', 0xC9, 'z', 0 };
            char dst[8] = {0};
            uint32_t words[4] = {0};
            BM bm;
            ldr_get_proc(&c, nt, "_wcsicmp", 0, &a, &e); CHECK(((f_wcsicmp)(uintptr_t)a)(a1, a2) == 0 && ((f_wcsicmp)(uintptr_t)a)(a1, a3) < 0);
            ldr_get_proc(&c, nt, "towupper", 0, &a, &e); CHECK(((f_towupper)(uintptr_t)a)(0x43f) == 0x41f);
            ldr_get_proc(&c, nt, "strlen", 0, &a, &e); CHECK(((f_strlen)(uintptr_t)a)("loos") == 4);
            ldr_get_proc(&c, nt, "memcpy", 0, &a, &e); ((f_memcpy)(uintptr_t)a)(dst, "abc", 4); CHECK(!strcmp(dst, "abc"));
            ldr_get_proc(&c, nt, "RtlInitializeBitMap", 0, &a, &e); ((f_initbm)(uintptr_t)a)(&bm, words, 100);
            ldr_get_proc(&c, nt, "RtlSetBits", 0, &a, &e); ((void (MS *)(BM *, uint32_t, uint32_t))(uintptr_t)a)(&bm, 0, 10);
            ldr_get_proc(&c, nt, "RtlFindClearBitsAndSet", 0, &a, &e); CHECK(((f_findset)(uintptr_t)a)(&bm, 5, 0) == 10);
            ldr_get_proc(&c, nt, "RtlNumberOfSetBits", 0, &a, &e); CHECK(((f_count)(uintptr_t)a)(&bm) == 15);
            {   char out[64];                                 /* the printf family from the linked image, variadic, with a double */
                ldr_get_proc(&c, nt, "sprintf", 0, &a, &e);
                CHECK(((int (MS *)(char *, const char *, ...))(uintptr_t)a)(out, "%s=%d %.2f %x", "n", -3, 2.675, 255u) == 12 && !strcmp(out, "n=-3 2.67 ff"));
            }
        }
        printf("  executed code from the loaded ntdll.dll image at %#llx\n", (unsigned long long)nt->addr);
    } else printf("  (mmap RWX unavailable: skipped execution checks)\n");

    CHECK_ST(ldr_unload_dll(&c, ldr_get_module(&c, "kernel32.dll")), LDR_OK);
    CHECK(ldr_get_module(&c, "kernel32.dll") == NULL && ldr_get_module(&c, "kernelbase.dll") == NULL && ldr_get_module(&c, "ntdll.dll") == NULL);

    if (fails) printf("artifacts: %d FAILED\n", fails); else printf("artifacts: all passed\n");
    return fails != 0;
}
