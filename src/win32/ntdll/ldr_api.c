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
#include "../include/loos_teb.h"
#include "../ldr/ldr.h"
#include "../compat/compat.h"
#include <string.h>

void rtl_set_version(const compat_version *v);

static ldr_context g_ldr;
static int g_ready;

/* The loader lock: the loader state is not thread safe, DllMain may load libraries (hence recursive). */
static RTL_CRITICAL_SECTION loader_lock;
static volatile int lock_state;                 /* 0 uninitialised, 1 initialising, 2 ready */
static void lock_loader(void)
{
    if (__atomic_load_n(&lock_state, __ATOMIC_ACQUIRE) != 2) {
        int zero = 0;
        if (__atomic_compare_exchange_n(&lock_state, &zero, 1, 0, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE)) {
            RtlInitializeCriticalSection(&loader_lock);
            __atomic_store_n(&lock_state, 2, __ATOMIC_RELEASE);
        } else while (__atomic_load_n(&lock_state, __ATOMIC_ACQUIRE) != 2) __builtin_ia32_pause();
    }
    RtlEnterCriticalSection(&loader_lock);
}
static void unlock_loader(void) { RtlLeaveCriticalSection(&loader_lock); }
static const compat_db *g_compat;                 /* optional profile database */
static compat_db g_default_db;

/* Test seam: how DLL entry points are invoked. Default is a direct Windows-ABI call. */
typedef int (NTAPI *entry_fn)(PVOID base, ULONG reason, PVOID reserved);
static int default_call(void *ctx, uint64_t fn, uint64_t base, uint32_t reason, void *reserved)
{ (void)ctx; return ((entry_fn)(uintptr_t)fn)((PVOID)(uintptr_t)base, reason, reserved) != 0; }
static int (*call_hook)(void *, uint64_t, uint64_t, uint32_t, void *) = default_call;
void ldr_api_set_entry_caller(int (*fn)(void *, uint64_t, uint64_t, uint32_t, void *)) { call_hook = fn ? fn : default_call; }
void ldr_api_set_compat_db(const compat_db *db) { g_compat = db; }
void ldr_api_reset(void) { g_ready = 0; lock_state = 0; }

/* ---------- file reading ---------- */
#define MAX_VIEWS 8
struct view { int used; void *mem; SIZE_T size; char path[260]; };
static struct view views[MAX_VIEWS];

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
        const char *src = (path && path[0] && path[1] == ':') ? path : cand;
        size_t pl = strlen(src);
        views[i].used = 1; views[i].mem = mem; views[i].size = sz;
        if (pl >= sizeof views[i].path) pl = 0;
        memcpy(views[i].path, src, pl); views[i].path[pl] = 0;
        *data = mem; *size = sz; *cookie = &views[i];
        return 1;
    }
    { SIZE_T z = 0; PVOID m = mem; NtFreeVirtualMemory(NtCurrentProcess(), &m, &z, MEM_RELEASE); }
    return 0;
}

static const char *be_module_path(void *ctx, void *cookie) { (void)ctx; return ((struct view *)cookie)->path; }

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
    be.call_entry = be_call; be.redirect_import = be_redirect; be.exe_mapped = be_exe_mapped; be.module_path = be_module_path;
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
    if (!narrow(name, n, sizeof n)) return STATUS_DLL_NOT_FOUND;
    lock_loader();
    ensure_ready();
    s = ldr_load_dll(&g_ldr, n, &m, NULL);
    if (s == LDR_OK) *handle = (PVOID)(uintptr_t)m->addr;
    unlock_loader();
    return s == LDR_OK ? STATUS_SUCCESS : to_nt(s);
}

NTSTATUS NTAPI LdrGetDllHandle(PWSTR path, ULONG *flags, UNICODE_STRING *name, PVOID *handle)
{
    char n[260]; ldr_module *m;
    (void)path; (void)flags;
    NTSTATUS st = STATUS_DLL_NOT_FOUND;
    if (!name || !handle) return STATUS_INVALID_PARAMETER;
    if (!narrow(name, n, sizeof n)) return STATUS_DLL_NOT_FOUND;
    lock_loader();
    ensure_ready();
    if ((m = ldr_get_module(&g_ldr, n)) != NULL) { *handle = (PVOID)(uintptr_t)m->addr; st = STATUS_SUCCESS; }
    unlock_loader();
    return st;
}

NTSTATUS NTAPI LdrGetProcedureAddress(PVOID module, ANSI_STRING *name, ULONG ordinal, PVOID *addr)
{
    ldr_module *m; uint64_t a = 0; char nm[128]; ldr_status s;
    if (!addr) return STATUS_INVALID_PARAMETER;
    if (name && (name->Length == 0 || name->Length >= sizeof nm)) return STATUS_INVALID_PARAMETER;
    if (!name && (ordinal == 0 || ordinal > 0xFFFF)) return STATUS_INVALID_PARAMETER;
    lock_loader();
    ensure_ready();
    if (!(m = module_from_handle(module))) { unlock_loader(); return STATUS_INVALID_HANDLE; }
    if (name) {
        memcpy(nm, name->Buffer, name->Length); nm[name->Length] = 0;
        s = ldr_get_proc(&g_ldr, m, nm, 0, &a, NULL);
    } else {
        s = ldr_get_proc(&g_ldr, m, NULL, (uint16_t)ordinal, &a, NULL);
    }
    unlock_loader();
    if (s != LDR_OK) return to_nt(s);
    *addr = (PVOID)(uintptr_t)a;
    return STATUS_SUCCESS;
}

NTSTATUS NTAPI LdrUnloadDll(PVOID module)
{
    ldr_module *m;
    ldr_status s;
    lock_loader();
    ensure_ready();
    if (!(m = module_from_handle(module))) { unlock_loader(); return STATUS_INVALID_HANDLE; }
    s = ldr_unload_dll(&g_ldr, m);
    unlock_loader();
    return to_nt(s);
}

NTSTATUS NTAPI LdrGetDllFullName(PVOID module, UNICODE_STRING *out)
{
    ldr_module *m;
    size_t i, n;
    NTSTATUS st = STATUS_SUCCESS;
    if (!out || !out->Buffer) return STATUS_INVALID_PARAMETER;
    lock_loader();
    ensure_ready();
    if (!(m = module_from_handle(module))) st = STATUS_INVALID_HANDLE;
    else {
        n = strlen(m->path);
        if ((n + 1) * sizeof(WCHAR) > out->MaximumLength) st = STATUS_BUFFER_TOO_SMALL;
        else { for (i = 0; i < n; i++) out->Buffer[i] = (WCHAR)(unsigned char)m->path[i]; out->Buffer[n] = 0; out->Length = (USHORT)(n * sizeof(WCHAR)); }
    }
    unlock_loader();
    return st;
}

PVOID NTAPI RtlPcToFileHeader(PVOID pc, PVOID *base)
{
    int i;
    PVOID found = NULL;
    uint64_t a = (uint64_t)(uintptr_t)pc;
    lock_loader();
    ensure_ready();
    for (i = 0; i < LDR_MAX_MODULES; i++)
        if (g_ldr.mod[i].state != LDR_FREE && a >= g_ldr.mod[i].addr && a < g_ldr.mod[i].addr + g_ldr.mod[i].size) { found = (PVOID)(uintptr_t)g_ldr.mod[i].addr; break; }
    unlock_loader();
    if (base) *base = found;
    return found;
}

void NTAPI LdrShutdownThread(void)
{
    lock_loader();
    ensure_ready();
    ldr_thread_notify(&g_ldr, LDR_REASON_THREAD_DETACH);
    unlock_loader();
}

static void ldr_initialize_thread(void)
{
    lock_loader();
    ensure_ready();
    ldr_thread_notify(&g_ldr, LDR_REASON_THREAD_ATTACH);
    unlock_loader();
}

void NTAPI RtlExitUserThread(NTSTATUS status)
{
    LdrShutdownThread();
    NtTerminateThread(NtCurrentThread(), status);
#ifndef LOOS_HOST_TEST
    for (;;) { }
#endif
}

/* ---------- process start ---------- */
/* Every thread except the first begins here: announce it to the DLLs, run the start routine, exit. */
void NTAPI RtlUserThreadStart(PVOID entry, PVOID arg)
{
    NTSTATUS rc;
    ldr_initialize_thread();
    rc = (NTSTATUS)((ULONG (NTAPI *)(PVOID))entry)(arg);
    RtlExitUserThread(rc);
}

/* Returns the entry point (0 on failure, with *status set) so the test harness can drive it. */
uint64_t ldr_api_prepare_process(const char *image_path, NTSTATUS *status)
{
    ldr_module *exe; ldr_error e; ldr_status s;
    uint64_t entry = 0;
    lock_loader();
    ensure_ready();
    s = ldr_load_exe(&g_ldr, image_path, &exe, &e);
    if (s != LDR_OK) *status = to_nt(s);
    else { *status = STATUS_SUCCESS; entry = ldr_entry_point(exe); }
    unlock_loader();
    return entry;
}

void NTAPI LdrStartProcess(PCWSTR image_path)
{
    char p[300]; size_t i; NTSTATUS st; uint64_t entry;
    for (i = 0; image_path[i] && i + 1 < sizeof p; i++) p[i] = image_path[i] > 0x7e ? '?' : (char)image_path[i];
    p[i] = 0;
    entry = ldr_api_prepare_process(p, &st);
    if (!entry) RtlExitUserProcess(st);
    RtlExitUserProcess((NTSTATUS)((ULONG (NTAPI *)(PVOID))(uintptr_t)entry)(NULL));      /* the first thread: returning from the image entry ends the process */
}
