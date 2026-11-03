/* Freestanding Linux GUI proof app: exercises the public bridge event loop. */
#include "liblooswin.h"

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;

static i64 sc(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f)
{
    i64 r;
    register i64 r10 __asm__("r10") = d;
    register i64 r8 __asm__("r8") = e;
    register i64 r9 __asm__("r9") = f;
    __asm__ volatile("syscall" : "=a"(r) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return r;
}

static u64 len(const char *s) { u64 n = 0; while (s[n]) n++; return n; }
static void out(const char *s) { sc(1, 1, (i64)s, (i64)len(s), 0, 0, 0); }

__attribute__((naked)) void _start(void)
{
    __asm__ volatile("mov %rsp,%rdi; and $-16,%rsp; call main; mov %eax,%edi; mov $231,%eax; syscall");
}

int main(void)
{
    const char *title = "LoOS Linux GUI";
    const char *label = "GUI proof";
    if (looswin_version() != LOOSWIN_VERSION) return 10;
    i64 hwnd = looswin_create_window(title, 80, 70, 360, 220, 0xcf0000);
    if (hwnd <= 0) return 11;
    if (looswin_show((u32)hwnd, 5) != 0) return 12;
    struct looswin_event event;
    int painted = 0;
    for (int tries = 0; tries != 32; ++tries) {
        long r = looswin_next_event((u32)hwnd, &event);
        if (r < 0) return 13;
        if (r > 0 && event.message == LOOSWIN_WM_PAINT) {
            painted = 1;
            break;
        }
        if (r == 0) looswin_invalidate((u32)hwnd);
    }
    if (!painted) return 14;
    /* COLORREF 0x0000c800 is RGB(0,200,0); Win32k composes it as 0xff00c800. */
    if (looswin_fill_rect((u32)hwnd, 10, 10, 220, 80, 0x0000c800) != 0) return 15;
    if (looswin_text((u32)hwnd, 20, 110, label, 0) != 0) return 16;
    if (looswin_line((u32)hwnd, 0, 0, 50, 50, 0x000000ff) != 0) return 17;
    out("LIN_GUI: window painted\n");
    if (looswin_destroy((u32)hwnd) != 0) return 18;
    for (int tries = 0; tries != 8; ++tries) {
        if (looswin_next_event(0, &event) == -1) {
            out("LIN_GUI: quit observed\n");
            return 0;
        }
    }
    return 19;
}
