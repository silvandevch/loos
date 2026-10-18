/* ntdll loader entry points: LdrLoadDll & co. over src/win32/ldr, with the
 * backend implemented on Nt* system calls (files are read into private memory and
 * mapped by copy until Mm offers SEC_IMAGE sections).
 *
 * Search policy (open_module): an explicit DOS path is used as given; bare names
 * are looked up in C:\Windows\System32 (the path Windows applications hardcode).
 * Application directory / PATH / SxS / KnownDlls are not implemented yet.
 *
 * TODO(W^X): images are mapped PAGE_EXECUTE_READWRITE; per-section protections
 * from the PE section table should be applied after relocation and binding. */
#include "../include/loos_win.h"
#include "../ldr/ldr.h"
#include "../compat/compat.h"
#include <string.h>

void rtl_set_version(const compat_version *v);

static ldr_context g_ldr;
static int g_ready;
static const compat_db *g_compat;                 /* optional profile database */
static compat_db g_default_db;

/* Test seam: how DLL entry points are invoked. Default is a direct Windows-ABI call. */
typedef int (NTAPI *entry_fn)(PVOID base, ULONG reason, PVOID reserved);
static int default_call(void *ctx, uint64_t fn, uint64_t base, uint32_t reason, void *reserved)
{ (void)ctx; return ((entry_fn)(uintptr_t)fn)((PVOID)(uintptr_t)base, reason, reserved) != 0; }
static int (*call_hook)(void *, uint64_t, uint64_t, uint32_t, void *) = default_call;
void ldr_api_set_entry_caller(int (*fn)(void *, uint64_t, uint64_t, uint32_t, void *)) { call_hook = fn ? fn : default_call; }
void ldr_api_set_compat_db(const compat_db *db) { g_compat = db; }
void ldr_api_reset(void) { g_ready = 0; }

/* ---------- file reading ---------- */
#define MAX_VIEWS 8
static struct { int used; void *mem; SIZE_T size; } views[MAX_VIEWS];

static int ascii_to_wide(const char *s, WCHAR *out, size_t cap)
{
    size_t n = 0;
    for (; s[n]; n++) { if (n + 1 >= cap) return 0; out[n] = (WCHAR)(unsigned char)s[n]; }
    out[n] = 0;
    return 1;
}

static int read_whole_file(const char *dos_or_nt, void **mem_out, SIZE_T *size_out)
{
    WCHAR path[600];
    UNICODE_STRING us;
    OBJECT_ATTRIBUTES oa;
    IO_STATUS_BLOCK iosb;
    FILE_STANDARD_INFORMATION si;
    HANDLE h = NULL;
    PVOID buf = NULL;
    SIZE_T alloc;
    ULONG done = 0;
    size_t n, pre = 0;
    static const char nt_prefix[] = "\\??\\";

    if (dos_or_nt[0] != '\\') { for (; nt_prefix[pre]; pre++) path[pre] = (WCHAR)nt_prefix[pre]; }
    if (!ascii_to_wide(dos_or_nt, path + pre, 600 - pre)) return 0;
    for (n = 0; path[n]; n++) if (path[n] == '/') path[n] = '\\';
    us.Buffer = path; us.Length = (USHORT)(n * 2); us.MaximumLength = (USHORT)(n * 2 + 2);
    InitializeObjectAttributes(&oa, &us, OBJ_CASE_INSENSITIVE, NULL, NULL);

    if (!NT_SUCCESS(NtOpenFile(&h, FILE_GENERIC_READ, &oa, &iosb, FILE_SHARE_READ | FILE_SHARE_DELETE,
                               FILE_NON_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT))) return 0;
    if (!NT_SUCCESS(NtQueryInformationFile(h, &iosb, &si, sizeof si, FileStandardInformation)) ||
        si.EndOfFile.QuadPart <= 0 || si.EndOfFile.QuadPart > 0x10000000) { NtClose(h); return 0; }
    alloc = (SIZE_T)si.EndOfFile.QuadPart;
    if (!NT_SUCCESS(NtAllocateVirtualMemory(NtCurrentProcess(), &buf, 0, &alloc, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE))) { NtClose(h); return 0; }
    while (done < (ULONG)si.EndOfFile.QuadPart) {
        ULONG chunk = (ULONG)si.EndOfFile.QuadPart - done;
        NTSTATUS st = NtReadFile(h, NULL, NULL, NULL, &iosb, (char *)buf + done, chunk, NULL, NULL);
        if (!NT_SUCCESS(st) || iosb.Information == 0) { SIZE_T z = 0; NtFreeVirtualMemory(NtCurrentProcess(), &buf, &z, MEM_RELEASE); NtClose(h); return 0; }
        done += (ULONG)iosb.Information;
    }
    NtClose(h);
    *mem_out = buf; *size_out = (SIZE_T)si.EndOfFile.QuadPart;
    return 1;
}

static int be_open(void *ctx, const char *name, const char *path, const void **data, size_t *size, void **cookie)
{
    char cand[320];
    void *mem; SIZE_T sz;
    int i;
    (void)ctx;
    if (path && path[0] && path[1] == ':') {
        if (!read_whole_file(path, &mem, &sz)) return 0;
    } else {
        static const char dir[] = "C:\\Windows\\System32\\";
        size_t dl = sizeof dir - 1, nl = strlen(name);
        if (dl + nl + 1 > sizeof cand) return 0;
        memcpy(cand, dir, dl); memcpy(cand + dl, name, nl + 1);
        if (!read_whole_file(cand, &mem, &sz)) return 0;
    }
    for (i = 0; i < MAX_VIEWS; i++) if (!views[i].used) {
        views[i].used = 1; views[i].mem = mem; views[i].size = sz;
        *data = mem; *size = sz; *cookie = &views[i];
        return 1;
    }
    { SIZE_T z = 0; PVOID m = mem; NtFreeVirtualMemory(NtCurrentProcess(), &m, &z, MEM_RELEASE); }
    return 0;
}

static void be_release(void *ctx, void *cookie)
{
    int i = (int)((char *)cookie - (char *)views) / (int)sizeof views[0];
    PVOID m; SIZE_T z = 0;
    (void)ctx;
    if (i < 0 || i >= MAX_VIEWS || !views[i].used) return;
    m = views[i].mem; views[i].used = 0;
    NtFreeVirtualMemory(NtCurrentProcess(), &m, &z, MEM_RELEASE);
}

static int be_alloc(void *ctx, uint64_t pref, size_t size, int fixed, void **mem, uint64_t *addr)
{
    PVOID base = (PVOID)(uintptr_t)pref;
    SIZE_T sz = size;
    (void)ctx;
    if (!NT_SUCCESS(NtAllocateVirtualMemory(NtCurrentProcess(), &base, 0, &sz, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE))) {
        if (fixed) return 0;
        base = NULL; sz = size;
        if (!NT_SUCCESS(NtAllocateVirtualMemory(NtCurrentProcess(), &base, 0, &sz, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE))) return 0;
    }
    if (fixed && (uint64_t)(uintptr_t)base != pref) { SIZE_T z = 0; NtFreeVirtualMemory(NtCurrentProcess(), &base, &z, MEM_RELEASE); return 0; }
    *mem = base; *addr = (uint64_t)(uintptr_t)base;
    return 1;
}

static void be_free(void *ctx, void *mem, uint64_t addr, size_t size)
{ PVOID b = mem; SIZE_T z = 0; (void)ctx; (void)addr; (void)size; NtFreeVirtualMemory(NtCurrentProcess(), &b, &z, MEM_RELEASE); }

static int be_call(void *ctx, uint64_t fn, uint64_t base, uint32_t reason, void *reserved)
{ return call_hook(ctx, fn, base, reason, reserved); }

static int be_redirect(void *ctx, const char *app, const char *dll, const char *name, char *td, size_t tds, char *ts, size_t tss)
{
    (void)ctx;
    return g_compat && compat_redirect_import(g_compat, app, dll, name, td, tds, ts, tss);
}

/* The manifest decides the reported OS version; do it before any DLL initialiser can ask. */
static void be_exe_mapped(void *ctx, ldr_module *exe)
{
    const char *xml = NULL; size_t len = 0;
    compat_claim c;
    (void)ctx;
    if (ldr_get_manifest(exe, &xml, &len) != LDR_OK) { xml = NULL; len = 0; }
    compat_resolve(g_compat ? g_compat : &g_default_db, g_ldr.app, xml, len, &c);
    rtl_set_version(compat_version_of(c.era));
}

static void ensure_ready(void)
{
    ldr_backend be;
    if (g_ready) return;
    memset(&be, 0, sizeof be);
    be.open_module = be_open; be.release_file = be_release; be.alloc_image = be_alloc; be.free_image = be_free;
    be.call_entry = be_call; be.redirect_import = be_redirect; be.exe_mapped = be_exe_mapped;
    ldr_init(&g_ldr, &be, 1);
    compat_db_init(&g_default_db);
    g_ready = 1;
}

/* ---------- Ldr* ---------- */
static int narrow(const UNICODE_STRING *us, char *out, size_t cap)
{
    size_t i, n = us->Length / sizeof(WCHAR);
    if (n == 0 || n + 1 > cap) return 0;
    for (i = 0; i < n; i++) { if (us->Buffer[i] > 0x7e) return 0; out[i] = (char)us->Buffer[i]; }
    out[n] = 0;
    return 1;
}

static NTSTATUS to_nt(ldr_status s)
{
    switch (s) {
    case LDR_OK: return STATUS_SUCCESS;
    default: return (NTSTATUS)ldr_status_to_ntstatus(s);
    }
}

static ldr_module *module_from_handle(PVOID h)
{
    int i;
    for (i = 0; i < LDR_MAX_MODULES; i++)
        if (g_ldr.mod[i].state != LDR_FREE && g_ldr.mod[i].addr == (uint64_t)(uintptr_t)h) return &g_ldr.mod[i];
    return NULL;
}

NTSTATUS NTAPI LdrLoadDll(PWSTR path, ULONG *flags, UNICODE_STRING *name, PVOID *handle)
{
    char n[260]; ldr_module *m; ldr_status s;
    (void)path; (void)flags;
    if (!name || !handle) return STATUS_INVALID_PARAMETER;
    ensure_ready();
    if (!narrow(name, n, sizeof n)) return STATUS_DLL_NOT_FOUND;
    s = ldr_load_dll(&g_ldr, n, &m, NULL);
    if (s != LDR_OK) return to_nt(s);
    *handle = (PVOID)(uintptr_t)m->addr;
    return STATUS_SUCCESS;
}

NTSTATUS NTAPI LdrGetDllHandle(PWSTR path, ULONG *flags, UNICODE_STRING *name, PVOID *handle)
{
    char n[260]; ldr_module *m;
    (void)path; (void)flags;
    if (!name || !handle) return STATUS_INVALID_PARAMETER;
    ensure_ready();
    if (!narrow(name, n, sizeof n) || !(m = ldr_get_module(&g_ldr, n))) return STATUS_DLL_NOT_FOUND;
    *handle = (PVOID)(uintptr_t)m->addr;
    return STATUS_SUCCESS;
}

NTSTATUS NTAPI LdrGetProcedureAddress(PVOID module, ANSI_STRING *name, ULONG ordinal, PVOID *addr)
{
    ldr_module *m; uint64_t a = 0; char nm[128]; ldr_status s;
    if (!addr) return STATUS_INVALID_PARAMETER;
    ensure_ready();
    if (!(m = module_from_handle(module))) return STATUS_INVALID_HANDLE;
    if (name) {
        if (name->Length == 0 || name->Length >= sizeof nm) return STATUS_INVALID_PARAMETER;
        memcpy(nm, name->Buffer, name->Length); nm[name->Length] = 0;
        s = ldr_get_proc(&g_ldr, m, nm, 0, &a, NULL);
    } else {
        if (ordinal == 0 || ordinal > 0xFFFF) return STATUS_INVALID_PARAMETER;
        s = ldr_get_proc(&g_ldr, m, NULL, (uint16_t)ordinal, &a, NULL);
    }
    if (s != LDR_OK) return to_nt(s);
    *addr = (PVOID)(uintptr_t)a;
    return STATUS_SUCCESS;
}

NTSTATUS NTAPI LdrUnloadDll(PVOID module)
{
    ldr_module *m;
    ensure_ready();
    if (!(m = module_from_handle(module))) return STATUS_INVALID_HANDLE;
    return to_nt(ldr_unload_dll(&g_ldr, m));
}

/* ---------- process start ---------- */
void NTAPI RtlUserThreadStart(PVOID entry, PVOID arg)
{
    int rc = ((int (NTAPI *)(PVOID))entry)(arg);
    RtlExitUserProcess(rc);
}

/* Returns the entry point (0 on failure, with *status set) so the test harness can drive it. */
uint64_t ldr_api_prepare_process(const char *image_path, NTSTATUS *status)
{
    ldr_module *exe; ldr_error e; ldr_status s;
    ensure_ready();
    s = ldr_load_exe(&g_ldr, image_path, &exe, &e);
    if (s != LDR_OK) { *status = to_nt(s); return 0; }
    *status = STATUS_SUCCESS;
    return ldr_entry_point(exe);
}

void NTAPI LdrStartProcess(PCWSTR image_path)
{
    char p[300]; size_t i; NTSTATUS st; uint64_t entry;
    for (i = 0; image_path[i] && i + 1 < sizeof p; i++) p[i] = image_path[i] > 0x7e ? '?' : (char)image_path[i];
    p[i] = 0;
    entry = ldr_api_prepare_process(p, &st);
    if (!entry) RtlExitUserProcess(st);
    RtlUserThreadStart((PVOID)(uintptr_t)entry, NULL);
}
