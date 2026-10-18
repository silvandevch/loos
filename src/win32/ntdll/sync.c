/* User-mode synchronisation: WaitOnAddress, critical sections, SRW locks, condition variables.
 *
 * The only blocking primitive is thread parking (NtWaitForAlertByThreadId / NtAlertThreadByThreadId);
 * everything else is a word in user memory plus a hashed queue of parked threads, as on Windows 10.
 * Every park site re-checks its condition in a loop, so spurious or stale alerts are harmless.
 *
 * Fairness: none promised. SRW readers can starve a writer under a continuous stream of readers.
 * Timeouts: a spurious wakeup restarts the full timeout rather than the remainder. */
#include "../include/loos_teb.h"
#include <string.h>

#define STATUS_RESOURCE_NOT_OWNED ((NTSTATUS)0xC0000264)

static void spin_lock(volatile int *l) { while (__atomic_exchange_n(l, 1, __ATOMIC_ACQUIRE)) __builtin_ia32_pause(); }
static void spin_unlock(volatile int *l) { __atomic_store_n(l, 0, __ATOMIC_RELEASE); }

/* ---------- WaitOnAddress ---------- */
typedef struct waiter { struct waiter *next, *prev; volatile void *addr; HANDLE tid; volatile int signaled; } waiter;
typedef struct { volatile int lock; waiter *head, *tail; } bucket;
#define NBUCKETS 64u
static bucket buckets[NBUCKETS];

static bucket *bucket_of(volatile void *a) { return &buckets[(((uintptr_t)a >> 3) * 2654435761u >> 7) % NBUCKETS]; }

static void unlink_waiter(bucket *b, waiter *w)
{
    if (w->prev) w->prev->next = w->next; else b->head = w->next;
    if (w->next) w->next->prev = w->prev; else b->tail = w->prev;
    w->next = w->prev = NULL;
}

static int same(volatile void *addr, const void *cmp, SIZE_T size)
{
    switch (size) {
    case 1: return __atomic_load_n((volatile uint8_t *)addr, __ATOMIC_ACQUIRE) == *(const uint8_t *)cmp;
    case 2: return __atomic_load_n((volatile uint16_t *)addr, __ATOMIC_ACQUIRE) == *(const uint16_t *)cmp;
    case 4: return __atomic_load_n((volatile uint32_t *)addr, __ATOMIC_ACQUIRE) == *(const uint32_t *)cmp;
    default: return __atomic_load_n((volatile uint64_t *)addr, __ATOMIC_ACQUIRE) == *(const uint64_t *)cmp;
    }
}

NTSTATUS NTAPI RtlWaitOnAddress(volatile void *addr, PVOID compare, SIZE_T size, LARGE_INTEGER *timeout)
{
    bucket *b;
    waiter w;
    if (!addr || !compare || (size != 1 && size != 2 && size != 4 && size != 8)) return STATUS_INVALID_PARAMETER;
    b = bucket_of(addr);
    spin_lock(&b->lock);
    if (!same(addr, compare, size)) { spin_unlock(&b->lock); return STATUS_SUCCESS; }
    w.addr = addr; w.tid = (HANDLE)(uintptr_t)k32_tid(); w.signaled = 0; w.next = NULL; w.prev = b->tail;
    if (b->tail) b->tail->next = &w; else b->head = &w;
    b->tail = &w;
    spin_unlock(&b->lock);
    for (;;) {
        NTSTATUS st = NtWaitForAlertByThreadId((PVOID)addr, timeout);
        if (__atomic_load_n(&w.signaled, __ATOMIC_ACQUIRE)) return STATUS_SUCCESS;
        if (st == STATUS_TIMEOUT) {
            spin_lock(&b->lock);
            if (w.signaled) { spin_unlock(&b->lock); return STATUS_SUCCESS; }        /* woken just as the wait expired */
            unlink_waiter(b, &w);
            spin_unlock(&b->lock);
            return STATUS_TIMEOUT;
        }
    }
}

static void wake(PVOID addr, int all)
{
    bucket *b = bucket_of(addr);
    waiter *w, *next;
    HANDLE tids[64];
    unsigned n = 0, i;
    for (;;) {
        n = 0;
        spin_lock(&b->lock);
        for (w = b->head; w && n < 64; w = next) {
            next = w->next;
            if (w->addr != addr) continue;
            unlink_waiter(b, w);
            tids[n++] = w->tid;                                 /* read before publishing: w's frame may vanish */
            __atomic_store_n(&w->signaled, 1, __ATOMIC_RELEASE);
            if (!all) break;
        }
        spin_unlock(&b->lock);
        for (i = 0; i < n; i++) NtAlertThreadByThreadId(tids[i]);
        if (!all || n < 64) return;                             /* more than 64 waiters: go around again */
    }
}
void NTAPI RtlWakeAddressSingle(PVOID addr) { wake(addr, 0); }
void NTAPI RtlWakeAddressAll(PVOID addr) { wake(addr, 1); }

/* ---------- critical sections ----------
 * Lock state is a futex word kept in the LockSemaphore field (0 free, 1 held, 2 held with waiters). */
static volatile uint32_t *cs_word(RTL_CRITICAL_SECTION *cs) { return (volatile uint32_t *)&cs->LockSemaphore; }

NTSTATUS NTAPI RtlInitializeCriticalSectionEx(RTL_CRITICAL_SECTION *cs, ULONG spin, ULONG flags)
{
    (void)flags;
    if (!cs) return STATUS_INVALID_PARAMETER;
    cs->DebugInfo = NULL; cs->LockCount = -1; cs->RecursionCount = 0; cs->OwningThread = NULL;
    cs->LockSemaphore = NULL; cs->SpinCount = spin & 0x00FFFFFFu;
    return STATUS_SUCCESS;
}
NTSTATUS NTAPI RtlInitializeCriticalSection(RTL_CRITICAL_SECTION *cs) { return RtlInitializeCriticalSectionEx(cs, 0, 0); }
NTSTATUS NTAPI RtlDeleteCriticalSection(RTL_CRITICAL_SECTION *cs)
{
    if (!cs) return STATUS_INVALID_PARAMETER;
    cs->LockCount = -1; cs->RecursionCount = 0; cs->OwningThread = NULL; cs->LockSemaphore = NULL;
    return STATUS_SUCCESS;
}

/* Only the owner ever stores its own id here, so a racing reader can at worst see someone else's id; atomics keep it well-defined. */
static int owner_is(RTL_CRITICAL_SECTION *cs, HANDLE me) { return __atomic_load_n(&cs->OwningThread, __ATOMIC_RELAXED) == me; }
static void set_owner(RTL_CRITICAL_SECTION *cs, HANDLE t) { __atomic_store_n(&cs->OwningThread, t, __ATOMIC_RELAXED); }

static int cs_try(RTL_CRITICAL_SECTION *cs)
{
    uint32_t expect = 0;
    return __atomic_compare_exchange_n(cs_word(cs), &expect, 1, 0, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED);
}

static void futex_lock(volatile uint32_t *w)
{
    uint32_t c = 0;
    if (__atomic_compare_exchange_n(w, &c, 1, 0, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) return;
    if (c != 2) c = __atomic_exchange_n(w, 2, __ATOMIC_ACQUIRE);          /* mark contended; c = what was there */
    while (c != 0) {
        uint32_t two = 2;
        RtlWaitOnAddress(w, &two, 4, NULL);                              /* sleep while the word is still "contended" */
        c = __atomic_exchange_n(w, 2, __ATOMIC_ACQUIRE);                 /* take it if free, keeping the contended mark */
    }
}

NTSTATUS NTAPI RtlEnterCriticalSection(RTL_CRITICAL_SECTION *cs)
{
    HANDLE me = (HANDLE)(uintptr_t)k32_tid();
    ULONG_PTR i;
    if (!cs) return STATUS_INVALID_PARAMETER;
    if (owner_is(cs, me)) { cs->RecursionCount++; cs->LockCount++; return STATUS_SUCCESS; }
    for (i = 0; i < cs->SpinCount; i++) { if (cs_try(cs)) goto acquired; __builtin_ia32_pause(); }
    futex_lock(cs_word(cs));
acquired:
    set_owner(cs, me); cs->RecursionCount = 1; cs->LockCount = 0;
    return STATUS_SUCCESS;
}

BOOLEAN NTAPI RtlTryEnterCriticalSection(RTL_CRITICAL_SECTION *cs)
{
    HANDLE me = (HANDLE)(uintptr_t)k32_tid();
    if (!cs) return FALSE;
    if (owner_is(cs, me)) { cs->RecursionCount++; cs->LockCount++; return TRUE; }
    if (!cs_try(cs)) return FALSE;
    set_owner(cs, me); cs->RecursionCount = 1; cs->LockCount = 0;
    return TRUE;
}

NTSTATUS NTAPI RtlLeaveCriticalSection(RTL_CRITICAL_SECTION *cs)
{
    HANDLE me = (HANDLE)(uintptr_t)k32_tid();
    if (!cs) return STATUS_INVALID_PARAMETER;
    if (!owner_is(cs, me)) return STATUS_RESOURCE_NOT_OWNED;
    if (--cs->RecursionCount > 0) { cs->LockCount--; return STATUS_SUCCESS; }
    set_owner(cs, NULL); cs->LockCount = -1;
    if (__atomic_fetch_sub(cs_word(cs), 1, __ATOMIC_RELEASE) != 1) {      /* it was 2: waiters exist */
        __atomic_store_n(cs_word(cs), 0, __ATOMIC_RELEASE);
        RtlWakeAddressSingle((PVOID)cs_word(cs));
    }
    return STATUS_SUCCESS;
}

/* ---------- SRW locks ----------
 * word: bit0 exclusive held, bit1 waiters present, bits 2.. shared count. */
#define SRW_EXCL    1ull
#define SRW_WAITERS 2ull
#define SRW_SHARED  4ull
static volatile uint64_t *srw(RTL_SRWLOCK *l) { return (volatile uint64_t *)&l->Ptr; }

void NTAPI RtlInitializeSRWLock(RTL_SRWLOCK *l) { __atomic_store_n(srw(l), 0, __ATOMIC_RELEASE); }

static void srw_wait(RTL_SRWLOCK *l, uint64_t seen)
{
    uint64_t s = seen;
    if (!(s & SRW_WAITERS)) {
        if (!__atomic_compare_exchange_n(srw(l), &s, seen | SRW_WAITERS, 0, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE)) return;   /* changed: retry */
        seen |= SRW_WAITERS;
    }
    RtlWaitOnAddress(srw(l), &seen, 8, NULL);
}

void NTAPI RtlAcquireSRWLockExclusive(RTL_SRWLOCK *l)
{
    for (;;) {
        uint64_t s = __atomic_load_n(srw(l), __ATOMIC_ACQUIRE);
        if ((s & ~SRW_WAITERS) == 0) {
            if (__atomic_compare_exchange_n(srw(l), &s, s | SRW_EXCL, 0, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) return;
            continue;
        }
        srw_wait(l, s);
    }
}

void NTAPI RtlAcquireSRWLockShared(RTL_SRWLOCK *l)
{
    for (;;) {
        uint64_t s = __atomic_load_n(srw(l), __ATOMIC_ACQUIRE);
        if (!(s & SRW_EXCL)) {
            if (__atomic_compare_exchange_n(srw(l), &s, s + SRW_SHARED, 0, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) return;
            continue;
        }
        srw_wait(l, s);
    }
}

BOOLEAN NTAPI RtlTryAcquireSRWLockExclusive(RTL_SRWLOCK *l)
{
    uint64_t s = __atomic_load_n(srw(l), __ATOMIC_ACQUIRE);
    return (s & ~SRW_WAITERS) == 0 && __atomic_compare_exchange_n(srw(l), &s, s | SRW_EXCL, 0, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED);
}
BOOLEAN NTAPI RtlTryAcquireSRWLockShared(RTL_SRWLOCK *l)
{
    uint64_t s = __atomic_load_n(srw(l), __ATOMIC_ACQUIRE);
    return !(s & SRW_EXCL) && __atomic_compare_exchange_n(srw(l), &s, s + SRW_SHARED, 0, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED);
}

void NTAPI RtlReleaseSRWLockExclusive(RTL_SRWLOCK *l)
{
    uint64_t s = __atomic_load_n(srw(l), __ATOMIC_RELAXED), n;
    do { n = s & ~(SRW_EXCL | SRW_WAITERS); } while (!__atomic_compare_exchange_n(srw(l), &s, n, 0, __ATOMIC_RELEASE, __ATOMIC_RELAXED));
    if (s & SRW_WAITERS) RtlWakeAddressAll((PVOID)srw(l));
}
void NTAPI RtlReleaseSRWLockShared(RTL_SRWLOCK *l)
{
    uint64_t s = __atomic_load_n(srw(l), __ATOMIC_RELAXED), n;
    do { n = s - SRW_SHARED; if (n < SRW_SHARED) n &= ~SRW_WAITERS; } while (!__atomic_compare_exchange_n(srw(l), &s, n, 0, __ATOMIC_RELEASE, __ATOMIC_RELAXED));
    if ((s & SRW_WAITERS) && (n >> 2) == 0) RtlWakeAddressAll((PVOID)srw(l));
}

/* ---------- condition variables: a sequence counter ---------- */
static volatile uint64_t *cvw(RTL_CONDITION_VARIABLE *cv) { return (volatile uint64_t *)&cv->Ptr; }
void NTAPI RtlInitializeConditionVariable(RTL_CONDITION_VARIABLE *cv) { __atomic_store_n(cvw(cv), 0, __ATOMIC_RELEASE); }

NTSTATUS NTAPI RtlSleepConditionVariableCS(RTL_CONDITION_VARIABLE *cv, RTL_CRITICAL_SECTION *cs, LARGE_INTEGER *timeout)
{
    uint64_t seq = __atomic_load_n(cvw(cv), __ATOMIC_ACQUIRE);
    LONG saved_rec = cs->RecursionCount;
    NTSTATUS st;
    cs->RecursionCount = 1;                                     /* release fully, whatever the nesting */
    RtlLeaveCriticalSection(cs);
    st = RtlWaitOnAddress(cvw(cv), &seq, 8, timeout);
    RtlEnterCriticalSection(cs);
    cs->RecursionCount = saved_rec;
    cs->LockCount = saved_rec - 1;
    return st;
}

NTSTATUS NTAPI RtlSleepConditionVariableSRW(RTL_CONDITION_VARIABLE *cv, RTL_SRWLOCK *l, LARGE_INTEGER *timeout, ULONG flags)
{
    uint64_t seq = __atomic_load_n(cvw(cv), __ATOMIC_ACQUIRE);
    int shared = (flags & 1u) != 0;                             /* CONDITION_VARIABLE_LOCKMODE_SHARED */
    NTSTATUS st;
    if (shared) RtlReleaseSRWLockShared(l); else RtlReleaseSRWLockExclusive(l);
    st = RtlWaitOnAddress(cvw(cv), &seq, 8, timeout);
    if (shared) RtlAcquireSRWLockShared(l); else RtlAcquireSRWLockExclusive(l);
    return st;
}

void NTAPI RtlWakeConditionVariable(RTL_CONDITION_VARIABLE *cv) { __atomic_fetch_add(cvw(cv), 1, __ATOMIC_RELEASE); RtlWakeAddressSingle((PVOID)cvw(cv)); }
void NTAPI RtlWakeAllConditionVariable(RTL_CONDITION_VARIABLE *cv) { __atomic_fetch_add(cvw(cv), 1, __ATOMIC_RELEASE); RtlWakeAddressAll((PVOID)cvw(cv)); }
