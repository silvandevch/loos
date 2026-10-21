/* Threads and processes: creation, queries, termination. */
#include "k32_internal.h"
#include <string.h>

static HANDLE create_thread(HANDLE process, SECURITY_ATTRIBUTES *sa, SIZE_T stack, LPTHREAD_START_ROUTINE start,
                            LPVOID param, DWORD flags, LPDWORD tid_out)
{
    OBJECT_ATTRIBUTES oa;
    PS_ATTRIBUTE_LIST attrs;
    CLIENT_ID cid;
    HANDLE h = NULL;
    NTSTATUS st;
    SIZE_T commit = 0, reserve = 0;

    if (!start) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return NULL; }
    if (stack) { if (flags & STACK_SIZE_PARAM_IS_A_RESERVATION) reserve = stack; else commit = stack; }
    InitializeObjectAttributes(&oa, NULL, (sa && sa->bInheritHandle) ? OBJ_INHERIT : 0, NULL, sa ? sa->lpSecurityDescriptor : NULL);
    memset(&attrs, 0, sizeof attrs); memset(&cid, 0, sizeof cid);
    attrs.TotalLength = sizeof(SIZE_T) + sizeof(PS_ATTRIBUTE);
    attrs.Attributes[0].Attribute = PS_ATTRIBUTE_CLIENT_ID; attrs.Attributes[0].Size = sizeof cid; attrs.Attributes[0].ValuePtr = &cid;
    st = NtCreateThreadEx(&h, THREAD_ALL_ACCESS, &oa, process, (PVOID)start, param, (flags & CREATE_SUSPENDED) ? THREAD_CREATE_SUSPENDED : 0,
                          0, commit, reserve, &attrs);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return NULL; }
    if (tid_out) *tid_out = (DWORD)(uintptr_t)cid.UniqueThread;
    return h;
}

HANDLE WINAPI CreateThread(SECURITY_ATTRIBUTES *sa, SIZE_T stack, LPTHREAD_START_ROUTINE start, LPVOID param, DWORD flags, LPDWORD tid)
{ return create_thread(NtCurrentProcess(), sa, stack, start, param, flags, tid); }
HANDLE WINAPI CreateRemoteThread(HANDLE process, SECURITY_ATTRIBUTES *sa, SIZE_T stack, LPTHREAD_START_ROUTINE start, LPVOID param, DWORD flags, LPDWORD tid)
{ return create_thread(process, sa, stack, start, param, flags, tid); }

void WINAPI ExitThread(DWORD code) { RtlExitUserThread((NTSTATUS)code); }

static BOOL thread_info(HANDLE h, THREAD_BASIC_INFORMATION *tbi)
{
    NTSTATUS st = NtQueryInformationThread(h, ThreadBasicInformation, tbi, sizeof *tbi, NULL);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}

BOOL WINAPI GetExitCodeThread(HANDLE h, LPDWORD code)
{
    THREAD_BASIC_INFORMATION t;
    if (!code) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    if (!thread_info(h, &t)) return FALSE;
    *code = t.ExitStatus == STATUS_PENDING ? STILL_ACTIVE : (DWORD)t.ExitStatus;
    return TRUE;
}

DWORD WINAPI SuspendThread(HANDLE h)
{
    ULONG prev = 0;
    NTSTATUS st = NtSuspendThread(h, &prev);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return (DWORD)-1; }
    return prev;
}
DWORD WINAPI ResumeThread(HANDLE h)
{
    ULONG prev = 0;
    NTSTATUS st = NtResumeThread(h, &prev);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return (DWORD)-1; }
    return prev;
}
BOOL WINAPI TerminateThread(HANDLE h, DWORD code)
{
    NTSTATUS st;
    if (!h) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    st = NtTerminateThread(h, (NTSTATUS)code);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}
HANDLE WINAPI OpenThread(DWORD access, BOOL inherit, DWORD tid)
{
    OBJECT_ATTRIBUTES oa;
    CLIENT_ID id;
    HANDLE h = NULL;
    NTSTATUS st;
    InitializeObjectAttributes(&oa, NULL, inherit ? OBJ_INHERIT : 0, NULL, NULL);
    id.UniqueProcess = NULL; id.UniqueThread = (HANDLE)(uintptr_t)tid;
    st = NtOpenThread(&h, access, &oa, &id);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st == STATUS_INVALID_PARAMETER ? STATUS_INVALID_PARAMETER : st); return NULL; }
    return h;
}
DWORD WINAPI GetThreadId(HANDLE h)
{
    THREAD_BASIC_INFORMATION t;
    return thread_info(h, &t) ? (DWORD)(uintptr_t)t.ClientId.UniqueThread : 0;
}
BOOL WINAPI SetThreadPriority(HANDLE h, int prio)
{
    LONG p = prio;
    NTSTATUS st;
    if (prio < THREAD_PRIORITY_IDLE || prio > THREAD_PRIORITY_TIME_CRITICAL) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    st = NtSetInformationThread(h, ThreadBasePriority, &p, sizeof p);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}
int WINAPI GetThreadPriority(HANDLE h)
{
    THREAD_BASIC_INFORMATION t;
    if (!thread_info(h, &t)) return THREAD_PRIORITY_ERROR_RETURN;
    return (int)t.BasePriority;
}
BOOL WINAPI SwitchToThread(void) { return NtYieldExecution() != (NTSTATUS)0x40000024; }       /* STATUS_NO_YIELD_PERFORMED */

/* ---------- process queries ---------- */
BOOL WINAPI GetExitCodeProcess(HANDLE h, LPDWORD code)
{
    PROCESS_BASIC_INFORMATION pbi;
    NTSTATUS st;
    if (!code) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    st = NtQueryInformationProcess(h, ProcessBasicInformation, &pbi, sizeof pbi, NULL);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    *code = pbi.ExitStatus == STATUS_PENDING ? STILL_ACTIVE : (DWORD)pbi.ExitStatus;
    return TRUE;
}
BOOL WINAPI TerminateProcess(HANDLE h, UINT code)
{
    NTSTATUS st;
    if (!h) { *k32_last_error_ptr() = ERROR_INVALID_HANDLE; return FALSE; }
    st = NtTerminateProcess(h, (NTSTATUS)code);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}
HANDLE WINAPI OpenProcess(DWORD access, BOOL inherit, DWORD pid)
{
    OBJECT_ATTRIBUTES oa;
    CLIENT_ID id;
    HANDLE h = NULL;
    NTSTATUS st;
    InitializeObjectAttributes(&oa, NULL, inherit ? OBJ_INHERIT : 0, NULL, NULL);
    id.UniqueProcess = (HANDLE)(uintptr_t)pid; id.UniqueThread = NULL;
    st = NtOpenProcess(&h, access, &oa, &id);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return NULL; }
    return h;
}
DWORD WINAPI GetProcessId(HANDLE h)
{
    PROCESS_BASIC_INFORMATION pbi;
    NTSTATUS st = NtQueryInformationProcess(h, ProcessBasicInformation, &pbi, sizeof pbi, NULL);
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return 0; }
    return (DWORD)pbi.UniqueProcessId;
}

/* kernelbase entry point: per-thread cleanup when a thread ends (loader calls it with THREAD_DETACH) */
LOOS_MSABI BOOL KernelBaseDllMain(HANDLE module, DWORD reason, LPVOID reserved)
{
    (void)module; (void)reserved;
    if (reason == 3 /* DLL_THREAD_DETACH */ || reason == 0 /* DLL_PROCESS_DETACH */) k32_fls_thread_exit();
    return TRUE;
}
