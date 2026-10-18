/* Fake ntdll for host tests: in-memory file system, kernel objects, VM and loader
 * stubs, plus recording of the arguments kernel32 passed so tests can assert the
 * exact NT-level translation. */
#define LOOS_HOST_TEST 1
#include "../../../src/win32/include/loos_win.h"
#include "fake.h"
#include <setjmp.h>
#include <stdlib.h>
#include <string.h>

fake_state F;
_Thread_local DWORD k32_host_last_error;
DWORD k32_host_pid = 1234, k32_host_tid = 5678;
void *k32_host_image_base = (void *)0x140000000ull;
jmp_buf fake_exit_jmp;

static WCHAR lc(WCHAR c) { return (c >= 'A' && c <= 'Z') ? (WCHAR)(c + 32) : c; }
static int wieq(const WCHAR *a, size_t an, const WCHAR *b, size_t bn)
{ size_t i; if (an != bn) return 0; for (i = 0; i < an; i++) if (lc(a[i]) != lc(b[i])) return 0; return 1; }

void fake_reset(void)
{
    int i;
    for (i = 0; i < F.nvm_blocks; i++) free(F.vm_blocks[i]);
    memset(&F, 0, sizeof F);
    F.perf_counter = 123456789012ll;
}

/* ---- path helpers: store as WCHAR strings of the NT path ---- */
static size_t wl(const WCHAR *s) { size_t n = 0; while (s[n]) n++; return n; }
void fake_wset(WCHAR *dst, const char *ascii) { size_t i = 0; for (; ascii[i]; i++) dst[i] = (WCHAR)(unsigned char)ascii[i]; dst[i] = 0; }

static int find_file(const WCHAR *p, size_t n)
{
    int i;
    if (n > 7 && p[n - 1] == '\\') n--;              /* a trailing separator on a directory is accepted */
    for (i = 0; i < FAKE_MAX_FILES; i++) if (F.files[i].used && wieq(F.files[i].path, wl(F.files[i].path), p, n)) return i;
    return -1;
}
int fake_add_file(const char *nt_path, int is_dir, const void *data, size_t size)
{
    int i; WCHAR w[FAKE_PATH];
    fake_wset(w, nt_path);
    for (i = 0; i < FAKE_MAX_FILES; i++) if (!F.files[i].used) {
        F.files[i].used = 1; F.files[i].is_dir = is_dir;
        memcpy(F.files[i].path, w, (wl(w) + 1) * sizeof(WCHAR));
        if (size) memcpy(F.files[i].data, data, size);
        F.files[i].size = size;
        F.files[i].attrs = is_dir ? FILE_ATTRIBUTE_DIRECTORY : FILE_ATTRIBUTE_ARCHIVE;
        return i;
    }
    return -1;
}
int fake_file_exists(const char *nt_path) { WCHAR w[FAKE_PATH]; fake_wset(w, nt_path); return find_file(w, wl(w)) >= 0; }
size_t fake_file_size(const char *nt_path) { WCHAR w[FAKE_PATH]; int i; fake_wset(w, nt_path); i = find_file(w, wl(w)); return i < 0 ? (size_t)-1 : F.files[i].size; }

static int parent_exists(const WCHAR *p, size_t n)
{
    size_t k = n;
    while (k > 0 && p[k - 1] != '\\') k--;
    if (k <= 1) return 1;
    k--;
    if (k <= 6) return 1;                 /* "\??\C:" root */
    return find_file(p, k) >= 0;
}

/* ---- handle table ---- */
static HANDLE new_handle(int kind, int obj)
{
    int i;
    for (i = 0; i < FAKE_MAX_HANDLES; i++) if (!F.h[i].used) {
        memset(&F.h[i], 0, sizeof F.h[i]);
        F.h[i].used = 1; F.h[i].kind = kind; F.h[i].obj = obj;
        return (HANDLE)(uintptr_t)(0x100 + 4 * i);
    }
    return NULL;
}
static fake_handle *get(HANDLE h)
{
    uintptr_t v = (uintptr_t)h;
    if (v < 0x100 || (v - 0x100) % 4) return NULL;
    v = (v - 0x100) / 4;
    if (v >= FAKE_MAX_HANDLES || !F.h[v].used) return NULL;
    return &F.h[v];
}

NTSTATUS NTAPI NtClose(HANDLE h)
{
    fake_handle *fh = get(h);
    F.close_calls++;
    if (!fh) return STATUS_INVALID_HANDLE;
    if (fh->kind == FK_FILE && fh->delete_on_close) F.files[fh->obj].used = 0;
    fh->used = 0;
    return STATUS_SUCCESS;
}

static NTSTATUS do_create(HANDLE *out, ULONG access, OBJECT_ATTRIBUTES *oa, IO_STATUS_BLOCK *iosb, ULONG attrs,
                          ULONG share, ULONG disp, ULONG options)
{
    const WCHAR *p; size_t n; int fi; HANDLE h;
    F.create_calls++;
    F.c_access = access; F.c_disp = disp; F.c_options = options; F.c_share = share; F.c_attrs = attrs;
    F.c_oattrs = oa->Attributes; F.c_sd = oa->SecurityDescriptor;
    if (!oa->ObjectName) return STATUS_INVALID_PARAMETER;
    p = oa->ObjectName->Buffer; n = oa->ObjectName->Length / 2;
    memcpy(F.c_path, p, n * sizeof(WCHAR)); F.c_path[n] = 0;
    fi = find_file(p, n);
    switch (disp) {
    case FILE_CREATE:
        if (fi >= 0) return STATUS_OBJECT_NAME_COLLISION;
        break;
    case FILE_OPEN: case FILE_OVERWRITE:
        if (fi < 0) return parent_exists(p, n) ? STATUS_OBJECT_NAME_NOT_FOUND : STATUS_OBJECT_PATH_NOT_FOUND;
        break;
    default: break;
    }
    if (fi < 0 && !parent_exists(p, n)) return STATUS_OBJECT_PATH_NOT_FOUND;
    if (fi >= 0) {
        if (F.files[fi].is_dir && (options & FILE_NON_DIRECTORY_FILE)) return STATUS_FILE_IS_A_DIRECTORY;
        if (!F.files[fi].is_dir && (options & FILE_DIRECTORY_FILE)) return STATUS_NOT_A_DIRECTORY;
        if (disp == FILE_OVERWRITE || disp == FILE_OVERWRITE_IF) { F.files[fi].size = 0; iosb->Information = FILE_OVERWRITTEN; }
        else iosb->Information = FILE_OPENED;
    } else {
        char tmp[FAKE_PATH]; size_t i;
        for (i = 0; i < n; i++) tmp[i] = (char)p[i];
        tmp[n] = 0;
        fi = fake_add_file(tmp, (options & FILE_DIRECTORY_FILE) != 0, NULL, 0);
        if (fi < 0) return STATUS_INSUFFICIENT_RESOURCES;
        F.files[fi].attrs = (attrs & ~FILE_ATTRIBUTE_NORMAL) | (options & FILE_DIRECTORY_FILE ? FILE_ATTRIBUTE_DIRECTORY : FILE_ATTRIBUTE_ARCHIVE);
        iosb->Information = FILE_CREATED;
    }
    h = new_handle(FK_FILE, fi);
    if (!h) return STATUS_INSUFFICIENT_RESOURCES;
    get(h)->delete_on_close = (options & FILE_DELETE_ON_CLOSE) != 0;
    get(h)->access = access;
    *out = h;
    iosb->Status = STATUS_SUCCESS;
    return STATUS_SUCCESS;
}
NTSTATUS NTAPI NtCreateFile(HANDLE *h, ULONG access, OBJECT_ATTRIBUTES *oa, IO_STATUS_BLOCK *iosb, LARGE_INTEGER *alloc,
                            ULONG attrs, ULONG share, ULONG disp, ULONG options, PVOID ea, ULONG ealen)
{ (void)alloc; (void)ea; (void)ealen; return do_create(h, access, oa, iosb, attrs, share, disp, options); }
NTSTATUS NTAPI NtOpenFile(HANDLE *h, ULONG access, OBJECT_ATTRIBUTES *oa, IO_STATUS_BLOCK *iosb, ULONG share, ULONG options)
{ F.open_calls++; return do_create(h, access, oa, iosb, 0, share, FILE_OPEN, options); }

NTSTATUS NTAPI NtReadFile(HANDLE h, HANDLE ev, PVOID apc, PVOID ctx, IO_STATUS_BLOCK *iosb, PVOID buf, ULONG len, LARGE_INTEGER *off, ULONG *key)
{
    fake_handle *fh = get(h); fake_file *f; size_t pos;
    (void)apc; (void)ctx; (void)key; (void)ev;
    F.last_read_off_given = off != NULL; if (off) F.last_read_off = off->QuadPart;
    if (!fh || fh->kind != FK_FILE) return STATUS_INVALID_HANDLE;
    f = &F.files[fh->obj];
    pos = off ? (size_t)off->QuadPart : (size_t)fh->pos;
    if (pos >= f->size) { iosb->Information = 0; return STATUS_END_OF_FILE; }
    if (len > f->size - pos) len = (ULONG)(f->size - pos);
    memcpy(buf, f->data + pos, len);
    iosb->Information = len; iosb->Status = STATUS_SUCCESS;
    if (!off) fh->pos = pos + len;
    return STATUS_SUCCESS;
}
NTSTATUS NTAPI NtWriteFile(HANDLE h, HANDLE ev, PVOID apc, PVOID ctx, IO_STATUS_BLOCK *iosb, const void *buf, ULONG len, LARGE_INTEGER *off, ULONG *key)
{
    fake_handle *fh = get(h); fake_file *f; size_t pos;
    (void)apc; (void)ctx; (void)key; (void)ev;
    if (!fh || fh->kind != FK_FILE) return STATUS_INVALID_HANDLE;
    f = &F.files[fh->obj];
    pos = off ? (size_t)off->QuadPart : (size_t)fh->pos;
    if (pos + len > FAKE_DATA) return STATUS_DISK_FULL;
    memcpy(f->data + pos, buf, len);
    if (pos + len > f->size) f->size = pos + len;
    iosb->Information = len; iosb->Status = STATUS_SUCCESS;
    if (!off) fh->pos = pos + len;
    return STATUS_SUCCESS;
}
NTSTATUS NTAPI NtQueryInformationFile(HANDLE h, IO_STATUS_BLOCK *iosb, PVOID info, ULONG len, FILE_INFORMATION_CLASS cls)
{
    fake_handle *fh = get(h); (void)iosb; (void)len;
    if (!fh || fh->kind != FK_FILE) return STATUS_INVALID_HANDLE;
    if (cls == FilePositionInformation) { ((FILE_POSITION_INFORMATION *)info)->CurrentByteOffset.QuadPart = (LONGLONG)fh->pos; return STATUS_SUCCESS; }
    if (cls == FileStandardInformation) {
        FILE_STANDARD_INFORMATION *s = info; memset(s, 0, sizeof *s);
        s->EndOfFile.QuadPart = (LONGLONG)F.files[fh->obj].size; s->Directory = F.files[fh->obj].is_dir; return STATUS_SUCCESS;
    }
    return STATUS_INVALID_INFO_CLASS;
}
NTSTATUS NTAPI NtSetInformationFile(HANDLE h, IO_STATUS_BLOCK *iosb, PVOID info, ULONG len, FILE_INFORMATION_CLASS cls)
{
    fake_handle *fh = get(h); (void)iosb; (void)len;
    if (!fh || fh->kind != FK_FILE) return STATUS_INVALID_HANDLE;
    if (cls == FilePositionInformation) { fh->pos = (uint64_t)((FILE_POSITION_INFORMATION *)info)->CurrentByteOffset.QuadPart; return STATUS_SUCCESS; }
    if (cls == FileEndOfFileInformation) {
        LONGLONG e = ((FILE_END_OF_FILE_INFORMATION *)info)->EndOfFile.QuadPart;
        if (e > FAKE_DATA) return STATUS_DISK_FULL;
        if ((size_t)e > F.files[fh->obj].size) memset(F.files[fh->obj].data + F.files[fh->obj].size, 0, (size_t)e - F.files[fh->obj].size);
        F.files[fh->obj].size = (size_t)e; return STATUS_SUCCESS;
    }
    if (cls == FileDispositionInformation) {
        F.disposition_calls++;
        if (F.files[fh->obj].is_dir && F.files[fh->obj].has_children) return STATUS_DIRECTORY_NOT_EMPTY;
        fh->delete_on_close = ((FILE_DISPOSITION_INFORMATION *)info)->DeleteFile; return STATUS_SUCCESS;
    }
    return STATUS_INVALID_INFO_CLASS;
}
NTSTATUS NTAPI NtQueryAttributesFile(OBJECT_ATTRIBUTES *oa, FILE_BASIC_INFORMATION *bi)
{
    int fi = find_file(oa->ObjectName->Buffer, oa->ObjectName->Length / 2);
    if (fi < 0) return parent_exists(oa->ObjectName->Buffer, oa->ObjectName->Length / 2) ? STATUS_OBJECT_NAME_NOT_FOUND : STATUS_OBJECT_PATH_NOT_FOUND;
    memset(bi, 0, sizeof *bi); bi->FileAttributes = F.files[fi].attrs;
    return STATUS_SUCCESS;
}
NTSTATUS NTAPI NtFlushBuffersFile(HANDLE h, IO_STATUS_BLOCK *iosb) { (void)iosb; F.flush_calls++; return get(h) ? STATUS_SUCCESS : STATUS_INVALID_HANDLE; }
NTSTATUS NTAPI NtDuplicateObject(HANDLE sp, HANDLE src, HANDLE dp, HANDLE *dst, ULONG access, ULONG attrs, ULONG opt)
{
    fake_handle *fh = get(src); (void)sp; (void)dp; (void)access; (void)opt;
    F.dup_attrs = attrs;
    if (!fh) return STATUS_INVALID_HANDLE;
    *dst = new_handle(fh->kind, fh->obj); get(*dst)->pos = fh->pos;
    return STATUS_SUCCESS;
}

/* ---- virtual memory (records only) ---- */
NTSTATUS NTAPI NtAllocateVirtualMemory(HANDLE p, PVOID *base, ULONG_PTR zb, SIZE_T *size, ULONG type, ULONG prot)
{
    (void)zb;
    F.vm_proc = p; F.vm_req_base = *base; F.vm_size = *size; F.vm_type = type; F.vm_prot = prot; F.vm_calls++;
    if (*size > 0x40000000u) return STATUS_NO_MEMORY;
    *size = (*size + 0xfff) & ~(SIZE_T)0xfff;
    /* real memory, so loader tests can map images into it; preferred bases are never honoured */
    if (F.nvm_blocks >= 64) return STATUS_NO_MEMORY;
    *base = calloc(1, *size);
    if (!*base) return STATUS_NO_MEMORY;
    F.vm_blocks[F.nvm_blocks++] = *base;
    return STATUS_SUCCESS;
}
NTSTATUS NTAPI NtFreeVirtualMemory(HANDLE p, PVOID *base, SIZE_T *size, ULONG type)
{
    int i;
    F.vm_proc = p; F.vm_req_base = *base; F.vm_size = *size; F.vm_type = type; F.vm_free_calls++;
    if (!*base) return STATUS_INVALID_PARAMETER;
    for (i = 0; i < F.nvm_blocks; i++) if (F.vm_blocks[i] == *base) { free(*base); F.vm_blocks[i] = F.vm_blocks[--F.nvm_blocks]; break; }
    return STATUS_SUCCESS;
}
NTSTATUS NTAPI NtProtectVirtualMemory(HANDLE p, PVOID *base, SIZE_T *size, ULONG np, ULONG *op)
{ (void)p; (void)base; (void)size; F.vm_prot = np; *op = PAGE_READWRITE; return STATUS_SUCCESS; }
NTSTATUS NTAPI NtQueryVirtualMemory(HANDLE p, PVOID a, int cls, PVOID buf, SIZE_T len, SIZE_T *ret)
{
    MEMORY_BASIC_INFORMATION *m = buf; (void)p; (void)len;
    F.vm_query_class = cls;
    if ((uintptr_t)a >= 0x800000000000ull) return STATUS_INVALID_PARAMETER;
    memset(m, 0, sizeof *m); m->BaseAddress = (PVOID)((uintptr_t)a & ~(uintptr_t)0xfff); m->RegionSize = 0x1000; m->State = MEM_COMMIT;
    *ret = sizeof *m; return STATUS_SUCCESS;
}

/* ---- sync objects ---- */
static void note_name(OBJECT_ATTRIBUTES *oa)
{
    F.obj_named = oa->ObjectName != NULL;
    F.obj_inherit = (oa->Attributes & OBJ_INHERIT) != 0;
    if (oa->ObjectName) { size_t n = oa->ObjectName->Length / 2; memcpy(F.obj_name, oa->ObjectName->Buffer, n * sizeof(WCHAR)); F.obj_name[n] = 0; }
    else F.obj_name[0] = 0;
}
static HANDLE create_named(int kind, OBJECT_ATTRIBUTES *oa, NTSTATUS *st, int *made_new)
{
    int i; HANDLE h;
    *st = STATUS_SUCCESS; *made_new = 1;
    if (oa->ObjectName) {
        size_t n = oa->ObjectName->Length / 2;
        for (i = 0; i < FAKE_MAX_HANDLES; i++)
            if (F.h[i].used && F.h[i].kind == kind && F.h[i].name[0] && wieq(F.h[i].name, wl(F.h[i].name), oa->ObjectName->Buffer, n)) {
                h = new_handle(kind, F.h[i].obj); get(h)->state = F.h[i].state; /* shared state mirrored at creation only */
                get(h)->shared_with = i + 1;
                *st = STATUS_OBJECT_NAME_EXISTS; *made_new = 0; memcpy(get(h)->name, F.h[i].name, sizeof F.h[i].name);
                return h;
            }
    }
    h = new_handle(kind, 0);
    if (h && oa->ObjectName) { size_t n = oa->ObjectName->Length / 2; memcpy(get(h)->name, oa->ObjectName->Buffer, n * sizeof(WCHAR)); get(h)->name[n] = 0; }
    return h;
}
NTSTATUS NTAPI NtCreateEvent(HANDLE *out, ULONG access, OBJECT_ATTRIBUTES *oa, EVENT_TYPE type, BOOLEAN initial)
{
    NTSTATUS st; int fresh; HANDLE h;
    F.ev_access = access; F.ev_type = type; F.ev_initial = initial; note_name(oa);
    if (!(h = create_named(FK_EVENT, oa, &st, &fresh))) return STATUS_INSUFFICIENT_RESOURCES;
    if (fresh) { get(h)->state = initial; get(h)->manual = type == EventNotificationType; }
    *out = h; return st;
}
static fake_handle *root_of(fake_handle *fh) { return fh->shared_with ? &F.h[fh->shared_with - 1] : fh; }
NTSTATUS NTAPI NtSetEvent(HANDLE h, LONG *prev) { fake_handle *fh = get(h); (void)prev; if (!fh || fh->kind != FK_EVENT) return STATUS_INVALID_HANDLE; root_of(fh)->state = 1; return STATUS_SUCCESS; }
NTSTATUS NTAPI NtResetEvent(HANDLE h, LONG *prev) { fake_handle *fh = get(h); (void)prev; if (!fh || fh->kind != FK_EVENT) return STATUS_INVALID_HANDLE; root_of(fh)->state = 0; return STATUS_SUCCESS; }
NTSTATUS NTAPI NtCreateMutant(HANDLE *out, ULONG access, OBJECT_ATTRIBUTES *oa, BOOLEAN owner)
{
    NTSTATUS st; int fresh; HANDLE h;
    F.ev_access = access; F.ev_initial = owner; note_name(oa);
    if (!(h = create_named(FK_MUTEX, oa, &st, &fresh))) return STATUS_INSUFFICIENT_RESOURCES;
    if (fresh) get(h)->state = owner ? 0 : 1;          /* state=1: free (signaled) */
    *out = h; return st;
}
NTSTATUS NTAPI NtReleaseMutant(HANDLE h, LONG *prev) { fake_handle *fh = get(h); (void)prev; if (!fh || fh->kind != FK_MUTEX) return STATUS_INVALID_HANDLE; if (fh->state) return STATUS_MUTANT_NOT_OWNED; fh->state = 1; return STATUS_SUCCESS; }
NTSTATUS NTAPI NtCreateSemaphore(HANDLE *out, ULONG access, OBJECT_ATTRIBUTES *oa, LONG initial, LONG maximum)
{
    NTSTATUS st; int fresh; HANDLE h;
    F.ev_access = access; note_name(oa);
    if (!(h = create_named(FK_SEM, oa, &st, &fresh))) return STATUS_INSUFFICIENT_RESOURCES;
    if (fresh) { get(h)->state = initial; get(h)->max = maximum; }
    *out = h; return st;
}
NTSTATUS NTAPI NtReleaseSemaphore(HANDLE h, LONG count, LONG *prev)
{
    fake_handle *fh = get(h);
    if (!fh || fh->kind != FK_SEM) return STATUS_INVALID_HANDLE;
    if (prev) *prev = fh->state;
    if (fh->state + count > fh->max) return STATUS_SEMAPHORE_LIMIT_EXCEEDED;
    fh->state += count; return STATUS_SUCCESS;
}

static int signaled(fake_handle *fh) { fake_handle *r = root_of(fh); return r->state != 0; }
static void consume(fake_handle *fh)
{
    fake_handle *r = root_of(fh);
    if (fh->kind == FK_EVENT && !r->manual) r->state = 0;
    else if (fh->kind == FK_SEM) r->state--;
    else if (fh->kind == FK_MUTEX) r->state = 0;
}
NTSTATUS NTAPI NtWaitForSingleObject(HANDLE h, BOOLEAN alertable, LARGE_INTEGER *timeout)
{
    fake_handle *fh = get(h);
    F.wait_calls++; F.wait_alertable = alertable; F.wait_timeout_null = timeout == NULL; if (timeout) F.wait_timeout = timeout->QuadPart;
    if (!fh) return STATUS_INVALID_HANDLE;
    if (fh->kind == FK_FILE) return STATUS_SUCCESS;
    if (fh->kind == FK_MUTEX && fh->abandoned) { fh->abandoned = 0; consume(fh); return STATUS_ABANDONED_WAIT_0; }
    if (signaled(fh)) { consume(fh); return STATUS_WAIT_0; }
    return STATUS_TIMEOUT;
}
NTSTATUS NTAPI NtWaitForMultipleObjects(ULONG n, HANDLE *hs, WAIT_TYPE type, BOOLEAN alertable, LARGE_INTEGER *timeout)
{
    ULONG i;
    F.wait_calls++; F.wait_alertable = alertable; F.wait_timeout_null = timeout == NULL; if (timeout) F.wait_timeout = timeout->QuadPart; F.wait_type = type; F.wait_n = n;
    for (i = 0; i < n; i++) if (!get(hs[i])) return STATUS_INVALID_HANDLE;
    if (type == WaitAnyObject) {
        for (i = 0; i < n; i++) { fake_handle *fh = get(hs[i]);
            if (fh->kind == FK_MUTEX && fh->abandoned) { fh->abandoned = 0; consume(fh); return (NTSTATUS)(STATUS_ABANDONED_WAIT_0 + i); }
            if (signaled(fh)) { consume(fh); return (NTSTATUS)(STATUS_WAIT_0 + i); } }
        return STATUS_TIMEOUT;
    }
    for (i = 0; i < n; i++) if (!signaled(get(hs[i]))) return STATUS_TIMEOUT;
    for (i = 0; i < n; i++) consume(get(hs[i]));
    return STATUS_WAIT_0;
}
NTSTATUS NTAPI NtDelayExecution(BOOLEAN alertable, LARGE_INTEGER *iv) { F.delay_calls++; F.delay_alertable = alertable; F.delay = iv->QuadPart; return F.delay_apc ? STATUS_USER_APC : STATUS_SUCCESS; }
NTSTATUS NTAPI NtQueryPerformanceCounter(LARGE_INTEGER *c, LARGE_INTEGER *f) { c->QuadPart = F.perf_counter; if (f) f->QuadPart = 10000000; return STATUS_SUCCESS; }
NTSTATUS NTAPI NtQuerySystemTime(LARGE_INTEGER *t) { t->QuadPart = 0; return STATUS_SUCCESS; }

#ifndef FAKE_NO_LDR
/* ---- loader fake: one module "known.dll" exporting Func (ordinal 1) ---- */
static int us_eq(UNICODE_STRING *u, const char *s) { size_t i, n = u->Length / 2; if (n != strlen(s)) return 0; for (i = 0; i < n; i++) if (lc(u->Buffer[i]) != (WCHAR)s[i]) return 0; return 1; }
NTSTATUS NTAPI LdrLoadDll(PWSTR path, ULONG *flags, UNICODE_STRING *name, PVOID *handle)
{ (void)path; (void)flags; if (us_eq(name, "known.dll")) { F.module_refs++; *handle = (PVOID)0x7ff700000000ull; return STATUS_SUCCESS; } return STATUS_DLL_NOT_FOUND; }
NTSTATUS NTAPI LdrGetDllHandle(PWSTR path, ULONG *flags, UNICODE_STRING *name, PVOID *handle)
{ (void)path; (void)flags; if (us_eq(name, "known.dll") && F.module_refs) { *handle = (PVOID)0x7ff700000000ull; return STATUS_SUCCESS; } return STATUS_DLL_NOT_FOUND; }
NTSTATUS NTAPI LdrGetProcedureAddress(PVOID m, ANSI_STRING *name, ULONG ord, PVOID *addr)
{
    if (m != (PVOID)0x7ff700000000ull) return STATUS_INVALID_HANDLE;
    if (name) { if (name->Length == 4 && !memcmp(name->Buffer, "Func", 4)) { *addr = (PVOID)0x7ff700001000ull; return STATUS_SUCCESS; } return STATUS_PROCEDURE_NOT_FOUND; }
    if (ord == 1) { *addr = (PVOID)0x7ff700001000ull; return STATUS_SUCCESS; }
    return STATUS_PROCEDURE_NOT_FOUND;
}
NTSTATUS NTAPI LdrUnloadDll(PVOID m) { if (m != (PVOID)0x7ff700000000ull || !F.module_refs) return STATUS_INVALID_HANDLE; F.module_refs--; return STATUS_SUCCESS; }
#endif
void NTAPI RtlExitUserProcess(NTSTATUS code) { F.exit_code = (DWORD)code; longjmp(fake_exit_jmp, 1); }
