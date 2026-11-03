/* user32.dll for LoOS: thin export stubs over the win32k services (crates/loos-win32k). Freestanding: no CRT. */
#include "loos_win32k.h"
#define API __attribute__((visibility("default"), ms_abi)) __declspec(dllexport)
typedef long long LL; typedef unsigned long long ULL; typedef unsigned int UINT; typedef int BOOL;
typedef void *HWND; typedef const unsigned short *LPCWSTR;
typedef struct { UINT cbSize, style; void *lpfnWndProc; int cbClsExtra, cbWndExtra; void *hInstance, *hIcon, *hCursor, *hbrBackground; LPCWSTR lpszMenuName, lpszClassName; void *hIconSm; } WNDCLASSEXW;
typedef struct { UINT style; void *lpfnWndProc; int cbClsExtra, cbWndExtra; void *hInstance, *hIcon, *hCursor, *hbrBackground; LPCWSTR lpszMenuName, lpszClassName; } WNDCLASSW;
typedef struct { HWND hwnd; UINT message; ULL wParam; LL lParam; UINT time; int x, y; UINT lPrivate; } MSG;

#define S(n, ...) loos_svc6(n, __VA_ARGS__)
static LL loos_svc6(LL n, LL a, LL b, LL c, LL d, LL e, LL f) { return loos_svc(n, a, b, c, d, e, f); }

API unsigned short RegisterClassExW(const WNDCLASSEXW *wc) { return (unsigned short)S(SVC_REGISTER_CLASS, (LL)wc, 0, 0, 0, 0, 0); }
API unsigned short RegisterClassW(const WNDCLASSW *w)
{
    WNDCLASSEXW x = { sizeof x, w->style, w->lpfnWndProc, w->cbClsExtra, w->cbWndExtra, w->hInstance, w->hIcon, w->hCursor, w->hbrBackground, w->lpszMenuName, w->lpszClassName, 0 };
    return RegisterClassExW(&x);
}
API HWND CreateWindowExW(UINT ex, LPCWSTR cls, LPCWSTR title, UINT style, int x, int y, int w, int h, HWND parent, void *menu, void *inst, void *param)
{
    ULL pack[11] = { ex, (ULL)cls, (ULL)title, style, (ULL)(LL)x, (ULL)(LL)y, (ULL)(LL)w, (ULL)(LL)h, (ULL)parent, (ULL)menu, (ULL)param };
    (void)inst;
    return (HWND)S(SVC_CREATE_WINDOW, (LL)pack, 0, 0, 0, 0, 0);
}
API BOOL ShowWindow(HWND h, int cmd) { return (BOOL)S(SVC_SHOW_WINDOW, (LL)h, cmd, 0, 0, 0, 0); }
API BOOL DestroyWindow(HWND h) { return (BOOL)S(SVC_DESTROY_WINDOW, (LL)h, 0, 0, 0, 0, 0); }
API BOOL UpdateWindow(HWND h) { return (BOOL)S(SVC_UPDATE_WINDOW, (LL)h, 0, 0, 0, 0, 0); }
API BOOL GetMessageW(MSG *m, HWND h, UINT lo, UINT hi)
{
    for (;;) {                                  /* the kernel parks the thread until its queue has work */
        LL r = S(SVC_GET_MESSAGE, (LL)m, (LL)h, lo, hi, 0, 0);
        if (r != LOOS_WOULD_BLOCK) return (BOOL)r;
        __asm__ volatile("pause");
    }
}
API BOOL PeekMessageW(MSG *m, HWND h, UINT lo, UINT hi, UINT rm) { return (BOOL)S(SVC_PEEK_MESSAGE, (LL)m, (LL)h, lo, hi, rm, 0); }
API BOOL TranslateMessage(const MSG *m) { return (BOOL)S(SVC_TRANSLATE_MESSAGE, (LL)m, 0, 0, 0, 0, 0); }
API LL DispatchMessageW(const MSG *m) { return S(SVC_DISPATCH_MESSAGE, (LL)m, 0, 0, 0, 0, 0); }
API void PostQuitMessage(int code) { S(SVC_POST_QUIT, code, 0, 0, 0, 0, 0); }
API LL DefWindowProcW(HWND h, UINT m, ULL w, LL l) { return S(SVC_DEF_WINDOW_PROC, (LL)h, m, (LL)w, l, 0, 0); }
API BOOL PostMessageW(HWND h, UINT m, ULL w, LL l) { return (BOOL)S(SVC_POST_MESSAGE, (LL)h, m, (LL)w, l, 0, 0); }
API LL SendMessageW(HWND h, UINT m, ULL w, LL l) { return S(SVC_SEND_MESSAGE, (LL)h, m, (LL)w, l, 0, 0); }
API void *BeginPaint(HWND h, void *ps) { return (void *)S(SVC_BEGIN_PAINT, (LL)h, (LL)ps, 0, 0, 0, 0); }
API BOOL EndPaint(HWND h, void *ps) { return (BOOL)S(SVC_END_PAINT, (LL)h, (LL)ps, 0, 0, 0, 0); }
API void *GetDC(HWND h) { return (void *)S(SVC_GET_DC, (LL)h, 0, 0, 0, 0, 0); }
API int ReleaseDC(HWND h, void *dc) { return (int)S(SVC_RELEASE_DC, (LL)h, (LL)dc, 0, 0, 0, 0); }
API int FillRect(void *dc, const void *rc, void *brush) { return (int)S(SVC_FILL_RECT, (LL)dc, (LL)rc, (LL)brush, 0, 0, 0); }
API BOOL InvalidateRect(HWND h, const void *rc, BOOL erase) { return (BOOL)S(SVC_INVALIDATE_RECT, (LL)h, (LL)rc, erase, 0, 0, 0); }
API BOOL GetClientRect(HWND h, void *rc) { return (BOOL)S(SVC_GET_CLIENT_RECT, (LL)h, (LL)rc, 0, 0, 0, 0); }
API BOOL GetWindowRect(HWND h, void *rc) { return (BOOL)S(SVC_GET_WINDOW_RECT, (LL)h, (LL)rc, 0, 0, 0, 0); }
API ULL SetTimer(HWND h, ULL id, UINT ms, void *proc) { (void)proc; return (ULL)S(SVC_SET_TIMER, (LL)h, (LL)id, ms, 0, 0, 0); }
API BOOL KillTimer(HWND h, ULL id) { return (BOOL)S(SVC_KILL_TIMER, (LL)h, (LL)id, 0, 0, 0, 0); }
API BOOL SetWindowTextW(HWND h, LPCWSTR t) { return (BOOL)S(SVC_SET_WINDOW_TEXT, (LL)h, (LL)t, 0, 0, 0, 0); }
API BOOL MoveWindow(HWND h, int x, int y, int w, int hh, BOOL repaint) { (void)repaint; return (BOOL)S(SVC_MOVE_WINDOW, (LL)h, x, y, w, hh, 0); }
API HWND SetFocus(HWND h) { return (HWND)S(SVC_SET_FOCUS, (LL)h, 0, 0, 0, 0, 0); }
API void *LoadCursorW(void *inst, LPCWSTR name) { (void)inst; (void)name; return (void *)1; }
API void *LoadIconW(void *inst, LPCWSTR name) { (void)inst; (void)name; return (void *)1; }
API int GetSystemMetrics(int i) { return i == 0 ? 1920 : i == 1 ? 1080 : 0; }
API int MessageBoxW(HWND owner, LPCWSTR text, LPCWSTR caption, UINT type)
{
    LL dlg = S(SVC_MESSAGE_BOX, (LL)owner, (LL)text, (LL)caption, type, 0, 0);
    if (!dlg) return 0;
    for (;;) {                                  /* modal loop: pump this thread's messages until the dialog answers */
        MSG m;
        while (PeekMessageW(&m, 0, 0, 0, 1)) { TranslateMessage(&m); DispatchMessageW(&m); }
        LL r = S(SVC_MESSAGE_BOX_RESULT, dlg, (LL)owner, 0, 0, 0, 0);
        if (r != LOOS_WOULD_BLOCK) return (int)r;
        __asm__ volatile("pause");
    }
}
int __attribute__((ms_abi)) DllMain(void *h, unsigned long reason, void *res) { (void)h; (void)reason; (void)res; return 1; }
