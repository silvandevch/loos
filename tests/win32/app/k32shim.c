/* k32shim — a tiny PE32+ DLL whose exports carry the same names a Win32 app
 * imports from kernel32.dll. Each export is real Windows-ABI code forwarding to
 * a host function pointer filled in by the harness after the LoOS loader has
 * mapped this DLL, so an app's real kernel32.dll imports bind onto the LoOS
 * kernel32 implementation compiled for the host, with no code patching.
 *
 * freestanding: no CRT, no imports of its own. Entry point is DllMain. */
typedef void *(*fn_std)(unsigned);
typedef int   (*fn_wr)(void *, const void *, unsigned, unsigned *, void *);
typedef int   (*fn_rd)(void *, void *, unsigned, unsigned *, void *);
typedef void *(*fn_cf)(const void *, unsigned, unsigned, void *, unsigned, unsigned, void *);
typedef int   (*fn_ch)(void *);
typedef unsigned (*fn_sz)(void *, unsigned *);
typedef unsigned (*fn_gfa)(const void *);
typedef int   (*fn_cd)(const void *, void *);

__declspec(dllexport) fn_std __loos_hook_GetStdHandle;
__declspec(dllexport) fn_wr  __loos_hook_WriteFile;
__declspec(dllexport) fn_rd  __loos_hook_ReadFile;
__declspec(dllexport) fn_cf  __loos_hook_CreateFileW;
__declspec(dllexport) fn_ch  __loos_hook_CloseHandle;
__declspec(dllexport) fn_sz  __loos_hook_GetFileSize;
__declspec(dllexport) fn_gfa __loos_hook_GetFileAttributesW;
__declspec(dllexport) fn_cd  __loos_hook_CreateDirectoryW;

__declspec(dllexport) void *GetStdHandle(unsigned which) { return __loos_hook_GetStdHandle ? __loos_hook_GetStdHandle(which) : (void *)0; }
__declspec(dllexport) int WriteFile(void *h, const void *b, unsigned n, unsigned *w, void *o) { return __loos_hook_WriteFile ? __loos_hook_WriteFile(h, b, n, w, o) : 0; }
__declspec(dllexport) int ReadFile(void *h, void *b, unsigned n, unsigned *r, void *o) { return __loos_hook_ReadFile ? __loos_hook_ReadFile(h, b, n, r, o) : 0; }
__declspec(dllexport) void *CreateFileW(const void *p, unsigned a, unsigned s, void *sa, unsigned d, unsigned f, void *t) { return __loos_hook_CreateFileW ? __loos_hook_CreateFileW(p, a, s, sa, d, f, t) : (void *)0; }
__declspec(dllexport) int CloseHandle(void *h) { return __loos_hook_CloseHandle ? __loos_hook_CloseHandle(h) : 0; }
__declspec(dllexport) unsigned GetFileSize(void *h, unsigned *hi) { return __loos_hook_GetFileSize ? __loos_hook_GetFileSize(h, hi) : 0; }
__declspec(dllexport) unsigned GetFileAttributesW(const void *p) { return __loos_hook_GetFileAttributesW ? __loos_hook_GetFileAttributesW(p) : 0; }
__declspec(dllexport) int CreateDirectoryW(const void *p, void *sa) { return __loos_hook_CreateDirectoryW ? __loos_hook_CreateDirectoryW(p, sa) : 0; }

__declspec(dllexport) int DllMain(void *m, unsigned reason, void *reserved)
{
    (void)m; (void)reason; (void)reserved;
    return 1;
}
