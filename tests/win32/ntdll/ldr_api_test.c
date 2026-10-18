#define LOOS_HOST_TEST 1
#include "../kernel32/fake.h"
#include "../common/pebuild.h"
#include "compat.h"
#include <stdio.h>
#include <string.h>

void ldr_api_set_entry_caller(int (*fn)(void *, uint64_t, uint64_t, uint32_t, void *));
void ldr_api_set_compat_db(const compat_db *db);
void ldr_api_reset(void);
uint64_t ldr_api_prepare_process(const char *image_path, NTSTATUS *status);

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)
#define CHECK_EQ(a, b) do { unsigned long long a_ = (unsigned long long)(a), b_ = (unsigned long long)(b); \
    if (a_ != b_) { printf("FAIL %s:%d: %s = %#llx, want %#llx\n", __FILE__, __LINE__, #a, a_, b_); fails++; } } while (0)

typedef struct { uint64_t fn, base; uint32_t reason; } call;
static call calls[32]; static int ncalls;
static int record(void *c, uint64_t fn, uint64_t base, uint32_t reason, void *res)
{ (void)c; (void)res; if (ncalls < 32) calls[ncalls++] = (call){ fn, base, reason }; return 1; }

static void us_init(UNICODE_STRING *u, const WCHAR *w) { size_t n = 0; while (w[n]) n++; u->Buffer = (PWSTR)w; u->Length = (USHORT)(n * 2); u->MaximumLength = (USHORT)(n * 2 + 2); }
static const WCHAR *W(const char *s) { static WCHAR b[4][300]; static int i; WCHAR *o = b[i++ & 3]; fake_wset(o, s); return o; }

static void put_image(const char *nt_path, const bspec *s)
{
    bimage im; bimage_build(s, &im);
    CHECK(fake_add_file(nt_path, 0, im.data, im.size) >= 0);
    bimage_free(&im);
}
static void addexp(bspec *s, const char *n) { s->exports[s->nexports].name = n; s->exports[s->nexports].forward = NULL; s->nexports++; }
static void addimp(bspec *s, const char *dll, const char *sym) { bimport *i = &s->imports[s->nimports++]; i->dll = dll; i->syms[0] = sym; i->syms[1] = NULL; }
static bspec dll(uint64_t base) { bspec s; memset(&s, 0, sizeof s); s.is64 = 1; s.is_dll = 1; s.entry = 1; s.relocs = 1; s.base = base; return s; }

extern void rtl_set_version(const void *);

int main(void)
{
    UNICODE_STRING us; PVOID h1 = NULL, h2 = NULL, a; ANSI_STRING as; NTSTATUS st; ULONG flags = 0;
    char fname[] = "Func";
    RTL_OSVERSIONINFOW v;

    fake_reset();
    ldr_api_reset();
    ldr_api_set_entry_caller(record);
    fake_add_file("\\??\\C:\\Windows", 1, NULL, 0);
    fake_add_file("\\??\\C:\\Windows\\System32", 1, NULL, 0);
    fake_add_file("\\??\\C:\\apps", 1, NULL, 0);

    { bspec b = dll(0x180000000ull); addexp(&b, "Func"); addexp(&b, "Other"); put_image("\\??\\C:\\Windows\\System32\\base.dll", &b); }
    { bspec u = dll(0x180100000ull); addexp(&u, "UserFn"); addimp(&u, "BASE.DLL", "Func"); put_image("\\??\\C:\\Windows\\System32\\user.dll", &u); }

    /* LoadLibrary: dependency first, handle = base address */
    us_init(&us, W("User"));
    st = LdrLoadDll(NULL, &flags, &us, &h1);
    CHECK_EQ(st, STATUS_SUCCESS); CHECK(h1 != NULL);
    CHECK_EQ(ncalls, 2);
    CHECK(calls[0].reason == 1 && calls[1].reason == 1 && calls[1].base == (uint64_t)(uintptr_t)h1 && calls[0].base != calls[1].base);
    us_init(&us, W("USER.DLL")); CHECK_EQ(LdrGetDllHandle(NULL, NULL, &us, &h2), STATUS_SUCCESS); CHECK(h2 == h1);
    us_init(&us, W("base.dll")); CHECK_EQ(LdrGetDllHandle(NULL, NULL, &us, &h2), STATUS_SUCCESS); CHECK((uint64_t)(uintptr_t)h2 == calls[0].base);
    us_init(&us, W("User")); CHECK_EQ(LdrLoadDll(NULL, NULL, &us, &h2), STATUS_SUCCESS); CHECK(h2 == h1); CHECK_EQ(ncalls, 2);   /* no second attach */

    /* GetProcAddress by name and by ordinal; errors */
    as.Buffer = fname; as.Length = 4; as.MaximumLength = 5;
    { PVOID base_h = (PVOID)(uintptr_t)calls[0].base;
      CHECK_EQ(LdrGetProcedureAddress(base_h, &as, 0, &a), STATUS_SUCCESS); CHECK_EQ((uint64_t)(uintptr_t)a, calls[0].base + B_FUNC_RVA(0));
      CHECK_EQ(LdrGetProcedureAddress(base_h, NULL, 2, &a), STATUS_SUCCESS); CHECK_EQ((uint64_t)(uintptr_t)a, calls[0].base + B_FUNC_RVA(1));
      CHECK_EQ((ULONG)LdrGetProcedureAddress(base_h, NULL, 9, &a), 0xC0000138u);          /* ordinal not found */
      as.Buffer = (char *)"Nope"; as.Length = 4; CHECK_EQ(LdrGetProcedureAddress(base_h, &as, 0, &a), STATUS_PROCEDURE_NOT_FOUND);
      CHECK_EQ(LdrGetProcedureAddress(base_h, NULL, 0, &a), STATUS_INVALID_PARAMETER);
      CHECK_EQ(LdrGetProcedureAddress((PVOID)0x1234, &as, 0, &a), STATUS_INVALID_HANDLE); }

    /* failures */
    us_init(&us, W("nothere.dll")); CHECK_EQ(LdrLoadDll(NULL, NULL, &us, &h2), STATUS_DLL_NOT_FOUND);
    us_init(&us, W("caf\xe9.dll")); CHECK_EQ(LdrLoadDll(NULL, NULL, &us, &h2), STATUS_DLL_NOT_FOUND);       /* non-ASCII not supported yet */
    CHECK_EQ(LdrLoadDll(NULL, NULL, NULL, &h2), STATUS_INVALID_PARAMETER);

    /* FreeLibrary detaches in reverse order and frees both modules */
    ncalls = 0;
    CHECK_EQ(LdrUnloadDll(h1), STATUS_SUCCESS);       /* explicit count was 2: loaded twice */
    CHECK_EQ(ncalls, 0);
    CHECK_EQ(LdrUnloadDll(h1), STATUS_SUCCESS);
    CHECK_EQ(ncalls, 2); CHECK(calls[0].reason == 0 && calls[0].base == (uint64_t)(uintptr_t)h1);            /* user.dll first, then base */
    CHECK_EQ(LdrUnloadDll(h1), STATUS_INVALID_HANDLE);
    us_init(&us, W("user.dll")); CHECK_EQ(LdrGetDllHandle(NULL, NULL, &us, &h2), STATUS_DLL_NOT_FOUND);

    /* process start: manifest (Win7 GUID) decides the reported OS version before DLLs initialise */
    { static const char manifest[] = "<supportedOS Id=\"{35138b9a-5d96-4fbd-8e2d-a2440225f93a}\"/>";
      bspec app; uint64_t entry;
      memset(&app, 0, sizeof app); app.is64 = 1; app.relocs = 1; app.base = 0x140000000ull; app.manifest = manifest;
      addimp(&app, "base.dll", "Func");
      put_image("\\??\\C:\\apps\\app.exe", &app);
      ncalls = 0;
      entry = ldr_api_prepare_process("C:\\apps\\app.exe", &st);
      CHECK_EQ(st, STATUS_SUCCESS); CHECK(entry != 0);
      memset(&v, 0, sizeof v); v.dwOSVersionInfoSize = sizeof v; RtlGetVersion(&v);
      CHECK_EQ(v.dwMajorVersion, 6); CHECK_EQ(v.dwMinorVersion, 1); CHECK_EQ(v.dwBuildNumber, 7601);
      CHECK_EQ(ncalls, 1);                                            /* base.dll attached; exe entry not called by the loader */
      CHECK(ldr_api_prepare_process("C:\\apps\\missing.exe", &st) == 0); CHECK_EQ(st, STATUS_DLL_NOT_FOUND);

      /* a user profile overrides the manifest, and redirect rules apply to the exe's imports */
      { static compat_db db; compat_db_init(&db);
        compat_db_set_claim(&db, "app.exe", COMPAT_ERA_WIN81);
        ldr_api_set_compat_db(&db);
        ldr_api_reset();                                              /* fresh process state */
        fake_reset();
        fake_add_file("\\??\\C:\\Windows", 1, NULL, 0); fake_add_file("\\??\\C:\\Windows\\System32", 1, NULL, 0); fake_add_file("\\??\\C:\\apps", 1, NULL, 0);
        { bspec b = dll(0x180000000ull); addexp(&b, "Func"); put_image("\\??\\C:\\Windows\\System32\\base.dll", &b); }
        put_image("\\??\\C:\\apps\\app.exe", &app);
        entry = ldr_api_prepare_process("C:\\apps\\app.exe", &st);
        CHECK_EQ(st, STATUS_SUCCESS);
        memset(&v, 0, sizeof v); v.dwOSVersionInfoSize = sizeof v; RtlGetVersion(&v);
        CHECK_EQ(v.dwMajorVersion, 6); CHECK_EQ(v.dwMinorVersion, 3); } }

    fake_reset();
    if (fails) printf("ldr_api: %d FAILED\n", fails); else printf("ldr_api: all passed\n");
    return fails != 0;
}
