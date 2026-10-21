/* kernelbase side of synchronisation: critical sections, SRW locks, condition variables, InitOnce,
 * WaitOnAddress, Interlocked*. Thin wrappers over the ntdll primitives in ntdll/sync.c. */
#include "k32_internal.h"

static LARGE_INTEGER *timeout_ms(DWORD ms, LARGE_INTEGER *t)
{
    if (ms == INFINITE) return NULL;
    t->QuadPart = -(LONGLONG)ms * 10000;
    return t;
}

void WINAPI InitializeCriticalSection(RTL_CRITICAL_SECTION *cs) { RtlInitializeCriticalSection(cs); }
BOOL WINAPI InitializeCriticalSectionAndSpinCount(RTL_CRITICAL_SECTION *cs, DWORD spin) { return NT_SUCCESS(RtlInitializeCriticalSectionEx(cs, spin, 0)); }
BOOL WINAPI InitializeCriticalSectionEx(RTL_CRITICAL_SECTION *cs, DWORD spin, DWORD flags) { return NT_SUCCESS(RtlInitializeCriticalSectionEx(cs, spin, flags)); }
void WINAPI DeleteCriticalSection(RTL_CRITICAL_SECTION *cs) { RtlDeleteCriticalSection(cs); }
void WINAPI EnterCriticalSection(RTL_CRITICAL_SECTION *cs) { RtlEnterCriticalSection(cs); }
void WINAPI LeaveCriticalSection(RTL_CRITICAL_SECTION *cs) { RtlLeaveCriticalSection(cs); }
BOOL WINAPI TryEnterCriticalSection(RTL_CRITICAL_SECTION *cs) { return RtlTryEnterCriticalSection(cs); }
DWORD WINAPI SetCriticalSectionSpinCount(RTL_CRITICAL_SECTION *cs, DWORD spin) { DWORD old = (DWORD)cs->SpinCount; cs->SpinCount = spin & 0x00FFFFFFu; return old; }

void WINAPI InitializeSRWLock(RTL_SRWLOCK *l) { RtlInitializeSRWLock(l); }
void WINAPI AcquireSRWLockExclusive(RTL_SRWLOCK *l) { RtlAcquireSRWLockExclusive(l); }
void WINAPI AcquireSRWLockShared(RTL_SRWLOCK *l) { RtlAcquireSRWLockShared(l); }
void WINAPI ReleaseSRWLockExclusive(RTL_SRWLOCK *l) { RtlReleaseSRWLockExclusive(l); }
void WINAPI ReleaseSRWLockShared(RTL_SRWLOCK *l) { RtlReleaseSRWLockShared(l); }
BOOLEAN WINAPI TryAcquireSRWLockExclusive(RTL_SRWLOCK *l) { return RtlTryAcquireSRWLockExclusive(l); }
BOOLEAN WINAPI TryAcquireSRWLockShared(RTL_SRWLOCK *l) { return RtlTryAcquireSRWLockShared(l); }

void WINAPI InitializeConditionVariable(RTL_CONDITION_VARIABLE *cv) { RtlInitializeConditionVariable(cv); }
void WINAPI WakeConditionVariable(RTL_CONDITION_VARIABLE *cv) { RtlWakeConditionVariable(cv); }
void WINAPI WakeAllConditionVariable(RTL_CONDITION_VARIABLE *cv) { RtlWakeAllConditionVariable(cv); }

static BOOL cv_result(NTSTATUS st)
{
    if (st == STATUS_TIMEOUT) { *k32_last_error_ptr() = ERROR_TIMEOUT; return FALSE; }
    if (!NT_SUCCESS(st)) { k32_set_nt_error(st); return FALSE; }
    return TRUE;
}
BOOL WINAPI SleepConditionVariableCS(RTL_CONDITION_VARIABLE *cv, RTL_CRITICAL_SECTION *cs, DWORD ms)
{ LARGE_INTEGER t; return cv_result(RtlSleepConditionVariableCS(cv, cs, timeout_ms(ms, &t))); }
BOOL WINAPI SleepConditionVariableSRW(RTL_CONDITION_VARIABLE *cv, RTL_SRWLOCK *l, DWORD ms, ULONG flags)
{ LARGE_INTEGER t; return cv_result(RtlSleepConditionVariableSRW(cv, l, timeout_ms(ms, &t), flags)); }

BOOL WINAPI WaitOnAddress(volatile void *addr, PVOID compare, SIZE_T size, DWORD ms)
{ LARGE_INTEGER t; return cv_result(RtlWaitOnAddress(addr, compare, size, timeout_ms(ms, &t))); }
void WINAPI WakeByAddressSingle(PVOID addr) { RtlWakeAddressSingle(addr); }
void WINAPI WakeByAddressAll(PVOID addr) { RtlWakeAddressAll(addr); }

/* ---------- one-time initialisation ----------
 * word: low 2 bits = state (0 untouched, 1 running, 2 done); when done the rest is the caller's context. */
typedef BOOL (WINAPI *INIT_ONCE_FN)(RTL_RUN_ONCE *once, PVOID param, PVOID *ctx);
#define INIT_ONCE_ASYNC 2u
#define INIT_ONCE_CHECK_ONLY 1u
#define INIT_ONCE_INIT_FAILED 4u
static volatile uintptr_t *once_word(RTL_RUN_ONCE *o) { return (volatile uintptr_t *)&o->Ptr; }

void WINAPI InitOnceInitialize(RTL_RUN_ONCE *o) { __atomic_store_n(once_word(o), 0, __ATOMIC_RELEASE); }

BOOL WINAPI InitOnceBeginInitialize(RTL_RUN_ONCE *o, DWORD flags, BOOL *pending, PVOID *ctx)
{
    for (;;) {
        uintptr_t s = __atomic_load_n(once_word(o), __ATOMIC_ACQUIRE);
        if ((s & 3) == 2) { *pending = FALSE; if (ctx) *ctx = (PVOID)(s & ~(uintptr_t)3); return TRUE; }
        if (flags & INIT_ONCE_CHECK_ONLY) { *k32_last_error_ptr() = ERROR_GEN_FAILURE; *pending = TRUE; return FALSE; }
        if ((s & 3) == 0) {
            if (__atomic_compare_exchange_n(once_word(o), &s, 1, 0, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE)) { *pending = TRUE; return TRUE; }
            continue;
        }
        if (flags & INIT_ONCE_ASYNC) { *pending = TRUE; return TRUE; }          /* async: every caller may run the initialiser */
        RtlWaitOnAddress(once_word(o), &s, sizeof s, NULL);                      /* another thread is running it: wait */
    }
}

BOOL WINAPI InitOnceComplete(RTL_RUN_ONCE *o, DWORD flags, PVOID ctx)
{
    uintptr_t v;
    if ((uintptr_t)ctx & 3) { *k32_last_error_ptr() = ERROR_INVALID_PARAMETER; return FALSE; }
    v = (flags & INIT_ONCE_INIT_FAILED) ? 0 : ((uintptr_t)ctx | 2);
    __atomic_store_n(once_word(o), v, __ATOMIC_RELEASE);
    RtlWakeAddressAll((PVOID)once_word(o));
    return TRUE;
}

BOOL WINAPI InitOnceExecuteOnce(RTL_RUN_ONCE *o, INIT_ONCE_FN fn, PVOID param, PVOID *ctx)
{
    BOOL pending;
    PVOID c = NULL;
    if (!InitOnceBeginInitialize(o, 0, &pending, ctx)) return FALSE;
    if (!pending) return TRUE;
    if (!fn(o, param, &c)) { InitOnceComplete(o, INIT_ONCE_INIT_FAILED, NULL); *k32_last_error_ptr() = ERROR_GEN_FAILURE; return FALSE; }
    if (!InitOnceComplete(o, 0, c)) return FALSE;
    if (ctx) *ctx = c;
    return TRUE;
}

/* ---------- Interlocked (the compiler normally inlines these; exported for GetProcAddress users) ---------- */
LONG WINAPI InterlockedIncrement(volatile LONG *p) { return __atomic_add_fetch(p, 1, __ATOMIC_SEQ_CST); }
LONG WINAPI InterlockedDecrement(volatile LONG *p) { return __atomic_sub_fetch(p, 1, __ATOMIC_SEQ_CST); }
LONG WINAPI InterlockedExchange(volatile LONG *p, LONG v) { return __atomic_exchange_n(p, v, __ATOMIC_SEQ_CST); }
LONG WINAPI InterlockedExchangeAdd(volatile LONG *p, LONG v) { return __atomic_fetch_add(p, v, __ATOMIC_SEQ_CST); }
LONG WINAPI InterlockedCompareExchange(volatile LONG *p, LONG exch, LONG cmp)
{ __atomic_compare_exchange_n(p, &cmp, exch, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST); return cmp; }
LONGLONG WINAPI InterlockedCompareExchange64(volatile LONGLONG *p, LONGLONG exch, LONGLONG cmp)
{ __atomic_compare_exchange_n(p, &cmp, exch, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST); return cmp; }
