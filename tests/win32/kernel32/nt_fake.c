/* Fake ntdll for host tests: in-memory file system, kernel objects, VM and loader
 * stubs, plus recording of the arguments kernel32 passed so tests can assert the
 * exact NT-level translation. */
#define LOOS_HOST_TEST 1
#include "../../../src/win32/include/loos_win.h"
#include "fake.h"
#include "../../../src/win32/include/loos_console.h"
#include <setjmp.h>
#include <pthread.h>
#include <time.h>
#include <errno.h>
#include <stdlib.h>
#include <string.h>

fake_state F;
_Thread_local DWORD k32_host_last_error;
DWORD k32_host_pid = 1234;
_Thread_local DWORD k32_host_tid = 5678;
_Thread_local PVOID k32_host_tls[64];
_Thread_local PVOID *k32_host_tls_expansion;
_Thread_local PVOID k32_host_fls;
void *k32_host_image_base = (void *)0x140000000ull;
RTL_USER_PROCESS_PARAMETERS *k32_host_params;
jmp_buf fake_exit_jmp;

static WCHAR lc(WCHAR c) { return (c >= 'A' && c <= 'Z') ? (WCHAR)(c + 32) : c; }
static int wieq(const WCHAR *a, size_t an, const WCHAR *b, size_t bn)
{ size_t i; if (an != bn) return 0; for (i = 0; i < an; i++) if (lc(a[i]) != lc(b[i])) return 0; return 1; }

extern void loos_heap_test_reset(void) __attribute__((weak));
extern void k32_env_test_reset(void) __attribute__((weak));

void fake_reset(void)
{
    int i;
    if (loos_heap_test_reset) loos_heap_test_reset();       /* cached process heap / env copies die with the address space */
    if (k32_env_test_reset) k32_env_test_reset();
    for (i = 0; i < F.nvm_blocks; i++) free(F.vm_blocks[i]);
    for (i = 0; i < 16; i++) { if (F.sec[i].used) free(F.sec[i].anon); if (F.view[i].used && !F.view[i].shared) free(F.view[i].base); }
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
        F.files[i].ctime = 132000000000000000ll + i; F.files[i].atime = F.files[i].ctime + 100; F.files[i].mtime = F.files[i].ctime + 200;
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
    if (n >= 8 && (wieq(p, n, (const WCHAR[]){'\\','?','?','\\','C','O','N','O','U','T','$'}, 11) || wieq(p, n, (const WCHAR[]){'\\','?','?','\\','C','O','N','I','N','$'}, 10))) {
        h = new_handle(FK_CONSOLE, 0);                                 /* \??\CONOUT$ / CONIN$ -> the console device */
        if (!h) return STATUS_INSUFFICIENT_RESOURCES;
        get(h)->access = access; get(h)->state = (n == 10);            /* state: 1 = input handle */
        if (!F.con.cp_out) { F.con.cp_in = F.con.cp_out = 437; F.con.mode_out = 3; F.con.mode_in = 7; }
        *out = h; iosb->Information = FILE_OPENED; return STATUS_SUCCESS;
    }
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
    if (fh && fh->kind == FK_CONSOLE) {
        size_t avail = F.con.nin - F.con.in_pos, take = len < avail ? len : avail;
        if (F.con.read_chunk && take > F.con.read_chunk) take = F.con.read_chunk;
        memcpy(buf, F.con.in + F.con.in_pos, take); F.con.in_pos += take; iosb->Information = take; iosb->Status = STATUS_SUCCESS;
        return STATUS_SUCCESS;
    }
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
    if (fh && fh->kind == FK_CONSOLE) {
        if (F.con.nout + len > sizeof F.con.out) return STATUS_DISK_FULL;
        memcpy(F.con.out + F.con.nout, buf, len); F.con.nout += len; iosb->Information = len; iosb->Status = STATUS_SUCCESS;
        return STATUS_SUCCESS;
    }
    if (!fh || fh->kind != FK_FILE) return STATUS_INVALID_HANDLE;
    f = &F.files[fh->obj];
    pos = off ? (size_t)off->QuadPart : (size_t)fh->pos;
    if (pos + len > FAKE_DATA || (F.write_limit && F.written_total + len > F.write_limit)) return STATUS_DISK_FULL;
    F.written_total += len;
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
        s->EndOfFile.QuadPart = (LONGLONG)F.files[fh->obj].size; s->Directory = F.files[fh->obj].is_dir; s->NumberOfLinks = 1; return STATUS_SUCCESS;
    }
    if (cls == FileBasicInformation) {
        FILE_BASIC_INFORMATION *b = info; fake_file *f = &F.files[fh->obj]; memset(b, 0, sizeof *b);
        b->FileAttributes = f->attrs; b->CreationTime.QuadPart = f->ctime; b->LastAccessTime.QuadPart = f->atime; b->LastWriteTime.QuadPart = f->mtime; return STATUS_SUCCESS;
    }
    if (cls == FileInternalInformation) { ((FILE_INTERNAL_INFORMATION *)info)->IndexNumber.QuadPart = 0x100000000ll + fh->obj; return STATUS_SUCCESS; }
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
    if (cls == FileBasicInformation) {
        FILE_BASIC_INFORMATION *b = info; fake_file *f = &F.files[fh->obj];
        if (b->FileAttributes) f->attrs = (b->FileAttributes == FILE_ATTRIBUTE_NORMAL ? 0 : b->FileAttributes) | (f->is_dir ? FILE_ATTRIBUTE_DIRECTORY : 0);
        if (b->CreationTime.QuadPart) f->ctime = b->CreationTime.QuadPart;
        if (b->LastAccessTime.QuadPart) f->atime = b->LastAccessTime.QuadPart;
        if (b->LastWriteTime.QuadPart) f->mtime = b->LastWriteTime.QuadPart;
        return STATUS_SUCCESS;
    }
    if (cls == FileRenameInformation) {
        FILE_RENAME_INFORMATION *r = info; size_t nl = r->FileNameLength / 2; int existing;
        if (F.rename_status) return F.rename_status;
        if (!parent_exists(r->FileName, nl)) return STATUS_OBJECT_PATH_NOT_FOUND;
        existing = find_file(r->FileName, nl);
        if (existing >= 0 && existing != fh->obj) { if (!r->ReplaceIfExists) return STATUS_OBJECT_NAME_COLLISION; F.files[existing].used = 0; }
        memcpy(F.files[fh->obj].path, r->FileName, nl * 2); F.files[fh->obj].path[nl] = 0;
        return STATUS_SUCCESS;
    }
    if (cls == FileDispositionInformation) {
        F.disposition_calls++;
        if (F.files[fh->obj].is_dir && F.files[fh->obj].has_children) return STATUS_DIRECTORY_NOT_EMPTY;
        fh->delete_on_close = ((FILE_DISPOSITION_INFORMATION *)info)->DeleteFile; return STATUS_SUCCESS;
    }
    return STATUS_INVALID_INFO_CLASS;
}
static int is_root(const WCHAR *p, size_t n) { return n >= 6 && n <= 7 && p[0] == '\\' && p[1] == '?' && p[2] == '?' && p[3] == '\\' && lc(p[4]) == 'c' && p[5] == ':' && (n == 6 || p[6] == '\\'); }

NTSTATUS NTAPI NtQueryFullAttributesFile(OBJECT_ATTRIBUTES *oa, FILE_NETWORK_OPEN_INFORMATION *ni)
{
    const WCHAR *p = oa->ObjectName->Buffer; size_t n = oa->ObjectName->Length / 2;
    int fi = find_file(p, n);
    memset(ni, 0, sizeof *ni);
    if (is_root(p, n)) { ni->FileAttributes = FILE_ATTRIBUTE_DIRECTORY; return STATUS_SUCCESS; }
    if (fi < 0) return parent_exists(p, n) ? STATUS_OBJECT_NAME_NOT_FOUND : STATUS_OBJECT_PATH_NOT_FOUND;
    ni->FileAttributes = F.files[fi].attrs; ni->EndOfFile.QuadPart = (LONGLONG)F.files[fi].size;
    ni->CreationTime.QuadPart = F.files[fi].ctime; ni->LastAccessTime.QuadPart = F.files[fi].atime; ni->LastWriteTime.QuadPart = F.files[fi].mtime;
    return STATUS_SUCCESS;
}

NTSTATUS NTAPI NtQueryAttributesFile(OBJECT_ATTRIBUTES *oa, FILE_BASIC_INFORMATION *bi)
{
    int fi;
    if (is_root(oa->ObjectName->Buffer, oa->ObjectName->Length / 2)) { memset(bi, 0, sizeof *bi); bi->FileAttributes = FILE_ATTRIBUTE_DIRECTORY; return STATUS_SUCCESS; }
    fi = find_file(oa->ObjectName->Buffer, oa->ObjectName->Length / 2);
    if (fi < 0) return parent_exists(oa->ObjectName->Buffer, oa->ObjectName->Length / 2) ? STATUS_OBJECT_NAME_NOT_FOUND : STATUS_OBJECT_PATH_NOT_FOUND;
    memset(bi, 0, sizeof *bi); bi->FileAttributes = F.files[fi].attrs;
    return STATUS_SUCCESS;
}
NTSTATUS NTAPI NtQueryVolumeInformationFile(HANDLE h, IO_STATUS_BLOCK *iosb, PVOID info, ULONG len, int cls)
{
    fake_handle *fh = get(h);
    FILE_FS_DEVICE_INFORMATION *d = info;
    (void)iosb; (void)len;
    if (!fh) return STATUS_INVALID_HANDLE;
    if (cls != FileFsDeviceInformation) return STATUS_INVALID_INFO_CLASS;
    d->Characteristics = 0;
    d->DeviceType = F.devtype_override ? F.devtype_override : fh->kind == FK_CONSOLE ? FILE_DEVICE_CONSOLE : fh->kind == FK_FILE ? FILE_DEVICE_DISK : FILE_DEVICE_UNKNOWN;
    return STATUS_SUCCESS;
}

NTSTATUS NTAPI NtDeviceIoControlFile(HANDLE h, HANDLE ev, PVOID apc, PVOID actx, IO_STATUS_BLOCK *iosb, ULONG code, PVOID in, ULONG in_len, PVOID out, ULONG out_len)
{
    fake_handle *fh = get(h);
    (void)ev; (void)apc; (void)actx; (void)in_len;
    iosb->Information = 0; iosb->Status = STATUS_SUCCESS;
    F.con.ioctl_calls++; F.con.last_ioctl = code;
    if (!fh) return STATUS_INVALID_HANDLE;
    if (fh->kind != FK_CONSOLE) return STATUS_INVALID_DEVICE_REQUEST;       /* a disk file is not a console */
    switch (code) {
    case IOCTL_CON_GET_MODE: *(ULONG *)out = fh->state ? F.con.mode_in : F.con.mode_out; return STATUS_SUCCESS;
    case IOCTL_CON_SET_MODE: if (fh->state) F.con.mode_in = *(ULONG *)in; else F.con.mode_out = *(ULONG *)in; return STATUS_SUCCESS;
    case IOCTL_CON_GET_INFO: {
        LOOS_CON_SCREEN_INFO *s = out; memset(s, 0, sizeof *s);
        s->size.x = 120; s->size.y = 300; s->cursor.x = F.con.cx; s->cursor.y = F.con.cy; s->attributes = F.con.attr;
        s->window_right = 119; s->window_bottom = 29; s->max_window_size.x = 120; s->max_window_size.y = 30; return STATUS_SUCCESS; }
    case IOCTL_CON_SET_CURSOR: F.con.cx = ((int16_t *)in)[0]; F.con.cy = ((int16_t *)in)[1]; return STATUS_SUCCESS;
    case IOCTL_CON_SET_ATTR: F.con.attr = *(uint16_t *)in; return STATUS_SUCCESS;
    case IOCTL_CON_SET_TITLE: if (in_len >= sizeof F.con.title) return STATUS_BUFFER_TOO_SMALL; memcpy(F.con.title, in, in_len); F.con.title[in_len] = 0; return STATUS_SUCCESS;
    case IOCTL_CON_GET_TITLE: { size_t l = strlen(F.con.title); if (l > out_len) return STATUS_BUFFER_TOO_SMALL; memcpy(out, F.con.title, l); iosb->Information = l; return STATUS_SUCCESS; }
    case IOCTL_CON_GET_CP: *(ULONG *)out = *(ULONG *)in ? F.con.cp_out : F.con.cp_in; return STATUS_SUCCESS;
    case IOCTL_CON_SET_CP: { LOOS_CON_SET_CP *s = in; if (s->is_output) F.con.cp_out = s->code_page; else F.con.cp_in = s->code_page; return STATUS_SUCCESS; }
    case IOCTL_CON_FLUSH_INPUT: F.con.in_pos = F.con.nin; return STATUS_SUCCESS;
    case IOCTL_CON_INPUT_COUNT: *(ULONG *)out = (ULONG)(F.con.nin - F.con.in_pos); return STATUS_SUCCESS;
    case IOCTL_CON_FILL: { LOOS_CON_FILL *f = in; F.con.last_fill_kind = (int)f->kind; F.con.last_fill_value = f->value; F.con.last_fill_count = f->count; F.con.fill_x = f->at.x; F.con.fill_y = f->at.y; return STATUS_SUCCESS; }
    default: return STATUS_INVALID_DEVICE_REQUEST;
    }
}

static int glob_match(const WCHAR *m, size_t mn, const WCHAR *s, size_t sn)
{
    if (mn == 0) return sn == 0;
    if (m[0] == '*') { size_t k; for (k = 0; k <= sn; k++) if (glob_match(m + 1, mn - 1, s + k, sn - k)) return 1; return 0; }
    if (sn == 0) return 0;
    if (m[0] == '?' || lc(m[0]) == lc(s[0])) return glob_match(m + 1, mn - 1, s + 1, sn - 1);
    return 0;
}

NTSTATUS NTAPI NtQueryDirectoryFile(HANDLE h, HANDLE ev, PVOID apc, PVOID actx, IO_STATUS_BLOCK *iosb, PVOID buf, ULONG len, FILE_INFORMATION_CLASS cls,
                                    BOOLEAN single, UNICODE_STRING *mask, BOOLEAN restart)
{
    fake_handle *fh = get(h); fake_file *dir;
    size_t dn, off = 0; int idx, matched_any = 0, count = 0, consumed_to;
    FILE_BOTH_DIR_INFORMATION *prev = NULL;
    (void)ev; (void)apc; (void)actx;
    F.dir_queries++; F.last_dir_mask_len = mask ? mask->Length : 0;
    if (!fh || fh->kind != FK_FILE) return STATUS_INVALID_HANDLE;
    if (cls != FileBothDirectoryInformation) return STATUS_INVALID_INFO_CLASS;
    dir = &F.files[fh->obj];
    if (!dir->is_dir) return STATUS_NOT_A_DIRECTORY;
    dn = wl(dir->path);
    if (restart) fh->dir_index = 0;
    consumed_to = fh->dir_index;
    /* candidate list: ".", ".." (not for a drive root), then the children in table order */
    for (idx = fh->dir_index; idx < FAKE_MAX_FILES + 2; idx++) {
        const WCHAR *name; size_t nn; fake_file *f = NULL; WCHAR dots[3] = { '.', '.', 0 };
        size_t need, k;
        FILE_BOTH_DIR_INFORMATION *e;
        if (idx == 0) { name = dots; nn = 1; }
        else if (idx == 1) { name = dots; nn = 2; }
        else {
            f = &F.files[idx - 2];
            if (!f->used || f == dir) continue;
            { size_t pl = wl(f->path); if (pl <= dn + 1 || memcmp(f->path, dir->path, dn * 2) || f->path[dn] != '\\') continue;
              name = f->path + dn + 1; nn = pl - dn - 1; for (k = 0; k < nn; k++) if (name[k] == '\\') break; if (k < nn) continue; }
        }
        if (mask && mask->Length && !glob_match(mask->Buffer, mask->Length / 2, name, nn)) continue;
        matched_any = 1;
        need = (offsetof(FILE_BOTH_DIR_INFORMATION, FileName) + nn * 2 + 7) & ~(size_t)7;
        if (off + need > len) { if (!count) return STATUS_BUFFER_OVERFLOW; break; }
        e = (FILE_BOTH_DIR_INFORMATION *)((char *)buf + off); memset(e, 0, need);
        e->FileNameLength = (ULONG)(nn * 2); memcpy(e->FileName, name, nn * 2);
        if (f) { e->FileAttributes = f->attrs; e->EndOfFile.QuadPart = (LONGLONG)f->size; e->CreationTime.QuadPart = f->ctime; e->LastAccessTime.QuadPart = f->atime; e->LastWriteTime.QuadPart = f->mtime; }
        else { e->FileAttributes = FILE_ATTRIBUTE_DIRECTORY; }
        if (f && nn > 8 && !f->is_dir) { e->ShortNameLength = 6 * 2; memcpy(e->ShortName, (const WCHAR[]){ 'S', 'H', 'O', 'R', 'T', '~' }, 12); }
        if (prev) prev->NextEntryOffset = (ULONG)((char *)e - (char *)prev);
        prev = e; off += need; count++; consumed_to = idx + 1;
        if (single) break;
    }
    if (!count) { fh->dir_index = FAKE_MAX_FILES + 2; return matched_any ? STATUS_SUCCESS : (restart ? STATUS_NO_SUCH_FILE : STATUS_NO_MORE_FILES); }
    fh->dir_index = consumed_to;
    iosb->Information = off; iosb->Status = STATUS_SUCCESS;
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
    memset(m, 0, sizeof *m); m->BaseAddress = (PVOID)((uintptr_t)a & ~(uintptr_t)0xfff); m->RegionSize = 0x1000;
    m->State = MEM_COMMIT; m->Protect = PAGE_READWRITE;                       /* every page is committed read/write ... */
    if ((uintptr_t)a < 0x10000) { m->State = 0x10000 /* MEM_FREE */; m->Protect = PAGE_NOACCESS; }   /* ... except the null page */
    if (F.noaccess_page && ((uintptr_t)a & ~(uintptr_t)0xfff) == F.noaccess_page) m->Protect = PAGE_NOACCESS;
    *ret = sizeof *m; return STATUS_SUCCESS;
}

static void note_name(OBJECT_ATTRIBUTES *oa);
/* ---- sections: file-backed views are private copies written back on flush/unmap; anon views alias the section ---- */
static int view_writeback(fake_view *v)
{
    fake_section *s = &F.sec[v->sec];
    if (!v->writable || v->shared || s->file < 0) return 0;
    if (v->off + v->len > FAKE_DATA) return 0;
    memcpy(F.files[s->file].data + v->off, v->base, v->len);
    if (v->off + v->len > F.files[s->file].size) F.files[s->file].size = v->off + v->len;
    return 1;
}
NTSTATUS NTAPI NtCreateSection(HANDLE *h, ULONG access, OBJECT_ATTRIBUTES *oa, LARGE_INTEGER *max, ULONG prot, ULONG attrs, HANDLE file)
{
    int i, made_new; NTSTATUS st; HANDLE r; fake_handle *fh;
    (void)access;
    F.map_protect = prot;
    if (oa) note_name(oa);
    for (i = 0; i < 16 && F.sec[i].used; i++) ;
    if (i == 16) return STATUS_INSUFFICIENT_RESOURCES;
    if (oa && oa->ObjectName) {                                   /* named: reuse an existing section */
        size_t n = oa->ObjectName->Length / 2; int k;
        for (k = 0; k < FAKE_MAX_HANDLES; k++)
            if (F.h[k].used && F.h[k].kind == FK_SECTION && F.h[k].name[0] && wieq(F.h[k].name, wl(F.h[k].name), oa->ObjectName->Buffer, n)) {
                r = new_handle(FK_SECTION, F.h[k].obj); *h = r; return r ? STATUS_OBJECT_NAME_EXISTS : STATUS_INSUFFICIENT_RESOURCES;
            }
    }
    (void)made_new; (void)st;
    F.sec[i].used = 1; F.sec[i].protect = prot; F.sec[i].attrs = attrs; F.sec[i].file = -1; F.sec[i].anon = NULL;
    if (file) {
        fake_handle *f = get(file);
        if (!f || f->kind != FK_FILE) { F.sec[i].used = 0; return STATUS_INVALID_HANDLE; }
        F.sec[i].file = f->obj;
        if (max && (uint64_t)max->QuadPart > F.files[f->obj].size) {
            if ((uint64_t)max->QuadPart > FAKE_DATA) { F.sec[i].used = 0; return STATUS_SECTION_TOO_BIG; }
            if (prot == PAGE_READONLY) { F.sec[i].used = 0; return STATUS_SECTION_TOO_BIG; }
            F.files[f->obj].size = (size_t)max->QuadPart;          /* a writable mapping extends the file */
        }
        F.sec[i].size = max ? (size_t)max->QuadPart : F.files[f->obj].size;
        if (!F.sec[i].size) { F.sec[i].used = 0; return STATUS_MAPPED_FILE_SIZE_ZERO; }
    } else {
        if (!max) { F.sec[i].used = 0; return STATUS_INVALID_PARAMETER; }
        F.sec[i].size = (size_t)max->QuadPart;
        F.sec[i].anon = calloc(1, F.sec[i].size);
        if (!F.sec[i].anon) { F.sec[i].used = 0; return STATUS_NO_MEMORY; }
    }
    r = new_handle(FK_SECTION, i);
    if (!r) { free(F.sec[i].anon); F.sec[i].used = 0; return STATUS_INSUFFICIENT_RESOURCES; }
    fh = get(r);
    if (oa && oa->ObjectName) { size_t n = oa->ObjectName->Length / 2; memcpy(fh->name, oa->ObjectName->Buffer, n * sizeof(WCHAR)); fh->name[n] = 0; }
    *h = r;
    return STATUS_SUCCESS;
}
NTSTATUS NTAPI NtOpenSection(HANDLE *h, ULONG access, OBJECT_ATTRIBUTES *oa)
{
    int k; size_t n;
    (void)access;
    if (!oa || !oa->ObjectName) return STATUS_INVALID_PARAMETER;
    n = oa->ObjectName->Length / 2;
    for (k = 0; k < FAKE_MAX_HANDLES; k++)
        if (F.h[k].used && F.h[k].kind == FK_SECTION && F.h[k].name[0] && wieq(F.h[k].name, wl(F.h[k].name), oa->ObjectName->Buffer, n)) {
            *h = new_handle(FK_SECTION, F.h[k].obj); return *h ? STATUS_SUCCESS : STATUS_INSUFFICIENT_RESOURCES;
        }
    return STATUS_OBJECT_NAME_NOT_FOUND;
}
NTSTATUS NTAPI NtMapViewOfSection(HANDLE section, HANDLE proc, PVOID *base, ULONG_PTR zb, SIZE_T commit, LARGE_INTEGER *offset, SIZE_T *size, SECTION_INHERIT inh, ULONG at, ULONG prot)
{
    fake_handle *fh = get(section); fake_section *s; fake_view *v; int i; uint64_t off = offset ? (uint64_t)offset->QuadPart : 0; size_t len;
    (void)proc; (void)zb; (void)commit; (void)inh; (void)at;
    F.map_calls++; F.vm_prot = prot;
    if (!fh || fh->kind != FK_SECTION) return STATUS_INVALID_HANDLE;
    s = &F.sec[fh->obj];
    if (off & 0xFFFF) return STATUS_MAPPED_ALIGNMENT;
    if (off >= s->size) return STATUS_INVALID_PARAMETER;
    len = *size ? *size : s->size - (size_t)off;
    if (off + len > s->size) return STATUS_INVALID_VIEW_SIZE;
    if ((prot & (PAGE_READWRITE | PAGE_EXECUTE_READWRITE)) && s->protect != PAGE_READWRITE && s->protect != PAGE_EXECUTE_READWRITE) return STATUS_SECTION_PROTECTION;
    for (i = 0; i < 16 && F.view[i].used; i++) ;
    if (i == 16) return STATUS_NO_MEMORY;
    v = &F.view[i]; memset(v, 0, sizeof *v);
    v->used = 1; v->sec = fh->obj; v->off = (size_t)off; v->len = len;
    v->writable = (prot & (PAGE_READWRITE | PAGE_EXECUTE_READWRITE)) != 0;
    if (s->file < 0) { v->base = s->anon + off; v->shared = 1; }
    else {
        v->base = calloc(1, len + 1);
        if (!v->base) { v->used = 0; return STATUS_NO_MEMORY; }
        if (off < F.files[s->file].size) { size_t have = F.files[s->file].size - (size_t)off; memcpy(v->base, F.files[s->file].data + off, have < len ? have : len); }
    }
    *base = v->base; *size = len;
    return STATUS_SUCCESS;
}
NTSTATUS NTAPI NtUnmapViewOfSection(HANDLE proc, PVOID base)
{
    int i; (void)proc;
    F.unmap_calls++;
    for (i = 0; i < 16; i++) if (F.view[i].used && F.view[i].base == base) {
        view_writeback(&F.view[i]);
        if (!F.view[i].shared) free(F.view[i].base);
        F.view[i].used = 0;
        return STATUS_SUCCESS;
    }
    return STATUS_NOT_MAPPED_VIEW;
}
NTSTATUS NTAPI NtFlushVirtualMemory(HANDLE proc, PVOID *base, SIZE_T *size, IO_STATUS_BLOCK *iosb)
{
    int i; (void)proc; (void)size;
    F.flush_view_calls++;
    if (iosb) { iosb->Status = 0; iosb->Information = 0; }
    for (i = 0; i < 16; i++) if (F.view[i].used && (uint8_t *)*base >= F.view[i].base && (uint8_t *)*base < F.view[i].base + F.view[i].len) { view_writeback(&F.view[i]); return STATUS_SUCCESS; }
    return STATUS_NOT_COMMITTED;
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
/* ---- thread parking: real blocking so multi-threaded tests exercise the user-mode locks ---- */
#define MAX_PARK 64
static pthread_mutex_t park_mu = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t park_cv = PTHREAD_COND_INITIALIZER;
static struct { DWORD tid; int alerted; int used; } park[MAX_PARK];
static int park_slot(DWORD tid)
{
    int i, free_i = -1;
    for (i = 0; i < MAX_PARK; i++) { if (park[i].used && park[i].tid == tid) return i; if (!park[i].used && free_i < 0) free_i = i; }
    if (free_i >= 0) { park[free_i].used = 1; park[free_i].tid = tid; park[free_i].alerted = 0; }
    return free_i;
}
NTSTATUS NTAPI NtWaitForAlertByThreadId(PVOID hint, LARGE_INTEGER *timeout)
{
    int s;
    struct timespec dl;
    (void)hint;
    pthread_mutex_lock(&park_mu);
    s = park_slot(k32_host_tid);
    if (timeout) {
        clock_gettime(CLOCK_REALTIME, &dl);
        { long long ns = timeout->QuadPart < 0 ? -timeout->QuadPart * 100 : 0;
          dl.tv_sec += ns / 1000000000; dl.tv_nsec += ns % 1000000000; if (dl.tv_nsec >= 1000000000) { dl.tv_sec++; dl.tv_nsec -= 1000000000; } }
    }
    while (s >= 0 && !park[s].alerted) {
        if (timeout) { if (pthread_cond_timedwait(&park_cv, &park_mu, &dl) == ETIMEDOUT) { pthread_mutex_unlock(&park_mu); return STATUS_TIMEOUT; } }
        else pthread_cond_wait(&park_cv, &park_mu);
    }
    if (s >= 0) park[s].alerted = 0;
    pthread_mutex_unlock(&park_mu);
    return STATUS_ALERTED;
}
NTSTATUS NTAPI NtAlertThreadByThreadId(HANDLE tid)
{
    int s;
    pthread_mutex_lock(&park_mu);
    s = park_slot((DWORD)(uintptr_t)tid);
    if (s >= 0) park[s].alerted = 1;
    pthread_cond_broadcast(&park_cv);
    pthread_mutex_unlock(&park_mu);
    return STATUS_SUCCESS;
}

/* ---- threads and processes ---- */
static HANDLE new_task(int kind, int id)
{
    HANDLE h = new_handle(kind, id);
    if (h) { get(h)->exit_status = STATUS_PENDING; }
    return h;
}
NTSTATUS NTAPI NtCreateThreadEx(HANDLE *out, ULONG access, OBJECT_ATTRIBUTES *oa, HANDLE proc, PVOID start, PVOID arg, ULONG flags,
                                SIZE_T zero, SIZE_T commit, SIZE_T reserve, PS_ATTRIBUTE_LIST *attrs)
{
    HANDLE h; int tid = 7000 + ++F.next_tid;
    (void)zero;
    F.thr.access = access; F.thr.flags = flags; F.thr.process = proc; F.thr.start = start; F.thr.arg = arg; F.thr.commit = commit;
    F.thr.reserve = reserve; F.thr.oattrs = oa ? oa->Attributes : 0; F.thr.calls++;
    if (!(h = new_task(FK_THREAD, tid))) return STATUS_INSUFFICIENT_RESOURCES;
    get(h)->suspend = (flags & THREAD_CREATE_SUSPENDED) ? 1 : 0;
    if (attrs && attrs->Attributes[0].Attribute == PS_ATTRIBUTE_CLIENT_ID) {
        CLIENT_ID *cid = attrs->Attributes[0].ValuePtr; cid->UniqueProcess = (HANDLE)(uintptr_t)k32_host_pid; cid->UniqueThread = (HANDLE)(uintptr_t)tid;
    }
    *out = h; return STATUS_SUCCESS;
}
NTSTATUS NTAPI NtCreateUserProcess(HANDLE *proc, HANDLE *thread, ULONG pa, ULONG ta, OBJECT_ATTRIBUTES *poa, OBJECT_ATTRIBUTES *toa,
                                   ULONG pflags, ULONG tflags, PVOID params, PS_CREATE_INFO *info, PS_ATTRIBUTE_LIST *attrs)
{
    RTL_USER_PROCESS_PARAMETERS *p = params;
    int pid = 9000 + ++F.next_pid, tid = 8000 + F.next_pid, i;
    HANDLE hp, ht;
    (void)info;
    F.cu.calls++; F.cu.pflags = pflags; F.cu.tflags = tflags; F.cu.paccess = pa; F.cu.taccess = ta;
    F.cu.poattrs = poa ? poa->Attributes : 0; F.cu.toattrs = toa ? toa->Attributes : 0;
    F.cu.params_len = p->Length < sizeof F.cu.params ? p->Length : sizeof F.cu.params;
    memcpy(F.cu.params, p, F.cu.params_len);
    for (i = 0; i < 2; i++) if (attrs->Attributes[i].Attribute == PS_ATTRIBUTE_IMAGE_NAME) {
        size_t l = attrs->Attributes[i].Size / 2; if (l > 511) l = 511;
        memcpy(F.cu.image, attrs->Attributes[i].ValuePtr, l * 2); F.cu.image[l] = 0;
    }
    if (F.cu.fail) return F.cu.fail;
    hp = new_task(FK_PROCESS, pid); ht = new_task(FK_THREAD, tid);
    get(ht)->suspend = (tflags & THREAD_CREATE_SUSPENDED) ? 1 : 0;
    for (i = 0; i < 2; i++) if (attrs->Attributes[i].Attribute == PS_ATTRIBUTE_CLIENT_ID) {
        CLIENT_ID *cid = attrs->Attributes[i].ValuePtr; cid->UniqueProcess = (HANDLE)(uintptr_t)pid; cid->UniqueThread = (HANDLE)(uintptr_t)tid;
    }
    *proc = hp; *thread = ht; return STATUS_SUCCESS;
}
NTSTATUS NTAPI NtTerminateThread(HANDLE h, NTSTATUS st)
{
    F.term_calls++; F.term_status = st; F.term_handle = h;
    if ((intptr_t)h == -2) return STATUS_SUCCESS;
    { fake_handle *fh = get(h); if (!fh || fh->kind != FK_THREAD) return STATUS_INVALID_HANDLE; fh->exit_status = st; return STATUS_SUCCESS; }
}
NTSTATUS NTAPI NtTerminateProcess(HANDLE h, NTSTATUS st)
{
    F.term_calls++; F.term_status = st; F.term_handle = h;
    if ((intptr_t)h == -1) return STATUS_SUCCESS;
    { fake_handle *fh = get(h); if (!fh || fh->kind != FK_PROCESS) return STATUS_INVALID_HANDLE; fh->exit_status = st; return STATUS_SUCCESS; }
}
NTSTATUS NTAPI NtSuspendThread(HANDLE h, ULONG *prev) { fake_handle *fh = get(h); F.suspend_calls++; if (!fh || fh->kind != FK_THREAD) return STATUS_INVALID_HANDLE; if (prev) *prev = (ULONG)fh->suspend; fh->suspend++; return STATUS_SUCCESS; }
NTSTATUS NTAPI NtResumeThread(HANDLE h, ULONG *prev) { fake_handle *fh = get(h); F.resume_calls++; if (!fh || fh->kind != FK_THREAD) return STATUS_INVALID_HANDLE; if (prev) *prev = (ULONG)fh->suspend; if (fh->suspend) fh->suspend--; return STATUS_SUCCESS; }
NTSTATUS NTAPI NtYieldExecution(void) { return F.yield_result ? (NTSTATUS)0x40000024 : STATUS_SUCCESS; }
NTSTATUS NTAPI NtOpenThread(HANDLE *out, ULONG access, OBJECT_ATTRIBUTES *oa, CLIENT_ID *id)
{
    (void)access; F.thr.oattrs = oa->Attributes;
    if ((uintptr_t)id->UniqueThread < 7000) return STATUS_INVALID_PARAMETER;
    *out = new_task(FK_THREAD, (int)(uintptr_t)id->UniqueThread); return *out ? STATUS_SUCCESS : STATUS_INSUFFICIENT_RESOURCES;
}
NTSTATUS NTAPI NtOpenProcess(HANDLE *out, ULONG access, OBJECT_ATTRIBUTES *oa, CLIENT_ID *id)
{
    (void)access; F.thr.oattrs = oa->Attributes;
    if ((uintptr_t)id->UniqueProcess < 9000) return STATUS_INVALID_PARAMETER;
    *out = new_task(FK_PROCESS, (int)(uintptr_t)id->UniqueProcess); return *out ? STATUS_SUCCESS : STATUS_INSUFFICIENT_RESOURCES;
}
NTSTATUS NTAPI NtQueryInformationThread(HANDLE h, THREAD_INFORMATION_CLASS cls, PVOID buf, ULONG len, ULONG *ret)
{
    THREAD_BASIC_INFORMATION *t = buf; fake_handle *fh = (intptr_t)h == -2 ? NULL : get(h);
    (void)len; if (ret) *ret = sizeof *t;
    if (cls != ThreadBasicInformation) return STATUS_INVALID_INFO_CLASS;
    memset(t, 0, sizeof *t);
    if ((intptr_t)h == -2) { t->ExitStatus = STATUS_PENDING; t->ClientId.UniqueProcess = (HANDLE)(uintptr_t)k32_host_pid; t->ClientId.UniqueThread = (HANDLE)(uintptr_t)k32_host_tid; return STATUS_SUCCESS; }
    if (!fh || fh->kind != FK_THREAD) return STATUS_INVALID_HANDLE;
    t->ExitStatus = fh->exit_status; t->ClientId.UniqueProcess = (HANDLE)(uintptr_t)k32_host_pid; t->ClientId.UniqueThread = (HANDLE)(uintptr_t)fh->obj; t->BasePriority = (LONG)fh->max;
    return STATUS_SUCCESS;
}
NTSTATUS NTAPI NtSetInformationThread(HANDLE h, THREAD_INFORMATION_CLASS cls, PVOID buf, ULONG len)
{
    fake_handle *fh = get(h); (void)len;
    if (!fh || fh->kind != FK_THREAD) return STATUS_INVALID_HANDLE;
    if (cls != ThreadBasePriority) return STATUS_INVALID_INFO_CLASS;
    fh->max = *(LONG *)buf; F.last_priority = *(LONG *)buf; return STATUS_SUCCESS;
}
NTSTATUS NTAPI NtQueryInformationProcess(HANDLE h, PROCESS_INFORMATION_CLASS cls, PVOID buf, ULONG len, ULONG *ret)
{
    PROCESS_BASIC_INFORMATION *p = buf; fake_handle *fh = get(h);
    (void)len; if (ret) *ret = sizeof *p;
    if (cls != ProcessBasicInformation) return STATUS_INVALID_INFO_CLASS;
    if (!fh || fh->kind != FK_PROCESS) return STATUS_INVALID_HANDLE;
    memset(p, 0, sizeof *p); p->ExitStatus = fh->exit_status; p->UniqueProcessId = (ULONG_PTR)fh->obj; return STATUS_SUCCESS;
}

NTSTATUS NTAPI NtDelayExecution(BOOLEAN alertable, LARGE_INTEGER *iv) { F.delay_calls++; F.delay_alertable = alertable; F.delay = iv->QuadPart; return F.delay_apc ? STATUS_USER_APC : STATUS_SUCCESS; }
NTSTATUS NTAPI NtQueryPerformanceCounter(LARGE_INTEGER *c, LARGE_INTEGER *f) { c->QuadPart = F.perf_counter; if (f) f->QuadPart = 10000000; return STATUS_SUCCESS; }
NTSTATUS NTAPI NtQuerySystemTime(LARGE_INTEGER *t) { t->QuadPart = F.system_time; return STATUS_SUCCESS; }
NTSTATUS NTAPI NtQuerySystemInformation(SYSTEM_INFORMATION_CLASS cls, PVOID buf, ULONG len, ULONG *ret)
{
    if (ret) *ret = 0;
    if (cls == SystemBasicInformation && len >= sizeof(SYSTEM_BASIC_INFORMATION)) {
        SYSTEM_BASIC_INFORMATION *b = buf; memset(b, 0, sizeof *b);
        b->PageSize = 4096; b->AllocationGranularity = 65536; b->MinimumUserModeAddress = 0x10000; b->MaximumUserModeAddress = 0x7FFFFFFEFFFFull;
        b->ActiveProcessorsAffinityMask = 0xF; b->NumberOfProcessors = 4; return STATUS_SUCCESS;
    }
    if (cls == SystemProcessorInformation && len >= sizeof(SYSTEM_PROCESSOR_INFORMATION)) {
        SYSTEM_PROCESSOR_INFORMATION *p = buf; memset(p, 0, sizeof *p); p->ProcessorArchitecture = 9; p->ProcessorLevel = 6; p->ProcessorRevision = 0x9A03; return STATUS_SUCCESS;
    }
    return STATUS_INVALID_INFO_CLASS;
}

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
NTSTATUS NTAPI LdrGetDllFullName(PVOID m, UNICODE_STRING *out)
{
    static const char path[] = "C:\\Windows\\System32\\known.dll";
    size_t i, n = sizeof path - 1;
    if (m != (PVOID)0x7ff700000000ull) return STATUS_INVALID_HANDLE;
    if ((n + 1) * 2 > out->MaximumLength) return STATUS_BUFFER_TOO_SMALL;
    for (i = 0; i <= n; i++) out->Buffer[i] = (WCHAR)path[i];
    out->Length = (USHORT)(n * 2); return STATUS_SUCCESS;
}
void NTAPI RtlExitUserThread(NTSTATUS st) { F.term_calls++; F.term_status = st; F.term_handle = (HANDLE)(intptr_t)-2; longjmp(fake_exit_jmp, 1); }
#endif
void NTAPI RtlExitUserProcess(NTSTATUS code) { F.exit_code = (DWORD)code; longjmp(fake_exit_jmp, 1); }
