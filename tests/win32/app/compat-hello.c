/* compat-hello — a genuine, minimal Win32 console application.
 *
 * It calls real Win32 APIs the way an ordinary Windows program does:
 *   GetStdHandle(STD_OUTPUT_HANDLE) then WriteFile(...).
 * Built with MinGW as a normal PE32+ that imports these from kernel32.dll.
 * Nothing here knows anything about LoOS; it is just a Windows app. */
__declspec(dllimport) void *GetStdHandle(unsigned which);
__declspec(dllimport) int   WriteFile(void *h, const void *buf, unsigned len, unsigned *written, void *ov);

static unsigned slen(const char *s) { unsigned n = 0; while (s[n]) n++; return n; }
static void puts_(void *h, const char *s) { unsigned w = 0; WriteFile(h, s, slen(s), &w, 0); }

int app_main(void)
{
    void *out = GetStdHandle((unsigned)-11);          /* STD_OUTPUT_HANDLE */

    puts_(out, "compat-hello: a real Win32 app, running through LoOS's own kernel32.\r\n");
    puts_(out, "compat-hello: GetStdHandle -> WriteFile -> NtWriteFile, all LoOS code.\r\n");
    return 7;                                          /* distinct exit code */
}
