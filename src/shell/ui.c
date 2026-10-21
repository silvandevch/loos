#include "ui.h"
#include <string.h>
#include <stdio.h>

uint32_t ui_id(const char *s)
{
    uint32_t h = 2166136261u;
    for (; *s; s++) { h ^= (unsigned char)*s; h *= 16777619u; }
    return h ? h : 1;
}

static void a11y_reg(ui_ctx *u, uint32_t id, ui_role role, const char *name, const char *value, shell_rect r, bool checked, bool focusable)
{
    if (u->na11y < UI_A11Y_MAX) {
        ui_a11y_item *it = &u->a11y[u->na11y++];
        it->role = role; it->id = id; it->rect = r; it->checked = checked; it->focused = id && u->focus == id;
        strncpy(it->name, name ? name : "", sizeof it->name - 1); it->name[sizeof it->name - 1] = 0;
        strncpy(it->value, value ? value : "", sizeof it->value - 1); it->value[sizeof it->value - 1] = 0;
    }
    if (focusable && u->norder < UI_A11Y_MAX) u->order[u->norder++] = id;
}
int ui_a11y_count(const ui_ctx *u) { return u->na11y; }
const ui_a11y_item *ui_a11y_get(const ui_ctx *u, int i) { return i >= 0 && i < u->na11y ? &u->a11y[i] : NULL; }
size_t ui_a11y_dump(const ui_ctx *u, char *out, size_t cap)
{
    static const char *const role[] = { "label", "button", "toggle", "radio", "text", "list", "tab", "slider", "progress" };
    size_t n = 0;
    for (int i = 0; i < u->na11y; i++) {
        const ui_a11y_item *it = &u->a11y[i];
        const char *state = it->role == A11Y_TOGGLE || it->role == A11Y_RADIO ? (it->checked ? " [on]" : " [off]") : "";
        int k = snprintf(out + n, n < cap ? cap - n : 0, "%s: %s%s%s%s%s\n", role[it->role], it->name, state,
                         it->value[0] ? " = " : "", it->value, it->focused ? " (focused)" : "");
        if (k > 0) n += (size_t)k;
    }
    if (cap) out[n < cap ? n : cap - 1] = 0;
    return n;
}
void ui_focus_next(ui_ctx *u, bool back)
{
    if (!u->norder) return;
    int cur = -1;
    for (int i = 0; i < u->norder; i++) if (u->order[i] == u->focus) cur = i;
    int next = cur < 0 ? (back ? u->norder - 1 : 0) : (cur + (back ? u->norder - 1 : 1)) % u->norder;
    u->focus = u->order[next];
}

void ui_begin(ui_ctx *u, shell_fb *fb, const shell_theme *t, int scale)
{
    u->na11y = 0; u->norder = 0;
    u->fb = fb; u->theme = t; u->scale = 100;           /* layout is in logical px; the framebuffer's own scale maps it to the device */
    if (scale > 0 && scale != 100 && fb->scale == 0) fb->scale = scale;     /* legacy callers that only pass a scale */
    u->clip = (shell_rect){ 0, 0, fb_lw(fb), fb_lh(fb) };
    u->hot = u->next_hot; u->next_hot = 0;
    u->frame++;
}
void ui_end(ui_ctx *u)
{
    for (int i = 0; i < u->nkeys; i++)                              /* Tab / Shift+Tab: move keyboard focus through the registered widgets */
        if (!u->keys[i].is_char && u->keys[i].key == KEY_TAB) { ui_focus_next(u, (u->keys[i].mods & MOD_SHIFT) != 0); break; }
    u->nkeys = 0;                                                  /* keys unread by a focused widget are dropped */
    if (u->released) u->active = 0;
    u->pressed = u->released = false; u->wheel = 0; u->consumed_click = false;     /* edges live for exactly one frame */
}

void ui_feed(ui_ctx *u, const shell_event *e)
{
    switch (e->type) {
    case EV_MOUSE_MOVE: u->mx = e->x; u->my = e->y; u->mods = e->mods; break;
    case EV_MOUSE_DOWN: u->mx = e->x; u->my = e->y; u->mods = e->mods; if (e->button == BTN_LEFT) { u->down = true; u->pressed = true; } break;
    case EV_MOUSE_UP: u->mx = e->x; u->my = e->y; if (e->button == BTN_LEFT) { u->down = false; u->released = true; } break;
    case EV_MOUSE_WHEEL: u->wheel += e->button; break;
    case EV_KEY_DOWN: case EV_CHAR:
        u->mods = e->mods;
        if (u->nkeys < UI_KEYQ) { u->keys[u->nkeys].key = e->key; u->keys[u->nkeys].mods = e->mods; u->keys[u->nkeys].is_char = e->type == EV_CHAR; u->nkeys++; }
        break;
    case EV_BLUR: u->down = false; u->active = 0; break;
    default: break;
    }
}

bool ui_key(ui_ctx *u, uint32_t id_or_zero, bool want_char, uint32_t *mods)
{
    (void)id_or_zero;
    for (int i = 0; i < u->nkeys; i++) {
        if (u->keys[i].is_char != want_char) continue;
        uint32_t k = u->keys[i].key;
        if (mods) *mods = u->keys[i].mods;
        memmove(&u->keys[i], &u->keys[i + 1], (size_t)(u->nkeys - i - 1) * sizeof u->keys[0]);
        u->nkeys--;
        (void)k;
        return true;
    }
    return false;
}
static bool pop_key(ui_ctx *u, bool want_char, uint32_t *key, uint32_t *mods)
{
    for (int i = 0; i < u->nkeys; i++) {
        if (u->keys[i].is_char != want_char) continue;
        if (!want_char && u->keys[i].key == KEY_TAB) continue;            /* Tab belongs to focus traversal */
        *key = u->keys[i].key; *mods = u->keys[i].mods;
        memmove(&u->keys[i], &u->keys[i + 1], (size_t)(u->nkeys - i - 1) * sizeof u->keys[0]);
        u->nkeys--;
        return true;
    }
    return false;
}

bool ui_take_key(ui_ctx *u, uint32_t key, uint32_t need_mods)
{
    for (int i = 0; i < u->nkeys; i++) {
        uint32_t k = u->keys[i].key;
        if (u->keys[i].is_char) continue;
        if (k >= 'A' && k <= 'Z') k += 32;
        if (k == key && (u->keys[i].mods & need_mods) == need_mods) {
            memmove(&u->keys[i], &u->keys[i + 1], (size_t)(u->nkeys - i - 1) * sizeof u->keys[0]);
            u->nkeys--;
            return true;
        }
    }
    return false;
}

bool ui_hovered(const ui_ctx *u, shell_rect r) { return rect_contains(r, u->mx, u->my) && rect_contains(u->clip, u->mx, u->my); }
void ui_clip_push(ui_ctx *u, shell_rect r, shell_rect *saved) { *saved = u->clip; u->clip = rect_intersect(u->clip, r); }
void ui_clip_pop(ui_ctx *u, shell_rect saved) { u->clip = saved; }

void ui_clear(ui_ctx *u, uint32_t c) { fb_fill(u->fb, u->clip, (shell_rect){ 0, 0, fb_lw(u->fb), fb_lh(u->fb) }, c); }
/* the shared full-screen backdrop of the installer and the sign-in screen: dark gradient with two soft glows */
void ui_backdrop(ui_ctx *u, int w, int h)
{
    shell_rect all = { 0, 0, w, h };
    fb_gradient_v(u->fb, u->clip, all, 0xFF0D1320u, 0xFF070A12u);
    fb_radial_glow(u->fb, u->clip, w * 80 / 100, h * 12 / 100, w * 60 / 100, 0xFF3B82F6u, 56);
    fb_radial_glow(u->fb, u->clip, w * 10 / 100, h * 98 / 100, w * 48 / 100, 0xFF7C3AEDu, 40);
}
void ui_panel(ui_ctx *u, shell_rect r, uint32_t c) { fb_fill_round(u->fb, u->clip, r, 6, c); }
void ui_label(ui_ctx *u, int x, int y, const char *s, uint32_t c)
{
    a11y_reg(u, 0, A11Y_LABEL, s, NULL, (shell_rect){ x, y, shell_text_width(s), 16 }, false, false);
    fb_text(u->fb, u->clip, x, y, s, c);
}
void ui_label_right(ui_ctx *u, int rx, int y, const char *s, uint32_t c) { fb_text(u->fb, u->clip, rx - shell_text_width(s), y, s, c); }
void ui_separator(ui_ctx *u, int x, int y, int w) { fb_hline(u->fb, u->clip, x, y, w, u->theme->border); }

void ui_heading(ui_ctx *u, int x, int y, const char *s)
{
    fb_text_ex(u->fb, u->clip, x, y, s, u->theme->text, (shell_font){ &loos_font_semibold, 26 });
}

static int pad(const ui_ctx *u, int v) { return v * u->scale / 100; }

/* common press/click logic: returns true on a completed click */
static bool click_logic(ui_ctx *u, uint32_t id, shell_rect r)
{
    bool over = ui_hovered(u, r);
    if (over) u->next_hot = id;
    bool clicked = false;
    if (u->pressed && over && !u->consumed_click) { u->active = id; u->focus = id; u->consumed_click = true; }
    if (u->released && u->active == id) { clicked = over; u->active = 0; }
    return clicked;
}

static bool kbd_activate(ui_ctx *u, uint32_t id)
{
    if (u->focus != id) return false;
    return ui_take_key(u, KEY_ENTER, 0) || ui_take_key(u, KEY_SPACE, 0);
}
static void focus_ring(ui_ctx *u, shell_rect r)
{
    fb_frame(u->fb, u->clip, r, u->theme->accent);
    if (u->high_contrast) fb_frame(u->fb, u->clip, (shell_rect){ r.x - 1, r.y - 1, r.w + 2, r.h + 2 }, u->theme->accent);
}

static bool button_impl(ui_ctx *u, shell_rect r, const char *label, bool accent)
{
    uint32_t id = ui_id(label) ^ (uint32_t)(r.x * 31 + r.y * 17);
    bool clicked = click_logic(u, id, r);
    a11y_reg(u, id, A11Y_BUTTON, label, NULL, r, false, true);
    clicked |= kbd_activate(u, id);
    const shell_theme *t = u->theme;
    bool hot = u->hot == id || u->next_hot == id, act = u->active == id;
    uint32_t bg = accent ? (act ? color_mix(t->accent, 0xFF000000u, 60) : hot ? color_mix(t->accent, 0xFFFFFFFFu, 40) : t->accent)
                         : (act ? color_mix(t->surface_hi, t->accent, 90) : hot ? color_mix(t->surface_hi, t->accent, 40) : t->surface_hi);
    fb_fill_round(u->fb, u->clip, r, 6, bg);
    if (u->focus == id) focus_ring(u, r);
    int tw = shell_text_width(label);
    fb_text(u->fb, u->clip, r.x + (r.w - tw) / 2, r.y + (r.h - 16) / 2, label, accent ? t->accent_text : t->text);
    return clicked;
}
bool ui_button(ui_ctx *u, shell_rect r, const char *label) { return button_impl(u, r, label, false); }
bool ui_button_accent(ui_ctx *u, shell_rect r, const char *label) { return button_impl(u, r, label, true); }

bool ui_toggle(ui_ctx *u, shell_rect r, const char *label, bool *value)
{
    uint32_t id = ui_id(label) ^ (uint32_t)(r.x * 31 + r.y * 17);
    bool clicked = click_logic(u, id, r);
    a11y_reg(u, id, A11Y_TOGGLE, label, NULL, r, *value, true);
    clicked |= kbd_activate(u, id);
    const shell_theme *t = u->theme;
    int tw = pad(u, 36), th = pad(u, 20);
    shell_rect sw = { r.x + r.w - tw, r.y + (r.h - th) / 2, tw, th };
    if (clicked) *value = !*value;
    bool on = *value;
    fb_text(u->fb, u->clip, r.x, r.y + (r.h - 16) / 2, label, t->text);
    fb_fill_round(u->fb, u->clip, sw, th / 2, on ? t->accent : t->border);
    int kd = th - 6;
    shell_rect knob = { on ? sw.x + sw.w - kd - 3 : sw.x + 3, sw.y + 3, kd, kd };
    fb_fill_round(u->fb, u->clip, knob, kd / 2, 0xFFFFFFFFu);
    if (u->focus == id) focus_ring(u, (shell_rect){ sw.x - 2, sw.y - 2, sw.w + 4, sw.h + 4 });
    return clicked;
}

bool ui_radio(ui_ctx *u, shell_rect r, const char *label, int *group, int value)
{
    uint32_t id = ui_id(label) ^ (uint32_t)(r.x * 31 + r.y * 17);
    bool clicked = click_logic(u, id, r);
    a11y_reg(u, id, A11Y_RADIO, label, NULL, r, *group == value, true);
    clicked |= kbd_activate(u, id);
    const shell_theme *t = u->theme;
    if (clicked) *group = value;
    shell_rect dot = { r.x, r.y + (r.h - 16) / 2, 16, 16 };
    fb_fill_round(u->fb, u->clip, dot, 8, t->border);
    fb_fill_round(u->fb, u->clip, (shell_rect){ dot.x + 2, dot.y + 2, 12, 12 }, 6, t->surface);
    if (*group == value) fb_fill_round(u->fb, u->clip, (shell_rect){ dot.x + 4, dot.y + 4, 8, 8 }, 4, t->accent);
    if (u->focus == id) focus_ring(u, (shell_rect){ r.x - 2, r.y, 20, r.h });
    fb_text(u->fb, u->clip, r.x + 24, r.y + (r.h - 16) / 2, label, t->text);
    return clicked;
}

uint32_t ui_textfield_id(const char *placeholder, shell_rect r) { return ui_id(placeholder ? placeholder : "textfield") ^ (uint32_t)(r.x * 31 + r.y * 17); }

/* width in px of buf[from, to) */
static int prefix_w(const char *buf, int from, int to)
{
    char tmp[256];
    int n = to - from;
    if (n <= 0) return 0;
    if (n >= (int)sizeof tmp) n = (int)sizeof tmp - 1;
    memcpy(tmp, buf + from, (size_t)n); tmp[n] = 0;
    return shell_text_width(tmp);
}
/* first visible byte: scroll horizontally just enough to keep the caret inside avail px */
static int text_off(const char *buf, int caret, int avail, int step)
{
    int off = 0;
    while (off < caret && prefix_w(buf, off, caret) > avail - 2) off += step;
    return off;
}

/* password fields draw one bullet (U+2022, 3 bytes) per character; editing still works on the real buffer */
static const char *shown_text(const char *buf, bool mask, char *disp, size_t cap)
{
    if (!mask) return buf;
    size_t n = 0;
    for (size_t i = 0; buf[i] && n + 4 <= cap; i++) { memcpy(disp + n, "\xe2\x80\xa2", 3); n += 3; }
    disp[n] = 0;
    return disp;
}

bool ui_textfield(ui_ctx *u, shell_rect r, char *buf, size_t cap, int *caret, const char *placeholder)
{
    uint32_t id = ui_id(placeholder ? placeholder : "textfield") ^ (uint32_t)(r.x * 31 + r.y * 17);
    bool mask = u->mask_next; u->mask_next = false;
    int step = mask ? 3 : 1;
    char disp[3 * 96 + 4];
    click_logic(u, id, r);
    a11y_reg(u, id, A11Y_TEXT, placeholder ? placeholder : "text", mask ? "" : buf, r, false, true);
    const shell_theme *t = u->theme;
    bool focused = u->focus == id, entered = false;
    int len = (int)strlen(buf);
    if (*caret > len) *caret = len;
    if (*caret < 0) *caret = 0;
    if (u->pressed && ui_hovered(u, r)) {                          /* click places the caret */
        const char *sh = shown_text(buf, mask, disp, sizeof disp - 1);
        int off0 = text_off(sh, *caret * step, r.w - 16, step);
        int pos = (off0 + shell_text_index_at(sh + off0, u->mx - (r.x + 8))) / step;
        *caret = pos < 0 ? 0 : pos > len ? len : pos;
    }
    if (focused) {
        for (int ki = 0; ki < u->nkeys;) {                             /* keys are applied in arrival order, chars and editing keys interleaved */
            uint32_t k = u->keys[ki].key; bool ch = u->keys[ki].is_char;
            if (!ch && k == KEY_TAB) { ki++; continue; }               /* Tab moves focus, the field leaves it queued */
            memmove(&u->keys[ki], &u->keys[ki + 1], (size_t)(u->nkeys - ki - 1) * sizeof u->keys[0]);
            u->nkeys--;
            if (ch) {
                if (k >= 32 && k < 127 && (size_t)len + 1 < cap) { memmove(buf + *caret + 1, buf + *caret, (size_t)(len - *caret) + 1); buf[(*caret)++] = (char)k; len++; }
            } else if (k == KEY_BACKSPACE && *caret > 0) { memmove(buf + *caret - 1, buf + *caret, (size_t)(len - *caret) + 1); (*caret)--; len--; }
            else if (k == KEY_DELETE && *caret < len) { memmove(buf + *caret, buf + *caret + 1, (size_t)(len - *caret)); len--; }
            else if (k == KEY_LEFT && *caret > 0) (*caret)--;
            else if (k == KEY_RIGHT && *caret < len) (*caret)++;
            else if (k == KEY_HOME) *caret = 0;
            else if (k == KEY_END) *caret = len;
            else if (k == KEY_ENTER) entered = true;
        }
    }
    fb_fill_round(u->fb, u->clip, r, 6, t->surface_hi);
    fb_frame(u->fb, u->clip, r, focused ? t->accent : t->border);
    if (focused && u->high_contrast) fb_frame(u->fb, u->clip, (shell_rect){ r.x + 1, r.y + 1, r.w - 2, r.h - 2 }, t->accent);
    shell_rect saved; ui_clip_push(u, (shell_rect){ r.x + 2, r.y + 2, r.w - 4, r.h - 4 }, &saved);
    int ty = r.y + (r.h - 16) / 2;
    const char *sh = shown_text(buf, mask, disp, sizeof disp - 1);
    int off = text_off(sh, *caret * step, r.w - 16, step);
    if (len) fb_text(u->fb, u->clip, r.x + 8, ty, sh + off, t->text);
    else if (placeholder) fb_text(u->fb, u->clip, r.x + 8, ty, placeholder, t->text_dim);
    if (focused && (u->reduce_motion || ((u->frame / 30) & 1) == 0)) fb_fill(u->fb, u->clip, (shell_rect){ r.x + 8 + prefix_w(sh, off, *caret * step), ty, 1, 16 }, t->text);
    ui_clip_pop(u, saved);
    return entered;
}

bool ui_textfield_password(ui_ctx *u, shell_rect r, char *buf, size_t cap, int *caret, const char *placeholder)
{
    u->mask_next = true;
    return ui_textfield(u, r, buf, cap, caret, placeholder);
}

int ui_tabs(ui_ctx *u, shell_rect r, const char *const *labels, int n, int *cur)
{
    const shell_theme *t = u->theme;
    int x = r.x, picked = -1;
    fb_hline(u->fb, u->clip, r.x, r.y + r.h - 1, r.w, t->border);
    for (int i = 0; i < n; i++) {
        int w = shell_text_width(labels[i]) + 28;
        shell_rect tr = { x, r.y, w, r.h };
        uint32_t id = ui_id(labels[i]) ^ 0x7ab5u;
        bool tc = click_logic(u, id, tr);
        a11y_reg(u, id, A11Y_TAB, labels[i], NULL, tr, *cur == i, true);
        tc |= kbd_activate(u, id);
        if (tc) { if (*cur != i) picked = i; *cur = i; }
        bool sel = *cur == i, hot = u->hot == id || u->next_hot == id;
        if (hot && !sel) fb_fill_round(u->fb, u->clip, tr, 6, t->surface_hi);
        fb_text(u->fb, u->clip, x + 14, r.y + (r.h - 16) / 2 - 1, labels[i], sel ? t->text : t->text_dim);
        if (sel) fb_fill(u->fb, u->clip, (shell_rect){ x + 8, r.y + r.h - 3, w - 16, 3 }, t->accent);
        x += w;
    }
    return picked;
}

void ui_progress(ui_ctx *u, shell_rect r, int pct)
{
    { char vs[16]; snprintf(vs, sizeof vs, "%d%%", pct); a11y_reg(u, 0, A11Y_PROGRESS, "progress", vs, r, false, false); }
    if (pct < 0) pct = 0;
    if (pct > 100) pct = 100;
    fb_fill_round(u->fb, u->clip, r, r.h / 2, u->theme->border);
    if (pct) fb_fill_round(u->fb, u->clip, (shell_rect){ r.x, r.y, r.w * pct / 100 > r.h ? r.w * pct / 100 : r.h, r.h }, r.h / 2, u->theme->accent);
}

bool ui_slider(ui_ctx *u, shell_rect r, const char *id_s, int *value, int lo, int hi)
{
    uint32_t id = ui_id(id_s) ^ (uint32_t)(r.x * 31 + r.y * 17);
    click_logic(u, id, r);
    { char vs[16]; snprintf(vs, sizeof vs, "%d", *value); a11y_reg(u, id, A11Y_SLIDER, id_s, vs, r, false, true); }
    bool changed = false;
    if (u->focus == id) {
        if (ui_take_key(u, KEY_RIGHT, 0) && *value < hi) { (*value)++; changed = true; }
        if (ui_take_key(u, KEY_LEFT, 0) && *value > lo) { (*value)--; changed = true; }
    }
    if (u->active == id && hi > lo) {
        int v = lo + (u->mx - r.x) * (hi - lo) / (r.w > 1 ? r.w - 1 : 1);
        if (v < lo) v = lo;
        if (v > hi) v = hi;
        if (v != *value) { *value = v; changed = true; }
    }
    const shell_theme *t = u->theme;
    shell_rect track = { r.x, r.y + r.h / 2 - 2, r.w, 4 };
    fb_fill_round(u->fb, u->clip, track, 2, t->border);
    int kx = hi > lo ? r.x + (*value - lo) * (r.w - 14) / (hi - lo) : r.x;
    fb_fill_round(u->fb, u->clip, (shell_rect){ r.x, track.y, kx - r.x + 7, 4 }, 2, t->accent);
    fb_fill_round(u->fb, u->clip, (shell_rect){ kx, r.y + r.h / 2 - 7, 14, 14 }, 7, 0xFFFFFFFFu);
    return changed;
}

int ui_list(ui_ctx *u, shell_rect r, const char *id_s, int n, int row_h, ui_row_fn draw, void *user, int *sel, int *scroll)
{
    uint32_t id = ui_id(id_s);
    const shell_theme *t = u->theme;
    { char vs[16]; snprintf(vs, sizeof vs, "%d/%d", *sel + 1, n); a11y_reg(u, id, A11Y_LIST, id_s, vs, r, false, true); }
    int visible = r.h / row_h, activated = -1;
    int maxscroll = n > visible ? n - visible : 0;
    bool over = ui_hovered(u, r);
    if (over && u->wheel) *scroll -= u->wheel;
    if (u->pressed && over && !u->consumed_click) {
        u->focus = id; u->consumed_click = true;
        int row = *scroll + (u->my - r.y) / row_h;
        if (row >= 0 && row < n) {
            if (u->lc_id == id && u->lc_row == row && u->now_ms - u->lc_ms <= 400) { activated = row; u->lc_id = 0; }   /* double-click */
            else { u->lc_id = id; u->lc_row = row; u->lc_ms = u->now_ms; }
            *sel = row;
        }
    }
    if (u->focus == id) {
        uint32_t k, m;
        while (pop_key(u, false, &k, &m)) {
            if (k == KEY_DOWN && *sel + 1 < n) (*sel)++;
            else if (k == KEY_UP && *sel > 0) (*sel)--;
            else if (k == KEY_HOME) *sel = 0;
            else if (k == KEY_END) *sel = n - 1;
            else if (k == KEY_PGDN) *sel = *sel + visible < n ? *sel + visible : n - 1;
            else if (k == KEY_PGUP) *sel = *sel - visible > 0 ? *sel - visible : 0;
            else if (k == KEY_ENTER && *sel >= 0 && *sel < n) activated = *sel;
            if (*sel < *scroll) *scroll = *sel;
            if (*sel >= *scroll + visible) *scroll = *sel - visible + 1;
        }
    }
    if (*scroll > maxscroll) *scroll = maxscroll;
    if (*scroll < 0) *scroll = 0;
    fb_fill_round(u->fb, u->clip, r, 6, t->surface);
    shell_rect saved; ui_clip_push(u, r, &saved);
    for (int i = 0; i < visible + 1 && *scroll + i < n; i++) {
        int idx = *scroll + i;
        shell_rect row = { r.x, r.y + i * row_h, r.w - (maxscroll ? 8 : 0), row_h };
        bool selected = idx == *sel;
        if (selected) fb_fill(u->fb, u->clip, row, u->focus == id ? color_mix(t->surface_hi, t->accent, 110) : t->surface_hi);
        else if (over && rect_contains(row, u->mx, u->my)) fb_fill(u->fb, u->clip, row, color_mix(t->surface, t->surface_hi, 128));
        draw(u, row, idx, selected, user);
    }
    if (maxscroll) {                                                 /* scrollbar */
        shell_rect sb = { r.x + r.w - 6, r.y + 2, 4, r.h - 4 };
        fb_fill_round(u->fb, u->clip, sb, 2, t->border);
        int th = sb.h * visible / n; if (th < 16) th = 16;
        int ty = sb.y + (sb.h - th) * *scroll / maxscroll;
        fb_fill_round(u->fb, u->clip, (shell_rect){ sb.x, ty, 4, th }, 2, t->text_dim);
    }
    ui_clip_pop(u, saved);
    return activated;
}
