/* win32k service numbers: generated from crates/loos-win32k/src/service.rs (checked by its tests). SVC_BASE + n. */
#ifndef LOOS_WIN32K_H
#define LOOS_WIN32K_H
#define LOOS_SVC_BASE 0x1000
#define LOOS_WOULD_BLOCK ((long long)0x8000000000000000ll)
#define SVC_REGISTER_CLASS (LOOS_SVC_BASE + 0)
#define SVC_CREATE_WINDOW (LOOS_SVC_BASE + 1)
#define SVC_SHOW_WINDOW (LOOS_SVC_BASE + 2)
#define SVC_DESTROY_WINDOW (LOOS_SVC_BASE + 3)
#define SVC_GET_MESSAGE (LOOS_SVC_BASE + 4)
#define SVC_PEEK_MESSAGE (LOOS_SVC_BASE + 5)
#define SVC_TRANSLATE_MESSAGE (LOOS_SVC_BASE + 6)
#define SVC_DISPATCH_MESSAGE (LOOS_SVC_BASE + 7)
#define SVC_POST_QUIT (LOOS_SVC_BASE + 8)
#define SVC_DEF_WINDOW_PROC (LOOS_SVC_BASE + 9)
#define SVC_POST_MESSAGE (LOOS_SVC_BASE + 10)
#define SVC_SEND_MESSAGE (LOOS_SVC_BASE + 11)
#define SVC_BEGIN_PAINT (LOOS_SVC_BASE + 12)
#define SVC_END_PAINT (LOOS_SVC_BASE + 13)
#define SVC_GET_DC (LOOS_SVC_BASE + 14)
#define SVC_RELEASE_DC (LOOS_SVC_BASE + 15)
#define SVC_FILL_RECT (LOOS_SVC_BASE + 16)
#define SVC_TEXT_OUT (LOOS_SVC_BASE + 17)
#define SVC_SET_TEXT_COLOR (LOOS_SVC_BASE + 18)
#define SVC_SET_BK_COLOR (LOOS_SVC_BASE + 19)
#define SVC_SET_BK_MODE (LOOS_SVC_BASE + 20)
#define SVC_CREATE_BRUSH (LOOS_SVC_BASE + 21)
#define SVC_CREATE_PEN (LOOS_SVC_BASE + 22)
#define SVC_SELECT_OBJECT (LOOS_SVC_BASE + 23)
#define SVC_DELETE_OBJECT (LOOS_SVC_BASE + 24)
#define SVC_INVALIDATE_RECT (LOOS_SVC_BASE + 25)
#define SVC_UPDATE_WINDOW (LOOS_SVC_BASE + 26)
#define SVC_GET_CLIENT_RECT (LOOS_SVC_BASE + 27)
#define SVC_GET_WINDOW_RECT (LOOS_SVC_BASE + 28)
#define SVC_SET_TIMER (LOOS_SVC_BASE + 29)
#define SVC_KILL_TIMER (LOOS_SVC_BASE + 30)
#define SVC_MESSAGE_BOX (LOOS_SVC_BASE + 31)
#define SVC_SET_WINDOW_TEXT (LOOS_SVC_BASE + 32)
#define SVC_MOVE_WINDOW (LOOS_SVC_BASE + 33)
#define SVC_LINE_TO (LOOS_SVC_BASE + 34)
#define SVC_MOVE_TO (LOOS_SVC_BASE + 35)
#define SVC_RECTANGLE (LOOS_SVC_BASE + 36)
#define SVC_GET_STOCK_OBJECT (LOOS_SVC_BASE + 37)
#define SVC_SET_FOCUS (LOOS_SVC_BASE + 38)
#define SVC_GET_TICK (LOOS_SVC_BASE + 39)
#define SVC_MESSAGE_BOX_RESULT (LOOS_SVC_BASE + 40)
#define SVC_GET_LAST_ERROR (LOOS_SVC_BASE + 41)
/* Services use the 6-register syscall ABI (rdi rsi rdx r10 r8 r9), result in rax; the PE stubs do not need stack arguments. */
static inline long long loos_svc(long long n, long long a, long long b, long long c, long long d, long long e, long long f)
{
    long long r; register long long r10 __asm__("r10") = d; register long long r8 __asm__("r8") = e; register long long r9 __asm__("r9") = f;
    __asm__ volatile("syscall" : "=a"(r) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return r;
}
#endif
