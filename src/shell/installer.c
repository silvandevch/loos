#include "installer.h"
#include <stdio.h>
#include <string.h>

/* ---------------------------------------------------------------------------------------------- validation */
bool installer_valid_username(const char *s)
{
    size_t n = strlen(s);
    if (n < 1 || n > 31 || s[0] < 'a' || s[0] > 'z') return false;
    for (size_t i = 0; i < n; i++) {
        char c = s[i];
        if (!((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_' || c == '-')) return false;
    }
    return strcmp(s, "root") != 0 && strcmp(s, "system") != 0 && strcmp(s, "admin") != 0;
}

bool installer_valid_hostname(const char *s)
{
    size_t n = strlen(s);
    if (n < 1 || n > 31 || s[0] == '-' || s[n - 1] == '-') return false;
    for (size_t i = 0; i < n; i++) {
        char c = s[i];
        if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '-')) return false;
    }
    return true;
}

const char *installer_password_problem(const char *pw, const char *pw2)
{
    size_t n = strlen(pw);
    if (n < 8) return "Use at least 8 characters.";
    if (strcmp(pw, pw2) != 0) return "The passwords do not match.";
    bool digit = false, alpha = false, other = false;
    for (size_t i = 0; i < n; i++) {
        if (pw[i] >= '0' && pw[i] <= '9') digit = true;
        else if ((pw[i] | 32) >= 'a' && (pw[i] | 32) <= 'z') alpha = true;
        else other = true;
    }
    if (!(digit + alpha + other >= 2)) return "Mix letters with digits or symbols.";
    return NULL;
}

void installer_suggest_username(const char *display, char *out, size_t cap)
{
    size_t n = 0;
    for (; *display && n + 1 < cap && n < 31; display++) {
        char c = *display;
        if (c >= 'A' && c <= 'Z') c = (char)(c + 32);
        if ((c >= 'a' && c <= 'z') || (n > 0 && ((c >= '0' && c <= '9') || c == '-' || c == '_'))) out[n++] = c;
        else if (c == ' ' && n > 0) break;                                   /* first name only */
    }
    out[n] = 0;
}

/* ---------------------------------------------------------------------------------------------- state */
void installer_init(shell_installer *in, const shell_inst_backend *be)
{
    memset(in, 0, sizeof *in);
    in->be = be;
    in->page = IP_WELCOME;
    in->sel_disk = -1;
    in->net_blocked = true;                         /* private by default: the network stays off until the user allows it */
    strcpy(in->host, "loos-pc");
    in->c_host = (int)strlen(in->host);
}

static void refresh_disks(shell_installer *in)
{
    in->ndisks = in->be->list_disks ? in->be->list_disks(in->be->user, in->disks, INST_MAX_DISKS) : 0;
    if (in->ndisks < 0) in->ndisks = 0;
    if (in->sel_disk >= in->ndisks || (in->sel_disk >= 0 && (in->disks[in->sel_disk].boot_media || in->disks[in->sel_disk].too_small))) in->sel_disk = -1;
    if (in->sel_disk < 0)
        for (int i = 0; i < in->ndisks; i++) if (!in->disks[i].boot_media && !in->disks[i].too_small) { in->sel_disk = i; break; }
}

static bool account_ok(const shell_installer *in)
{
    return in->display[0] && installer_valid_username(in->user) && installer_valid_hostname(in->host) && !installer_password_problem(in->pw, in->pw2);
}

/* ---------------------------------------------------------------------------------------------- drawing */
#define COL_DANGER 0xFFEF4444u

static void disk_row(ui_ctx *u, shell_rect row, int i, bool selected, void *user)
{
    shell_installer *in = user;
    const inst_disk *d = &in->disks[i];
    const shell_theme *t = u->theme;
    bool usable = !d->boot_media && !d->too_small;
    uint32_t fg = usable ? (selected ? t->accent_text : t->text) : t->text_dim;
    char size[24]; fmt_size(d->bytes, size, sizeof size);
    ui_label(u, row.x + 14, row.y + 6, d->name, fg);
    ui_label(u, row.x + 14, row.y + 26, d->model[0] ? d->model : "Unknown model", selected && usable ? t->accent_text : t->text_dim);
    ui_label_right(u, row.x + row.w - 14, row.y + 6, size, fg);
    const char *note = d->boot_media ? "Installation media" : d->too_small ? "Too small (needs 2 GB)" : "";
    if (note[0]) ui_label_right(u, row.x + row.w - 14, row.y + 26, note, d->boot_media ? t->text_dim : COL_DANGER);
}

static bool button_disabled(ui_ctx *u, shell_rect r, const char *label)
{
    fb_fill_round(u->fb, u->clip, r, 6, 0x28FFFFFFu);
    fb_text(u->fb, u->clip, r.x + (r.w - shell_text_width(label)) / 2, r.y + (r.h - 16) / 2, label, u->theme->text_dim);
    return false;
}

static const char *const step_names[5] = { "Welcome", "Disk", "Account", "Privacy", "Review" };

static void field(ui_ctx *u, int x, int y, int w, const char *label, char *buf, size_t cap, int *caret, const char *ph, bool password, const char *problem)
{
    ui_label(u, x, y, label, u->theme->text_dim);
    shell_rect r = { x, y + 20, w, 30 };
    if (password) ui_textfield_password(u, r, buf, cap, caret, ph); else ui_textfield(u, r, buf, cap, caret, ph);
    if (problem) ui_label(u, x + 4, y + 54, problem, COL_DANGER);
}

void installer_frame(shell_installer *in, ui_ctx *u, int w, int h)
{
    const shell_theme *t = u->theme;
    in->frames++;
    int cw = w - 48 < 860 ? w - 48 : 860, ch = h - 48 < 580 ? h - 48 : 580;
    shell_rect card = { (w - cw) / 2, (h - ch) / 2, cw, ch };
    /* the surface persists between frames: paint the wallpaper once per size/scale, afterwards only under the card and its shadow */
    uint32_t key = (uint32_t)w * 7919u ^ (uint32_t)h * 104729u ^ (uint32_t)(u->fb->scale ? u->fb->scale : 100) * 31u ^ 0x5eed0000u;
    if (in->bg_key != key) { ui_backdrop(u, w, h); in->bg_key = key; }
    else { shell_rect saved; ui_clip_push(u, (shell_rect){ card.x - 70, card.y - 70, card.w + 140, card.h + 170 }, &saved); ui_backdrop(u, w, h); ui_clip_pop(u, saved); }
    fb_shadow(u->fb, u->clip, card, 14, 36, 10, 0x80000000u);
    fb_fill_round(u->fb, u->clip, card, 14, 0xFF111826u);
    fb_stroke_round(u->fb, u->clip, card, 14, 1, 0xFF2A354Au);

    int side_w = 200;
    bool wizard = in->page <= IP_REVIEW;
    shell_rect body = { card.x + (wizard ? side_w : 0), card.y, card.w - (wizard ? side_w : 0), card.h };
    if (wizard) {                                                              /* step list */
        shell_rect side = { card.x, card.y, side_w, card.h };
        fb_fill_round(u->fb, u->clip, side, 14, 0xFF0E1420u);
        fb_fill(u->fb, u->clip, (shell_rect){ side.x + side.w - 20, side.y, 20, side.h }, 0xFF0E1420u);
        fb_fill(u->fb, u->clip, (shell_rect){ side.x + side.w - 1, side.y + 14, 1, side.h - 28 }, 0xFF1F2940u);
        fb_text_ex(u->fb, u->clip, side.x + 24, side.y + 26, "LoOS", 0xFFF3F4F6u, (shell_font){ &loos_font_semibold, 22 });
        fb_fill_circle(u->fb, u->clip, side.x + 24 + shell_text_width_ex("LoOS", (shell_font){ &loos_font_semibold, 22 }) + 7, side.y + 26 + 20, 3, 0xFF3B82F6u);
        fb_text(u->fb, u->clip, side.x + 24, side.y + 56, "Setup", t->text_dim);
        for (int i = 0; i < 5; i++) {
            int y = side.y + 110 + i * 40;
            bool cur = (int)in->page == i, done = (int)in->page > i;
            fb_fill_circle(u->fb, u->clip, side.x + 34, y + 8, 11, cur ? t->accent : done ? 0xFF1E3A8A : 0xFF1F2940u);
            char num[4] = { (char)('1' + i), 0 };
            fb_text(u->fb, u->clip, side.x + 34 - shell_text_width(done ? "\xe2\x9c\x93" : num) / 2, y, done ? "\xe2\x9c\x93" : num, cur || done ? 0xFFFFFFFFu : t->text_dim);
            fb_text_ex(u->fb, u->clip, side.x + 58, y, step_names[i], cur ? 0xFFF3F4F6u : done ? 0xFFC8D4F0u : t->text_dim, cur ? (shell_font){ &loos_font_semibold, 14 } : SHELL_FONT_BODY);
        }
    }
    int bx = body.x + 32, by = body.y + 28, bw = body.w - 64;
    int foot_y = card.y + card.h - 64;
    bool next = false, back = false;

    switch (in->page) {
    case IP_WELCOME: {
        ui_heading(u, bx, by, "Welcome to LoOS");
        ui_label(u, bx, by + 52, "A private, local-only operating system.", t->text);
        static const char *const points[] = {
            "No telemetry, ever. Nothing is collected, nothing phones home.",
            "No cloud account. Your user lives on this computer only.",
            "The network is off until you allow it, and the tray LED always tells the truth.",
            "Windows and Linux programs run natively; your files stay yours." };
        for (int i = 0; i < 4; i++) {
            fb_fill_circle(u->fb, u->clip, bx + 5, by + 106 + i * 32 + 8, 4, 0xFF3B82F6u);
            ui_label(u, bx + 20, by + 106 + i * 32, points[i], t->text_dim);
        }
        ui_label(u, bx, by + 260, "This installer erases the disk you choose and sets LoOS up on it.", t->text_dim);
        if (ui_button_accent(u, (shell_rect){ bx, foot_y, 170, 38 }, "Install LoOS")) { refresh_disks(in); in->page = IP_DISK; }
        if (ui_button(u, (shell_rect){ bx + 186, foot_y, 230, 38 }, "Try LoOS without installing")) in->live_requested = true;
        break;
    }
    case IP_DISK: {
        ui_heading(u, bx, by, "Choose a disk");
        ui_label(u, bx, by + 50, "LoOS will be installed on the selected disk.", t->text_dim);
        shell_rect list = { bx, by + 84, bw, 56 * (in->ndisks > 4 ? 4 : in->ndisks > 0 ? in->ndisks : 1) + 4 };
        fb_fill_round(u->fb, u->clip, list, 8, 0xFF0E1420u);
        if (in->ndisks == 0) ui_label(u, list.x + 16, list.y + 18, "No disks found. Attach a disk (IDE/SATA in legacy mode) and restart.", COL_DANGER);
        else {
            int pick = in->sel_disk;
            ui_list(u, list, "installer-disks", in->ndisks, 56, disk_row, in, &pick, &in->disk_scroll);
            if (pick != in->sel_disk && pick >= 0 && pick < in->ndisks && !in->disks[pick].boot_media && !in->disks[pick].too_small) in->sel_disk = pick;
        }
        ui_label(u, bx, list.y + list.h + 16, "Everything on the selected disk will be erased.", COL_DANGER);
        if (ui_button(u, (shell_rect){ bx, foot_y, 100, 38 }, "Back")) back = true;
        bool ok = in->sel_disk >= 0;
        if (ok ? ui_button_accent(u, (shell_rect){ bx + bw - 130, foot_y, 130, 38 }, "Continue") : button_disabled(u, (shell_rect){ bx + bw - 130, foot_y, 130, 38 }, "Continue")) next = true;
        break;
    }
    case IP_ACCOUNT: {
        ui_heading(u, bx, by, "Create your account");
        int half = (bw - 20) / 2;
        const char *p_user = in->user[0] && !installer_valid_username(in->user) ? "Lowercase letters, digits, - and _." : NULL;
        const char *p_host = in->host[0] && !installer_valid_hostname(in->host) ? "Letters, digits and hyphens only." : NULL;
        const char *p_pw = in->pw[0] ? installer_password_problem(in->pw, in->pw2) : NULL;
        int y0 = by + 48;
        char before[48]; strcpy(before, in->display);
        field(u, bx, y0, half, "Your name", in->display, sizeof in->display, &in->c_display, "Ada Lovelace", false, NULL);
        if (strcmp(before, in->display) && !in->user_edited) { installer_suggest_username(in->display, in->user, sizeof in->user); in->c_user = (int)strlen(in->user); }
        char ub[32]; strcpy(ub, in->user);
        field(u, bx + half + 20, y0, half, "Username", in->user, sizeof in->user, &in->c_user, "ada", false, p_user);
        if (strcmp(ub, in->user)) in->user_edited = true;
        char hb[32]; strcpy(hb, in->host);
        field(u, bx, y0 + 84, half, "Computer name", in->host, sizeof in->host, &in->c_host, "loos-pc", false, p_host);
        if (strcmp(hb, in->host)) in->host_edited = true;
        field(u, bx, y0 + 168, half, "Password", in->pw, sizeof in->pw, &in->c_pw, "At least 8 characters", true, NULL);
        field(u, bx + half + 20, y0 + 168, half, "Confirm password", in->pw2, sizeof in->pw2, &in->c_pw2, "Repeat the password", true, p_pw && in->pw2[0] ? p_pw : NULL);
        if (p_pw && !in->pw2[0]) ui_label(u, bx + 4, y0 + 222, p_pw, u->theme->text_dim);
        ui_label(u, bx, y0 + 252, "The password never leaves this computer. It is stored only as a salted PBKDF2 hash.", t->text_dim);
        if (ui_button(u, (shell_rect){ bx, foot_y, 100, 38 }, "Back")) back = true;
        bool ok = account_ok(in);
        if (ok ? ui_button_accent(u, (shell_rect){ bx + bw - 130, foot_y, 130, 38 }, "Continue") : button_disabled(u, (shell_rect){ bx + bw - 130, foot_y, 130, 38 }, "Continue")) next = true;
        break;
    }
    case IP_PRIVACY: {
        ui_heading(u, bx, by, "Privacy");
        ui_label(u, bx, by + 50, "LoOS collects nothing. These two choices are yours:", t->text_dim);
        ui_toggle(u, (shell_rect){ bx, by + 100, bw, 28 }, "Keep the network blocked until I allow it", &in->net_blocked);
        ui_label(u, bx + 52, by + 130, "A hard switch in the taskbar turns it on whenever you want.", t->text_dim);
        ui_toggle(u, (shell_rect){ bx, by + 176, bw, 28 }, "Check for updates automatically", &in->updates);
        ui_label(u, bx + 52, by + 206, "Off by default. Updates come from a mirror you choose, never from us.", t->text_dim);
        ui_separator(u, bx, by + 252, bw);
        ui_label(u, bx, by + 268, "Telemetry, crash uploads and advertising identifiers do not exist in LoOS.", t->text_dim);
        if (ui_button(u, (shell_rect){ bx, foot_y, 100, 38 }, "Back")) back = true;
        if (ui_button_accent(u, (shell_rect){ bx + bw - 130, foot_y, 130, 38 }, "Continue")) next = true;
        break;
    }
    case IP_REVIEW: {
        ui_heading(u, bx, by, "Ready to install");
        const inst_disk *d = in->sel_disk >= 0 ? &in->disks[in->sel_disk] : NULL;
        char line[160], size[24] = "";
        if (d) fmt_size(d->bytes, size, sizeof size);
        shell_rect box = { bx, by + 54, bw, 188 };
        fb_fill_round(u->fb, u->clip, box, 8, 0xFF0E1420u);
        struct { const char *k; char v[120]; } rows[5];
        snprintf(rows[0].v, sizeof rows[0].v, "%s  %s", d ? d->name : "-", size);
        snprintf(rows[1].v, sizeof rows[1].v, "%s (%s)", in->display, in->user);
        snprintf(rows[2].v, sizeof rows[2].v, "%s", in->host);
        snprintf(rows[3].v, sizeof rows[3].v, "%s", in->net_blocked ? "Blocked until you allow it" : "Allowed");
        snprintf(rows[4].v, sizeof rows[4].v, "%s", in->updates ? "Automatic checks on" : "Off (opt-in)");
        static const char *const keys[5] = { "Install on", "Account", "Computer name", "Network", "Updates" };
        for (int i = 0; i < 5; i++) { ui_label(u, box.x + 18, box.y + 16 + i * 32, keys[i], t->text_dim); ui_label(u, box.x + 170, box.y + 16 + i * 32, rows[i].v, t->text); }
        snprintf(line, sizeof line, "All data on %s will be permanently erased.", d ? d->name : "the disk");
        ui_label(u, bx, box.y + box.h + 18, line, COL_DANGER);
        if (ui_button(u, (shell_rect){ bx, foot_y, 100, 38 }, "Back")) back = true;
        if (ui_button_accent(u, (shell_rect){ bx + bw - 200, foot_y, 200, 38 }, "Erase disk and install")) {
            inst_config cfg;
            memset(&cfg, 0, sizeof cfg);
            cfg.disk = in->sel_disk; cfg.net_blocked = in->net_blocked; cfg.updates = in->updates;
            strncpy(cfg.display, in->display, sizeof cfg.display - 1);
            strncpy(cfg.username, in->user, sizeof cfg.username - 1);
            strncpy(cfg.hostname, in->host, sizeof cfg.hostname - 1);
            strncpy(cfg.password, in->pw, sizeof cfg.password - 1);
            in->err[0] = 0; in->percent = 0; strcpy(in->msg, "Preparing");
            if (in->be->begin && in->be->begin(in->be->user, &cfg, in->err, sizeof in->err)) in->page = IP_INSTALL;
            else { if (!in->err[0]) strcpy(in->err, "The installer could not start."); in->page = IP_ERROR; }
            memset(&cfg, 0, sizeof cfg);                                           /* the password does not linger in the wizard's stack */
        }
        break;
    }
    case IP_INSTALL: {
        ui_heading(u, bx, by, "Installing LoOS");
        ui_label(u, bx, by + 50, "Please keep the computer powered on.", t->text_dim);
        ui_progress(u, (shell_rect){ bx, by + 120, bw, 14 }, in->percent);
        char pct[16]; snprintf(pct, sizeof pct, "%d%%", in->percent);
        ui_label_right(u, bx + bw, by + 144, pct, t->text);
        ui_label(u, bx, by + 144, in->msg, t->text_dim);
        int rc = in->be->step ? in->be->step(in->be->user, &in->percent, in->msg, sizeof in->msg, in->err, sizeof in->err) : -1;
        if (rc > 0) { in->percent = 100; in->page = IP_DONE; }
        else if (rc < 0) { if (!in->err[0]) strcpy(in->err, "The installation failed."); in->page = IP_ERROR; }
        break;
    }
    case IP_DONE: {
        ui_heading(u, bx, by, "LoOS is installed");
        ui_label(u, bx, by + 54, "Remove the installation media, then restart to start LoOS.", t->text);
        char line[120]; snprintf(line, sizeof line, "Sign in as %s with the password you just chose.", in->user);
        ui_label(u, bx, by + 86, line, t->text_dim);
        if (ui_button_accent(u, (shell_rect){ bx, foot_y, 170, 38 }, "Restart now") && in->be->reboot) in->be->reboot(in->be->user);
        break;
    }
    case IP_ERROR: {
        ui_heading(u, bx, by, "Installation failed");
        ui_label(u, bx, by + 54, in->err, COL_DANGER);
        ui_label(u, bx, by + 86, "Nothing else was changed. You can go back and try again.", t->text_dim);
        if (ui_button_accent(u, (shell_rect){ bx, foot_y, 170, 38 }, "Back to review")) in->page = IP_REVIEW;
        break;
    }
    }
    if (next && in->page < IP_REVIEW) in->page = (inst_page)(in->page + 1);
    else if (back && in->page > IP_WELCOME) in->page = (inst_page)(in->page - 1);
}
