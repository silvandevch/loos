/* Console API and GetFileType, a client of the console device contract in include/loos_console.h.
 * Data moves as UTF-8 through NtReadFile / NtWriteFile; control goes through NtDeviceIoControlFile. */
#include "k32_internal.h"
#include <string.h>

static NTSTATUS ioctl(HANDLE h, ULONG code, PVOID in, ULONG in_len, PVOID out, ULONG out_len, ULONG_PTR *info)
{
    IO_STATUS_BLOCK iosb;
    NTSTATUS st;
    memset(&iosb, 0, sizeof iosb);
    st = NtDeviceIoControlFile(h, NULL, NULL, NULL, &iosb, code, in, in_len, out, out_len);
    if (st == STATUS_PENDING) { st = NtWaitForSingleObject(h, FALSE, NULL); if (NT_SUCCESS(st)) st = iosb.Status; }
    if (info) *info = iosb.Information;
    return st;
}

/* a console ioctl that failed means "this handle is not a console" as far as Win32 callers can tell */
static BOOL con_fail(NTSTATUS st)
{
    *k32_last_error_ptr() = (st == STATUS_INVALID_DEVICE_REQUEST || st == STATUS_INVALID_HANDLE) ? ERROR_INVALID_HANDLE : RtlNtStatusToDosError(st);
    return FALSE;
}

DWORD WINAPI GetFileType(HANDLE h)
{
    IO_STATUS_BLOCK iosb;
    FILE_FS_DEVICE_INFORMATION di;
    NTSTATUS st;
    if (h == NULL || h == INVALID_HANDLE_VALUE) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FILE_TYPE_UNKNOWN; }
    st = NtQueryVolumeInformationFile(h, &iosb, &di, sizeof di, FileFsDeviceInformation);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FILE_TYPE_UNKNOWN; }
    *k32_last_error_ptr() = ERROR_SUCCESS;
    switch (di.DeviceType) {
    case 0x02: case FILE_DEVICE_DISK: case 0x14: case 0x08: case 0x09: case 0x24: return FILE_TYPE_DISK;   /* cdrom, disk, network fs, file system, virtual disk */
    case FILE_DEVICE_NAMED_PIPE: case 0x12: return FILE_TYPE_PIPE;                                          /* named pipe, network (sockets) */
    case FILE_DEVICE_CONSOLE: case FILE_DEVICE_NULL: case 0x1b: case 0x16: case 0x0b: case 0x05: return FILE_TYPE_CHAR;
    default: return FILE_TYPE_CHAR;
    }
}

/* ---------- output ---------- */
BOOL WINAPI WriteConsoleW(HANDLE h, const void *buf, DWORD n, LPDWORD written, LPVOID reserved)
{
    const WCHAR *w = buf;
    DWORD done = 0;
    (void)reserved;
    if (written) *written = 0;
    if (!buf && n) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    while (done < n) {
        char u8[4096 * 3];
        IO_STATUS_BLOCK iosb;
        DWORD chunk = n - done > 4096 ? 4096 : n - done;
        int bytes;
        NTSTATUS st;
        if (chunk < n - done && w[done + chunk - 1] >= 0xD800 && w[done + chunk - 1] <= 0xDBFF) chunk--;     /* never split a surrogate pair */
        bytes = WideCharToMultiByte(CP_UTF8, 0, w + done, (int)chunk, u8, sizeof u8, NULL, NULL);
        if (bytes <= 0) return FALSE;
        st = NtWriteFile(h, NULL, NULL, NULL, &iosb, u8, (ULONG)bytes, NULL, NULL);
        if (!NT_SUCCESS(st)) { if (written) *written = done; return con_fail(st); }
        done += chunk;
    }
    if (written) *written = done;
    return TRUE;
}

BOOL WINAPI WriteConsoleA(HANDLE h, const void *buf, DWORD n, LPDWORD written, LPVOID reserved)
{
    UINT cp = GetConsoleOutputCP();
    const char *a = buf;
    DWORD done = 0;
    if (written) *written = 0;
    if (!buf && n) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    if (cp == 0) cp = GetOEMCP();
    while (done < n) {
        WCHAR w[2048];
        DWORD chunk = n - done > 2048 ? 2048 : n - done, wrote;
        int wn = MultiByteToWideChar(cp, 0, a + done, (int)chunk, w, 2048);
        /* a multi-byte sequence cut at the chunk edge would be mangled; UTF-8 is the only multi-byte console page */
        if (cp == CP_UTF8 && chunk < n - done) {
            while (chunk > 1 && ((unsigned char)a[done + chunk - 1] & 0xC0) == 0x80) chunk--;
            if (chunk > 1 && ((unsigned char)a[done + chunk - 1] & 0xC0) == 0xC0) chunk--;
            wn = MultiByteToWideChar(cp, 0, a + done, (int)chunk, w, 2048);
        }
        if (wn <= 0) return FALSE;
        if (!WriteConsoleW(h, w, (DWORD)wn, &wrote, reserved)) { if (written) *written = done; return FALSE; }
        done += chunk;
    }
    if (written) *written = done;
    return TRUE;
}

/* ---------- input ---------- */
/* Bytes of a UTF-8 sequence that arrived split across two reads are held here until the rest comes. */
static struct { HANDLE h; unsigned char bytes[4]; unsigned n; } carry;

BOOL WINAPI ReadConsoleW(HANDLE h, LPVOID buf, DWORD n, LPDWORD read, LPVOID input_control)
{
    unsigned char raw[4096 + 4];
    IO_STATUS_BLOCK iosb;
    WCHAR *w = buf;
    unsigned have = 0, keep = 0;
    ULONG want;
    int wn;
    NTSTATUS st;
    (void)input_control;
    if (read) *read = 0;
    if (!buf || !read) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    if (n == 0) return TRUE;
    if (carry.h == h && carry.n) { memcpy(raw, carry.bytes, carry.n); have = carry.n; carry.n = 0; }
    want = n > 4096 ? 4096 : n;                             /* at most n bytes: UTF-8 never yields more UTF-16 units than bytes */
    if (have >= want) want = have + 1;
    st = NtReadFile(h, NULL, NULL, NULL, &iosb, raw + have, want - have, NULL, NULL);
    if (st == STATUS_PENDING) { st = NtWaitForSingleObject(h, FALSE, NULL); if (NT_SUCCESS(st)) st = iosb.Status; }
    if (st == STATUS_END_OF_FILE) { iosb.Information = 0; st = STATUS_SUCCESS; }
    if (!NT_SUCCESS(st)) return con_fail(st);
    have += (unsigned)iosb.Information;
    /* hold back an incomplete trailing sequence */
    if (have) {
        unsigned i = have, back = 0;
        while (i > 0 && back < 4 && (raw[i - 1] & 0xC0) == 0x80) { i--; back++; }
        if (i > 0) {
            unsigned char lead = raw[i - 1];
            unsigned need = lead >= 0xF0 ? 4 : lead >= 0xE0 ? 3 : lead >= 0xC0 ? 2 : 1;
            if (need > 1 && back + 1 < need) keep = back + 1;
        }
        if (keep) { memcpy(carry.bytes, raw + have - keep, keep); carry.n = keep; carry.h = h; have -= keep; }
    }
    if (have == 0) return TRUE;
    wn = MultiByteToWideChar(CP_UTF8, 0, (const char *)raw, (int)have, w, (int)n);
    if (wn <= 0) return FALSE;
    *read = (DWORD)wn;
    return TRUE;
}

BOOL WINAPI ReadConsoleA(HANDLE h, LPVOID buf, DWORD n, LPDWORD read, LPVOID input_control)
{
    WCHAR w[4096];
    DWORD wr = 0;
    UINT cp = GetConsoleCP();
    int bn;
    if (read) *read = 0;
    if (!buf || !read) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    if (cp == 0) cp = GetOEMCP();
    if (!ReadConsoleW(h, w, n > 4096 ? 4096 : n, &wr, input_control)) return FALSE;
    if (wr == 0) return TRUE;
    bn = WideCharToMultiByte(cp, 0, w, (int)wr, buf, (int)n, NULL, NULL);
    if (bn <= 0) return FALSE;
    *read = (DWORD)bn;
    return TRUE;
}

/* ---------- control ---------- */
BOOL WINAPI GetConsoleMode(HANDLE h, LPDWORD mode)
{
    ULONG m = 0;
    NTSTATUS st;
    if (!mode) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    if (h == NULL || h == INVALID_HANDLE_VALUE) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    st = ioctl(h, IOCTL_CON_GET_MODE, NULL, 0, &m, sizeof m, NULL);
    if (!NT_SUCCESS(st)) return con_fail(st);
    *mode = m;
    return TRUE;
}
BOOL WINAPI SetConsoleMode(HANDLE h, DWORD mode)
{
    ULONG m = mode;
    NTSTATUS st = ioctl(h, IOCTL_CON_SET_MODE, &m, sizeof m, NULL, 0, NULL);
    return NT_SUCCESS(st) ? TRUE : con_fail(st);
}
BOOL WINAPI GetConsoleScreenBufferInfo(HANDLE h, CONSOLE_SCREEN_BUFFER_INFO *info)
{
    LOOS_CON_SCREEN_INFO si;
    NTSTATUS st;
    if (!info) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    st = ioctl(h, IOCTL_CON_GET_INFO, NULL, 0, &si, sizeof si, NULL);
    if (!NT_SUCCESS(st)) return con_fail(st);
    info->dwSize.X = si.size.x; info->dwSize.Y = si.size.y; info->dwCursorPosition.X = si.cursor.x; info->dwCursorPosition.Y = si.cursor.y;
    info->wAttributes = si.attributes; info->Left = si.window_left; info->Top = si.window_top; info->Right = si.window_right; info->Bottom = si.window_bottom;
    info->dwMaximumWindowSize.X = si.max_window_size.x; info->dwMaximumWindowSize.Y = si.max_window_size.y;
    return TRUE;
}
BOOL WINAPI SetConsoleCursorPosition(HANDLE h, COORD pos)
{
    int16_t xy[2] = { pos.X, pos.Y };
    NTSTATUS st = ioctl(h, IOCTL_CON_SET_CURSOR, xy, sizeof xy, NULL, 0, NULL);
    return NT_SUCCESS(st) ? TRUE : con_fail(st);
}
BOOL WINAPI SetConsoleTextAttribute(HANDLE h, WORD attr)
{
    uint16_t a = attr;
    NTSTATUS st = ioctl(h, IOCTL_CON_SET_ATTR, &a, sizeof a, NULL, 0, NULL);
    return NT_SUCCESS(st) ? TRUE : con_fail(st);
}
static BOOL fill(HANDLE h, ULONG kind, ULONG value, DWORD n, COORD at, LPDWORD written)
{
    LOOS_CON_FILL f;
    NTSTATUS st;
    f.kind = kind; f.value = value; f.count = n; f.at.x = at.X; f.at.y = at.Y;
    st = ioctl(h, IOCTL_CON_FILL, &f, sizeof f, NULL, 0, NULL);
    if (!NT_SUCCESS(st)) { if (written) *written = 0; return con_fail(st); }
    if (written) *written = n;
    return TRUE;
}
BOOL WINAPI FillConsoleOutputCharacterW(HANDLE h, WCHAR ch, DWORD n, COORD at, LPDWORD written) { return fill(h, 0, ch, n, at, written); }
BOOL WINAPI FillConsoleOutputAttribute(HANDLE h, WORD attr, DWORD n, COORD at, LPDWORD written) { return fill(h, 1, attr, n, at, written); }

static HANDLE console_handle(void)
{
    RTL_USER_PROCESS_PARAMETERS *p = k32_params();
    return p ? p->ConsoleHandle : NULL;
}
static UINT get_cp(ULONG is_output)
{
    ULONG cp = 0;
    HANDLE h = console_handle();
    NTSTATUS st;
    if (!h) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return 0; }
    st = ioctl(h, IOCTL_CON_GET_CP, &is_output, sizeof is_output, &cp, sizeof cp, NULL);
    if (!NT_SUCCESS(st)) { con_fail(st); return 0; }
    return cp;
}
static BOOL set_cp(ULONG is_output, UINT cp)
{
    LOOS_CON_SET_CP s;
    HANDLE h = console_handle();
    NTSTATUS st;
    if (!h) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    if (!IsValidCodePage(cp)) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    s.is_output = is_output; s.code_page = cp;
    st = ioctl(h, IOCTL_CON_SET_CP, &s, sizeof s, NULL, 0, NULL);
    return NT_SUCCESS(st) ? TRUE : con_fail(st);
}
UINT WINAPI GetConsoleCP(void) { return get_cp(0); }
UINT WINAPI GetConsoleOutputCP(void) { return get_cp(1); }
BOOL WINAPI SetConsoleCP(UINT cp) { return set_cp(0, cp); }
BOOL WINAPI SetConsoleOutputCP(UINT cp) { return set_cp(1, cp); }

BOOL WINAPI SetConsoleTitleW(LPCWSTR title)
{
    char u8[1024 * 3];
    HANDLE h = console_handle();
    int n;
    NTSTATUS st;
    if (!title || !h) { *k32_last_error_ptr() = title ? ERROR_INVALID_HANDLE : ERROR_INVALID_PARAMETER; return FALSE; }
    n = WideCharToMultiByte(CP_UTF8, 0, title, -1, u8, sizeof u8, NULL, NULL);
    if (n <= 0) return FALSE;
    st = ioctl(h, IOCTL_CON_SET_TITLE, u8, (ULONG)(n - 1), NULL, 0, NULL);
    return NT_SUCCESS(st) ? TRUE : con_fail(st);
}
DWORD WINAPI GetConsoleTitleW(LPWSTR buf, DWORD n)
{
    char u8[1024 * 3];
    ULONG_PTR got = 0;
    HANDLE h = console_handle();
    int wn;
    NTSTATUS st;
    if (!buf || n == 0 || !h) { *k32_last_error_ptr() = h ? ERROR_INVALID_PARAMETER : ERROR_INVALID_HANDLE; return 0; }
    st = ioctl(h, IOCTL_CON_GET_TITLE, NULL, 0, u8, sizeof u8, &got);
    if (!NT_SUCCESS(st)) { con_fail(st); return 0; }
    wn = MultiByteToWideChar(CP_UTF8, 0, u8, (int)got, NULL, 0);
    if ((DWORD)wn + 1 > n) { buf[0] = 0; *k32_last_error_ptr() = ERROR_INSUFFICIENT_BUFFER; return 0; }
    if (wn) MultiByteToWideChar(CP_UTF8, 0, u8, (int)got, buf, wn);
    buf[wn] = 0;
    return (DWORD)wn;
}

BOOL WINAPI FlushConsoleInputBuffer(HANDLE h)
{
    NTSTATUS st = ioctl(h, IOCTL_CON_FLUSH_INPUT, NULL, 0, NULL, 0, NULL);
    if (NT_SUCCESS(st)) carry.n = 0;
    return NT_SUCCESS(st) ? TRUE : con_fail(st);
}
BOOL WINAPI GetNumberOfConsoleInputEvents(HANDLE h, LPDWORD count)
{
    ULONG c = 0;
    NTSTATUS st;
    if (!count) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    st = ioctl(h, IOCTL_CON_INPUT_COUNT, NULL, 0, &c, sizeof c, NULL);
    if (!NT_SUCCESS(st)) return con_fail(st);
    *count = c;
    return TRUE;
}
HANDLE WINAPI GetConsoleWindow(void) { return NULL; }

/* ---------- Ctrl+C / Ctrl+Break / Close ---------- */
#define MAX_CTRL_HANDLERS 32
static PHANDLER_ROUTINE ctrl_handlers[MAX_CTRL_HANDLERS];
static unsigned n_ctrl;
static int ctrl_c_ignored;
static volatile int ctrl_lock;

BOOL WINAPI SetConsoleCtrlHandler(PHANDLER_ROUTINE routine, BOOL add)
{
    unsigned i;
    BOOL ok = TRUE;
    while (__atomic_exchange_n(&ctrl_lock, 1, __ATOMIC_ACQUIRE)) __builtin_ia32_pause();
    if (!routine) ctrl_c_ignored = add;
    else if (add) { if (n_ctrl < MAX_CTRL_HANDLERS) ctrl_handlers[n_ctrl++] = routine; else { *k32_last_error_ptr() = ERROR_NOT_ENOUGH_MEMORY; ok = FALSE; } }
    else {
        ok = FALSE;
        for (i = 0; i < n_ctrl; i++) if (ctrl_handlers[i] == routine) { memmove(ctrl_handlers + i, ctrl_handlers + i + 1, (n_ctrl - i - 1) * sizeof ctrl_handlers[0]); n_ctrl--; ok = TRUE; break; }
        if (!ok) *k32_last_error_ptr() = ERROR_INVALID_PARAMETER;
    }
    __atomic_store_n(&ctrl_lock, 0, __ATOMIC_RELEASE);
    return ok;
}

/* Newest handler first; the first to return TRUE stops the chain. The default handler ends the process. */
DWORD WINAPI CtrlRoutine(LPVOID type_arg)
{
    DWORD type = (DWORD)(uintptr_t)type_arg;
    PHANDLER_ROUTINE copy[MAX_CTRL_HANDLERS];
    unsigned n, i;
    int ignore;
    while (__atomic_exchange_n(&ctrl_lock, 1, __ATOMIC_ACQUIRE)) __builtin_ia32_pause();
    n = n_ctrl; memcpy(copy, ctrl_handlers, n * sizeof copy[0]); ignore = ctrl_c_ignored;
    __atomic_store_n(&ctrl_lock, 0, __ATOMIC_RELEASE);
    if (ignore && type == CTRL_C_EVENT) return 0;
    for (i = n; i > 0; i--) if (copy[i - 1](type)) return 0;
    ExitProcess(0xC000013Au);                                      /* STATUS_CONTROL_C_EXIT */
    return 0;
}
