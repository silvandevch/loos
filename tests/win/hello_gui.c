/* Bring-up GUI program (freestanding, user32/gdi32 only): window + WM_PAINT text. Not a compatibility target. */
#include <windows.h>
static LRESULT CALLBACK proc(HWND h, UINT m, WPARAM w, LPARAM l)
{
    if (m == WM_PAINT) {
        PAINTSTRUCT ps; HDC dc = BeginPaint(h, &ps);
        SetBkMode(dc, 1);
        TextOutW(dc, 12, 12, L"Hello from a Win64 PE on LoOS", 29);
        TextOutW(dc, 12, 36, L"user32 + gdi32 via loos-win32k", 29);
        EndPaint(h, &ps); return 0;
    }
    if (m == WM_DESTROY) { PostQuitMessage(0); return 0; }
    return DefWindowProcW(h, m, w, l);
}
void start(void)
{
    WNDCLASSW c = {0}; c.lpfnWndProc = proc; c.lpszClassName = L"LoHello"; c.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
    RegisterClassW(&c);
    HWND w = CreateWindowExW(0, L"LoHello", L"LoOS Win64 test", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 120, 90, 420, 160, 0, 0, 0, 0);
    ShowWindow(w, 1); UpdateWindow(w);
    MSG m; while (GetMessageW(&m, 0, 0, 0) > 0) { TranslateMessage(&m); DispatchMessageW(&m); }
    ExitProcess(0);
}
