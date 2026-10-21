#include "notify.h"
#include <string.h>
#include <stdio.h>

static void overlay(shell_comp *c, shell_fb *fb, shell_rect clip, void *u);
static void dirty(shell_notify *n) { if (n->comp) comp_damage_all(n->comp); }

void notify_init(shell_notify *n, shell_perms *p, shell_comp *c) { memset(n, 0, sizeof *n); n->perms = p; n->comp = c; n->next_id = 1; }
void notify_install(shell_notify *n) { if (n->comp) comp_add_overlay(n->comp, overlay, n); }

static bool rate_ok(shell_notify *n, const char *app)
{
    int i;
    for (i = 0; i < n->nrate; i++) if (!strcmp(n->rate[i].app, app)) break;
    if (i == n->nrate) {
        if (n->nrate == 16) { memmove(&n->rate[0], &n->rate[1], 15 * sizeof n->rate[0]); i = 15; n->nrate = 15; }
        n->nrate++;
        memset(&n->rate[i], 0, sizeof n->rate[i]);
        strncpy(n->rate[i].app, app, 31);
    }
    int keep = 0;
    for (int k = 0; k < n->rate[i].n; k++) if (n->now_ms - n->rate[i].stamp[k] < NOTIFY_RATE_WINDOW_MS) n->rate[i].stamp[keep++] = n->rate[i].stamp[k];
    n->rate[i].n = keep;
    if (keep >= NOTIFY_RATE_COUNT) return false;
    n->rate[i].stamp[n->rate[i].n++] = n->now_ms;
    return true;
}

static uint32_t add_note(shell_notify *n, const char *app, const char *title, const char *body, note_urgency u, const char *action, uint32_t aid)
{
    shell_note *slot = NULL;
    for (int i = 0; i < NOTIFY_MAX; i++) if (!n->note[i].id) { slot = &n->note[i]; break; }
    if (!slot) {                                                    /* history full: evict the oldest read, else the oldest */
        uint64_t best = UINT64_MAX; shell_note *pick = NULL;
        for (int i = 0; i < NOTIFY_MAX; i++) { uint64_t key = n->note[i].time_ms + (n->note[i].read ? 0 : (1ull << 40)); if (key < best) { best = key; pick = &n->note[i]; } }
        slot = pick; n->dropped_full++;
    }
    memset(slot, 0, sizeof *slot);
    slot->id = n->next_id++;
    strncpy(slot->app, app, sizeof slot->app - 1); strncpy(slot->title, title, sizeof slot->title - 1);
    strncpy(slot->body, body ? body : "", sizeof slot->body - 1); strncpy(slot->action, action ? action : "", sizeof slot->action - 1);
    slot->action_id = aid; slot->urgency = u; slot->time_ms = n->now_ms;
    if (!n->dnd || u == NOTE_URGENT) slot->toast_until = u == NOTE_URGENT ? UINT64_MAX : n->now_ms + NOTIFY_TOAST_MS;
    n->published++;
    dirty(n);
    return slot->id;
}

static void perm_done(const char *app, perm_res r, bool allowed, void *ctx)
{
    shell_notify *n = ctx;
    (void)r;
    for (int i = 0; i < n->npending;) {
        if (strcmp(n->pending[i].app, app)) { i++; continue; }
        if (allowed) add_note(n, app, n->pending[i].title, n->pending[i].body, n->pending[i].urgency, n->pending[i].action, n->pending[i].action_id);
        else n->dropped_denied++;
        memmove(&n->pending[i], &n->pending[i + 1], (size_t)(n->npending - i - 1) * sizeof n->pending[0]); n->npending--;
    }
}

uint32_t notify_publish(shell_notify *n, const char *app, const char *title, const char *body, note_urgency u, const char *action, uint32_t aid)
{
    if (!app || !*app || !title) return 0;
    perm_state s = perms_check(n->perms, app, RES_NOTIFY);
    if (s == PERM_DENY) { n->dropped_denied++; return 0; }
    if (!rate_ok(n, app)) { n->dropped_rate++; return 0; }
    if (s == PERM_ASK) {                                              /* park it and ask the user once */
        if (n->npending >= NOTIFY_PENDING) { n->dropped_full++; return 0; }
        bool asked = false;
        for (int k = 0; k < n->npending; k++) if (!strcmp(n->pending[k].app, app)) asked = true;
        int i = n->npending++;
        strncpy(n->pending[i].app, app, 31); n->pending[i].app[31] = 0;
        strncpy(n->pending[i].title, title, 47); n->pending[i].title[47] = 0;
        strncpy(n->pending[i].body, body ? body : "", 119); n->pending[i].body[119] = 0;
        strncpy(n->pending[i].action, action ? action : "", 23); n->pending[i].action[23] = 0;
        n->pending[i].action_id = aid; n->pending[i].urgency = u;
        if (!asked) perms_request(n->perms, app, RES_NOTIFY, perm_done, n);       /* one prompt per app, however many notifications wait */
        return 0;
    }
    return add_note(n, app, title, body, u, action, aid);
}

void notify_tick(shell_notify *n, uint64_t now)
{
    n->now_ms = now;
    for (int i = 0; i < NOTIFY_MAX; i++)
        if (n->note[i].id && n->note[i].toast_until && n->note[i].toast_until != UINT64_MAX && now >= n->note[i].toast_until) { n->note[i].toast_until = 0; dirty(n); }
}

const shell_note *notify_get(const shell_notify *n, uint32_t id) { for (int i = 0; i < NOTIFY_MAX; i++) if (n->note[i].id == id) return &n->note[i]; return NULL; }
void notify_dismiss(shell_notify *n, uint32_t id) { for (int i = 0; i < NOTIFY_MAX; i++) if (n->note[i].id == id) { memset(&n->note[i], 0, sizeof n->note[i]); dirty(n); return; } }
void notify_clear_all(shell_notify *n) { memset(n->note, 0, sizeof n->note); n->scroll = n->sel = 0; dirty(n); }
void notify_clear_app(shell_notify *n, const char *app) { for (int i = 0; i < NOTIFY_MAX; i++) if (n->note[i].id && !strcmp(n->note[i].app, app)) memset(&n->note[i], 0, sizeof n->note[i]); dirty(n); }
int notify_count(const shell_notify *n) { int c = 0; for (int i = 0; i < NOTIFY_MAX; i++) c += n->note[i].id != 0; return c; }
int notify_unread(const shell_notify *n) { int c = 0; for (int i = 0; i < NOTIFY_MAX; i++) c += n->note[i].id && !n->note[i].read; return c; }
int notify_visible_toasts(const shell_notify *n) { int c = 0; for (int i = 0; i < NOTIFY_MAX; i++) c += n->note[i].id && n->note[i].toast_until; return c; }
void notify_set_dnd(shell_notify *n, bool on) { n->dnd = on; if (on) for (int i = 0; i < NOTIFY_MAX; i++) if (n->note[i].urgency != NOTE_URGENT) n->note[i].toast_until = 0; dirty(n); }
void notify_set_center(shell_notify *n, bool open)
{
    n->center_open = open;
    if (open) for (int i = 0; i < NOTIFY_MAX; i++) { n->note[i].read = n->note[i].id ? true : false; n->note[i].toast_until = 0; if (n->note[i].urgency == NOTE_URGENT && n->note[i].id) n->note[i].toast_until = 0; }
    dirty(n);
}

shell_rect notify_center_rect(const shell_comp *c) { return (shell_rect){ comp_lw(c) - 380, 8, 372, comp_lh(c) - 40 - 16 }; }
shell_rect notify_bell_rect(const shell_comp *c) { return (shell_rect){ comp_lw(c) - 150 + 70, comp_lh(c) - 40 + 8, 24, 24 }; }

/* newest first */
static int ordered(const shell_notify *n, int *idx)
{
    int cnt = 0;
    for (int i = 0; i < NOTIFY_MAX; i++) {
        if (!n->note[i].id) continue;
        int pos = cnt++;
        while (pos > 0 && n->note[idx[pos - 1]].id < n->note[i].id) { idx[pos] = idx[pos - 1]; pos--; }
        idx[pos] = i;
    }
    return cnt;
}
#define ROW_H 76
static shell_rect row_rect(const shell_comp *c, int row, int scroll) { shell_rect r = notify_center_rect(c); return (shell_rect){ r.x + 8, r.y + 76 + (row - scroll) * ROW_H, r.w - 16, ROW_H - 6 }; }
static shell_rect toast_rect(const shell_comp *c, int slot) { return (shell_rect){ comp_lw(c) - 340, 8 + slot * 84, 332, 76 }; }

bool notify_mouse(shell_notify *n, int x, int y, bool down)
{
    shell_comp *c = n->comp;
    x = comp_logical(c, x); y = comp_logical(c, y);
    if (down && rect_contains(notify_bell_rect(c), x, y)) { notify_set_center(n, !n->center_open); return true; }
    if (n->center_open) {
        shell_rect cr = notify_center_rect(c);
        if (!rect_contains(cr, x, y)) { if (down) notify_set_center(n, false); return false; }
        if (!down) return true;
        if (rect_contains((shell_rect){ cr.x + cr.w - 110, cr.y + 10, 100, 28 }, x, y)) { notify_clear_all(n); return true; }
        if (rect_contains((shell_rect){ cr.x + 12, cr.y + 42, 200, 24 }, x, y)) { notify_set_dnd(n, !n->dnd); return true; }
        int idx[NOTIFY_MAX], cnt = ordered(n, idx);
        for (int row = n->scroll; row < cnt; row++) {
            shell_rect rr = row_rect(c, row, n->scroll);
            if (rr.y + rr.h > cr.y + cr.h) break;
            if (!rect_contains(rr, x, y)) continue;
            shell_note *nt = &n->note[idx[row]];
            if (rect_contains((shell_rect){ rr.x + rr.w - 26, rr.y + 4, 22, 22 }, x, y)) { notify_dismiss(n, nt->id); return true; }
            if (nt->action[0] && rect_contains((shell_rect){ rr.x + rr.w - 108, rr.y + rr.h - 30, 100, 24 }, x, y)) {
                uint32_t id = nt->id, aid = nt->action_id;
                if (n->on_action) n->on_action(id, aid, n->ctx);
                notify_dismiss(n, id);
                return true;
            }
        }
        return true;
    }
    if (down) {                                                         /* toasts: click dismisses, or triggers the action */
        int slot = 0;
        for (int i = 0; i < NOTIFY_MAX && slot < NOTIFY_MAX_TOASTS; i++) {
            if (!n->note[i].id || !n->note[i].toast_until) continue;
            shell_rect tr = toast_rect(c, slot++);
            if (!rect_contains(tr, x, y)) continue;
            uint32_t id = n->note[i].id, aid = n->note[i].action_id; bool act = n->note[i].action[0] && x > tr.x + tr.w - 110 && y > tr.y + tr.h - 34;
            if (act && n->on_action) n->on_action(id, aid, n->ctx);
            notify_dismiss(n, id);
            return true;
        }
    }
    return false;
}

bool notify_key(shell_notify *n, uint32_t key, bool down, uint32_t mods)
{
    if (!down) return false;
    if (key == 'n' && (mods & MOD_SUPER)) { notify_set_center(n, !n->center_open); return true; }
    if (n->center_open && key == KEY_ESC) { notify_set_center(n, false); return true; }
    if (n->center_open) {
        int cnt = notify_count(n);
        if (key == KEY_DOWN && n->scroll + 1 < cnt) { n->scroll++; dirty(n); return true; }
        if (key == KEY_UP && n->scroll > 0) { n->scroll--; dirty(n); return true; }
        if (key == KEY_DELETE) { notify_clear_all(n); return true; }
    }
    return false;
}

static void age(uint64_t now, uint64_t t, char *out, size_t cap)
{
    uint64_t s = now > t ? (now - t) / 1000 : 0;
    if (s < 10) snprintf(out, cap, "now"); else if (s < 60) snprintf(out, cap, "%llus", (unsigned long long)s);
    else if (s < 3600) snprintf(out, cap, "%llum", (unsigned long long)s / 60); else snprintf(out, cap, "%lluh", (unsigned long long)s / 3600);
}

static void draw_note(shell_comp *c, shell_fb *fb, shell_rect clip, shell_rect r, const shell_note *nt, uint64_t now, bool toast)
{
    const shell_theme *t = &c->theme;
    if (toast) fb_fill_round(fb, clip, (shell_rect){ r.x - 2, r.y - 2, r.w + 4, r.h + 4 }, 10, nt->urgency == NOTE_URGENT ? t->danger : t->border);
    fb_fill_round(fb, clip, r, 8, toast ? t->surface : t->surface_hi);
    fb_text(fb, clip, r.x + 10, r.y + 6, nt->app, t->text_dim);
    char tm[12]; age(now, nt->time_ms, tm, sizeof tm);
    fb_text(fb, clip, r.x + r.w - 30 - shell_text_width(tm), r.y + 6, tm, t->text_dim);
    fb_text(fb, clip, r.x + r.w - 20, r.y + 4, "x", t->text_dim);
    fb_text(fb, clip, r.x + 10, r.y + 24, nt->title, t->text);
    char l1[44]; strncpy(l1, nt->body, sizeof l1 - 1); l1[sizeof l1 - 1] = 0;
    fb_text(fb, clip, r.x + 10, r.y + 44, l1, t->text_dim);
    if (nt->action[0]) {
        shell_rect b = { r.x + r.w - 108, r.y + r.h - 30, 100, 24 };
        fb_fill_round(fb, clip, b, 5, t->accent);
        fb_text(fb, clip, b.x + (b.w - shell_text_width(nt->action)) / 2, b.y + 4, nt->action, t->accent_text);
    }
}

static void overlay(shell_comp *c, shell_fb *fb, shell_rect clip, void *u)
{
    shell_notify *n = u;
    shell_fb view = comp_view(c, fb); shell_rect lclip = comp_clip_logical(c, clip);
    fb = &view; clip = lclip;
    const shell_theme *t = &c->theme;
    shell_rect bell = notify_bell_rect(c);
    int unread = notify_unread(n);
    fb_fill_round(fb, clip, bell, 6, n->center_open ? color_mix(t->surface_hi, t->accent, 100) : t->surface);
    fb_text(fb, clip, bell.x + 8, bell.y + 4, n->dnd ? "z" : "!", unread && !n->dnd ? t->accent : t->text_dim);
    if (unread && !n->dnd) fb_fill_round(fb, clip, (shell_rect){ bell.x + 16, bell.y - 2, 10, 10 }, 5, t->danger);
    int idx[NOTIFY_MAX], cnt = ordered(n, idx);
    if (n->center_open) {
        shell_rect cr = notify_center_rect(c);
        fb_fill_round(fb, clip, (shell_rect){ cr.x - 2, cr.y - 2, cr.w + 4, cr.h + 4 }, 12, t->border);
        fb_fill_round(fb, clip, cr, 10, t->surface);
        fb_text(fb, clip, cr.x + 14, cr.y + 14, "Notifications", t->text);
        shell_rect clr = { cr.x + cr.w - 110, cr.y + 10, 100, 28 };
        fb_fill_round(fb, clip, clr, 6, t->surface_hi);
        fb_text(fb, clip, clr.x + 12, clr.y + 6, "Clear all", t->text);
        fb_text(fb, clip, cr.x + 14, cr.y + 46, n->dnd ? "[x] Do not disturb" : "[ ] Do not disturb", t->text_dim);
        if (!cnt) fb_text(fb, clip, cr.x + 14, cr.y + 90, "Nothing new.", t->text_dim);
        for (int row = n->scroll; row < cnt; row++) {
            shell_rect rr = row_rect(c, row, n->scroll);
            if (rr.y + rr.h > cr.y + cr.h) break;
            draw_note(c, fb, clip, rr, &n->note[idx[row]], n->now_ms, false);
        }
        return;
    }
    int slot = 0;
    for (int i = 0; i < NOTIFY_MAX && slot < NOTIFY_MAX_TOASTS; i++) {
        if (!n->note[i].id || !n->note[i].toast_until) continue;
        draw_note(c, fb, clip, toast_rect(c, slot++), &n->note[i], n->now_ms, true);
    }
}
