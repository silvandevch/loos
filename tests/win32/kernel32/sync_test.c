#define LOOS_HOST_TEST 1
#include "fake.h"
#include "../../../src/win32/kernel32/k32_internal.h"
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)
#define CHECK_EQ(a, b) do { unsigned long long a_ = (unsigned long long)(a), b_ = (unsigned long long)(b); \
    if (a_ != b_) { printf("FAIL %s:%d: %s = %llu, want %llu\n", __FILE__, __LINE__, #a, a_, b_); fails++; } } while (0)

static DWORD next_tid = 100;
typedef struct { void (*fn)(void *); void *arg; DWORD tid; } start;
static void *tramp(void *p) { start *s = p; k32_host_tid = s->tid; s->fn(s->arg); return NULL; }
static void run_threads(int n, void (*fn)(void *), void *arg)
{
    pthread_t t[16]; start st[16]; int i;
    for (i = 0; i < n; i++) { st[i].fn = fn; st[i].arg = arg; st[i].tid = next_tid++; pthread_create(&t[i], NULL, tramp, &st[i]); }
    for (i = 0; i < n; i++) pthread_join(t[i], NULL);
}

/* ---- critical section: mutual exclusion + recursion ---- */
static CRITICAL_SECTION cs;
static long counter_unprotected; static int in_section;
static void cs_worker(void *arg)
{
    int i; (void)arg;
    for (i = 0; i < 20000; i++) {
        EnterCriticalSection(&cs);
        if (__atomic_fetch_add(&in_section, 1, __ATOMIC_SEQ_CST) != 0) { printf("FAIL: two threads inside the section\n"); __atomic_add_fetch(&fails, 1, __ATOMIC_RELAXED); }
        EnterCriticalSection(&cs);                                   /* recursive */
        counter_unprotected++;
        LeaveCriticalSection(&cs);
        __atomic_fetch_sub(&in_section, 1, __ATOMIC_SEQ_CST);
        LeaveCriticalSection(&cs);
    }
}
static int other_got;
static void try_other(void *a) { (void)a; other_got = TryEnterCriticalSection(&cs); }
static void test_cs(void)
{
    InitializeCriticalSectionAndSpinCount(&cs, 0);
    run_threads(8, cs_worker, NULL);
    CHECK_EQ(counter_unprotected, 8 * 20000);
    CHECK_EQ(cs.RecursionCount, 0); CHECK(cs.OwningThread == NULL); CHECK_EQ(cs.LockCount, -1);
    CHECK(TryEnterCriticalSection(&cs) && cs.RecursionCount == 1);
    CHECK(TryEnterCriticalSection(&cs) && cs.RecursionCount == 2);       /* recursive try succeeds */
    LeaveCriticalSection(&cs); LeaveCriticalSection(&cs);
    EnterCriticalSection(&cs);
    run_threads(1, try_other, NULL);                                    /* another thread cannot take it */
    CHECK_EQ(other_got, 0);
    CHECK(RtlLeaveCriticalSection(&cs) == STATUS_SUCCESS);
    CHECK((ULONG)RtlLeaveCriticalSection(&cs) == 0xC0000264u);           /* not owned */
    DeleteCriticalSection(&cs);
    InitializeCriticalSectionAndSpinCount(&cs, 2000);                    /* spinning variant works too */
    counter_unprotected = 0; run_threads(4, cs_worker, NULL); CHECK_EQ(counter_unprotected, 4 * 20000);
    CHECK_EQ(SetCriticalSectionSpinCount(&cs, 5), 2000);
    DeleteCriticalSection(&cs);
}

/* ---- SRW ---- */
static SRWLOCK srwl; static long shared_val; static int readers_in, writer_in, max_readers;
static void srw_reader(void *a)
{
    int i; (void)a;
    for (i = 0; i < 5000; i++) {
        AcquireSRWLockShared(&srwl);
        { int r = __atomic_add_fetch(&readers_in, 1, __ATOMIC_SEQ_CST), m = __atomic_load_n(&max_readers, __ATOMIC_RELAXED); while (r > m && !__atomic_compare_exchange_n(&max_readers, &m, r, 0, __ATOMIC_RELAXED, __ATOMIC_RELAXED)) { } }
        if (__atomic_load_n(&writer_in, __ATOMIC_SEQ_CST)) { printf("FAIL: reader saw a writer inside\n"); __atomic_add_fetch(&fails, 1, __ATOMIC_RELAXED); }
        (void)__atomic_load_n(&shared_val, __ATOMIC_RELAXED);
        __atomic_sub_fetch(&readers_in, 1, __ATOMIC_SEQ_CST);
        ReleaseSRWLockShared(&srwl);
    }
}
static void srw_writer(void *a)
{
    int i; (void)a;
    for (i = 0; i < 3000; i++) {
        AcquireSRWLockExclusive(&srwl);
        if (__atomic_load_n(&readers_in, __ATOMIC_SEQ_CST) || __atomic_exchange_n(&writer_in, 1, __ATOMIC_SEQ_CST)) { printf("FAIL: writer not alone\n"); __atomic_add_fetch(&fails, 1, __ATOMIC_RELAXED); }
        __atomic_fetch_add(&shared_val, 1, __ATOMIC_RELAXED);
        __atomic_store_n(&writer_in, 0, __ATOMIC_SEQ_CST);
        ReleaseSRWLockExclusive(&srwl);
    }
}
static void srw_mixed(void *a) { static int n; if (__atomic_fetch_add(&n, 1, __ATOMIC_SEQ_CST) % 2) srw_writer(a); else srw_reader(a); }
static void test_srw(void)
{
    InitializeSRWLock(&srwl);
    CHECK(TryAcquireSRWLockShared(&srwl) && TryAcquireSRWLockShared(&srwl) && !TryAcquireSRWLockExclusive(&srwl));
    ReleaseSRWLockShared(&srwl); ReleaseSRWLockShared(&srwl);
    CHECK(TryAcquireSRWLockExclusive(&srwl) && !TryAcquireSRWLockShared(&srwl) && !TryAcquireSRWLockExclusive(&srwl));
    ReleaseSRWLockExclusive(&srwl);
    CHECK(srwl.Ptr == NULL);
    run_threads(6, srw_mixed, NULL);
    CHECK_EQ(shared_val, 3 * 3000); CHECK(srwl.Ptr == NULL);
}

/* ---- condition variable: bounded queue ---- */
static CRITICAL_SECTION qcs; static CONDITION_VARIABLE not_empty, not_full;
static int queue[8], qn, produced, consumed_sum;
static void producer(void *a)
{
    int i; (void)a;
    for (i = 1; i <= 2000; i++) {
        EnterCriticalSection(&qcs);
        while (qn == 8) SleepConditionVariableCS(&not_full, &qcs, INFINITE);
        queue[qn++] = i; produced++;
        WakeConditionVariable(&not_empty);
        LeaveCriticalSection(&qcs);
    }
}
static void consumer(void *a)
{
    int got = 0; (void)a;
    while (got < 2000) {
        EnterCriticalSection(&qcs);
        while (qn == 0) SleepConditionVariableCS(&not_empty, &qcs, INFINITE);
        consumed_sum += queue[--qn]; got++;
        WakeConditionVariable(&not_full);
        LeaveCriticalSection(&qcs);
    }
}
static void pc(void *a) { static int n; if (__atomic_fetch_add(&n, 1, __ATOMIC_SEQ_CST) == 0) producer(a); else consumer(a); }
static void test_condvar(void)
{
    InitializeCriticalSection(&qcs); InitializeConditionVariable(&not_empty); InitializeConditionVariable(&not_full);
    run_threads(2, pc, NULL);
    CHECK_EQ(produced, 2000); CHECK_EQ(consumed_sum, 2000 * 2001 / 2); CHECK_EQ(qn, 0);
    { /* timeout path */
      EnterCriticalSection(&qcs);
      CHECK(!SleepConditionVariableCS(&not_empty, &qcs, 20)); CHECK_EQ(GetLastError(), ERROR_TIMEOUT);
      CHECK_EQ(qcs.RecursionCount, 1);                                  /* ownership restored after the wait */
      LeaveCriticalSection(&qcs); }
    { SRWLOCK l; InitializeSRWLock(&l); AcquireSRWLockExclusive(&l);
      CHECK(!SleepConditionVariableSRW(&not_empty, &l, 20, 0));
      ReleaseSRWLockExclusive(&l);
      AcquireSRWLockShared(&l); CHECK(!SleepConditionVariableSRW(&not_empty, &l, 10, 1)); ReleaseSRWLockShared(&l); }
}

/* ---- WaitOnAddress ---- */
static volatile int flag32; static volatile LONG woke;
static void waiter_fn(void *a) { int zero = 0; (void)a; while (__atomic_load_n(&flag32, __ATOMIC_ACQUIRE) == 0) WaitOnAddress(&flag32, &zero, 4, INFINITE); __atomic_add_fetch(&woke, 1, __ATOMIC_SEQ_CST); }
static void waker_fn(void *a)
{
    struct timespec ts = { 0, 30 * 1000 * 1000 }; (void)a;
    nanosleep(&ts, NULL);
    CHECK_EQ(__atomic_load_n(&woke, __ATOMIC_SEQ_CST), 0);              /* nobody got through before the flag changed */
    __atomic_store_n(&flag32, 1, __ATOMIC_SEQ_CST);
    WakeByAddressAll((PVOID)&flag32);
}
static void waits_and_wakes(void *a) { static int n; if (__atomic_fetch_add(&n, 1, __ATOMIC_SEQ_CST) < 4) waiter_fn(a); else waker_fn(a); }
static void test_wait_on_address(void)
{
    int v = 5; short sv = 1; long long big = 7;
    CHECK(WaitOnAddress(&v, &(int){6}, 4, 0));                           /* value differs: returns at once */
    CHECK(!WaitOnAddress(&v, &(int){5}, 4, 30)); CHECK_EQ(GetLastError(), ERROR_TIMEOUT);
    CHECK(!WaitOnAddress(&sv, &(short){1}, 2, 10) && !WaitOnAddress(&big, &(long long){7}, 8, 10));
    CHECK(!WaitOnAddress(&v, &(int){5}, 3, 10)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);       /* bad size */
    run_threads(5, waits_and_wakes, NULL);
    CHECK_EQ(woke, 4);
    WakeByAddressSingle(&v);                                             /* nothing waiting: harmless */
}

/* ---- InitOnce ---- */
static INIT_ONCE once; static int init_runs;
static BOOL WINAPI do_init(INIT_ONCE *o, PVOID p, PVOID *ctx)
{
    struct timespec ts = { 0, 5 * 1000 * 1000 }; (void)o; (void)p;
    nanosleep(&ts, NULL);
    __atomic_add_fetch(&init_runs, 1, __ATOMIC_SEQ_CST);
    *ctx = (PVOID)0x1230;
    return TRUE;
}
static void once_worker(void *a)
{
    PVOID ctx = NULL; (void)a;
    CHECK(InitOnceExecuteOnce(&once, do_init, NULL, &ctx)); CHECK(ctx == (PVOID)0x1230);
}
static BOOL WINAPI failing_init(INIT_ONCE *o, PVOID p, PVOID *ctx) { (void)o; (void)p; (void)ctx; return FALSE; }
static void test_init_once(void)
{
    INIT_ONCE o2; BOOL pending; PVOID ctx;
    InitOnceInitialize(&once);
    run_threads(8, once_worker, NULL);
    CHECK_EQ(init_runs, 1);
    InitOnceInitialize(&o2);
    CHECK(!InitOnceExecuteOnce(&o2, failing_init, NULL, &ctx));          /* failure leaves it re-runnable */
    CHECK(InitOnceBeginInitialize(&o2, 0, &pending, &ctx) && pending);
    CHECK(!InitOnceComplete(&o2, 0, (PVOID)0x3));                        /* context low bits must be clear */
    CHECK(InitOnceComplete(&o2, 0, (PVOID)0x4000));
    CHECK(InitOnceBeginInitialize(&o2, 0, &pending, &ctx) && !pending && ctx == (PVOID)0x4000);
    { INIT_ONCE o3; InitOnceInitialize(&o3); CHECK(!InitOnceBeginInitialize(&o3, 1 /* CHECK_ONLY */, &pending, &ctx) && pending); }
}

/* ---- TLS / FLS ---- */
static DWORD tls_idx; static int fls_runs; static PVOID fls_last;
static void WINAPI fls_cb(PVOID v) { fls_runs++; fls_last = v; }
static void tls_worker(void *a)
{
    int i; (void)a;
    CHECK(TlsGetValue(tls_idx) == NULL);                                 /* a new thread starts with zero */
    for (i = 0; i < 1000; i++) { TlsSetValue(tls_idx, (PVOID)(uintptr_t)(k32_host_tid * 1000 + i)); CHECK(TlsGetValue(tls_idx) == (PVOID)(uintptr_t)(k32_host_tid * 1000 + i)); }
    k32_host_tls_expansion = NULL;
}
static void test_tls(void)
{
    DWORD a, b, i, all[1100];
    PVOID *exp_before;
    a = TlsAlloc(); b = TlsAlloc(); CHECK(a != TLS_OUT_OF_INDEXES && b != a);
    CHECK(TlsSetValue(a, (PVOID)0x11) && TlsSetValue(b, (PVOID)0x22) && TlsGetValue(a) == (PVOID)0x11 && TlsGetValue(b) == (PVOID)0x22);
    CHECK_EQ(GetLastError(), ERROR_SUCCESS);
    CHECK(TlsFree(a)); CHECK(!TlsFree(a)); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER);
    CHECK(TlsAlloc() == a); CHECK(TlsGetValue(a) == NULL);               /* reused index is cleared */
    CHECK(TlsGetValue(5000) == NULL); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER); CHECK(!TlsSetValue(5000, NULL)); CHECK(!TlsFree(5000));
    TlsFree(a); TlsFree(b);
    tls_idx = TlsAlloc();
    run_threads(4, tls_worker, NULL);
    TlsFree(tls_idx);
    /* exhaust all 1088 indexes: indexes above 63 live in the lazily allocated expansion array */
    for (i = 0; i < 1100; i++) { all[i] = TlsAlloc(); if (all[i] == TLS_OUT_OF_INDEXES) break; }
    CHECK_EQ(i, 1088); CHECK_EQ(GetLastError(), ERROR_NOT_ENOUGH_MEMORY);
    exp_before = k32_host_tls_expansion;
    CHECK(TlsSetValue(1000, (PVOID)0x77) && TlsGetValue(1000) == (PVOID)0x77);
    CHECK(k32_host_tls_expansion != NULL && exp_before == NULL);
    { DWORD n = i; for (i = 0; i < n; i++) TlsFree(all[i]); }
    k32_host_tls_expansion = NULL;                                       /* the array came from the process heap; fake_reset reclaims it */

    /* FLS */
    { DWORD f = FlsAlloc(fls_cb), g = FlsAlloc(NULL);
      CHECK(f != FLS_OUT_OF_INDEXES && g != f);
      CHECK(FlsGetValue(f) == NULL && FlsSetValue(f, (PVOID)0xAA) && FlsGetValue(f) == (PVOID)0xAA);
      CHECK(FlsSetValue(g, (PVOID)0xBB));
      CHECK(FlsFree(f)); CHECK_EQ(fls_runs, 1); CHECK(fls_last == (PVOID)0xAA);          /* destructor runs on free */
      CHECK(FlsGetValue(f) == NULL); CHECK_EQ(GetLastError(), ERROR_INVALID_PARAMETER); CHECK(!FlsFree(f));
      CHECK(!FlsSetValue(500, NULL));
      { DWORD h = FlsAlloc(fls_cb); FlsSetValue(h, (PVOID)0xCC);
        k32_fls_thread_exit();                                                          /* thread exit: destructors for what is left */
        CHECK_EQ(fls_runs, 2); CHECK(fls_last == (PVOID)0xCC);
        CHECK(FlsGetValue(h) == NULL);                                                  /* fresh array after exit */
        FlsFree(h); }
      FlsFree(g); }
    { DWORD idx[130], n; for (n = 0; n < 130; n++) { idx[n] = FlsAlloc(NULL); if (idx[n] == FLS_OUT_OF_INDEXES) break; } CHECK_EQ(n, 128); for (i = 0; i < n; i++) FlsFree(idx[i]); }
}

int main(void)
{
    fake_reset();
    test_cs(); test_srw(); test_condvar(); test_wait_on_address(); test_init_once(); test_tls();
    fake_reset();
    if (fails) printf("sync: %d FAILED\n", fails); else printf("sync: all passed\n");
    return fails != 0;
}
