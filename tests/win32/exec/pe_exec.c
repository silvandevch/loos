/* pe_exec — run a real PE32+ executable through the LoOS loader on the host.
 *
 * Unlike the structural loader tests (which synthesise images), this loads a
 * genuine, compiler-produced executable (loos-echo.exe) plus its dependency
 * (losshost.dll), lets the LoOS loader map / relocate / bind / initialise them
 * with a host backend, then calls the executable's own entry point with the
 * Windows x64 ABI and reports its exit code. The executable's imported call to
 * host_write is routed back into this process, so you see the program's real
 * output.
 *
 *   usage: pe_exec <dir containing loos-echo.exe and losshost.dll>
 */
#include "ldr.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)

static const char *dir;
static int mem_is_exec = 1;

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
    if (fixed) return 0;                       /* never the preferred base: always relocate */
    p = mmap(NULL, size, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) { mem_is_exec = 0; p = calloc(1, size); if (!p) return 0; }
    *mem = p; *addr = (uint64_t)(uintptr_t)p;
    return 1;
}
static void be_free(void *c, void *mem, uint64_t addr, size_t size)
{
    (void)c; (void)addr;
    if (mem_is_exec) munmap(mem, size); else free(mem);
}
static int be_call(void *c, uint64_t fn, uint64_t base, uint32_t reason, void *reserved)
{
    typedef int (__attribute__((ms_abi)) *entry_fn)(uint64_t, uint32_t, void *);
    (void)c;
    return ((entry_fn)(uintptr_t)fn)(base, reason, reserved);
}

/* The host side of the imported call: same Windows x64 ABI as the caller. */
static long __attribute__((ms_abi)) host_write_impl(int fd, const void *buf, long len)
{
    (void)fd;
    if (buf && len > 0) { fwrite(buf, 1, (size_t)len, stdout); fflush(stdout); }
    return len;
}

int main(int argc, char **argv)
{
    ldr_context c; ldr_backend be;
    ldr_module *exe, *host;
    ldr_error e;
    uint64_t slot_addr, entry;
    int rc = 0;

    if (argc < 2) { printf("usage: pe_exec <dir with loos-echo.exe + losshost.dll>\n"); return 2; }
    dir = argv[1];

    memset(&be, 0, sizeof be);
    be.open_module = be_open; be.release_file = be_release;
    be.alloc_image = be_alloc; be.free_image = be_free; be.call_entry = be_call;
    ldr_init(&c, &be, 1);

    if (ldr_load_exe(&c, "loos-echo.exe", &exe, &e) != LDR_OK) {
        printf("load failed: %s %s\n", e.module, e.symbol);
        return 1;
    }
    host = ldr_get_module(&c, "losshost.dll");
    CHECK(exe && host);

    /* fill the DLL's exported hook slot so its host_write lands in this process */
    CHECK(ldr_get_proc(&c, host, "__loos_host_write_hook", 0, &slot_addr, &e) == LDR_OK);
    *(void **)(uintptr_t)slot_addr = (void *)&host_write_impl;

    entry = ldr_entry_point(exe);
    CHECK(entry > exe->addr && entry < exe->addr + exe->size);
    CHECK(host->addr != exe->addr);

    if (mem_is_exec) {
        typedef int (__attribute__((ms_abi)) *exe_main_fn)(void);
        printf("pe_exec: running %s (mapped at %#llx) ...\n", exe->name,
               (unsigned long long)exe->addr);
        rc = ((exe_main_fn)(uintptr_t)entry)();
        printf("pe_exec: the executable returned %d\n", rc);
        CHECK(rc == 42);
    } else {
        printf("  (mmap RWX unavailable: skipped execution)\n");
    }

    ldr_shutdown(&c);
    if (fails) printf("pe_exec: %d FAILED\n", fails);
    else printf("pe_exec: all passed\n");
    return fails != 0;
}
