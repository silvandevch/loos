/* Test DLL: exports add() and attached(); DllMain counts attaches. Freestanding, kernel32 import only. */
#include <windows.h>
static int g_attach;
__declspec(dllexport) int add(int a, int b) { return a + b; }
__declspec(dllexport) int attached(void) { return g_attach; }
BOOL WINAPI DllMain(HINSTANCE h, DWORD why, LPVOID r) { if (why == DLL_PROCESS_ATTACH) g_attach++; return TRUE; }
