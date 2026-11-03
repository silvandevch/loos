#ifndef LOOSWIN_H
#define LOOSWIN_H

#include <stdint.h>

/* Linux-side LoOS window bridge. COLORREF values are 0x00BBGGRR. */
#define LOOSWIN_VERSION 0x0A000001L
#define LOOSWIN_WM_PAINT 0x000F

struct looswin_event {
    uint32_t hwnd;
    uint32_t message;
    uint64_t wparam;
    int64_t lparam;
    int32_t x;
    int32_t y;
};

long looswin_version(void);
long looswin_tick(void);
long looswin_create_window(const char *title, int x, int y, int w, int h, uint32_t style);
long looswin_show(uint32_t hwnd, int cmd);
long looswin_next_event(uint32_t hwnd, struct looswin_event *event);
long looswin_fill_rect(uint32_t hwnd, int left, int top, int right, int bottom, uint32_t colorref);
long looswin_text(uint32_t hwnd, int x, int y, const char *text, uint32_t colorref);
long looswin_destroy(uint32_t hwnd);
long looswin_invalidate(uint32_t hwnd);
long looswin_set_title(uint32_t hwnd, const char *title);
long looswin_line(uint32_t hwnd, int x0, int y0, int x1, int y1, uint32_t colorref);
long looswin_post_quit(int code);

#endif
