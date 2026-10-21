/* Hosts an immediate-mode app inside a compositor window: owns the event pump, the client surface bookkeeping
 * and redraw policy, so every shell app (settings, file manager, task monitor, ...) is just a frame function. */
#ifndef LOOS_SHELL_APPHOST_H
#define LOOS_SHELL_APPHOST_H
#include "ui.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct shell_app_window {
    shell_comp *comp;
    int win;
    ui_ctx ui;
    uint32_t *px; int cap_w, cap_h;        /* caller-provided surface storage */
    int w, h;                              /* current client size */
    void (*frame)(void *app, ui_ctx *ui, int w, int h);
    void (*on_close)(void *app);           /* optional; default = destroy the window */
    void *app;
    bool dirty, closed;
    uint32_t frames;
} shell_app_window;

bool appwin_open(shell_app_window *aw, shell_comp *c, const char *title, shell_rect rect, uint32_t flags, uint32_t owner,
                 uint32_t *px, int cap_w, int cap_h, void (*frame)(void *, ui_ctx *, int, int), void *app);
void appwin_pump(shell_app_window *aw);   /* drain events, redraw if needed, invalidate the compositor region */
void appwin_close(shell_app_window *aw);
void appwin_redraw(shell_app_window *aw);

#ifdef __cplusplus
}
#endif
#endif
