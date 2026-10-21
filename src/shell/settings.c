#include "settings.h"
#include <string.h>
#include <stdio.h>

const char *const settings_page_name[PG_COUNT] = { "Privacy", "Network firewall", "App permissions", "Updates", "Appearance", "Accessibility", "Accounts" };
const uint32_t settings_accents[6] = { 0xFF3B82F6u, 0xFF8B5CF6u, 0xFF10B981u, 0xFFF59E0Bu, 0xFFEF4444u, 0xFFEC4899u };
static const char *const page_ids[PG_COUNT] = { "privacy", "firewall", "permissions", "updates", "appearance", "accessibility", "accounts" };

bool settings_page_by_id(const char *id, settings_page *out)
{
    if (strncmp(id, "settings:", 9)) return false;
    for (int i = 0; i < PG_COUNT; i++) if (!strcmp(id + 9, page_ids[i])) { *out = (settings_page)i; return true; }
    return false;
}

void settings_apply(shell_settings *s)
{
    const shell_backend *b = s->be;
    s->theme = be_cfg_int(b, "ui.theme", 0);
    s->accent = be_cfg_int(b, "ui.accent", 0);
    s->scale = be_cfg_int(b, "ui.scale", 100);
    if (s->accent < 0 || s->accent > 5) s->accent = 0;
    if (be_cfg_bool(b, "ui.high_contrast", false)) s->theme = 2;
    if (s->comp) {
        shell_comp *c = s->comp;
        if (s->theme == 1) shell_theme_light(&c->theme, settings_accents[s->accent]);
        else if (s->theme == 2) shell_theme_high_contrast(&c->theme);
        else shell_theme_dark(&c->theme, settings_accents[s->accent]);
        if (c->scale_pct != (uint32_t)s->scale) comp_set_scale(c, (uint32_t)s->scale);
        c->reduce_motion = be_cfg_bool(b, "ui.reduce_motion", false);
        c->high_contrast = s->theme == 2;
        comp_damage_all(c);
    }
    if (s->taskbar) {
        bool off = b->net_hard_off_get(b->user);
        if (s->taskbar->net_hard_off != off) { s->taskbar->net_hard_off = off; damage_add(&s->comp->damage, taskbar_rect(s->comp)); }
    }
}

void settings_init(shell_settings *s, const shell_backend *be, shell_comp *comp, shell_taskbar *tb)
{
    memset(s, 0, sizeof *s);
    s->be = be; s->comp = comp; s->taskbar = tb;
    if (!be->cfg_get(be->user, "updates.mirror", s->mirror, sizeof s->mirror)) s->mirror[0] = 0;
    s->mirror_caret = (int)strlen(s->mirror);
    settings_apply(s);
}

shell_net_state settings_net_state(const shell_settings *s)
{
    if (s->be->net_hard_off_get(s->be->user)) return NET_BLOCKED;
    return s->nconns > 0 ? NET_ACTIVE : NET_IDLE;
}

static void changed(shell_settings *s) { s->changes++; settings_apply(s); }

static bool cfg_toggle(shell_settings *s, ui_ctx *ui, shell_rect r, const char *label, const char *key, bool def)
{
    bool v = be_cfg_bool(s->be, key, def);
    if (ui_toggle(ui, r, label, &v)) { be_cfg_set_bool(s->be, key, v); changed(s); return true; }
    return false;
}

static void row_conn(ui_ctx *u, shell_rect row, int i, bool sel, void *user)
{
    shell_settings *s = user;
    const net_conn *c = &s->conns[i];
    char rem[64];
    snprintf(rem, sizeof rem, "%s:%u", c->remote, (unsigned)c->port);
    fw_action a = s->be->fw_check(s->be->user, c->proc);
    uint32_t tc = sel ? u->theme->accent_text : u->theme->text;
    ui_label(u, row.x + 8, row.y + 4, c->proc, tc);
    ui_label(u, row.x + 150, row.y + 4, rem, tc);
    ui_label(u, row.x + 340, row.y + 4, c->proto, u->theme->text_dim);
    ui_label(u, row.x + 390, row.y + 4, c->state, u->theme->text_dim);
    ui_label_right(u, row.x + row.w - 8, row.y + 4, a == FW_ALLOW ? "allowed" : a == FW_DENY ? "blocked" : "ask", a == FW_DENY ? u->theme->danger : u->theme->text_dim);
}
static void row_app(ui_ctx *u, shell_rect row, int i, bool sel, void *user)
{
    shell_settings *s = user;
    int granted = 0;
    for (int r = 0; r < RES_COUNT; r++) granted += s->be->perm_get(s->be->user, s->apps[i], (perm_res)r);
    char buf[24]; snprintf(buf, sizeof buf, "%d granted", granted);
    ui_label(u, row.x + 8, row.y + 4, s->apps[i], sel ? u->theme->accent_text : u->theme->text);
    ui_label_right(u, row.x + row.w - 8, row.y + 4, buf, u->theme->text_dim);
}
static void row_text(ui_ctx *u, shell_rect row, int i, bool sel, void *user)
{
    const char *const *names = user;
    ui_label(u, row.x + 8, row.y + 4, names[i], sel ? u->theme->accent_text : u->theme->text);
}

static void page_privacy(shell_settings *s, ui_ctx *u, shell_rect c)
{
    const shell_theme *t = u->theme;
    int y = c.y;
    ui_heading(u, c.x, y, "Privacy"); y += 44;
    ui_label(u, c.x, y, "Telemetry: none. This system contains no telemetry or analytics code.", t->text); y += 28;
    ui_label(u, c.x, y, "Nothing leaves this device unless you start it, and the network LED shows when anything does.", t->text_dim); y += 40;
    char line[96];
    snprintf(line, sizeof line, "Open network connections: %d", s->nconns);
    ui_label(u, c.x, y, line, t->text); y += 24;
    int blocked = 0;
    fw_rule rules[MEM_MAX_RULES]; int nr = s->be->fw_list(s->be->user, rules, MEM_MAX_RULES);
    for (int i = 0; i < nr; i++) blocked += rules[i].action == FW_DENY;
    snprintf(line, sizeof line, "Firewall rules: %d (%d blocking)", nr, blocked);
    ui_label(u, c.x, y, line, t->text); y += 24;
    int grants = 0;
    for (int a = 0; a < s->napps; a++) for (int r = 0; r < RES_COUNT; r++) grants += s->be->perm_get(s->be->user, s->apps[a], (perm_res)r);
    snprintf(line, sizeof line, "Permission grants across %d apps: %d", s->napps, grants);
    ui_label(u, c.x, y, line, t->text); y += 40;
    bool off = s->be->net_hard_off_get(s->be->user);
    if (ui_toggle(u, (shell_rect){ c.x, y, 360, 28 }, "Block all network access", &off)) { s->be->net_hard_off_set(s->be->user, off); changed(s); }
    y += 40;
    ui_label(u, c.x, y, "Updates are opt-in; see the Updates page.", t->text_dim);
}

static void page_network(shell_settings *s, ui_ctx *u, shell_rect c)
{
    const shell_theme *t = u->theme;
    ui_heading(u, c.x, c.y, "Network firewall");
    shell_net_state ns = settings_net_state(s);
    uint32_t led = ns == NET_BLOCKED ? 0xFFEF4444u : ns == NET_ACTIVE ? 0xFFF59E0Bu : 0xFF4B5563u;
    fb_fill_round(u->fb, u->clip, (shell_rect){ c.x + c.w - 150, c.y + 8, 12, 12 }, 6, led);
    ui_label(u, c.x + c.w - 130, c.y + 6, ns == NET_BLOCKED ? "blocked" : ns == NET_ACTIVE ? "active" : "idle", t->text_dim);
    bool off = s->be->net_hard_off_get(s->be->user);
    if (ui_toggle(u, (shell_rect){ c.x, c.y + 44, 300, 26 }, "Block all network access", &off)) { s->be->net_hard_off_set(s->be->user, off); changed(s); }
    ui_label(u, c.x, c.y + 80, "Process", t->text_dim); ui_label(u, c.x + 150, c.y + 80, "Remote", t->text_dim);
    ui_label(u, c.x + 340, c.y + 80, "Proto", t->text_dim); ui_label(u, c.x + 390, c.y + 80, "State", t->text_dim);
    s->nconns = s->be->net_query(s->be->user, s->conns, 32);
    shell_rect lr = { c.x, c.y + 100, c.w, 220 };
    ui_list(u, lr, "conns", s->nconns, 24, row_conn, s, &s->conn_sel, &s->conn_scroll);
    if (s->nconns == 0) ui_label(u, lr.x + 12, lr.y + 12, "No open connections.", t->text_dim);
    int y = lr.y + lr.h + 12;
    if (s->conn_sel >= 0 && s->conn_sel < s->nconns) {
        const char *proc = s->conns[s->conn_sel].proc;
        char hd[64]; snprintf(hd, sizeof hd, "Rule for %s:", proc);
        ui_label(u, c.x, y + 6, hd, t->text);
        fw_action cur = s->be->fw_check(s->be->user, proc);
        if (ui_button(u, (shell_rect){ c.x + 240, y, 90, 28 }, "Allow") && cur != FW_ALLOW) { s->be->fw_set(s->be->user, proc, FW_ALLOW); changed(s); }
        if (ui_button(u, (shell_rect){ c.x + 336, y, 90, 28 }, "Block") && cur != FW_DENY) { s->be->fw_set(s->be->user, proc, FW_DENY); changed(s); }
        if (ui_button(u, (shell_rect){ c.x + 432, y, 90, 28 }, "Ask")) { s->be->fw_set(s->be->user, proc, FW_ASK); changed(s); }
    } else ui_label(u, c.x, y + 6, "Select a connection to allow or block its process.", t->text_dim);
}

static void page_permissions(shell_settings *s, ui_ctx *u, shell_rect c)
{
    const shell_theme *t = u->theme;
    ui_heading(u, c.x, c.y, "App permissions");
    s->napps = s->be->perm_apps(s->be->user, s->apps, MEM_MAX_APPS);
    shell_rect lr = { c.x, c.y + 50, 240, 280 };
    ui_list(u, lr, "perm_apps", s->napps, 26, row_app, s, &s->app_sel, &s->app_scroll);
    if (!s->napps) ui_label(u, lr.x + 10, lr.y + 10, "No apps registered.", t->text_dim);
    if (s->app_sel >= 0 && s->app_sel < s->napps) {
        const char *app = s->apps[s->app_sel];
        int x = c.x + 270, y = c.y + 56;
        ui_label(u, x, y, app, t->text); y += 32;
        for (int r = 0; r < RES_COUNT; r++, y += 36) {
            bool v = s->be->perm_get(s->be->user, app, (perm_res)r);
            if (ui_toggle(u, (shell_rect){ x, y, 300, 28 }, perm_res_name[r], &v)) { s->be->perm_set(s->be->user, app, (perm_res)r, v); changed(s); }
        }
        ui_label(u, x, y + 8, "Apps start with nothing granted.", t->text_dim);
    }
}

static void page_updates(shell_settings *s, ui_ctx *u, shell_rect c)
{
    const shell_theme *t = u->theme;
    ui_heading(u, c.x, c.y, "Updates");
    cfg_toggle(s, u, (shell_rect){ c.x, c.y + 50, 420, 28 }, "Check for updates (opt-in)", "updates.enabled", false);
    cfg_toggle(s, u, (shell_rect){ c.x, c.y + 90, 420, 28 }, "Offline mode (never contact a mirror)", "updates.offline", false);
    ui_label(u, c.x, c.y + 136, "Update mirror (self-hostable):", t->text);
    if (ui_textfield(u, (shell_rect){ c.x, c.y + 160, 420, 30 }, s->mirror, sizeof s->mirror, &s->mirror_caret, "https://mirror.example/loos")) {
        s->be->cfg_set(s->be->user, "updates.mirror", s->mirror); changed(s);
    }
    ui_label(u, c.x, c.y + 204, "Press Enter to save. The default mirror is empty: no address is built in.", t->text_dim);
    bool en = be_cfg_bool(s->be, "updates.enabled", false), off = be_cfg_bool(s->be, "updates.offline", false);
    ui_label(u, c.x, c.y + 240, !en ? "Status: updates are off." : off ? "Status: offline mode, no checks." : s->mirror[0] ? "Status: will check your mirror when you press Check." : "Status: set a mirror first.", t->text);
}

static void page_appearance(shell_settings *s, ui_ctx *u, shell_rect c)
{
    const shell_theme *t = u->theme;
    ui_heading(u, c.x, c.y, "Appearance");
    ui_label(u, c.x, c.y + 48, "Theme", t->text_dim);
    int th = s->theme == 2 && !be_cfg_bool(s->be, "ui.high_contrast", false) ? 2 : s->theme;
    static const char *const names[3] = { "Dark", "Light", "High contrast" };
    for (int i = 0; i < 3; i++)
        if (ui_radio(u, (shell_rect){ c.x + i * 150, c.y + 72, 140, 24 }, names[i], &th, i)) {
            be_cfg_set_int(s->be, "ui.theme", i); be_cfg_set_bool(s->be, "ui.high_contrast", i == 2); changed(s);
        }
    ui_label(u, c.x, c.y + 116, "Accent", t->text_dim);
    for (int i = 0; i < 6; i++) {
        shell_rect sw = { c.x + i * 44, c.y + 140, 32, 32 };
        bool hot = ui_hovered(u, sw);
        fb_fill_round(u->fb, u->clip, sw, 8, settings_accents[i]);
        if (s->accent == i) fb_frame(u->fb, u->clip, (shell_rect){ sw.x - 3, sw.y - 3, sw.w + 6, sw.h + 6 }, t->text);
        else if (hot) fb_frame(u->fb, u->clip, (shell_rect){ sw.x - 2, sw.y - 2, sw.w + 4, sw.h + 4 }, t->text_dim);
        if (u->pressed && hot) { be_cfg_set_int(s->be, "ui.accent", i); changed(s); }
    }
    ui_label(u, c.x, c.y + 192, "Scale", t->text_dim);
    static const int scales[4] = { 100, 125, 150, 200 };
    int cur = s->scale;
    for (int i = 0; i < 4; i++) {
        char lb[8]; snprintf(lb, sizeof lb, "%d%%", scales[i]);
        if (ui_radio(u, (shell_rect){ c.x + i * 90, c.y + 216, 80, 24 }, lb, &cur, scales[i])) { be_cfg_set_int(s->be, "ui.scale", scales[i]); changed(s); }
    }
    cfg_toggle(s, u, (shell_rect){ c.x, c.y + 262, 380, 28 }, "Reduce motion", "ui.reduce_motion", false);
}

static void page_access(shell_settings *s, ui_ctx *u, shell_rect c)
{
    const shell_theme *t = u->theme;
    ui_heading(u, c.x, c.y, "Accessibility");
    cfg_toggle(s, u, (shell_rect){ c.x, c.y + 50, 420, 28 }, "High contrast", "ui.high_contrast", false);
    cfg_toggle(s, u, (shell_rect){ c.x, c.y + 90, 420, 28 }, "Reduce motion", "ui.reduce_motion", false);
    bool large = s->scale >= 150;
    if (ui_toggle(u, (shell_rect){ c.x, c.y + 130, 420, 28 }, "Large text and controls (150%)", &large)) { be_cfg_set_int(s->be, "ui.scale", large ? 150 : 100); changed(s); }
    cfg_toggle(s, u, (shell_rect){ c.x, c.y + 170, 420, 28 }, "Sticky keys", "access.sticky_keys", false);
    cfg_toggle(s, u, (shell_rect){ c.x, c.y + 210, 420, 28 }, "Screen reader bridge (UI Automation)", "access.uia", false);
    ui_label(u, c.x, c.y + 256, "Full keyboard navigation is always on. Ctrl+Space opens the command palette.", t->text_dim);
}

static void page_accounts(shell_settings *s, ui_ctx *u, shell_rect c)
{
    const shell_theme *t = u->theme;
    ui_heading(u, c.x, c.y, "Accounts");
    static char list[128]; static const char *names[8]; int n = 0;
    if (!s->be->cfg_get(s->be->user, "login.accounts", list, sizeof list)) strcpy(list, "owner");
    for (char *p = list; *p && n < 8;) { names[n++] = p; char *comma = strchr(p, ','); if (!comma) break; *comma = 0; p = comma + 1; }
    ui_label(u, c.x, c.y + 48, "Local accounts only. There is no cloud account and no PIN sync.", t->text_dim);
    ui_list(u, (shell_rect){ c.x, c.y + 76, 280, 130 }, "accounts", n, 26, row_text, names, &s->account_sel, &s->account_scroll);
    cfg_toggle(s, u, (shell_rect){ c.x, c.y + 224, 420, 28 }, "Sign in automatically", "login.auto", false);
}

void settings_frame(shell_settings *s, ui_ctx *u, int w, int h)
{
    const shell_theme *t = u->theme;
    ui_clear(u, t->surface);
    int sbw = 190;
    fb_fill(u->fb, u->clip, (shell_rect){ 0, 0, sbw, h }, color_mix(t->surface, t->desktop, 160));
    fb_vline(u->fb, u->clip, sbw, 0, h, t->border);
    for (int i = 0; i < PG_COUNT; i++) {
        shell_rect r = { 8, 12 + i * 38, sbw - 16, 32 };
        bool sel = s->page == (settings_page)i;
        if (sel) fb_fill_round(u->fb, u->clip, r, 6, color_mix(t->surface_hi, t->accent, 70));
        if (ui_hovered(u, r) && u->pressed) s->page = (settings_page)i;
        else if (ui_hovered(u, r) && !sel) fb_fill_round(u->fb, u->clip, r, 6, color_mix(t->surface, t->surface_hi, 140));
        fb_text(u->fb, u->clip, r.x + 12, r.y + 8, settings_page_name[i], sel ? t->text : t->text_dim);
    }
    shell_rect c = { sbw + 28, 24, w - sbw - 56, h - 48 };
    s->nconns = s->be->net_query(s->be->user, s->conns, 32);
    s->napps = s->be->perm_apps(s->be->user, s->apps, MEM_MAX_APPS);
    switch (s->page) {
    case PG_PRIVACY: page_privacy(s, u, c); break;
    case PG_NETWORK: page_network(s, u, c); break;
    case PG_PERMISSIONS: page_permissions(s, u, c); break;
    case PG_UPDATES: page_updates(s, u, c); break;
    case PG_APPEARANCE: page_appearance(s, u, c); break;
    case PG_ACCESS: page_access(s, u, c); break;
    case PG_ACCOUNTS: page_accounts(s, u, c); break;
    default: break;
    }
}
