#include "shell.h"
#include <string.h>

#define SNAP_EDGE 6
#define SNAP_CORNER 48

void wm_init(shell_wm *wm, shell_comp *c)
{
    memset(wm, 0, sizeof *wm);
    wm->comp = c; wm->gap = 6; wm->ndesktops = SHELL_DESKTOPS; wm->master_pct = 55;
    wm_add_monitor(wm, (shell_rect){ 0, 0, c->screen_w, c->screen_h }, c->scale_pct, "primary");
}

bool wm_add_monitor(shell_wm *wm, shell_rect r, uint32_t scale, const char *name)
{
    if (wm->nmon >= SHELL_MAX_MONITORS) return false;
    shell_monitor *m = &wm->mon[wm->nmon++];
    m->rect = r; m->scale_pct = scale ? scale : 100;
    strncpy(m->name, name ? name : "", sizeof m->name - 1); m->name[sizeof m->name - 1] = 0;
    return true;
}

int wm_monitor_at(const shell_wm *wm, int x, int y)
{
    int best = 0; long bestd = -1;
    for (int i = 0; i < wm->nmon; i++) {
        shell_rect r = wm->mon[i].rect;
        if (rect_contains(r, x, y)) return i;
        int dx = x < r.x ? r.x - x : x >= r.x + r.w ? x - (r.x + r.w - 1) : 0;
        int dy = y < r.y ? r.y - y : y >= r.y + r.h ? y - (r.y + r.h - 1) : 0;
        long d = (long)dx * dx + (long)dy * dy;
        if (bestd < 0 || d < bestd) { bestd = d; best = i; }
    }
    return best;
}

shell_rect wm_monitor_work_area(const shell_wm *wm, int mon)
{
    shell_rect r = wm->mon[mon].rect;
    shell_rect wa = rect_intersect(r, wm->comp->work_area);             /* work area already excludes the taskbar strip */
    return rect_empty(wa) ? r : wa;
}

shell_rect wm_snap_rect(const shell_wm *wm, int mon, shell_snap s)
{
    shell_rect a = wm_monitor_work_area(wm, mon);
    int g = wm->gap, hw = a.w / 2, hh = a.h / 2, t = a.w / 3;
    shell_rect r = a;
    switch (s) {
    case SNAP_LEFT: r = (shell_rect){ a.x, a.y, hw, a.h }; break;
    case SNAP_RIGHT: r = (shell_rect){ a.x + hw, a.y, a.w - hw, a.h }; break;
    case SNAP_MAXIMIZE: return a;
    case SNAP_TL: r = (shell_rect){ a.x, a.y, hw, hh }; break;
    case SNAP_TR: r = (shell_rect){ a.x + hw, a.y, a.w - hw, hh }; break;
    case SNAP_BL: r = (shell_rect){ a.x, a.y + hh, hw, a.h - hh }; break;
    case SNAP_BR: r = (shell_rect){ a.x + hw, a.y + hh, a.w - hw, a.h - hh }; break;
    case SNAP_THIRD_L: r = (shell_rect){ a.x, a.y, t, a.h }; break;
    case SNAP_THIRD_C: r = (shell_rect){ a.x + t, a.y, t, a.h }; break;
    case SNAP_THIRD_R: r = (shell_rect){ a.x + 2 * t, a.y, a.w - 2 * t, a.h }; break;
    case SNAP_TWO_THIRDS_L: r = (shell_rect){ a.x, a.y, 2 * t, a.h }; break;
    case SNAP_TWO_THIRDS_R: r = (shell_rect){ a.x + t, a.y, a.w - t, a.h }; break;
    default: return a;
    }
    /* gaps: half a gap on interior edges, none against the screen edge */
    int l = r.x > a.x ? g / 2 : 0, rr = r.x + r.w < a.x + a.w ? g / 2 : 0, tp = r.y > a.y ? g / 2 : 0, b = r.y + r.h < a.y + a.h ? g / 2 : 0;
    r.x += l; r.w -= l + rr; r.y += tp; r.h -= tp + b;
    return r;
}

shell_snap wm_detect_snap(const shell_wm *wm, int x, int y, int *mon)
{
    int m = wm_monitor_at(wm, x, y);
    if (mon) *mon = m;
    shell_rect a = wm->mon[m].rect;
    bool l = x < a.x + SNAP_EDGE, r = x >= a.x + a.w - SNAP_EDGE, t = y < a.y + SNAP_EDGE, b = y >= a.y + a.h - SNAP_EDGE - TASKBAR_H * (m == 0);
    bool nl = x < a.x + SNAP_CORNER, nr = x >= a.x + a.w - SNAP_CORNER, nt = y < a.y + SNAP_CORNER;
    if ((l && nt) || (t && nl)) return SNAP_TL;
    if ((r && nt) || (t && nr)) return SNAP_TR;
    if (l && y >= a.y + a.h - SNAP_CORNER - TASKBAR_H) return SNAP_BL;
    if (r && y >= a.y + a.h - SNAP_CORNER - TASKBAR_H) return SNAP_BR;
    if (t) return SNAP_MAXIMIZE;
    if (l) return SNAP_LEFT;
    if (r) return SNAP_RIGHT;
    (void)b;
    return SNAP_NONE;
}

bool wm_snap_window(shell_wm *wm, int win, shell_snap s, int mon)
{
    shell_comp *c = wm->comp;
    shell_window *w = comp_get(c, win);
    if (!w || (w->flags & WF_NORESIZE) || mon < 0 || mon >= wm->nmon) return false;
    if (s == SNAP_NONE) { wm_unsnap(wm, win); return true; }
    if (!(w->flags & (WF_SNAPPED | WF_MAXIMIZED))) w->restore = w->rect;       /* remember the free-floating geometry once */
    shell_rect r = wm_snap_rect(wm, mon, s);
    if (s == SNAP_MAXIMIZE) { w->flags |= WF_MAXIMIZED; w->flags &= ~(uint32_t)WF_SNAPPED; }
    else { w->flags |= WF_SNAPPED; w->flags &= ~(uint32_t)WF_MAXIMIZED; }
    comp_set_rect(c, win, r);
    comp_notify(c, win);
    return true;
}

void wm_unsnap(shell_wm *wm, int win)
{
    shell_window *w = comp_get(wm->comp, win);
    if (!w || !(w->flags & (WF_SNAPPED | WF_MAXIMIZED))) return;
    w->flags &= ~(uint32_t)(WF_SNAPPED | WF_MAXIMIZED);
    comp_set_rect(wm->comp, win, w->restore);
    comp_notify(wm->comp, win);
}

static void apply_dpi(shell_wm *wm, shell_window *w)
{
    int mon = wm_monitor_at(wm, w->rect.x + w->rect.w / 2, w->rect.y + w->rect.h / 2);
    uint32_t dpi = 96 * wm->mon[mon].scale_pct / 100;
    if (dpi != w->dpi) { w->dpi = dpi; comp_post_event(wm->comp, w->id, (shell_event){ EV_DPI_CHANGED, (int)dpi, 0, 0, 0, 0 }); }
}

void wm_move_to_monitor(shell_wm *wm, int win, int mon)
{
    shell_window *w = comp_get(wm->comp, win);
    if (!w || mon < 0 || mon >= wm->nmon) return;
    int from = wm_monitor_at(wm, w->rect.x + w->rect.w / 2, w->rect.y + w->rect.h / 2);
    if (from == mon) return;
    shell_rect a = wm_monitor_work_area(wm, from), b = wm_monitor_work_area(wm, mon);
    if (w->flags & (WF_MAXIMIZED | WF_SNAPPED)) {                             /* keep the layout, re-resolve it on the new screen */
        shell_snap s = SNAP_MAXIMIZE;
        if (w->flags & WF_SNAPPED) {
            bool left = w->rect.x < a.x + a.w / 4, right = w->rect.x + w->rect.w > a.x + a.w - a.w / 4;
            s = left && !right ? SNAP_LEFT : right && !left ? SNAP_RIGHT : SNAP_MAXIMIZE;
        }
        shell_rect keep = w->restore;
        wm_snap_window(wm, win, s, mon);
        w->restore = (shell_rect){ b.x + (keep.x - a.x), b.y + (keep.y - a.y), keep.w, keep.h };
    } else {
        shell_rect r = w->rect;
        r.x = b.x + (r.x - a.x) * b.w / (a.w ? a.w : 1); r.y = b.y + (r.y - a.y) * b.h / (a.h ? a.h : 1);
        if (r.w > b.w) r.w = b.w;
        if (r.h > b.h) r.h = b.h;
        comp_set_rect(wm->comp, win, r);
    }
    apply_dpi(wm, w);
}

/* ---- virtual desktops ---- */
void wm_switch_desktop(shell_wm *wm, int n)
{
    shell_comp *c = wm->comp;
    if (n < 0 || n >= wm->ndesktops || n == c->cur_desktop) return;
    shell_window *f = comp_get(c, c->focus);
    if (f && !(f->flags & WF_STICKY)) { comp_post_event(c, f->id, (shell_event){ EV_BLUR, 0, 0, 0, 0, 0 }); c->focus = 0; }
    c->cur_desktop = n;
    c->grab_win = c->capture = c->press_win = 0;
    comp_damage_all(c);
    for (int i = c->nwin - 1; i >= 0; i--) {                                      /* focus the topmost window of the new desktop */
        shell_window *w = comp_get(c, c->order[i]);
        if (w && comp_is_shown(c, w) && !(w->flags & WF_NOFOCUS)) { comp_focus(c, w->id); break; }
    }
    if (wm->tiling) wm_retile(wm);
    comp_notify(c, 0);
}

bool wm_move_to_desktop(shell_wm *wm, int win, int n)
{
    shell_comp *c = wm->comp;
    shell_window *w = comp_get(c, win);
    if (!w || n < 0 || n >= wm->ndesktops) return false;
    if (w->desktop == n) return true;
    bool had_focus = c->focus == win;
    w->desktop = n;
    comp_damage_all(c);
    if (!comp_is_shown(c, w) && had_focus) {
        c->focus = 0;
        comp_post_event(c, win, (shell_event){ EV_BLUR, 0, 0, 0, 0, 0 });
        for (int i = c->nwin - 1; i >= 0; i--) {
            shell_window *o = comp_get(c, c->order[i]);
            if (o && comp_is_shown(c, o) && !(o->flags & WF_NOFOCUS)) { comp_focus(c, o->id); break; }
        }
    }
    if (wm->tiling) wm_retile(wm);
    comp_notify(c, win);
    return true;
}

/* ---- tiling: master on the left, the rest stacked on the right ---- */
void wm_set_tiling(shell_wm *wm, bool on) { wm->tiling = on; if (on) wm_retile(wm); }

void wm_retile(shell_wm *wm)
{
    shell_comp *c = wm->comp;
    int ids[SHELL_MAX_WINDOWS], n = 0;
    for (int i = 0; i < c->nwin; i++) {                                           /* z-order back..front; master = most recently raised */
        shell_window *w = comp_get(c, c->order[i]);
        if (w && comp_is_shown(c, w) && !(w->flags & (WF_NORESIZE | WF_NOCHROME | WF_TOPMOST))) ids[n++] = w->id;
    }
    if (!n) return;
    shell_rect a = wm_monitor_work_area(wm, 0);
    int g = wm->gap;
    shell_rect area = { a.x + g, a.y + g, a.w - 2 * g, a.h - 2 * g };
    for (int i = 0; i < n; i++) { shell_window *w = comp_get(c, ids[i]); w->flags &= ~(uint32_t)WF_MAXIMIZED; w->flags |= WF_SNAPPED; }
    if (n == 1) { comp_set_rect(c, ids[0], area); return; }
    int mw = area.w * wm->master_pct / 100;
    comp_set_rect(c, ids[n - 1], (shell_rect){ area.x, area.y, mw - g / 2, area.h });
    int stack = n - 1, sh = (area.h - (stack - 1) * g) / stack;
    for (int i = 0; i < stack; i++) {
        int y = area.y + i * (sh + g);
        int h = i == stack - 1 ? area.y + area.h - y : sh;
        comp_set_rect(c, ids[n - 2 - i], (shell_rect){ area.x + mw + g / 2, y, area.w - mw - g / 2, h });
    }
}

/* ---- pointer-driven behaviour ---- */
void wm_after_mouse(shell_wm *wm)
{
    shell_comp *c = wm->comp;
    int g = c->grab_win;
    shell_window *w = comp_get(c, g ? g : wm->was_grab);
    if (g && c->grab_zone == Z_TITLE && w) {
        int mon;
        shell_snap s = wm_detect_snap(wm, c->mouse_x, c->mouse_y, &mon);
        if (s != wm->preview || mon != wm->preview_mon) {
            if (wm->preview != SNAP_NONE) damage_add(&c->damage, wm_snap_rect(wm, wm->preview_mon, wm->preview));
            wm->preview = s; wm->preview_mon = mon;
            if (s != SNAP_NONE) damage_add(&c->damage, wm_snap_rect(wm, mon, s));
        }
    } else if (!g && wm->was_grab) {                                              /* drag finished */
        if (wm->preview != SNAP_NONE && w) {
            int win = w->id; shell_snap s = wm->preview; int mon = wm->preview_mon;
            damage_add(&c->damage, wm_snap_rect(wm, mon, s));
            wm->preview = SNAP_NONE;
            wm_snap_window(wm, win, s, mon);
        }
        if (w) apply_dpi(wm, w);
        wm->preview = SNAP_NONE;
        if (wm->tiling) wm_retile(wm);
    }
    wm->was_grab = g;
    if (wm->focus_mode == FOCUS_FOLLOWS_MOUSE && !g && !c->capture) {
        int hw; shell_zone z = comp_hit_test(c, c->mouse_x, c->mouse_y, &hw);
        if (z != Z_NONE && hw != c->focus) comp_focus(c, hw);
    }
}

void wm_draw_preview(shell_comp *c, shell_fb *fb, shell_rect clip, void *user)
{
    shell_wm *wm = user;
    if (wm->preview == SNAP_NONE) return;
    shell_rect r = wm_snap_rect(wm, wm->preview_mon, wm->preview);
    fb_fill_round(fb, clip, r, 8, (c->theme.accent & 0x00FFFFFFu) | 0x30000000u);
    shell_rect in = { r.x + 1, r.y + 1, r.w - 2, r.h - 2 };
    fb_frame(fb, clip, r, c->theme.accent); fb_frame(fb, clip, in, (c->theme.accent & 0x00FFFFFFu) | 0x80000000u);
}
void wm_install(shell_wm *wm) { comp_add_overlay(wm->comp, wm_draw_preview, wm); }

/* ---- keyboard: Super+arrows snap, Super+digit desktops, Super+T tiling ---- */
bool wm_key(shell_wm *wm, uint32_t key, bool down, uint32_t mods)
{
    shell_comp *c = wm->comp;
    if (!(mods & MOD_SUPER)) return false;
    if (!down) return true;
    int f = c->focus;
    shell_window *w = comp_get(c, f);
    if (key >= '1' && key < '1' + (uint32_t)wm->ndesktops) {
        int n = (int)(key - '1');
        if (mods & MOD_SHIFT) { if (f) wm_move_to_desktop(wm, f, n); } else wm_switch_desktop(wm, n);
        return true;
    }
    if (key == 't' || key == 'T') { wm_set_tiling(wm, !wm->tiling); return true; }
    if (!w) return key == KEY_LEFT || key == KEY_RIGHT || key == KEY_UP || key == KEY_DOWN;
    int mon = wm_monitor_at(wm, w->rect.x + w->rect.w / 2, w->rect.y + w->rect.h / 2);
    bool left_half = (w->flags & WF_SNAPPED) && w->rect.x < wm_monitor_work_area(wm, mon).x + 4;
    bool right_half = (w->flags & WF_SNAPPED) && w->rect.x > wm_monitor_work_area(wm, mon).x + 4;
    if (mods & MOD_SHIFT) {
        if (key == KEY_LEFT && mon > 0) { wm_move_to_monitor(wm, f, mon - 1); return true; }
        if (key == KEY_RIGHT && mon + 1 < wm->nmon) { wm_move_to_monitor(wm, f, mon + 1); return true; }
        return key == KEY_LEFT || key == KEY_RIGHT;
    }
    switch (key) {
    case KEY_LEFT:
        if (right_half) wm_unsnap(wm, f); else wm_snap_window(wm, f, SNAP_LEFT, mon);
        return true;
    case KEY_RIGHT:
        if (left_half) wm_unsnap(wm, f); else wm_snap_window(wm, f, SNAP_RIGHT, mon);
        return true;
    case KEY_UP: wm_snap_window(wm, f, SNAP_MAXIMIZE, mon); return true;
    case KEY_DOWN:
        if (w->flags & (WF_MAXIMIZED | WF_SNAPPED)) wm_unsnap(wm, f); else comp_minimize(c, f);
        return true;
    default: return false;
    }
}
