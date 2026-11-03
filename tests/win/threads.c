/* Console self-test: CreateThread, critical sections, events, mutex, TLS, Sleep, WaitForMultipleObjects. */
#include <windows.h>
static HANDLE out; static CRITICAL_SECTION cs; static volatile int counter; static HANDLE ev, mtx; static DWORD tls; static volatile int tlsok;
static void say(const char *s) { DWORD n; int l = 0; while (s[l]) l++; WriteFile(out, s, l, &n, 0); }
static void num(int v) { char b[16]; int i = 15; b[i] = 0; if (!v) b[--i] = '0'; while (v) { b[--i] = '0' + v % 10; v /= 10; } say(b + i); }
static DWORD WINAPI worker(LPVOID p)
{
    TlsSetValue(tls, p);
    for (int i = 0; i < 500; i++) {
        EnterCriticalSection(&cs);
        int c = counter; if (i % 100 == 0) Sleep(1); counter = c + 1;   /* Sleep inside the lock: others must wait */
        LeaveCriticalSection(&cs);
    }
    if (TlsGetValue(tls) == p) tlsok++;
    WaitForSingleObject(mtx, INFINITE); ReleaseMutex(mtx);
    if ((int)(long long)p == 1) SetEvent(ev);
    return 7 + (DWORD)(long long)p;
}
void start(void)
{
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    InitializeCriticalSection(&cs); tls = TlsAlloc(); ev = CreateEventW(0, 1, 0, 0); mtx = CreateMutexW(0, 0, 0);
    HANDLE t[3]; DWORD id[3];
    for (int i = 0; i < 3; i++) t[i] = CreateThread(0, 0, worker, (LPVOID)(long long)(i + 1), 0, &id[i]);
    DWORD w = WaitForMultipleObjects(3, t, TRUE, 10000);
    int fail = 0;
    say("threads: wait-all="); num(w); say(" counter="); num(counter); say(" tls-ok="); num(tlsok); say("\n");
    if (w != 0 || counter != 1500 || tlsok != 3) fail++;
    DWORD code = 0; GetExitCodeThread(t[1], &code);
    say("threads: exit code of #2="); num(code); say(" distinct-ids="); num(id[0] != id[1] && id[1] != id[2] && id[0] != GetCurrentThreadId()); say("\n");
    if (code != 9) fail++;
    say("threads: event "); say(WaitForSingleObject(ev, 0) == 0 ? "signalled\n" : "FAIL\n");
    say("threads: timeout "); say(WaitForSingleObject(CreateEventW(0, 0, 0, 0), 20) == WAIT_TIMEOUT ? "ok\n" : "FAIL\n");
    say(fail ? "threads: FAILED\n" : "threads: PASS\n");
    ExitProcess(fail);
}
