/*
 * Linux-side liblooswin preload ABI.
 *
 * The library is intentionally a thin syscall ABI client: it has no sockets,
 * threads, telemetry, or host-window fallback.  On LoOS the bridge syscall
 * range is serviced by crates/loos-lin::bridge; on another Linux host calls
 * fail with the host's normal ENOSYS result.
 */
#define _GNU_SOURCE
#include "liblooswin.h"
#include <sys/syscall.h>
#include <unistd.h>

#define LW_BASE 0x4c4f0000UL

static long lw0(unsigned long n) { return syscall((long)(LW_BASE + n)); }
static long lw1(unsigned long n, unsigned long a0) { return syscall((long)(LW_BASE + n), a0); }
static long lw2(unsigned long n, unsigned long a0, unsigned long a1) { return syscall((long)(LW_BASE + n), a0, a1); }
static long lw6(unsigned long n, unsigned long a0, unsigned long a1, unsigned long a2,
                unsigned long a3, unsigned long a4, unsigned long a5);

__attribute__((visibility("default"))) long looswin_version(void) { return lw0(0); }
__attribute__((visibility("default"))) long looswin_tick(void) { return lw0(1); }
__attribute__((visibility("default"))) long looswin_create_window(const char *title, int x, int y, int w, int h, uint32_t style)
{
    return lw6(2, (unsigned long)title, (unsigned long)(uint32_t)x, (unsigned long)(uint32_t)y,
               (unsigned long)(uint32_t)w, (unsigned long)(uint32_t)h, style);
}

/* Use a packed syscall helper for the calls with six bridge arguments. */
static long lw6(unsigned long n, unsigned long a0, unsigned long a1, unsigned long a2,
                unsigned long a3, unsigned long a4, unsigned long a5)
{
    register unsigned long r10 __asm__("r10") = a3;
    register unsigned long r8 __asm__("r8") = a4;
    register unsigned long r9 __asm__("r9") = a5;
    long out;
    __asm__ volatile("syscall" : "=a"(out) : "a"(LW_BASE + n), "D"(a0), "S"(a1), "d"(a2), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return out;
}

__attribute__((visibility("default"))) long looswin_show(uint32_t hwnd, int cmd) { return lw2(3, hwnd, (unsigned long)(uint32_t)cmd); }
__attribute__((visibility("default"))) long looswin_next_event(uint32_t hwnd, struct looswin_event *event) { return lw2(4, hwnd, (unsigned long)event); }
__attribute__((visibility("default"))) long looswin_fill_rect(uint32_t hwnd, int l, int t, int r, int b, uint32_t colorref) { return lw6(5, hwnd, (uint32_t)l, (uint32_t)t, (uint32_t)r, (uint32_t)b, colorref); }
__attribute__((visibility("default"))) long looswin_text(uint32_t hwnd, int x, int y, const char *text, uint32_t colorref) { return lw6(6, hwnd, (uint32_t)x, (uint32_t)y, (unsigned long)text, colorref, 0); }
__attribute__((visibility("default"))) long looswin_destroy(uint32_t hwnd) { return lw1(7, hwnd); }
__attribute__((visibility("default"))) long looswin_invalidate(uint32_t hwnd) { return lw1(8, hwnd); }
__attribute__((visibility("default"))) long looswin_set_title(uint32_t hwnd, const char *title) { return lw2(9, hwnd, (unsigned long)title); }
__attribute__((visibility("default"))) long looswin_line(uint32_t hwnd, int x0, int y0, int x1, int y1, uint32_t colorref) { return lw6(10, hwnd, (uint32_t)x0, (uint32_t)y0, (uint32_t)x1, (uint32_t)y1, colorref); }
__attribute__((visibility("default"))) long looswin_post_quit(int code) { return lw1(11, (unsigned long)(uint32_t)code); }
