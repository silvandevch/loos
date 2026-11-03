/* Console self-test of the Win64 runtime: LoadLibrary/GetProcAddress/DllMain, file I/O, FindFirstFile. */
#include <windows.h>
static HANDLE out;
static void say(const char *s) { DWORD n; int l = 0; while (s[l]) l++; WriteFile(out, s, l, &n, 0); }
static void num(int v) { char b[16]; int i = 15; b[i] = 0; if (!v) b[--i] = '0'; int neg = v < 0; if (neg) v = -v; while (v) { b[--i] = '0' + v % 10; v /= 10; } if (neg) b[--i] = '-'; say(b + i); }
void start(void)
{
    out = GetStdHandle(STD_OUTPUT_HANDLE);
    int fail = 0;
    HMODULE m = LoadLibraryW(L"testdll.dll");
    say("selftest: LoadLibrary "); say(m ? "ok\n" : "FAIL\n"); if (!m) fail++;
    int (*add)(int, int) = m ? (int (*)(int, int))GetProcAddress(m, "add") : 0;
    int (*attached)(void) = m ? (int (*)(void))GetProcAddress(m, "attached") : 0;
    say("selftest: add(40,2)="); if (add) num(add(40, 2)); say("\n"); if (!add || add(40, 2) != 42) fail++;
    say("selftest: DllMain attach count="); if (attached) num(attached()); say("\n"); if (!attached || attached() != 1) fail++;
    say("selftest: GetProcAddress(kernel32,GetTickCount) "); say(GetProcAddress(GetModuleHandleW(L"kernel32.dll"), "GetTickCount") ? "ok\n" : "FAIL\n");
    HANDLE f = CreateFileW(L"C:\\Users\\t.txt", GENERIC_WRITE, 0, 0, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
    DWORD n; WriteFile(f, "hello file", 10, &n, 0); CloseHandle(f);
    f = CreateFileW(L"C:\\Users\\t.txt", GENERIC_READ, 0, 0, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0);
    char buf[32] = {0}; ReadFile(f, buf, 31, &n, 0); CloseHandle(f);
    say("selftest: file read back='"); say(buf); say("' size="); num(n); say("\n"); if (n != 10 || buf[0] != 'h') fail++;
    WIN32_FIND_DATAW fd; HANDLE h = FindFirstFileW(L"C:\\Users\\*", &fd);
    say("selftest: FindFirstFile "); say(h != INVALID_HANDLE_VALUE && fd.cFileName[0] == 't' ? "ok\n" : "FAIL\n");
    say(fail ? "selftest: FAILED\n" : "selftest: PASS\n");
    ExitProcess(fail);
}
