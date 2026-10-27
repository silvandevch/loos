/* app_compat — run a REAL, unmodified Win32 application on the LoOS compat
 * layer, on the host, with no Wine involved.
 *
 * The app (compat-hello.exe) imports GetStdHandle and WriteFile from
 * kernel32.dll. We load it with the LoOS PE loader (src/win32/ldr), redirect its
 * kernel32.dll imports to a shim DLL, and fill that shim's hooks with the LoOS
 * kernel32 implementations (src/win32/kernel32), compiled for the host. Those
 * call the LoOS Nt* surface (faked here, like the kernel32 unit tests) which
 * routes console output. Then we call the app's entry point with the Windows
 * x64 ABI and show what it printed.
 *
 *   usage: app_compat <dir with compat-hello.exe + k32shim.dll>
 */
#define LOOS_HOST_TEST 1
#include "fake.h"
#include "../../../src/win32/kernel32/k32_internal.h"
#include "../../../src/win32/compat/compat.h"
#include "ldr.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/mman.h>

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)

static const char *dir;

static int contains_bytes(const unsigned char *hay, size_t hn, const char *needle)
{
    size_t nn = strlen(needle), i;
    if (!nn || nn > hn) return 0;
    for (i = 0; i + nn <= hn; i++) if (!memcmp(hay + i, needle, nn)) return 1;
    return 0;
}

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

static int mem_is_exec = 1;
static int be_alloc(void *c, uint64_t pref, size_t size, int fixed, void **mem, uint64_t *addr)
{
    void *p; (void)c; (void)pref;
    if (fixed) return 0;
    p = mmap(NULL, size, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) { mem_is_exec = 0; p = calloc(1, size); if (!p) return 0; }
    *mem = p; *addr = (uint64_t)(uintptr_t)p;
    return 1;
}
static void be_free(void *c, void *mem, uint64_t addr, size_t size)
{ (void)c; (void)addr; if (mem_is_exec) munmap(mem, size); else free(mem); }
static int be_call(void *c, uint64_t fn, uint64_t base, uint32_t reason, void *reserved)
{
    typedef int (__attribute__((ms_abi)) *entry_fn)(uint64_t, uint32_t, void *);
    (void)c;
    return ((entry_fn)(uintptr_t)fn)(base, reason, reserved);
}

/* Redirect every kernel32.dll import to the shim module, keeping the name. */
static int be_redirect(void *c, const char *app, const char *dll, const char *name,
                       char *to_dll, size_t to_dll_sz, char *to_sym, size_t to_sym_sz)
{
    (void)c; (void)app;
    if (dll && name && !strcasecmp(dll, "kernel32.dll")) {
        strncpy(to_dll, "k32shim.dll", to_dll_sz);
        if (to_sym_sz) to_sym[0] = '\0';        /* empty to_sym = keep the original symbol name */
        return 1;
    }
    return 0;
}

static void *wide(const char *s)
{
    static WCHAR b[512];
    size_t i = 0;
    for (; s[i]; i++) b[i] = (WCHAR)(unsigned char)s[i];
    b[i] = 0;
    return b;
}

int main(int argc, char **argv)
{
    ldr_context c; ldr_backend be;
    ldr_module *exe, *shim;
    ldr_error e;
    uint64_t h_std = 0, h_wr = 0, entry;
    HANDLE conout;
    int rc = 0;

    if (argc < 2) { printf("usage: app_compat <dir with the app + k32shim.dll> [exe-name]\n"); return 2; }
    dir = argv[1];
    const char *appname = argc > 2 ? argv[2] : "compat-hello.exe";

    /* the process environment the LoOS kernel32 sees: a console on stdout */
    fake_reset();
    {
        RTL_USER_PROCESS_PARAMETERS PP; static RTL_USER_PROCESS_PARAMETERS keep;
        memset(&PP, 0, sizeof PP); keep = PP; k32_host_params = &keep;
        conout = CreateFileW((LPCWSTR)wide("CONOUT$"), GENERIC_READ | GENERIC_WRITE,
                             FILE_SHARE_WRITE, NULL, OPEN_EXISTING, 0, NULL);
        CHECK(conout != INVALID_HANDLE_VALUE);
        keep.StandardOutput = conout;
    }

    memset(&be, 0, sizeof be);
    be.open_module = be_open; be.release_file = be_release;
    be.alloc_image = be_alloc; be.free_image = be_free; be.call_entry = be_call;
    be.redirect_import = be_redirect;
    ldr_init(&c, &be, 1);

    if (ldr_load_exe(&c, appname, &exe, &e) != LDR_OK) {
        printf("load failed: module=%s symbol=%s\n", e.module, e.symbol);
        return 1;
    }
    shim = ldr_get_module(&c, "k32shim.dll");
    CHECK(exe && shim);

    /* bind the app's kernel32 imports to the LoOS kernel32 implementations */
    {
        static const struct { const char *sym; void *fn; } hooks[] = {
            { "__loos_hook_GetStdHandle",     (void *)&GetStdHandle },
            { "__loos_hook_WriteFile",        (void *)&WriteFile },
            { "__loos_hook_ReadFile",         (void *)&ReadFile },
            { "__loos_hook_CreateFileW",      (void *)&CreateFileW },
            { "__loos_hook_CloseHandle",      (void *)&CloseHandle },
            { "__loos_hook_GetFileSize",      (void *)&GetFileSize },
            { "__loos_hook_GetFileAttributesW", (void *)&GetFileAttributesW },
            { "__loos_hook_CreateDirectoryW", (void *)&CreateDirectoryW },
            { "__loos_hook_GetProcessHeap",   (void *)&GetProcessHeap },
            { "__loos_hook_HeapAlloc",         (void *)&HeapAlloc },
            { "__loos_hook_HeapReAlloc",       (void *)&HeapReAlloc },
            { "__loos_hook_HeapFree",          (void *)&HeapFree },
        };
        size_t i;
        for (i = 0; i < sizeof hooks / sizeof hooks[0]; i++) {
            uint64_t slot;
            if (ldr_get_proc(&c, shim, hooks[i].sym, 0, &slot, &e) == LDR_OK)
                *(void **)(uintptr_t)slot = hooks[i].fn;
        }
        (void)h_std; (void)h_wr;
    }

    entry = ldr_entry_point(exe);
    CHECK(entry > exe->addr && entry < exe->addr + exe->size);

    if (mem_is_exec) {
        typedef int (__attribute__((ms_abi)) *app_fn)(void);
        printf("app_compat: running an unmodified Win32 app on the LoOS compat layer ...\n");
        rc = ((app_fn)(uintptr_t)entry)();
         printf("app_compat: the app returned %d\n", rc);
        printf("app_compat: ---- the app's console output, captured by NtWriteFile ----\n");
        fwrite(F.con.out, 1, F.con.nout, stdout);
        printf("app_compat: ---------------------------------------------------------------\n");
         CHECK(F.con.nout > 0);
         if (!strcmp(appname, "compat-hello.exe")) CHECK(rc == 7);
         if (!strcmp(appname, "ucrt-hello.exe")) {
             static const char marker[] = "ucrt-hello: ucrtbase";
             CHECK(rc == 13);
             CHECK(contains_bytes(F.con.out, F.con.nout, marker));
         }
    } else {
        printf("  (mmap RWX unavailable: skipped execution)\n");
    }

    ldr_shutdown(&c);
    if (fails) { printf("app_compat: %d FAILED\n", fails); return 1; }
    printf("app_compat: all passed\n");
    return 0;
}
