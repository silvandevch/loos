#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <math.h>
#include <wchar.h>
#include "../../branding/brand.h"

#define ID_DISPLAY 100
#define ID_BUTTON_BASE 200

static HWND display;
static HWND buttons[20];
static wchar_t expression[256];
static size_t expression_len;
static int solved;

static const wchar_t *button_labels[20] = {
    L"C", L"Back", L"(", L")",
    L"7", L"8", L"9", L"/",
    L"4", L"5", L"6", L"*",
    L"1", L"2", L"3", L"-",
    L"0", L".", L"=", L"+"
};

typedef struct { const wchar_t *p; int failed; } parser;

static void skip_space(parser *p)
{
    while (*p->p == L' ' || *p->p == L'\t') ++p->p;
}

static double parse_sum(parser *p);

static double parse_atom(parser *p)
{
    double value;
    wchar_t *end;
    skip_space(p);
    if (*p->p == L'+') { ++p->p; return parse_atom(p); }
    if (*p->p == L'-') { ++p->p; return -parse_atom(p); }
    if (*p->p == L'(') {
        ++p->p;
        value = parse_sum(p);
        skip_space(p);
        if (*p->p != L')') p->failed = 1;
        else ++p->p;
        return value;
    }
    if (!((*p->p >= L'0' && *p->p <= L'9') || *p->p == L'.')) {
        p->failed = 1;
        return 0.0;
    }
    value = wcstod(p->p, &end);
    if (end == p->p) { p->failed = 1; return 0.0; }
    p->p = end;
    return value;
}

static double parse_product(parser *p)
{
    double value = parse_atom(p);
    for (;;) {
        wchar_t op;
        double rhs;
        skip_space(p);
        op = *p->p;
        if (op != L'*' && op != L'/') return value;
        ++p->p;
        rhs = parse_atom(p);
        if (op == L'/' && rhs == 0.0) { p->failed = 1; return 0.0; }
        value = op == L'*' ? value * rhs : value / rhs;
        if (!isfinite(value)) p->failed = 1;
    }
}

static double parse_sum(parser *p)
{
    double value = parse_product(p);
    for (;;) {
        wchar_t op;
        double rhs;
        skip_space(p);
        op = *p->p;
        if (op != L'+' && op != L'-') return value;
        ++p->p;
        rhs = parse_product(p);
        value = op == L'+' ? value + rhs : value - rhs;
        if (!isfinite(value)) p->failed = 1;
    }
}

static int evaluate(double *result)
{
    parser p = { expression, 0 };
    double value;
    if (!expression_len) return 0;
    value = parse_sum(&p);
    skip_space(&p);
    if (p.failed || *p.p != 0 || !isfinite(value)) return 0;
    *result = value;
    return 1;
}

static void show_expression(void)
{
    SetWindowTextW(display, expression_len ? expression : L"0");
}

static void clear_expression(void)
{
    expression_len = 0;
    expression[0] = 0;
    solved = 0;
    show_expression();
}

static void append_text(const wchar_t *text)
{
    size_t n = wcslen(text);
    if (solved) {
        expression_len = 0;
        expression[0] = 0;
        solved = 0;
    }
    if (n > (sizeof expression / sizeof expression[0]) - expression_len - 1)
        return;
    memcpy(expression + expression_len, text, n * sizeof(wchar_t));
    expression_len += n;
    expression[expression_len] = 0;
    show_expression();
}

static void do_equals(void)
{
    double value;
    wchar_t output[80];
    if (!evaluate(&value)) {
        SetWindowTextW(display, L"Invalid expression");
        solved = 1;
        return;
    }
    if (swprintf(output, sizeof output / sizeof output[0], L"%.12g", value) < 0)
        wcscpy(output, L"Error");
    wcscpy(expression, output);
    expression_len = wcslen(expression);
    solved = 1;
    show_expression();
}

static void press_button(unsigned index)
{
    if (index >= 20) return;
    if (index == 0) { clear_expression(); return; }
    if (index == 1) {
        if (solved) { clear_expression(); return; }
        if (expression_len) expression[--expression_len] = 0;
        show_expression();
        return;
    }
    if (index == 18) { do_equals(); return; }
    append_text(button_labels[index]);
}

static void layout(HWND window)
{
    RECT client;
    int width, height, gap = 7, margin = 12, display_h = 48;
    int cell_w, cell_h;
    unsigned i;
    GetClientRect(window, &client);
    width = client.right - client.left;
    height = client.bottom - client.top;
    cell_w = (width - 2 * margin - 3 * gap) / 4;
    cell_h = (height - 2 * margin - display_h - 4 * gap) / 5;
    if (cell_w < 1 || cell_h < 1) return;
    MoveWindow(display, margin, margin, width - 2 * margin, display_h, TRUE);
    for (i = 0; i < 20; ++i) {
        int row = (int)(i / 4), col = (int)(i % 4);
        int x = margin + col * (cell_w + gap);
        int y = margin + display_h + gap + row * (cell_h + gap);
        MoveWindow(buttons[i], x, y, cell_w, cell_h, TRUE);
    }
}

static LRESULT CALLBACK window_proc(HWND window, UINT message, WPARAM wparam,
                                    LPARAM lparam)
{
    unsigned i;
    switch (message) {
    case WM_CREATE: {
        HFONT font = (HFONT)GetStockObject(DEFAULT_GUI_FONT);
        display = CreateWindowExW(WS_EX_CLIENTEDGE, L"EDIT", L"0",
            WS_CHILD | WS_VISIBLE | ES_RIGHT | ES_READONLY | ES_AUTOHSCROLL,
            0, 0, 0, 0, window, (HMENU)(INT_PTR)ID_DISPLAY,
            GetModuleHandleW(NULL), NULL);
        if (!display) return -1;
        SendMessageW(display, WM_SETFONT, (WPARAM)font, TRUE);
        for (i = 0; i < 20; ++i) {
            buttons[i] = CreateWindowExW(0, L"BUTTON", button_labels[i],
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
                0, 0, 0, 0, window, (HMENU)(INT_PTR)(ID_BUTTON_BASE + i),
                GetModuleHandleW(NULL), NULL);
            if (!buttons[i]) return -1;
            SendMessageW(buttons[i], WM_SETFONT, (WPARAM)font, TRUE);
        }
        layout(window);
        return 0;
    }
    case WM_SIZE:
        layout(window);
        return 0;
    case WM_COMMAND:
        if (HIWORD(wparam) == BN_CLICKED &&
            LOWORD(wparam) >= ID_BUTTON_BASE &&
            LOWORD(wparam) < ID_BUTTON_BASE + 20) {
            press_button((unsigned)(LOWORD(wparam) - ID_BUTTON_BASE));
            return 0;
        }
        break;
    case WM_CHAR:
        if ((wparam >= L'0' && wparam <= L'9') || wparam == L'.' ||
            wparam == L'+' || wparam == L'-' || wparam == L'*' ||
            wparam == L'/' || wparam == L'(' || wparam == L')') {
            wchar_t text[2] = { (wchar_t)wparam, 0 };
            append_text(text);
            return 0;
        }
        if (wparam == L'=' || wparam == L'\r') { do_equals(); return 0; }
        if (wparam == L'\b') { press_button(1); return 0; }
        if (wparam == 27) { clear_expression(); return 0; }
        break;
    case WM_KEYDOWN:
        if (wparam == VK_RETURN) { do_equals(); return 0; }
        if (wparam == VK_ESCAPE) { clear_expression(); return 0; }
        break;
    case WM_DESTROY:
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(window, message, wparam, lparam);
}

int WINAPI wWinMain(HINSTANCE instance, HINSTANCE previous, PWSTR command_line,
                    int show)
{
    WNDCLASSW wc = {0};
    HWND window;
    MSG message;
    (void)previous;
    (void)command_line;
    wc.lpfnWndProc = window_proc;
    wc.hInstance = instance;
    wc.hCursor = LoadCursorW(NULL, MAKEINTRESOURCEW(32512));
    wc.hbrBackground = CreateSolidBrush(RGB(LOOS_BRAND_SURFACE_R,
                                              LOOS_BRAND_SURFACE_G,
                                              LOOS_BRAND_SURFACE_B));
    wc.lpszClassName = L"LoOSCalculatorWindow";
    if (!RegisterClassW(&wc)) return 1;
    window = CreateWindowExW(0, wc.lpszClassName, L"LoOS Calculator",
        WS_OVERLAPPEDWINDOW, CW_USEDEFAULT, CW_USEDEFAULT, 360, 470,
        NULL, NULL, instance, NULL);
    if (!window) return 1;
    ShowWindow(window, show);
    UpdateWindow(window);
    while (GetMessageW(&message, NULL, 0, 0) > 0) {
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    return (int)message.wParam;
}
