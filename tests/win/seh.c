/* SEH self-test (clang, __try/__except/__finally): software and hardware exceptions, filters, unwinding. */
#include <windows.h>
static HANDLE out;
static void say(const char *s) { DWORD n; int l = 0; while (s[l]) l++; WriteFile(out, s, l, &n, 0); }
static volatile int finally_ran, depth_ran;
static int filter_code(DWORD c, DWORD *save) { *save = c; return EXCEPTION_EXECUTE_HANDLER; }
static void thrower(void) { RaiseException(0xE0000001, 0, 0, 0); }
static int inner(void) { __try { thrower(); } __finally { finally_ran++; } return 0; }
static int divide(volatile int z) { __try { return 10 / z; } __except (EXCEPTION_EXECUTE_HANDLER) { return -1; } }
static int deref(volatile int *p) { __try { return *p; } __except (EXCEPTION_EXECUTE_HANDLER) { return -2; } }
void start(void)
{
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    int fail = 0; DWORD code = 0;
    __try { inner(); say("seh: NOT CAUGHT\n"); fail++; } __except (filter_code(GetExceptionCode(), &code)) { say("seh: caught software exception, finally ran="); say(finally_ran == 1 ? "1\n" : "WRONG\n"); if (finally_ran != 1 || code != 0xE0000001) fail++; }
    int d = divide(0); say("seh: divide by zero -> "); say(d == -1 ? "handled\n" : "FAIL\n"); if (d != -1) fail++;
    int r = deref((volatile int *)0); say("seh: null deref -> "); say(r == -2 ? "handled\n" : "FAIL\n"); if (r != -2) fail++;
    int n = 0; __try { __try { RaiseException(0xE0000002, 0, 0, 0); } __except (EXCEPTION_CONTINUE_SEARCH) { n = 99; } } __except (EXCEPTION_EXECUTE_HANDLER) { n++; }
    say("seh: nested continue-search -> "); say(n == 1 ? "outer handled\n" : "FAIL\n"); if (n != 1) fail++;
    say(fail ? "seh: FAILED\n" : "seh: PASS\n");
    ExitProcess(fail);
}
