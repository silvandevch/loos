/* Freestanding liblooswin client used by the ring-3 proof app. */
#include "liblooswin.h"

#define LOOSWIN_BASE 0x4c4f0000UL

static long looswin_call(unsigned long nr, unsigned long a0, unsigned long a1,
                         unsigned long a2, unsigned long a3, unsigned long a4,
                         unsigned long a5)
{
    register unsigned long r10 __asm__("r10") = a3;
    register unsigned long r8 __asm__("r8") = a4;
    register unsigned long r9 __asm__("r9") = a5;
    long result;
    __asm__ volatile("syscall"
                     : "=a"(result)
                     : "a"(LOOSWIN_BASE + nr), "D"(a0), "S"(a1), "d"(a2),
                       "r"(r10), "r"(r8), "r"(r9)
                     : "rcx", "r11", "memory");
    return result;
}

long looswin_version(void) { return looswin_call(0, 0, 0, 0, 0, 0, 0); }
long looswin_tick(void) { return looswin_call(1, 0, 0, 0, 0, 0, 0); }
long looswin_create_window(const char *title, int x, int y, int w, int h, uint32_t style)
{
    return looswin_call(2, (unsigned long)title, (unsigned long)(uint32_t)x,
                        (unsigned long)(uint32_t)y, (unsigned long)(uint32_t)w,
                        (unsigned long)(uint32_t)h, style);
}
long looswin_show(uint32_t hwnd, int cmd) { return looswin_call(3, hwnd, (uint32_t)cmd, 0, 0, 0, 0); }
long looswin_next_event(uint32_t hwnd, struct looswin_event *event) { return looswin_call(4, hwnd, (unsigned long)event, 0, 0, 0, 0); }
long looswin_fill_rect(uint32_t hwnd, int left, int top, int right, int bottom, uint32_t colorref)
{
    return looswin_call(5, hwnd, (uint32_t)left, (uint32_t)top, (uint32_t)right,
                        (uint32_t)bottom, colorref);
}
long looswin_text(uint32_t hwnd, int x, int y, const char *text, uint32_t colorref)
{
    return looswin_call(6, hwnd, (uint32_t)x, (uint32_t)y, (unsigned long)text, colorref, 0);
}
long looswin_destroy(uint32_t hwnd) { return looswin_call(7, hwnd, 0, 0, 0, 0, 0); }
long looswin_invalidate(uint32_t hwnd) { return looswin_call(8, hwnd, 0, 0, 0, 0, 0); }
long looswin_set_title(uint32_t hwnd, const char *title) { return looswin_call(9, hwnd, (unsigned long)title, 0, 0, 0, 0); }
long looswin_line(uint32_t hwnd, int x0, int y0, int x1, int y1, uint32_t colorref)
{
    return looswin_call(10, hwnd, (uint32_t)x0, (uint32_t)y0, (uint32_t)x1,
                        (uint32_t)y1, colorref);
}
long looswin_post_quit(int code) { return looswin_call(11, (uint32_t)code, 0, 0, 0, 0, 0); }
