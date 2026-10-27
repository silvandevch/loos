#include "shell.h"
#include "raster.h"
#include <string.h>

#define CURSOR_W 14          /* logical size of the arrow incl. outline, for damage */
#define CURSOR_H 22
#define SHADOW_BLUR 22       /* logical px */
#define SHADOW_DY 7
#define DBLCLICK_MS 400

static void metrics_for(shell_metrics *m, const shell_style *st, uint32_t pct)
{
    int th = st->title_h > 0 ? st->title_h : 28, bd = st->border > 0 ? st->border : 1, rad = st->radius >= 0 ? st->radius : 8;
    m->title_h = (int)((uint32_t)th * pct / 100); m->border = st->border > 0 ? (int)((uint32_t)bd * (pct >= 150 ? 2 : 1)) : (pct >= 150 ? 2 : 1);
    m->button_w = (int)(40 * pct / 100); m->edge_grab = (int)(6 * pct / 100); m->corner_r = (int)((uint32_t)rad * pct / 100);
}

void comp_init(shell_comp *c, int w, int h)
{
    memset(c, 0, sizeof *c);
    c->screen_w = w; c->screen_h = h; c->next_id = 1; c->scale_pct = 100;
    c->style = (shell_style){ -1, 0, 0, 100, 100 };
    metrics_for(&c->m, &c->style, 100);
    shell_theme_dark(&c->theme, 0xFF3B82F6u);
    c->work_area = (shell_rect){ 0, 0, w, h };
    c->mouse_x = w / 2; c->mouse_y = h / 2;
    comp_damage_all(c);
}
bool comp_add_overlay(shell_comp *c, void (*fn)(shell_comp *, shell_fb *, shell_rect, void *), void *user)
{
    for (int i = 0; i < 4; i++) if (!c->overlay[i].fn) { c->overlay[i].fn = fn; c->overlay[i].user = user; return true; }
    return false;
}
void comp_set_scale(shell_comp *c, uint32_t pct)
{
    if (pct < 100) pct = 100;
    if (pct > 400) pct = 400;
    c->scale_pct = pct;
    metrics_for(&c->m, &c->style, pct);
    if (c->bottom_reserve) c->work_area = (shell_rect){ 0, 0, c->screen_w, c->screen_h - c->bottom_reserve * (int)pct / 100 };
    for (int i = 0; i < SHELL_MAX_WINDOWS; i++) {            /* every window re-lays out at the new scale */
        shell_window *w = &c->win[i];
        if (!w->id) continue;
        w->dpi = 96 * pct / 100;
        comp_post_event(c, w->id, (shell_event){ EV_DPI_CHANGED, (int)w->dpi, 0, 0, 0, 0 });
    }
    comp_damage_all(c);
}
void comp_set_style(shell_comp *c, const shell_style *st)
{
    shell_style n = *st;
    n.radius = n.radius < -1 ? -1 : n.radius > 24 ? 24 : n.radius;
    n.title_h = n.title_h <= 0 ? 0 : n.title_h < 20 ? 20 : n.title_h > 48 ? 48 : n.title_h;
    n.border = n.border < 0 ? 0 : n.border > 4 ? 4 : n.border;
    n.shadow_pct = n.shadow_pct < 0 ? 0 : n.shadow_pct > 150 ? 150 : n.shadow_pct;
    n.glow_pct = n.glow_pct < 0 ? 0 : n.glow_pct > 150 ? 150 : n.glow_pct;
    c->style = n;
    metrics_for(&c->m, &c->style, c->scale_pct);
    comp_damage_all(c);
}
int comp_lw(const shell_comp *c) { return c->screen_w * 100 / (int)c->scale_pct; }
int comp_lh(const shell_comp *c) { return c->screen_h * 100 / (int)c->scale_pct; }
int comp_logical(const shell_comp *c, int px) { return px * 100 / (int)c->scale_pct; }
shell_rect comp_clip_logical(const shell_comp *c, shell_rect r)
{
    int sc = (int)c->scale_pct;
    int x0 = r.x * 100 / sc, y0 = r.y * 100 / sc;
    int x1 = ((r.x + r.w) * 100 + sc - 1) / sc, y1 = ((r.y + r.h) * 100 + sc - 1) / sc;       /* round outwards */
    return (shell_rect){ x0, y0, x1 - x0, y1 - y0 };
}
shell_fb comp_view(const shell_comp *c, const shell_fb *fb) { shell_fb v = *fb; v.scale = (int)c->scale_pct; return v; }
void comp_damage_logical(shell_comp *c, shell_rect r)
{
    int sc = (int)c->scale_pct;
    damage_add(&c->damage, (shell_rect){ r.x * sc / 100 - 1, r.y * sc / 100 - 1, (r.w * sc + 99) / 100 + 3, (r.h * sc + 99) / 100 + 3 });
}
void comp_damage_all(shell_comp *c) { shell_rect all = { 0, 0, c->screen_w, c->screen_h }; damage_add(&c->damage, all); }
void comp_set_work_area(shell_comp *c, shell_rect wa)
{
    c->work_area = wa;
    for (int i = 0; i < SHELL_MAX_WINDOWS; i++) {          /* maximized windows follow the work area */
        shell_window *w = &c->win[i];
        if (w->id && (w->flags & WF_MAXIMIZED)) { w->rect = wa; shell_event e = { EV_RESIZE, wa.w, wa.h, 0, 0, 0 }; (void)e; }
    }
    comp_damage_all(c);
}

shell_window *comp_get(shell_comp *c, int id)
{
    if (id <= 0) return NULL;
    for (int i = 0; i < SHELL_MAX_WINDOWS; i++) if (c->win[i].id == id) return &c->win[i];
    return NULL;
}
void comp_notify(shell_comp *c, int win) { if (c->on_change) c->on_change(c, win, c->on_change_user); }

static void push_event(shell_window *w, shell_event e)
{
    if (w->q_count == SHELL_EVQ) {
        if (e.type == EV_MOUSE_MOVE) return;                    /* drop the newest motion */
        w->q_head = (w->q_head + 1) % SHELL_EVQ; w->q_count--;   /* otherwise drop the oldest */
    }
    w->q[(w->q_head + w->q_count++) % SHELL_EVQ] = e;
}
bool comp_pop_event(shell_comp *c, int id, shell_event *out)
{
    shell_window *w = comp_get(c, id);
    if (!w || !w->q_count) return false;
    *out = w->q[w->q_head];
    w->q_head = (w->q_head + 1) % SHELL_EVQ; w->q_count--;
    return true;
}

bool comp_is_shown(const shell_comp *c, const shell_window *w)
{
    return w->id && (w->flags & WF_VISIBLE) && !(w->flags & WF_MINIMIZED) && ((w->flags & WF_STICKY) || w->desktop == c->cur_desktop);
}
static shell_rect outer_with_shadow(const shell_comp *c, shell_rect r)
{
    int m = (SHADOW_BLUR * 3 / 2 + SHADOW_DY + 2) * (int)c->scale_pct / 100;
    r.x -= m; r.y -= m; r.w += 2 * m; r.h += 2 * m + SHADOW_DY * (int)c->scale_pct / 100;
    return r;
}
static void damage_window(shell_comp *c, const shell_window *w) { damage_add(&c->damage, outer_with_shadow(c, w->rect)); }

shell_rect comp_client_rect(const shell_comp *c, const shell_window *w)
{
    if (w->flags & WF_NOCHROME) return w->rect;
    int b = c->m.border;
    shell_rect r = { w->rect.x + b, w->rect.y + b + c->m.title_h, w->rect.w - 2 * b, w->rect.h - 2 * b - c->m.title_h };
    if (r.w < 0) r.w = 0;
    if (r.h < 0) r.h = 0;
    return r;
}

static void order_remove(shell_comp *c, int id)
{
    for (int i = 0; i < c->nwin; i++)
        if (c->order[i] == id) { memmove(&c->order[i], &c->order[i + 1], (size_t)(c->nwin - i - 1) * sizeof(int)); c->nwin--; return; }
}
static void order_insert_top(shell_comp *c, int id)
{
    shell_window *w = comp_get(c, id);
    int pos = c->nwin;
    if (!(w->flags & WF_TOPMOST))
        while (pos > 0 && (comp_get(c, c->order[pos - 1])->flags & WF_TOPMOST)) pos--;
    memmove(&c->order[pos + 1], &c->order[pos], (size_t)(c->nwin - pos) * sizeof(int));
    c->order[pos] = id; c->nwin++;
}

int comp_create(shell_comp *c, const char *title, shell_rect rect, uint32_t flags, uint32_t owner)
{
    shell_window *w = NULL;
    for (int i = 0; i < SHELL_MAX_WINDOWS; i++) if (!c->win[i].id) { w = &c->win[i]; break; }
    if (!w) return 0;
    memset(w, 0, sizeof *w);
    w->id = c->next_id++;
    w->flags = flags | WF_VISIBLE;
    w->owner = owner; w->desktop = c->cur_desktop; w->dpi = 96 * c->scale_pct / 100;
    w->min_w = 160; w->min_h = c->m.title_h + 60;
    strncpy(w->title, title ? title : "", SHELL_TITLE_MAX - 1);
    w->rect = rect;
    if (w->rect.w < w->min_w) w->rect.w = w->min_w;
    if (w->rect.h < w->min_h) w->rect.h = w->min_h;
    order_insert_top(c, w->id);
    damage_window(c, w);
    if (!(w->flags & WF_NOFOCUS)) comp_focus(c, w->id);
    comp_notify(c, w->id);
    return w->id;
}

static void focus_next_shown(shell_comp *c)
{
    for (int i = c->nwin - 1; i >= 0; i--) {
        shell_window *w = comp_get(c, c->order[i]);
        if (w && comp_is_shown(c, w) && !(w->flags & WF_NOFOCUS)) { comp_focus(c, w->id); return; }
    }
    comp_focus(c, 0);
}

void comp_destroy(shell_comp *c, int id)
{
    shell_window *w = comp_get(c, id);
    if (!w) return;
    damage_window(c, w);
    order_remove(c, id);
    if (c->grab_win == id) c->grab_win = 0;
    if (c->capture == id) c->capture = 0;
    if (c->hover == id) c->hover = 0;
    bool was_focus = c->focus == id;
    memset(w, 0, sizeof *w);
    if (was_focus) { c->focus = 0; focus_next_shown(c); }
    comp_notify(c, id);
}

void comp_set_title(shell_comp *c, int id, const char *t)
{
    shell_window *w = comp_get(c, id);
    if (!w) return;
    strncpy(w->title, t, SHELL_TITLE_MAX - 1); w->title[SHELL_TITLE_MAX - 1] = 0;
    shell_rect tb = { w->rect.x, w->rect.y, w->rect.w, c->m.title_h + c->m.border };
    damage_add(&c->damage, tb);
    comp_notify(c, id);
}

void comp_set_rect(shell_comp *c, int id, shell_rect r)
{
    shell_window *w = comp_get(c, id);
    if (!w) return;
    if (r.w < w->min_w) r.w = w->min_w;
    if (r.h < w->min_h) r.h = w->min_h;
    bool resized = r.w != w->rect.w || r.h != w->rect.h, moved = r.x != w->rect.x || r.y != w->rect.y;
    damage_window(c, w);
    w->rect = r;
    damage_window(c, w);
    shell_rect cr = comp_client_rect(c, w);
    if (resized) push_event(w, (shell_event){ EV_RESIZE, cr.w, cr.h, 0, 0, 0 });
    if (moved) push_event(w, (shell_event){ EV_MOVE, r.x, r.y, 0, 0, 0 });
}

void comp_set_client(shell_comp *c, int id, uint32_t *px, int cw, int ch, int stride)
{
    shell_window *w = comp_get(c, id);
    if (!w) return;
    w->client = px; w->client_w = cw; w->client_h = ch; w->client_stride = stride;
    damage_add(&c->damage, comp_client_rect(c, w));
}
void comp_invalidate(shell_comp *c, int id, shell_rect rr)
{
    shell_window *w = comp_get(c, id);
    if (!w || !comp_is_shown(c, w)) return;
    shell_rect cr = comp_client_rect(c, w);
    shell_rect abs = { cr.x + rr.x, cr.y + rr.y, rr.w, rr.h };
    damage_add(&c->damage, rect_intersect(abs, cr));
}

void comp_raise(shell_comp *c, int id)
{
    shell_window *w = comp_get(c, id);
    if (!w || (c->nwin && c->order[c->nwin - 1] == id)) return;
    order_remove(c, id); order_insert_top(c, id);
    damage_window(c, w);
}
void comp_lower(shell_comp *c, int id)
{
    shell_window *w = comp_get(c, id);
    if (!w) return;
    order_remove(c, id);
    memmove(&c->order[1], &c->order[0], (size_t)c->nwin * sizeof(int));
    c->order[0] = id; c->nwin++;
    damage_window(c, w);
    if (c->focus == id) focus_next_shown(c);
}

void comp_focus(shell_comp *c, int id)
{
    shell_window *nw = id ? comp_get(c, id) : NULL;
    if (id && (!nw || (nw->flags & WF_NOFOCUS))) return;
    if (c->focus == id) return;
    shell_window *old = comp_get(c, c->focus);
    if (old) { push_event(old, (shell_event){ EV_BLUR, 0, 0, 0, 0, 0 }); damage_window(c, old); }
    c->focus = id;
    if (nw) { push_event(nw, (shell_event){ EV_FOCUS, 0, 0, 0, 0, 0 }); damage_window(c, nw); }
    comp_notify(c, id);
}

void comp_minimize(shell_comp *c, int id)
{
    shell_window *w = comp_get(c, id);
    if (!w || (w->flags & WF_MINIMIZED)) return;
    damage_window(c, w);
    w->flags |= WF_MINIMIZED;
    if (c->focus == id) { c->focus = 0; push_event(w, (shell_event){ EV_BLUR, 0, 0, 0, 0, 0 }); focus_next_shown(c); }
    comp_notify(c, id);
}
void comp_restore(shell_comp *c, int id)
{
    shell_window *w = comp_get(c, id);
    if (!w) return;
    if (w->flags & WF_MINIMIZED) { w->flags &= ~(uint32_t)WF_MINIMIZED; comp_raise(c, id); damage_window(c, w); }
    comp_focus(c, id);
    comp_notify(c, id);
}
void comp_toggle_maximize(shell_comp *c, int id)
{
    shell_window *w = comp_get(c, id);
    if (!w || (w->flags & WF_NORESIZE)) return;
    if (w->flags & (WF_MAXIMIZED | WF_SNAPPED)) {
        w->flags &= ~(uint32_t)(WF_MAXIMIZED | WF_SNAPPED);
        comp_set_rect(c, id, w->restore);
    } else {
        w->restore = w->rect;
        w->flags |= WF_MAXIMIZED;
        comp_set_rect(c, id, c->work_area);
    }
    comp_notify(c, id);
}

/* ---------- hit testing ---------- */
static shell_zone zone_at(const shell_comp *c, const shell_window *w, int x, int y)
{
    if (!rect_contains(w->rect, x, y)) return Z_NONE;
    if (w->flags & WF_NOCHROME) return Z_CLIENT;
    const shell_metrics *m = &c->m;
    bool can_size = !(w->flags & (WF_NORESIZE | WF_MAXIMIZED));
    int lx = x - w->rect.x, ly = y - w->rect.y, rx = w->rect.w - 1 - lx, by = w->rect.h - 1 - ly;
    if (can_size) {
        int g = m->edge_grab, cg = g * 2;
        bool l = lx < g, r = rx < g, t = ly < g, b = by < g;
        if ((l && ly < cg) || (t && lx < cg)) return Z_CORNER_TL;
        if ((r && ly < cg) || (t && rx < cg)) return Z_CORNER_TR;
        if ((l && by < cg) || (b && lx < cg)) return Z_CORNER_BL;
        if ((r && by < cg) || (b && rx < cg)) return Z_CORNER_BR;
        if (l) return Z_EDGE_L;
        if (r) return Z_EDGE_R;
        if (t) return Z_EDGE_T;
        if (b) return Z_EDGE_B;
    }
    if (ly < m->border + m->title_h) {
        int from_right = w->rect.w - m->border - lx;
        if (from_right >= 0 && from_right < m->button_w) return Z_CLOSE;
        if (from_right >= m->button_w && from_right < 2 * m->button_w && !(w->flags & WF_NORESIZE)) return Z_MAXIMIZE;
        if (from_right >= 2 * m->button_w && from_right < 3 * m->button_w) return Z_MINIMIZE;
        return Z_TITLE;
    }
    return Z_CLIENT;
}
shell_zone comp_hit_test(const shell_comp *c, int x, int y, int *win_id)
{
    for (int i = c->nwin - 1; i >= 0; i--) {
        shell_window *w = (shell_window *)&c->win[0];
        for (int k = 0; k < SHELL_MAX_WINDOWS; k++) if (c->win[k].id == c->order[i]) { w = (shell_window *)&c->win[k]; break; }
        if (!comp_is_shown(c, w)) continue;
        shell_zone z = zone_at(c, w, x, y);
        if (z != Z_NONE) { if (win_id) *win_id = w->id; return z; }
    }
    if (win_id) *win_id = 0;
    return Z_NONE;
}

/* ---------- input ---------- */
static void cursor_damage(shell_comp *c)
{
    int sc = (int)c->scale_pct;
    damage_add(&c->damage, (shell_rect){ c->mouse_x - 2, c->mouse_y - 2, CURSOR_W * sc / 100 + 4, CURSOR_H * sc / 100 + 4 });
}

static shell_rect resize_rect(const shell_comp *c, shell_zone z, shell_rect orig, int dx, int dy, const shell_window *w)
{
    shell_rect r = orig;
    bool L = z == Z_EDGE_L || z == Z_CORNER_TL || z == Z_CORNER_BL, R = z == Z_EDGE_R || z == Z_CORNER_TR || z == Z_CORNER_BR;
    bool T = z == Z_EDGE_T || z == Z_CORNER_TL || z == Z_CORNER_TR, B = z == Z_EDGE_B || z == Z_CORNER_BL || z == Z_CORNER_BR;
    if (L) { r.x = orig.x + dx; r.w = orig.w - dx; if (r.w < w->min_w) { r.x = orig.x + orig.w - w->min_w; r.w = w->min_w; } }
    if (R) { r.w = orig.w + dx; if (r.w < w->min_w) r.w = w->min_w; }
    if (T) { r.y = orig.y + dy; r.h = orig.h - dy; if (r.h < w->min_h) { r.y = orig.y + orig.h - w->min_h; r.h = w->min_h; } }
    if (B) { r.h = orig.h + dy; if (r.h < w->min_h) r.h = w->min_h; }
    (void)c;
    return r;
}

void comp_mouse_move(shell_comp *c, int x, int y)
{
    cursor_damage(c);
    if (x < 0) x = 0;
    if (y < 0) y = 0;
    if (x >= c->screen_w) x = c->screen_w - 1;
    if (y >= c->screen_h) y = c->screen_h - 1;
    c->mouse_x = x; c->mouse_y = y;
    cursor_damage(c);

    if (c->grab_win) {
        shell_window *w = comp_get(c, c->grab_win);
        if (!w) { c->grab_win = 0; return; }
        int dx = x - c->grab_dx, dy = y - c->grab_dy;
        shell_rect r = c->grab_rect;
        if (c->grab_zone == Z_TITLE) {
            if (w->flags & (WF_MAXIMIZED | WF_SNAPPED)) {          /* dragging a maximized window restores it under the cursor */
                shell_rect rs = w->restore;
                w->flags &= ~(uint32_t)(WF_MAXIMIZED | WF_SNAPPED);
                int rel = c->grab_rect.w ? (c->grab_dx - c->grab_rect.x) * 1000 / c->grab_rect.w : 500;
                r.w = rs.w; r.h = rs.h; r.x = x - rs.w * rel / 1000; r.y = y - c->m.title_h / 2;
                c->grab_rect = r; c->grab_dx = x; c->grab_dy = y;
            } else {
                r.x += dx; r.y += dy;
            }
            if (r.y < c->work_area.y) r.y = c->work_area.y;                       /* title bar never leaves the work area */
            if (r.y > c->work_area.y + c->work_area.h - c->m.title_h) r.y = c->work_area.y + c->work_area.h - c->m.title_h;
            if (r.x + r.w < 64) r.x = 64 - r.w;
            if (r.x > c->screen_w - 64) r.x = c->screen_w - 64;
        } else {
            r = resize_rect(c, c->grab_zone, c->grab_rect, dx, dy, w);
        }
        comp_set_rect(c, w->id, r);
        return;
    }
    if (c->capture) {
        shell_window *w = comp_get(c, c->capture);
        if (w) { shell_rect cr = comp_client_rect(c, w); push_event(w, (shell_event){ EV_MOUSE_MOVE, x - cr.x, y - cr.y, 0, 0, c->kbd_mods }); }
        return;
    }
    int hw; shell_zone z = comp_hit_test(c, x, y, &hw);
    if (hw != c->hover) {
        shell_window *o = comp_get(c, c->hover), *n = comp_get(c, hw);
        if (o) { push_event(o, (shell_event){ EV_LEAVE, 0, 0, 0, 0, 0 }); damage_add(&c->damage, (shell_rect){ o->rect.x, o->rect.y, o->rect.w, c->m.title_h + c->m.border }); }
        if (n) push_event(n, (shell_event){ EV_ENTER, 0, 0, 0, 0, 0 });
        c->hover = hw;
    }
    shell_window *h = comp_get(c, hw);
    if (h) {
        damage_add(&c->damage, (shell_rect){ h->rect.x, h->rect.y, h->rect.w, c->m.title_h + c->m.border });   /* button hover state */
        if (z == Z_CLIENT) { shell_rect cr = comp_client_rect(c, h); push_event(h, (shell_event){ EV_MOUSE_MOVE, x - cr.x, y - cr.y, 0, 0, c->kbd_mods }); }
    }
}

void comp_mouse_button(shell_comp *c, int button, bool down)
{
    if (down) c->buttons |= (uint32_t)button; else c->buttons &= ~(uint32_t)button;
    if (!down) {
        if (c->grab_win) {
            int gw = c->grab_win; c->grab_win = 0;
            comp_notify(c, gw);
        }
        if (c->capture) {
            shell_window *w = comp_get(c, c->capture);
            if (w) { shell_rect cr = comp_client_rect(c, w); push_event(w, (shell_event){ EV_MOUSE_UP, c->mouse_x - cr.x, c->mouse_y - cr.y, button, 0, c->kbd_mods }); }
            c->capture = 0;
        }
        if (c->press_win) {                                          /* chrome button: activate on release over the same button */
            int hw; shell_zone z = comp_hit_test(c, c->mouse_x, c->mouse_y, &hw);
            int pw = c->press_win; shell_zone pz = c->press_zone; c->press_win = 0;
            if (hw == pw && z == pz) {
                shell_window *w = comp_get(c, pw);
                if (pz == Z_CLOSE && w) push_event(w, (shell_event){ EV_CLOSE_REQUEST, 0, 0, 0, 0, 0 });
                else if (pz == Z_MAXIMIZE) comp_toggle_maximize(c, pw);
                else if (pz == Z_MINIMIZE) comp_minimize(c, pw);
            }
        }
        return;
    }
    int hw; shell_zone z = comp_hit_test(c, c->mouse_x, c->mouse_y, &hw);
    shell_window *w = comp_get(c, hw);
    if (!w) { comp_focus(c, 0); return; }
    comp_raise(c, hw);
    comp_focus(c, hw);
    switch (z) {
    case Z_CLIENT: {
        c->capture = hw;
        shell_rect cr = comp_client_rect(c, w);
        push_event(w, (shell_event){ EV_MOUSE_DOWN, c->mouse_x - cr.x, c->mouse_y - cr.y, button, 0, c->kbd_mods });
        break;
    }
    case Z_TITLE:
        if (button != BTN_LEFT) break;
        if (c->last_click_win == hw && c->now_ms - c->last_click_ms <= DBLCLICK_MS && !(w->flags & WF_NORESIZE)) {
            c->last_click_win = 0;
            comp_toggle_maximize(c, hw);
            break;
        }
        c->last_click_win = hw; c->last_click_ms = c->now_ms;
        c->grab_win = hw; c->grab_zone = Z_TITLE; c->grab_dx = c->mouse_x; c->grab_dy = c->mouse_y; c->grab_rect = w->rect;
        break;
    case Z_CLOSE: case Z_MAXIMIZE: case Z_MINIMIZE:
        c->press_win = hw; c->press_zone = z;
        break;
    default:
        if (z != Z_NONE && button == BTN_LEFT) { c->grab_win = hw; c->grab_zone = z; c->grab_dx = c->mouse_x; c->grab_dy = c->mouse_y; c->grab_rect = w->rect; }
    }
}

void comp_post_event(shell_comp *c, int id, shell_event e) { shell_window *w = comp_get(c, id); if (w) push_event(w, e); }

void comp_request_close(shell_comp *c, int id)
{
    shell_window *w = comp_get(c, id);
    if (w) push_event(w, (shell_event){ EV_CLOSE_REQUEST, 0, 0, 0, 0, 0 });
}

void comp_mouse_wheel(shell_comp *c, int delta)
{
    int hw; shell_zone z = comp_hit_test(c, c->mouse_x, c->mouse_y, &hw);
    shell_window *w = comp_get(c, hw);
    if (w && z == Z_CLIENT) { shell_rect cr = comp_client_rect(c, w); push_event(w, (shell_event){ EV_MOUSE_WHEEL, c->mouse_x - cr.x, c->mouse_y - cr.y, delta, 0, c->kbd_mods }); }
}
void comp_key(shell_comp *c, uint32_t key, bool down, uint32_t mods)
{
    c->kbd_mods = mods;
    shell_window *w = comp_get(c, c->focus);
    if (w) push_event(w, (shell_event){ down ? EV_KEY_DOWN : EV_KEY_UP, 0, 0, 0, key, mods });
}
void comp_char(shell_comp *c, uint32_t cp)
{
    shell_window *w = comp_get(c, c->focus);
    if (w) push_event(w, (shell_event){ EV_CHAR, 0, 0, 0, cp, 0 });
}

/* ---------- rendering ---------- */
static void draw_button_glyph(shell_fb *fb, shell_rect clip, shell_zone z, shell_rect b, uint32_t col, int scale)
{
    int cx = b.x + b.w / 2, cy = b.y + b.h / 2;
    int u = (scale + 50) / 100; if (u < 1) u = 1;                  /* device px per logical px for crisp strokes */
    if (z == Z_MINIMIZE) fb_fill(fb, clip, (shell_rect){ cx - 5 * u, cy + 2 * u, 10 * u, u }, col);
    else if (z == Z_MAXIMIZE) {
        shell_rect r = { cx - 5 * u, cy - 5 * u, 10 * u, 10 * u };
        fb_frame(fb, clip, r, col);
        fb_fill(fb, clip, (shell_rect){ r.x, r.y + u, r.w, u }, col);
    } else {                                                        /* close: an anti-aliased multiplication sign from the UI font */
        shell_font f = { &loos_font_regular, 20 * scale / 100 };
        int tw = shell_text_width_ex("\xc3\x97", f) ;
        fb_text_ex(fb, clip, cx - tw / 2, cy - f.size * 55 / 100, "\xc3\x97", col, f);
    }
}

/* ---- cursor: an anti-aliased arrow rendered once per scale from a polygon ---- */
#define CUR_MAX 80
static uint8_t cur_fill[CUR_MAX * CUR_MAX], cur_edge[CUR_MAX * CUR_MAX];
static int cur_scale, cur_w, cur_h, cur_ox, cur_oy;

static void cursor_prepare(int scale)
{
    if (cur_scale == scale) return;
    /* arrow outline in 1/16 px units, y down on screen (flipped for the path), tip at the origin */
    static const int16_t pts_xy[] = { 0, 0,  0, -264,  61, -205,  102, -298,  142, -280,  101, -189,  179, -189 };
    static const uint8_t on[7] = { 1, 1, 1, 1, 1, 1, 1 };
    static const uint16_t ends[1] = { 7 };
    int m = (scale * 2 + 99) / 100 + 1;                                           /* margin for the outline */
    cur_w = (14 * scale + 99) / 100 + 2 * m; cur_h = (22 * scale + 99) / 100 + 2 * m;
    if (cur_w > CUR_MAX) cur_w = CUR_MAX;
    if (cur_h > CUR_MAX) cur_h = CUR_MAX;
    cur_ox = m; cur_oy = m;
    raster_path p = { pts_xy, on, ends, 1 };
    int32_t k = (int32_t)(((int64_t)scale << 24) / (100 * 16));
    raster_fill(&p, k, m * 256, m * 256, cur_w, cur_h, cur_fill);
    int t = (scale + 50) / 100; if (t < 1) t = 1;                                  /* outline thickness in device px */
    static uint8_t tmp[CUR_MAX * CUR_MAX];
    memcpy(cur_edge, cur_fill, (size_t)cur_w * (size_t)cur_h);
    for (int pass = 0; pass < t; pass++) {                                         /* dilate by one pixel per pass */
        memcpy(tmp, cur_edge, (size_t)cur_w * (size_t)cur_h);
        for (int y = 0; y < cur_h; y++) for (int x = 0; x < cur_w; x++) {
            int best = tmp[y * cur_w + x];
            for (int dy = -1; dy <= 1; dy++) for (int dx = -1; dx <= 1; dx++) {
                int xx = x + dx, yy = y + dy;
                if (xx < 0 || yy < 0 || xx >= cur_w || yy >= cur_h) continue;
                int v = tmp[yy * cur_w + xx];
                if (v > best) best = v;
            }
            cur_edge[y * cur_w + x] = (uint8_t)best;
        }
    }
    cur_scale = scale;
}
static void draw_cursor(shell_fb *fb, shell_rect clip, int x, int y, int scale)
{
    cursor_prepare(scale);
    shell_rect box = rect_intersect((shell_rect){ x - cur_ox, y - cur_oy, cur_w, cur_h }, rect_intersect((shell_rect){ 0, 0, fb->w, fb->h }, clip));
    for (int py = box.y; py < box.y + box.h; py++)
        for (int px = box.x; px < box.x + box.w; px++) {
            int i = (py - (y - cur_oy)) * cur_w + (px - (x - cur_ox));
            uint32_t *d = &fb->px[(size_t)py * (size_t)fb->stride + px];
            if (cur_edge[i]) *d = color_blend(*d, (uint32_t)cur_edge[i] << 24 | 0x000000u);
            if (cur_fill[i]) *d = color_blend(*d, (uint32_t)cur_fill[i] << 24 | 0xFFFFFFu);
        }
}

/* ---- wallpaper: smooth dithered gradient with two soft glows, derived from the theme ---- */
static const uint8_t bayer4[16] = { 0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5 };
static void paint_wallpaper(const shell_comp *c, shell_fb *fb, shell_rect clip)
{
    const shell_theme *t = &c->theme;
    shell_rect v = rect_intersect(clip, (shell_rect){ 0, 0, c->screen_w, c->screen_h });
    if (rect_empty(v)) return;
    if (c->high_contrast) { fb_fill(fb, v, v, t->desktop); return; }
    uint32_t d = t->desktop;
    int lum = (int)(((d >> 16) & 255) + ((d >> 8) & 255) + (d & 255));
    bool light = lum > 384;
    uint32_t top = light ? color_mix(d, 0xFFFFFFFFu, 150) : color_mix(d, t->accent, 30);
    uint32_t bot = light ? color_mix(d, t->accent, 26) : color_mix(d, 0xFF000000u, 70);
    uint32_t glow_a = t->accent, glow_b = light ? 0xFFFFFFFFu : 0xFF7C3AEDu;       /* accent and a violet counter-glow */
    int W = c->screen_w, H = c->screen_h;
    int gx1 = W * 78 / 100, gy1 = H * 18 / 100, gr1 = W * 55 / 100;
    int gx2 = W * 14 / 100, gy2 = H * 92 / 100, gr2 = W * 45 / 100;
    int g1 = (light ? 70 : 100) * c->style.glow_pct / 100, g2 = (light ? 40 : 60) * c->style.glow_pct / 100;   /* peak glow strength, 0..256 */
    if (g1 == 0 && g2 == 0) {                                                     /* no glows: one fill per row, no per-pixel math */
        for (int y = v.y; y < v.y + v.h; y++) fb_fill(fb, v, (shell_rect){ v.x, y, v.w, 1 }, color_mix(top, bot, H > 1 ? y * 256 / (H - 1) : 0));
        return;
    }
    int64_t inv1 = (256ll << 24) / ((int64_t)gr1 * gr1), inv2 = (256ll << 24) / ((int64_t)gr2 * gr2);   /* multiply instead of dividing per pixel */
    int ar = (int)((glow_a >> 16) & 255), ag = (int)((glow_a >> 8) & 255), ab = (int)(glow_a & 255);
    int br2 = (int)((glow_b >> 16) & 255), bg2 = (int)((glow_b >> 8) & 255), bb2 = (int)(glow_b & 255);
    for (int y = v.y; y < v.y + v.h; y++) {
        uint32_t base = color_mix(top, bot, H > 1 ? y * 256 / (H - 1) : 0);
        int br = (base >> 16) & 255, bg = (base >> 8) & 255, bb = base & 255;
        int64_t dy1 = y - gy1, dy2 = y - gy2;
        int64_t d1y = dy1 * dy1, d2y = dy2 * dy2;
        uint32_t *row = fb->px + (size_t)y * (size_t)fb->stride;
        for (int x = v.x; x < v.x + v.w; x++) {
            int64_t dx1 = x - gx1, dx2 = x - gx2;
            int64_t q1 = ((dx1 * dx1 + d1y) * inv1) >> 24, q2 = ((dx2 * dx2 + d2y) * inv2) >> 24;
            int a1 = q1 >= 256 ? 0 : (int)((256 - q1) * (256 - q1) * g1 >> 16);
            int a2 = q2 >= 256 ? 0 : (int)((256 - q2) * (256 - q2) * g2 >> 16);
            /* mix in 8.8 fixed point, then dither with a 4x4 ordered matrix */
            int r = br * 256 + (ar - br) * a1 + (br2 - br) * a2;
            int g = bg * 256 + (ag - bg) * a1 + (bg2 - bg) * a2;
            int b = bb * 256 + (ab - bb) * a1 + (bb2 - bb) * a2;
            int th = bayer4[(y & 3) * 4 + (x & 3)] * 16 + 8;
            r = (r + th) >> 8; g = (g + th) >> 8; b = (b + th) >> 8;
            row[x] = 0xFF000000u | (uint32_t)(r < 0 ? 0 : r > 255 ? 255 : r) << 16 | (uint32_t)(g < 0 ? 0 : g > 255 ? 255 : g) << 8 | (uint32_t)(b < 0 ? 0 : b > 255 ? 255 : b);
        }
    }
}

/* client surface blitted under the window's rounded bottom corners (coverage-blended in the corner squares) */
static void blit_client(shell_fb *fb, shell_rect clip, shell_rect cr, const shell_window *w, shell_rect outer, int rad)
{
    shell_rect src = { 0, 0, w->client_w, w->client_h };
    if (rad > 0) fb_blit_rounded(fb, clip, cr.x, cr.y, w->client, w->client_w, w->client_h, w->client_stride, src, outer, rad);
    else fb_blit(fb, clip, cr.x, cr.y, w->client, w->client_w, w->client_h, w->client_stride, src);
}

static void draw_window(shell_comp *c, shell_fb *fb, shell_rect clip, const shell_window *w, shell_zone hover_zone)
{
    const shell_theme *t = &c->theme;
    int sc = (int)c->scale_pct;
    bool focused = c->focus == w->id;
    if (w->flags & WF_NOCHROME) {
        shell_rect cr = w->rect;
        if (w->client) { shell_rect src = { 0, 0, w->client_w, w->client_h }; fb_blit(fb, clip, cr.x, cr.y, w->client, w->client_w, w->client_h, w->client_stride, src); }
        else fb_fill(fb, clip, cr, t->surface);
        return;
    }
    bool max = (w->flags & WF_MAXIMIZED) != 0;
    int rad = max ? 0 : c->m.corner_r;
    if (!max) {
        int sp = c->style.shadow_pct;
        if (sp > 0) fb_shadow(fb, clip, w->rect, rad, SHADOW_BLUR * sp / 100 * sc / 100, SHADOW_DY * sc / 100, ((uint32_t)((focused ? 0x70 : 0x48) * sp / 100 > 255 ? 255 : (focused ? 0x70 : 0x48) * sp / 100)) << 24);
    }
    int b = c->m.border;
    shell_rect tb = { w->rect.x + b, w->rect.y + b, w->rect.w - 2 * b, c->m.title_h };
    uint32_t tcol = focused ? color_mix(t->surface_hi, t->accent, 46) : t->title_inactive;
    fb_fill_round(fb, clip, w->rect, rad, t->surface);                                       /* body */
    shell_rect cr = comp_client_rect(c, w);
    fb_fill_round(fb, clip, (shell_rect){ tb.x, tb.y, tb.w, tb.h + rad }, rad > b ? rad - b : 0, tcol);
    fb_fill(fb, clip, cr, t->surface);                                                       /* square off the client area */
    fb_fill(fb, clip, (shell_rect){ tb.x, tb.y + tb.h - 1, tb.w, 1 }, color_mix(tcol, t->border, 120));   /* hairline under the title */
    /* title text, ellipsized to the space left of the buttons */
    int avail = tb.w - 3 * c->m.button_w - 20 * sc / 100;
    shell_font tf = { &loos_font_semibold, 13 * sc / 100 };
    char buf[SHELL_TITLE_MAX];
    strncpy(buf, w->title, sizeof buf - 1); buf[sizeof buf - 1] = 0;
    if (shell_text_width_ex(buf, tf) > avail) {                                              /* ellipsize at the title font's metrics */
        int room = avail - shell_text_width_ex("\xe2\x80\xa6", tf);
        int n = 0; int acc = 0; const char *p = w->title;
        for (;;) { const char *q = p; uint32_t cp = font_next_cp(&q); if (!cp) break; acc += font_advance_q8(tf, cp) >> 8; if (acc > room) break; p = q; }
        n = (int)(p - w->title);
        if (n > (int)sizeof buf - 4) n = (int)sizeof buf - 4;
        memcpy(buf, w->title, (size_t)n); memcpy(buf + n, "\xe2\x80\xa6", 4);
    }
    fb_text_ex(fb, clip, tb.x + 12 * sc / 100, tb.y + (tb.h - tf.size) / 2 - sc / 100, buf, focused ? t->text : t->text_dim, tf);
    /* caption buttons */
    static const shell_zone zs[3] = { Z_MINIMIZE, Z_MAXIMIZE, Z_CLOSE };
    for (int i = 0; i < 3; i++) {
        if (zs[i] == Z_MAXIMIZE && (w->flags & WF_NORESIZE)) continue;
        int slot = zs[i] == Z_CLOSE ? 0 : zs[i] == Z_MAXIMIZE ? 1 : 2;
        shell_rect br = { w->rect.x + w->rect.w - b - (slot + 1) * c->m.button_w, tb.y, c->m.button_w, tb.h - 1 };
        if (hover_zone == zs[i]) {
            if (zs[i] == Z_CLOSE && rad > 0) fb_fill_round(fb, clip, (shell_rect){ br.x, br.y, br.w, br.h + rad }, rad > b ? rad - b : 0, t->danger), fb_fill(fb, clip, (shell_rect){ br.x, br.y + br.h - 1, br.w, rad + 1 }, t->surface);
            else fb_fill(fb, clip, br, zs[i] == Z_CLOSE ? t->danger : color_mix(tcol, t->text, 40));
        }
        draw_button_glyph(fb, clip, zs[i], br, hover_zone == zs[i] && zs[i] == Z_CLOSE ? 0xFFFFFFFFu : t->text, sc);
    }
    if (w->client) blit_client(fb, clip, cr, w, w->rect, rad);
    fb_stroke_round(fb, clip, w->rect, rad, b, focused ? t->accent : t->border);          /* crisp AA frame over everything */
}

/* True when `w` paints every pixel of `clip` opaquely: its body is an opaque fill, so only the rounded corners
 * (and NOCHROME windows, whose client may carry alpha) can let what is below show through. */
static bool window_hides(const shell_comp *c, const shell_window *w, shell_rect clip)
{
    if (w->flags & WF_NOCHROME) return false;
    int rad = (w->flags & WF_MAXIMIZED) ? 0 : c->m.corner_r;
    shell_rect in = { w->rect.x + rad, w->rect.y + rad, w->rect.w - 2 * rad, w->rect.h - 2 * rad };
    return !rect_empty(in) && !rect_empty(clip) && clip.x >= in.x && clip.y >= in.y && clip.x + clip.w <= in.x + in.w && clip.y + clip.h <= in.y + in.h;
}

static void paint(shell_comp *c, shell_fb *fb, shell_rect clip)
{
    /* Occlusion culling: start at the topmost window that fully hides `clip`; everything beneath it (including the
     * costly wallpaper gradient) cannot show. A maximized window turns every repaint inside it into a single draw. */
    int start = 0;
    if (!c->no_cull) {
        for (int i = c->nwin - 1; i >= 0; i--) {
            shell_window *w = comp_get(c, c->order[i]);
            if (w && comp_is_shown(c, w) && window_hides(c, w, clip)) { start = i; break; }
        }
    }
    if (start > 0) c->stat_culled++; else paint_wallpaper(c, fb, clip);
    int hz_win = 0; shell_zone hz = comp_hit_test(c, c->mouse_x, c->mouse_y, &hz_win);
    for (int i = start; i < c->nwin; i++) {
        shell_window *w = comp_get(c, c->order[i]);
        if (!w || !comp_is_shown(c, w)) continue;
        if (!rect_overlaps(outer_with_shadow(c, w->rect), clip)) continue;
        draw_window(c, fb, clip, w, w->id == hz_win ? hz : Z_NONE);
    }
    for (int i = 0; i < 4; i++) if (c->overlay[i].fn) c->overlay[i].fn(c, fb, clip, c->overlay[i].user);
    draw_cursor(fb, clip, c->mouse_x, c->mouse_y, (int)c->scale_pct);
}

void comp_render(shell_comp *c, shell_fb *fb)
{
    shell_damage d = c->damage;
    damage_clear(&c->damage);
    for (int i = 0; i < d.n; i++) paint(c, fb, d.r[i]);
}
void comp_render_full(shell_comp *c, shell_fb *fb)
{
    damage_clear(&c->damage);
    paint(c, fb, (shell_rect){ 0, 0, c->screen_w, c->screen_h });
}
