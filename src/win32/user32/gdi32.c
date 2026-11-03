/* gdi32.dll for LoOS: export stubs over the win32k GDI services. */
#include "loos_win32k.h"
#define API __attribute__((visibility("default"), ms_abi)) __declspec(dllexport)
typedef long long LL; typedef unsigned int UINT; typedef int BOOL; typedef const unsigned short *LPCWSTR; typedef void *HDC; typedef void *HGDIOBJ;
#define S(n, ...) loos_svc(n, __VA_ARGS__)

API BOOL TextOutW(HDC dc, int x, int y, LPCWSTR s, int n) { return (BOOL)S(SVC_TEXT_OUT, (LL)dc, x, y, (LL)s, n, 0); }
API UINT SetTextColor(HDC dc, UINT c) { return (UINT)S(SVC_SET_TEXT_COLOR, (LL)dc, c, 0, 0, 0, 0); }
API UINT SetBkColor(HDC dc, UINT c) { return (UINT)S(SVC_SET_BK_COLOR, (LL)dc, c, 0, 0, 0, 0); }
API int SetBkMode(HDC dc, int m) { return (int)S(SVC_SET_BK_MODE, (LL)dc, m, 0, 0, 0, 0); }
API HGDIOBJ CreateSolidBrush(UINT c) { return (HGDIOBJ)S(SVC_CREATE_BRUSH, c, 0, 0, 0, 0, 0); }
API HGDIOBJ CreatePen(int style, int w, UINT c) { return (HGDIOBJ)S(SVC_CREATE_PEN, style, w, c, 0, 0, 0); }
API HGDIOBJ SelectObject(HDC dc, HGDIOBJ o) { return (HGDIOBJ)S(SVC_SELECT_OBJECT, (LL)dc, (LL)o, 0, 0, 0, 0); }
API BOOL DeleteObject(HGDIOBJ o) { return (BOOL)S(SVC_DELETE_OBJECT, (LL)o, 0, 0, 0, 0, 0); }
API BOOL MoveToEx(HDC dc, int x, int y, void *old) { (void)old; return (BOOL)S(SVC_MOVE_TO, (LL)dc, x, y, 0, 0, 0); }
API BOOL LineTo(HDC dc, int x, int y) { return (BOOL)S(SVC_LINE_TO, (LL)dc, x, y, 0, 0, 0); }
API BOOL Rectangle(HDC dc, int l, int t, int r, int b) { return (BOOL)S(SVC_RECTANGLE, (LL)dc, l, t, r, b, 0); }
API HGDIOBJ GetStockObject(int i) { return (HGDIOBJ)S(SVC_GET_STOCK_OBJECT, i, 0, 0, 0, 0, 0); }
int __attribute__((ms_abi)) DllMain(void *h, unsigned long reason, void *res) { (void)h; (void)reason; (void)res; return 1; }
