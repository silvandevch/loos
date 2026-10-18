#include "k32_internal.h"

static HANDLE finish_create(NTSTATUS st, HANDLE h)
{
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return NULL; }
    *k32_last_error_ptr() = (st == STATUS_OBJECT_NAME_EXISTS) ? ERROR_ALREADY_EXISTS : ERROR_SUCCESS;
    return h;
}

HANDLE WINAPI CreateEventW(SECURITY_ATTRIBUTES *sa, BOOL manual_reset, BOOL initial, LPCWSTR name)
{
    WCHAR buf[512]; UNICODE_STRING us; OBJECT_ATTRIBUTES oa; HANDLE h = NULL; DWORD err; NTSTATUS st;
    if ((err = k32_object_attrs(sa, name, buf, 512, &us, &oa))) { *k32_last_error_ptr() = err; return NULL; }
    st = NtCreateEvent(&h, EVENT_ALL_ACCESS, &oa, manual_reset ? EventNotificationType : EventSynchronizationType, initial != 0);
    return finish_create(st, h);
}

BOOL WINAPI SetEvent(HANDLE h)
{
    NTSTATUS st = NtSetEvent(h, NULL);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

BOOL WINAPI ResetEvent(HANDLE h)
{
    NTSTATUS st = NtResetEvent(h, NULL);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

HANDLE WINAPI CreateMutexW(SECURITY_ATTRIBUTES *sa, BOOL initial_owner, LPCWSTR name)
{
    WCHAR buf[512]; UNICODE_STRING us; OBJECT_ATTRIBUTES oa; HANDLE h = NULL; DWORD err; NTSTATUS st;
    if ((err = k32_object_attrs(sa, name, buf, 512, &us, &oa))) { *k32_last_error_ptr() = err; return NULL; }
    st = NtCreateMutant(&h, MUTANT_ALL_ACCESS, &oa, initial_owner != 0);
    return finish_create(st, h);
}

BOOL WINAPI ReleaseMutex(HANDLE h)
{
    NTSTATUS st = NtReleaseMutant(h, NULL);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

HANDLE WINAPI CreateSemaphoreW(SECURITY_ATTRIBUTES *sa, LONG initial, LONG maximum, LPCWSTR name)
{
    WCHAR buf[512]; UNICODE_STRING us; OBJECT_ATTRIBUTES oa; HANDLE h = NULL; DWORD err; NTSTATUS st;
    if (maximum <= 0 || initial < 0 || initial > maximum) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
    if ((err = k32_object_attrs(sa, name, buf, 512, &us, &oa))) { *k32_last_error_ptr() = err; return NULL; }
    st = NtCreateSemaphore(&h, SEMAPHORE_ALL_ACCESS, &oa, initial, maximum);
    return finish_create(st, h);
}

BOOL WINAPI ReleaseSemaphore(HANDLE h, LONG count, LONG *prev)
{
    NTSTATUS st;
    if (count <= 0) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    st = NtReleaseSemaphore(h, count, prev);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

/* milliseconds -> NT relative timeout (negative, 100ns units); INFINITE -> NULL */
static LARGE_INTEGER *timeout_from_ms(DWORD ms, LARGE_INTEGER *t)
{
    if (ms == INFINITE) return NULL;
    t->QuadPart = -(LONGLONG)ms * 10000;
    return t;
}

static DWORD wait_result(NTSTATUS st)
{
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return WAIT_FAILED; }
    if (st == STATUS_TIMEOUT) return WAIT_TIMEOUT;
    if (st == STATUS_USER_APC) return WAIT_IO_COMPLETION;
    if ((ULONG)st >= STATUS_ABANDONED_WAIT_0 && (ULONG)st < STATUS_ABANDONED_WAIT_0 + MAXIMUM_WAIT_OBJECTS)
        return WAIT_ABANDONED_0 + ((ULONG)st - STATUS_ABANDONED_WAIT_0);
    return WAIT_OBJECT_0 + (ULONG)st;                    /* STATUS_WAIT_0 + index */
}

DWORD WINAPI WaitForSingleObjectEx(HANDLE h, DWORD ms, BOOL alertable)
{
    LARGE_INTEGER t;
    if (h == NULL || h == INVALID_HANDLE_VALUE) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return WAIT_FAILED; }
    return wait_result(NtWaitForSingleObject(h, alertable != 0, timeout_from_ms(ms, &t)));
}
DWORD WINAPI WaitForSingleObject(HANDLE h, DWORD ms) { return WaitForSingleObjectEx(h, ms, FALSE); }

DWORD WINAPI WaitForMultipleObjectsEx(DWORD n, const HANDLE *h, BOOL wait_all, DWORD ms, BOOL alertable)
{
    LARGE_INTEGER t;
    DWORD i;
    if (n == 0 || n > MAXIMUM_WAIT_OBJECTS || !h) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return WAIT_FAILED; }
    for (i = 0; i < n; i++)
        if (h[i] == NULL || h[i] == INVALID_HANDLE_VALUE) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return WAIT_FAILED; }
    return wait_result(NtWaitForMultipleObjects(n, (HANDLE *)h, wait_all ? WaitAllObjects : WaitAnyObject,
                                                alertable != 0, timeout_from_ms(ms, &t)));
}
DWORD WINAPI WaitForMultipleObjects(DWORD n, const HANDLE *h, BOOL all, DWORD ms) { return WaitForMultipleObjectsEx(n, h, all, ms, FALSE); }

DWORD WINAPI SleepEx(DWORD ms, BOOL alertable)
{
    LARGE_INTEGER t;
    NTSTATUS st;
    if (ms == INFINITE) t.QuadPart = (LONGLONG)0x8000000000000000ull;      /* longest possible relative delay */
    else t.QuadPart = -(LONGLONG)ms * 10000;
    st = NtDelayExecution(alertable != 0, &t);
    return st == STATUS_USER_APC ? WAIT_IO_COMPLETION : 0;
}
void WINAPI Sleep(DWORD ms) { SleepEx(ms, FALSE); }

BOOL WINAPI QueryPerformanceCounter(LARGE_INTEGER *c)
{
    NTSTATUS st = NtQueryPerformanceCounter(c, NULL);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}
BOOL WINAPI QueryPerformanceFrequency(LARGE_INTEGER *f)
{
    LARGE_INTEGER c;
    NTSTATUS st = NtQueryPerformanceCounter(&c, f);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

ULONGLONG WINAPI GetTickCount64(void)
{
    LARGE_INTEGER c, f;
    ULONGLONG cnt, fr;
    if (!NT_SUCCESS(NtQueryPerformanceCounter(&c, &f)) || f.QuadPart <= 0) return 0;
    cnt = (ULONGLONG)c.QuadPart; fr = (ULONGLONG)f.QuadPart;
    return (cnt / fr) * 1000 + (cnt % fr) * 1000 / fr;      /* split so cnt*1000 cannot overflow */
}
