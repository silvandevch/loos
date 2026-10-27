#include "compat_console.h"
#include <string.h>
#include <stdio.h>

static const char *const era_label[COMPAT_ERA_COUNT] = { "Windows 7", "Windows 8", "Windows 8.1", "Windows 10", "Windows 11" };
static const char *const src_label[] = { "default", "manifest", "profile" };

static void changed(shell_compat_console *c) { c->changes++; if (c->persist) c->persist(c->db, c->user); }

void cc_refresh(shell_compat_console *c)
{
    c->napps = 0;
    for (int i = 0; c->inv && i < c->inv->n && c->napps < SHELL_MAX_APPS; i++)
        if (c->inv->item[i].kind == ITEM_APP) { strncpy(c->apps[c->napps], c->inv->item[i].target, COMPAT_NAME_MAX - 1); c->apps[c->napps][COMPAT_NAME_MAX - 1] = 0; c->napps++; }
    for (unsigned i = 0; i < c->db->napps && c->napps < SHELL_MAX_APPS; i++) {
        bool have = false;
        for (int k = 0; k < c->napps; k++) if (!strcmp(c->apps[k], c->db->apps[i].name)) have = true;
        if (!have) { strcpy(c->apps[c->napps++], c->db->apps[i].name); }
    }
    if (c->sel >= c->napps) c->sel = c->napps ? c->napps - 1 : 0;
}

void cc_init(shell_compat_console *c, compat_db *db, const shell_inventory *inv)
{
    memset(c, 0, sizeof *c);
    c->db = db; c->inv = inv;
    cc_refresh(c);
}

bool cc_set_claim(shell_compat_console *c, const char *app, compat_era era)
{
    compat_status s = compat_db_set_claim(c->db, app, era);
    if (s != COMPAT_OK) { snprintf(c->status, sizeof c->status, "Could not save the profile (%d)", (int)s); return false; }
    snprintf(c->status, sizeof c->status, "%s now reports %s", app, era_label[era]);
    cc_refresh(c); changed(c);
    return true;
}
void cc_clear_claim(shell_compat_console *c, const char *app)
{
    compat_db_clear_claim(c->db, app);
    snprintf(c->status, sizeof c->status, "%s uses the automatic version", app);
    cc_refresh(c); changed(c);
}

static void split_sym(const char *in, char *dll, size_t dsz, char *sym, size_t ssz)
{
    const char *bang = strchr(in, '!');
    size_t n = bang ? (size_t)(bang - in) : strlen(in);
    if (n >= dsz) n = dsz - 1;
    memcpy(dll, in, n); dll[n] = 0;
    if (bang) { strncpy(sym, bang + 1, ssz - 1); sym[ssz - 1] = 0; } else sym[0] = 0;
}

bool cc_add_redirect(shell_compat_console *c, const char *app, const char *from, const char *to)
{
    char fd[COMPAT_SYM_MAX], fs[COMPAT_SYM_MAX], td[COMPAT_SYM_MAX], ts[COMPAT_SYM_MAX];
    split_sym(from, fd, sizeof fd, fs, sizeof fs); split_sym(to, td, sizeof td, ts, sizeof ts);
    if (!fd[0] || !td[0]) { snprintf(c->status, sizeof c->status, "Both sides need a DLL name (dll or dll!symbol)"); return false; }
    if (compat_db_add_redirect(c->db, app, fd, fs, td, ts) != COMPAT_OK) { snprintf(c->status, sizeof c->status, "Redirect table is full or invalid"); return false; }
    snprintf(c->status, sizeof c->status, "Redirect added for %s", app);
    changed(c);
    return true;
}

void cc_describe(const shell_compat_console *c, const char *app, compat_claim *claim, char *shims, size_t cap)
{
    const char *mf = NULL; size_t mlen = 0;
    if (c->manifest_of) mf = c->manifest_of(app, &mlen, c->user);
    compat_resolve(c->db, app, mf, mlen, claim);
    const compat_shim_profile *sp = compat_shim_profile_of(claim->era);
    const compat_version *v = &sp->version;
    char *o = shims; size_t n = 0;
    #define ADD(...) do { int k_ = snprintf(o + n, n < cap ? cap - n : 0, __VA_ARGS__); if (k_ > 0) n += (size_t)k_; } while (0)
    ADD("reports %u.%u build %u%s%s\n", v->major, v->minor, v->build, v->csd[0] ? " " : "", v->csd);
    ADD("version-report shim: %s\n", sp->features & COMPAT_SHIM_VERSION_REPORT ? "on" : "off (native)");
    ADD("DPI: %s\n", sp->features & COMPAT_SHIM_PER_MONITOR_DPI_V2 ? "per-monitor v2" : sp->features & COMPAT_SHIM_PER_MONITOR_DPI_V1 ? "per-monitor v1" : sp->features & COMPAT_SHIM_DPI_AWARE ? "system aware" : "unaware (bitmap scaled)");
    ADD("long paths: %s", sp->features & COMPAT_SHIM_LONG_PATH_AWARE ? "supported" : "legacy 260 chars");
    #undef ADD
    if (cap) o[n < cap ? n : cap - 1] = 0;
}

static void row_app(ui_ctx *u, shell_rect row, int i, bool sel, void *user)
{
    shell_compat_console *c = user;
    compat_claim cl; char tmp[256];
    cc_describe(c, c->apps[i], &cl, tmp, sizeof tmp);
    ui_label(u, row.x + 8, row.y + 3, c->apps[i], sel ? u->theme->accent_text : u->theme->text);
    char b[40]; snprintf(b, sizeof b, "%s (%s)", era_label[cl.era], src_label[cl.source]);
    ui_label_right(u, row.x + row.w - 8, row.y + 3, b, sel ? u->theme->accent_text : u->theme->text_dim);
}
static void row_redirect(ui_ctx *u, shell_rect row, int i, bool sel, void *user)
{
    shell_compat_console *c = user;
    int seen = 0;
    const char *app = c->sel < c->napps ? c->apps[c->sel] : "";
    for (unsigned k = 0; k < c->db->nredirects; k++) {
        const compat_redirect *r = &c->db->redirects[k];
        if (strcmp(r->app, app) && strcmp(r->app, "*")) continue;
        if (seen++ != i) continue;
        char b[320]; snprintf(b, sizeof b, "%s%s%s -> %s%s%s%s", r->from_dll, r->from_sym[0] ? "!" : "", r->from_sym, r->to_dll, r->to_sym[0] ? "!" : "", r->to_sym, strcmp(r->app, "*") ? "" : "   (all apps)");
        ui_label(u, row.x + 8, row.y + 3, b, sel ? u->theme->accent_text : u->theme->text);
        return;
    }
}

void cc_frame(shell_compat_console *c, ui_ctx *u, int w, int h)
{
    const shell_theme *t = u->theme;
    ui_clear(u, t->surface);
    ui_heading(u, 16, 12, "Compatibility");
    /* default era */
    ui_label(u, 16, 54, "Default for apps without a profile:", t->text_dim);
    int def = (int)c->db->default_era;
    for (int i = 0; i < COMPAT_ERA_COUNT; i++)
        if (ui_radio(u, (shell_rect){ 16 + i * 140, 76, 132, 22 }, era_label[i], &def, i)) {
            char line[40]; snprintf(line, sizeof line, "default %s\n", compat_version_of((compat_era)i)->name);
            compat_db_parse(c->db, line, strlen(line), NULL);
            changed(c);
        }
    /* app list */
    cc_refresh(c);
    shell_rect lr = { 16, 120, w / 2 - 24, h - 140 };
    ui_list(u, lr, "compat_apps", c->napps, 24, row_app, c, &c->sel, &c->scroll);
    if (!c->napps) { ui_label(u, lr.x + 12, lr.y + 12, "No apps installed yet.", t->text_dim); return; }
    /* detail */
    const char *app = c->apps[c->sel];
    int x = w / 2 + 8, y = 120;
    ui_label(u, x, y, app, t->text); y += 28;
    compat_claim cl; char shims[256];
    cc_describe(c, app, &cl, shims, sizeof shims);
    char b[160];
    snprintf(b, sizeof b, "Claimed version: %s", era_label[cl.era]);
    ui_label(u, x, y, b, t->accent); y += 22;
    snprintf(b, sizeof b, "Why: %s", cl.reason);
    ui_label(u, x, y, b, t->text_dim); y += 28;
    int era = (int)cl.era;
    for (int i = 0; i < COMPAT_ERA_COUNT; i++, y += 24)
        if (ui_radio(u, (shell_rect){ x, y, 200, 22 }, era_label[i], &era, i)) cc_set_claim(c, app, (compat_era)i);
    if (ui_button(u, (shell_rect){ x, y + 4, 200, 28 }, "Use automatic version")) cc_clear_claim(c, app);
    y += 44;
    ui_label(u, x, y, "Shims applied:", t->text_dim); y += 20;
    char *line = shims;
    while (*line) {
        char *nl = strchr(line, '\n');
        if (nl) *nl = 0;
        ui_label(u, x + 8, y, line, t->text); y += 18;
        if (!nl) break;
        line = nl + 1;
    }
    y += 10;
    ui_label(u, x, y, "Import redirects:", t->text_dim); y += 20;
    ui_list(u, (shell_rect){ x, y, w - x - 16, 66 }, "redirects", 8, 22, row_redirect, c, &c->redir_sel, &c->redir_scroll);
    y += 74;
    ui_textfield(u, (shell_rect){ x, y, (w - x - 24) / 2, 26 }, c->r_from, sizeof c->r_from, &c->r_from_caret, "from dll!symbol");
    ui_textfield(u, (shell_rect){ x + (w - x - 24) / 2 + 8, y, (w - x - 24) / 2, 26 }, c->r_to, sizeof c->r_to, &c->r_to_caret, "to dll!symbol");
    if (ui_button_accent(u, (shell_rect){ x, y + 32, 140, 28 }, "Add redirect")) {
        if (cc_add_redirect(c, app, c->r_from, c->r_to)) { c->r_from[0] = c->r_to[0] = 0; c->r_from_caret = c->r_to_caret = 0; }
    }
    ui_label(u, 16, h - 18, c->status, t->text_dim);
}
