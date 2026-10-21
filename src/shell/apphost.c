#include "apphost.h"
#include <string.h>

bool appwin_open(shell_app_window *aw, shell_comp *c, const char *title, shell_rect rect, uint32_t flags, uint32_t owner,
                 uint32_t *px, int cap_w, int cap_h, void (*frame)(void *, ui_ctx *, int, int), void *app)
{
    memset(aw, 0, sizeof *aw);
    aw->comp = c; aw->px = px; aw->cap_w = cap_w; aw->cap_h = cap_h; aw->frame = frame; aw->app = app;
    aw->win = comp_create(c, title, rect, flags, owner);
    if (!aw->win) return false;
    shell_rect cr = comp_client_rect(c, comp_get(c, aw->win));
    aw->w = cr.w > cap_w ? cap_w : cr.w; aw->h = cr.h > cap_h ? cap_h : cr.h;
    comp_set_client(c, aw->win, px, aw->w, aw->h, cap_w);
    aw->dirty = true;
    appwin_redraw(aw);
    return true;
}

/* UI scale of this window in percent: the client surface is device resolution, the app lays out in logical px */
static int win_scale(const shell_app_window *aw)
{
    const shell_window *w = comp_get(aw->comp, aw->win);
    return w && w->dpi >= 48 ? (int)(w->dpi * 100 / 96) : 100;
}

void appwin_redraw(shell_app_window *aw)
{
    int sc = win_scale(aw);
    shell_fb fb = { aw->px, aw->w, aw->h, aw->cap_w, sc };
    shell_comp *c = aw->comp;
    ui_begin(&aw->ui, &fb, &c->theme, 100);
    aw->ui.reduce_motion = c->reduce_motion; aw->ui.high_contrast = c->high_contrast;
    aw->frame(aw->app, &aw->ui, fb_lw(&fb), fb_lh(&fb));
    ui_end(&aw->ui);
    aw->dirty = false;
    aw->frames++;
    comp_invalidate(c, aw->win, (shell_rect){ 0, 0, aw->w, aw->h });
}

void appwin_close(shell_app_window *aw)
{
    if (aw->closed) return;
    aw->closed = true;
    comp_destroy(aw->comp, aw->win);
}

void appwin_pump(shell_app_window *aw)
{
    shell_comp *c = aw->comp;
    if (aw->closed || !comp_get(c, aw->win)) return;
    shell_event e;
    bool any = false;
    aw->ui.now_ms = c->now_ms;
    while (comp_pop_event(c, aw->win, &e)) {
        switch (e.type) {
        case EV_RESIZE:
            aw->w = e.x > aw->cap_w ? aw->cap_w : e.x; aw->h = e.y > aw->cap_h ? aw->cap_h : e.y;
            comp_set_client(c, aw->win, aw->px, aw->w, aw->h, aw->cap_w);
            aw->dirty = true;
            break;
        case EV_CLOSE_REQUEST:
            if (aw->on_close) aw->on_close(aw->app); else appwin_close(aw);
            return;
        case EV_DPI_CHANGED: aw->dirty = true; break;
        default:
            if (e.type == EV_MOUSE_MOVE || e.type == EV_MOUSE_DOWN || e.type == EV_MOUSE_UP || e.type == EV_MOUSE_WHEEL) {
                int sc = win_scale(aw);                                  /* device px -> logical px */
                e.x = e.x * 100 / sc; e.y = e.y * 100 / sc;
            }
            ui_feed(&aw->ui, &e);
            any = true;
        }
        if (e.type == EV_FOCUS || e.type == EV_BLUR) aw->dirty = true;
    }
    if (any || aw->dirty) {
        /* a frame can need two passes: the first applies a click, the second lays out the new state */
        appwin_redraw(aw);
    }
}
