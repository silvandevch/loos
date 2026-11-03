/* Freestanding Windows service stand-in: prints "[cmdline] pid N session S up" through kernel32, then either exits
 * (autochk) or sleeps forever like a resident subsystem process. Built with mingw, no CRT. */
typedef unsigned long DWORD;
typedef void *HANDLE;
#define IMP __declspec(dllimport) __stdcall
IMP HANDLE GetStdHandle(DWORD);
IMP int WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
IMP char *GetCommandLineA(void);
IMP DWORD GetCurrentProcessId(void);
IMP int ProcessIdToSessionId(DWORD, DWORD *);
IMP void Sleep(DWORD);
IMP void ExitProcess(unsigned);

static int len(const char *s) { int n = 0; while (s[n]) n++; return n; }
static char *put(char *d, const char *s) { while (*s) *d++ = *s++; return d; }
static char *dec(char *d, unsigned v)
{
    char t[12]; int n = 0;
    do { t[n++] = (char)('0' + v % 10); v /= 10; } while (v);
    while (n) *d++ = t[--n];
    return d;
}
static int has(const char *s, const char *w)
{
    for (; *s; s++) { int i = 0; while (w[i] && s[i] == w[i]) i++; if (!w[i]) return 1; }
    return 0;
}

void __stdcall start(void)
{
    char buf[256], *p = buf;
    const char *cl = GetCommandLineA();
    DWORD sid = 99;
    ProcessIdToSessionId(GetCurrentProcessId(), &sid);
    p = put(p, "["); 
    for (int i = 0; cl[i] && cl[i] != ' ' && i < 32; i++) *p++ = cl[i];
    p = put(p, "] pid "); p = dec(p, GetCurrentProcessId());
    p = put(p, " session "); p = dec(p, sid);
    p = put(p, has(cl, "autochk") ? " checked, exiting\n" : " up\n");
    DWORD w = 0;
    WriteFile(GetStdHandle((DWORD)-11), buf, (DWORD)(p - buf), &w, 0);
    (void)len;
    if (has(cl, "autochk")) ExitProcess(0);
    Sleep(0xFFFFFFFFu);
    ExitProcess(1);
}
