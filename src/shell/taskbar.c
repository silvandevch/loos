#include "shell.h"
#include <string.h>

#define START_W 60
#define TRAY_W 150
#define LED_AMBER 0xFFF59E0Bu
#define LED_RED 0xFFEF4444u
#define LED_GREY 0xFF4B5563u

static void overlay(shell_comp *c, shell_fb *fb, shell_rect clip, void *u)
{
    shell_taskbar *t = u;
    shell_fb v = comp_view(c, fb);
    shell_rect lc = comp_clip_logical(c, clip);
    taskbar_draw(t, c, &v, lc);
    if (t->launcher) launcher_draw(t->launcher, c, &v, lc);
}

void taskbar_init(shell_taskbar *t, shell_comp *c, shell_launcher *l)
{
    memset(t, 0, sizeof *t);
    t->comp = c; t->launcher = l; t->hover_btn = -1;
    comp_add_overlay(c, overlay, t);
    c->bottom_reserve = TASKBAR_H;
    comp_set_work_area(c, (shell_rect){ 0, 0, c->screen_w, c->screen_h - TASKBAR_H * (int)c->scale_pct / 100 });
}

shell_rect taskbar_rect(const shell_comp *c) { return (shell_rect){ 0, comp_lh(c) - TASKBAR_H, comp_lw(c), TASKBAR_H }; }     /* logical px */
static void dirty(shell_taskbar *t) { comp_damage_logical(t->comp, taskbar_rect(t->comp)); }

void taskbar_set_time(shell_taskbar *t, int h, int m) { if (t->hour != h || t->minute != m) { t->hour = h; t->minute = m; dirty(t); } }
void taskbar_set_net(shell_taskbar *t, int sockets) { if (t->net_sockets != sockets) { t->net_sockets = sockets; dirty(t); } }
shell_net_state taskbar_net_state(const shell_taskbar *t)
{
    if (t->net_hard_off) return NET_BLOCKED;
    return t->net_sockets > 0 ? NET_ACTIVE : NET_IDLE;
}

/* window buttons: every taskbar-eligible window on the current desktop, in creation order */
static int collect(const shell_comp *c, int *ids, int max)
{
    int n = 0;
    for (int i = 0; i < SHELL_MAX_WINDOWS; i++) {
        const shell_window *w = &c->win[i];
        if (!w->id || !(w->flags & WF_VISIBLE) || (w->flags & WF_NOTASKBAR)) continue;
        if (!(w->flags & WF_STICKY) && w->desktop != c->cur_desktop) continue;
        int pos = n < max ? n : max - 1;
        if (n < max) n++;
        while (pos > 0 && ids[pos - 1] > w->id) { ids[pos] = ids[pos - 1]; pos--; }
        ids[pos] = w->id;
    }
    return n;
}
int taskbar_button_count(const shell_taskbar *t, const shell_comp *c) { int ids[SHELL_MAX_WINDOWS]; (void)t; return collect(c, ids, SHELL_MAX_WINDOWS); }
int taskbar_button_window(const shell_taskbar *t, const shell_comp *c, int index)
{
    int ids[SHELL_MAX_WINDOWS]; (void)t;
    int n = collect(c, ids, SHELL_MAX_WINDOWS);
    return index >= 0 && index < n ? ids[index] : 0;
}
shell_rect taskbar_button_rect(const shell_taskbar *t, const shell_comp *c, int index)
{
    int ids[SHELL_MAX_WINDOWS]; (void)t;
    int n = collect(c, ids, SHELL_MAX_WINDOWS);
    shell_rect bar = taskbar_rect(c);
    int x0 = 8 + START_W + 8, avail = comp_lw(c) - TRAY_W - 8 - x0;
    if (n < 1) n = 1;
    int w = avail / n;
    if (w > 180) w = 180;
    if (w < 40) w = 40;
    return (shell_rect){ x0 + index * (w + 4), bar.y + 4, w, TASKBAR_H - 8 };
}
static shell_rect start_rect(const shell_comp *c) { shell_rect b = taskbar_rect(c); return (shell_rect){ 8, b.y + 4, START_W, TASKBAR_H - 8 }; }
static shell_rect net_toggle_rect(const shell_comp *c) { shell_rect b = taskbar_rect(c); return (shell_rect){ comp_lw(c) - TRAY_W + 24, b.y + 6, 40, TASKBAR_H - 12 }; }
static shell_rect led_rect(const shell_comp *c) { shell_rect b = taskbar_rect(c); return (shell_rect){ comp_lw(c) - TRAY_W + 6, b.y + TASKBAR_H / 2 - 5, 10, 10 }; }

static int hit_button(const shell_taskbar *t, const shell_comp *c, int x, int y)
{
    if (rect_contains(start_rect(c), x, y)) return 0;
    if (rect_contains(net_toggle_rect(c), x, y)) return -2;
    if (rect_contains(led_rect(c), x, y)) return -3;
    int n = taskbar_button_count(t, c);
    for (int i = 0; i < n; i++) if (rect_contains(taskbar_button_rect(t, c, i), x, y)) return 1 + i;
    return -1;
}

static void activate_window(shell_comp *c, int id)
{
    shell_window *w = comp_get(c, id);
    if (!w) return;
    if (w->flags & WF_MINIMIZED) comp_restore(c, id);
    else if (c->focus == id) comp_minimize(c, id);
    else { comp_raise(c, id); comp_focus(c, id); }
}

bool taskbar_mouse(shell_taskbar *t, shell_comp *c, int x, int y, int button, bool down)
{
    x = comp_logical(c, x); y = comp_logical(c, y);                          /* device px in, logical px inside */
    bool in_bar = rect_contains(taskbar_rect(c), x, y);
    if (t->launcher && t->launcher->open) {
        bool in_start = rect_contains(start_rect(c), x, y);
        if (down && !in_start && launcher_mouse(t->launcher, c, x, y, true)) { comp_damage_all(c); return true; }
        if (down && !in_start && !rect_contains(launcher_rect(c), x, y)) { launcher_close(t->launcher); comp_damage_all(c); }
        if (!down && rect_contains(launcher_rect(c), x, y)) return true;
    }
    int b = in_bar ? hit_button(t, c, x, y) : -1;
    if (b != t->hover_btn) { t->hover_btn = b; dirty(t); }
    if (!in_bar) return false;
    if (!down || button != BTN_LEFT) return true;
    if (b == 0 && t->launcher) {
        if (t->launcher->open) launcher_close(t->launcher); else launcher_open(t->launcher);
        comp_damage_all(c);
    } else if (b == -2) {
        t->net_hard_off = !t->net_hard_off;
        dirty(t);
        if (t->on_net_toggle) t->on_net_toggle(t->net_hard_off, t->user);
    } else if (b >= 1) {
        activate_window(c, taskbar_button_window(t, c, b - 1));
    }
    return true;
}

bool taskbar_key(shell_taskbar *t, shell_comp *c, uint32_t key, bool down, uint32_t mods)
{
    if (!down) return false;
    if (t->launcher && t->launcher->open) { bool r = launcher_key(t->launcher, key, mods); comp_damage_all(c); return r; }
    if (key == KEY_SUPER || (key == KEY_SPACE && (mods & MOD_CTRL))) {            /* Super or Ctrl+Space: launcher / command palette */
        if (t->launcher) { launcher_open(t->launcher); comp_damage_all(c); }
        return true;
    }
    if (key == KEY_TAB && (mods & MOD_ALT)) {
        int ids[SHELL_MAX_WINDOWS], n = collect(c, ids, SHELL_MAX_WINDOWS);
        if (!n) return true;
        int cur = -1;
        for (int i = 0; i < n; i++) if (ids[i] == c->focus) cur = i;
        int step = (mods & MOD_SHIFT) ? n - 1 : 1;
        int next = ids[(cur + step + n) % n];
        if (cur < 0) next = ids[0];
        shell_window *w = comp_get(c, next);
        if (w && (w->flags & WF_MINIMIZED)) comp_restore(c, next); else { comp_raise(c, next); comp_focus(c, next); }
        return true;
    }
    if (key == KEY_F4 && (mods & MOD_ALT)) { if (c->focus) comp_request_close(c, c->focus); return true; }
    return false;
}

static void ellipsize(char *out, size_t cap, const char *s, int max_px)
{
    if (max_px < 24) { out[0] = 0; return; }
    shell_ellipsize(out, cap, s, max_px);
}

void taskbar_draw(const shell_taskbar *t, const shell_comp *c, shell_fb *fb, shell_rect clip)
{
    const shell_theme *th = &c->theme;
    shell_rect bar = taskbar_rect(c);
    if (!rect_overlaps(bar, clip)) return;
    fb_fill(fb, clip, bar, th->surface);
    fb_hline(fb, clip, bar.x, bar.y, bar.w, th->border);
    /* start button */
    shell_rect sb = start_rect(c);
    bool open = t->launcher && t->launcher->open;
    fb_fill_round(fb, clip, sb, 6, open || t->hover_btn == 0 ? color_mix(th->accent, 0xFFFFFFFFu, 40) : th->accent);
    fb_text(fb, clip, sb.x + (sb.w - 32) / 2, sb.y + (sb.h - 16) / 2, "LoOS", th->accent_text);
    /* window buttons */
    int n = taskbar_button_count(t, c);
    for (int i = 0; i < n; i++) {
        const shell_window *w = comp_get((shell_comp *)c, taskbar_button_window(t, c, i));
        if (!w) continue;
        shell_rect r = taskbar_button_rect(t, c, i);
        bool focused = c->focus == w->id && !(w->flags & WF_MINIMIZED), hov = t->hover_btn == 1 + i;
        uint32_t bg = focused ? color_mix(th->surface_hi, th->accent, 60) : hov ? th->surface_hi : th->surface;
        fb_fill_round(fb, clip, r, 6, bg);
        char label[SHELL_TITLE_MAX];
        ellipsize(label, sizeof label, w->title, r.w - 20);
        fb_text(fb, clip, r.x + 10, r.y + (r.h - 16) / 2, label, (w->flags & WF_MINIMIZED) ? th->text_dim : th->text);
        if (!(w->flags & WF_MINIMIZED)) fb_fill(fb, clip, (shell_rect){ r.x + r.w / 2 - (focused ? 12 : 4), r.y + r.h - 3, focused ? 24 : 8, 2 }, focused ? th->accent : th->text_dim);
    }
    /* tray: network LED, hard switch, clock */
    shell_net_state ns = taskbar_net_state(t);
    uint32_t led = ns == NET_BLOCKED ? LED_RED : ns == NET_ACTIVE ? LED_AMBER : LED_GREY;
    shell_rect lr = led_rect(c);
    if (ns == NET_ACTIVE) fb_fill_round(fb, clip, (shell_rect){ lr.x - 2, lr.y - 2, lr.w + 4, lr.h + 4 }, 7, (led & 0x00FFFFFFu) | 0x50000000u);   /* glow */
    fb_fill_round(fb, clip, lr, 5, led);
    shell_rect tg = net_toggle_rect(c);
    bool off = t->net_hard_off;
    fb_fill_round(fb, clip, tg, 5, off ? th->danger : (t->hover_btn == -2 ? th->surface_hi : th->surface));
    fb_frame(fb, clip, tg, off ? th->danger : th->border);
    fb_text(fb, clip, tg.x + (tg.w - 24) / 2, tg.y + (tg.h - 16) / 2, off ? "OFF" : "NET", off ? 0xFFFFFFFFu : th->text_dim);
    char clock[8] = { (char)('0' + t->hour / 10 % 10), (char)('0' + t->hour % 10), ':', (char)('0' + t->minute / 10 % 10), (char)('0' + t->minute % 10), 0 };
    fb_text(fb, clip, comp_lw(c) - 8 - 40 - 6, bar.y + (TASKBAR_H - 16) / 2, clock, th->text);
}
