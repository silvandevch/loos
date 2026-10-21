#include "perms.h"
#include <string.h>
#include <stdio.h>

static void overlay(shell_comp *c, shell_fb *fb, shell_rect clip, void *u);

void perms_init(shell_perms *p, const shell_backend *be, shell_comp *comp) { memset(p, 0, sizeof *p); p->be = be; p->comp = comp; }
void perms_install(shell_perms *p) { if (p->comp) comp_add_overlay(p->comp, overlay, p); }

static bool has_once(const shell_perms *p, const char *app, perm_res r)
{
    for (int i = 0; i < p->nonce; i++) if (p->once[i].res == r && !strcmp(p->once[i].app, app)) return true;
    return false;
}

perm_state perms_check(shell_perms *p, const char *app, perm_res r)
{
    perm_state s = p->be->perm_state_get(p->be->user, app, r);
    if (s == PERM_ASK && has_once(p, app, r)) return PERM_ALLOW;
    return s;
}

static void dirty(shell_perms *p) { if (p->comp) comp_damage_all(p->comp); }

perm_state perms_request(shell_perms *p, const char *app, perm_res r,
                         void (*done)(const char *, perm_res, bool, void *), void *ctx)
{
    perm_state s = perms_check(p, app, r);
    if (s != PERM_ASK) { if (done) done(app, r, s == PERM_ALLOW, ctx); return s; }
    if (p->nq >= PERMS_MAX_REQ) { if (done) done(app, r, false, ctx); return PERM_DENY; }      /* flooded: fail closed */
    perm_request_t *q = &p->q[p->nq++];
    memset(q, 0, sizeof *q);
    strncpy(q->app, app, sizeof q->app - 1); q->res = r; q->done = done; q->ctx = ctx;
    if (p->nq == 1) p->prompts_shown++;
    dirty(p);
    return PERM_ASK;
}

bool perms_prompting(const shell_perms *p) { return p->nq > 0; }

/* resolve every queued request for the same app+resource as the head */
static void resolve_head(shell_perms *p, bool allowed)
{
    char app[32]; perm_res r = p->q[0].res;
    strcpy(app, p->q[0].app);
    perm_request_t done[PERMS_MAX_REQ]; int nd = 0;
    for (int i = 0; i < p->nq;) {
        if (p->q[i].res == r && !strcmp(p->q[i].app, app)) { done[nd++] = p->q[i]; memmove(&p->q[i], &p->q[i + 1], (size_t)(p->nq - i - 1) * sizeof p->q[0]); p->nq--; }
        else i++;
    }
    if (!allowed) p->denied_total += nd;
    for (int i = 0; i < nd; i++) if (done[i].done) done[i].done(app, r, allowed, done[i].ctx);
    if (p->nq) p->prompts_shown++;
    dirty(p);
}

void perms_answer(shell_perms *p, perm_answer a)
{
    if (!p->nq) return;
    const char *app = p->q[0].app; perm_res r = p->q[0].res;
    switch (a) {
    case PERM_ANSWER_ALLOW_ONCE:
        if (p->nonce < 32) { strcpy(p->once[p->nonce].app, app); p->once[p->nonce].res = r; p->nonce++; }
        resolve_head(p, true);
        break;
    case PERM_ANSWER_ALLOW_ALWAYS: p->be->perm_set(p->be->user, app, r, true); resolve_head(p, true); break;
    default: p->be->perm_set(p->be->user, app, r, false); resolve_head(p, false);
    }
}

void perms_revoke_all(shell_perms *p, const char *app)
{
    for (int r = 0; r < RES_COUNT; r++) p->be->perm_reset(p->be->user, app, (perm_res)r);
    for (int i = 0; i < p->nonce;) if (!strcmp(p->once[i].app, app)) p->once[i] = p->once[--p->nonce]; else i++;
    for (int i = 0; i < p->nuse;) if (!strcmp(p->use[i].app, app)) p->use[i] = p->use[--p->nuse]; else i++;   /* revoked apps lose their indicators and their access */
    dirty(p);
}
void perms_end_session(shell_perms *p) { p->nonce = 0; }

void perms_set_in_use(shell_perms *p, const char *app, perm_res r, bool active)
{
    for (int i = 0; i < p->nuse; i++)
        if (p->use[i].res == r && !strcmp(p->use[i].app, app)) {
            if (!active) { p->use[i] = p->use[--p->nuse]; dirty(p); }
            return;
        }
    if (active && p->nuse < PERMS_MAX_USE && perms_check(p, app, r) == PERM_ALLOW) {        /* only an allowed app can be "using" it */
        strncpy(p->use[p->nuse].app, app, 31); p->use[p->nuse].app[31] = 0; p->use[p->nuse].res = r; p->nuse++;
        dirty(p);
    }
}
bool perms_in_use(const shell_perms *p, perm_res r, char *app_out, size_t cap)
{
    for (int i = 0; i < p->nuse; i++) if (p->use[i].res == r) { if (app_out && cap) { strncpy(app_out, p->use[i].app, cap - 1); app_out[cap - 1] = 0; } return true; }
    return false;
}

/* ---- prompt UI ---- */
shell_rect perms_prompt_rect(const shell_comp *c) { return (shell_rect){ comp_lw(c) / 2 - 230, comp_lh(c) / 2 - 80, 460, 150 }; }
static shell_rect btn_rect(const shell_comp *c, int i)
{
    shell_rect r = perms_prompt_rect(c);
    return (shell_rect){ r.x + 16 + i * 146, r.y + r.h - 50, 136, 34 };
}

bool perms_mouse(shell_perms *p, int x, int y, bool down)
{
    if (!p->nq || !p->comp) return false;
    if (!down) return true;
    x = comp_logical(p->comp, x); y = comp_logical(p->comp, y);
    for (int i = 0; i < 3; i++)
        if (rect_contains(btn_rect(p->comp, i), x, y)) { perms_answer(p, i == 0 ? PERM_ANSWER_ALLOW_ONCE : i == 1 ? PERM_ANSWER_ALLOW_ALWAYS : PERM_ANSWER_DENY); return true; }
    return true;                                                              /* modal: swallow clicks elsewhere */
}
bool perms_key(shell_perms *p, uint32_t key)
{
    if (!p->nq) return false;
    if (key == KEY_ESC) { resolve_head(p, false); p->denied_total += 0; }      /* decline without remembering: it will ask again */
    return true;                                                              /* modal: nothing reaches the app behind it */
}

static void overlay(shell_comp *c, shell_fb *fb, shell_rect clip, void *u)
{
    shell_perms *p = u;
    shell_fb view = comp_view(c, fb); shell_rect lclip = comp_clip_logical(c, clip);
    fb = &view; clip = lclip;
    const shell_theme *t = &c->theme;
    /* in-use indicators sit left of the tray LED: camera, microphone, location */
    static const struct { perm_res r; uint32_t col; const char *tag; } ind[3] = { { RES_CAMERA, 0xFFEF4444u, "CAM" }, { RES_MIC, 0xFFF59E0Bu, "MIC" }, { RES_LOCATION, 0xFF10B981u, "LOC" } };
    int x = comp_lw(c) - 150 - 12;
    for (int i = 0; i < 3; i++) {
        if (!perms_in_use(p, ind[i].r, NULL, 0)) continue;
        x -= 44;
        shell_rect r = { x, comp_lh(c) - 40 + 8, 40, 24 };
        fb_fill_round(fb, clip, r, 6, ind[i].col);
        fb_text(fb, clip, r.x + 8, r.y + 4, ind[i].tag, 0xFFFFFFFFu);
    }
    if (!p->nq) return;
    shell_rect r = perms_prompt_rect(c);
    fb_fill(fb, clip, (shell_rect){ 0, 0, comp_lw(c), comp_lh(c) }, 0x66000000u);          /* dim everything behind the modal */
    fb_fill_round(fb, clip, (shell_rect){ r.x - 2, r.y - 2, r.w + 4, r.h + 4 }, 12, t->accent);
    fb_fill_round(fb, clip, r, 10, t->surface);
    char line[96];
    snprintf(line, sizeof line, "%s wants to use", p->q[0].app);
    fb_text(fb, clip, r.x + 20, r.y + 18, line, t->text);
    fb_text(fb, clip, r.x + 20, r.y + 42, perm_res_name[p->q[0].res], t->accent);
    int waiting = p->nq - 1;
    if (waiting) { snprintf(line, sizeof line, "%d more request(s) waiting", waiting); fb_text(fb, clip, r.x + 20, r.y + 68, line, t->text_dim); }
    static const char *const lb[3] = { "Allow once", "Always allow", "Block" };
    for (int i = 0; i < 3; i++) {
        shell_rect b = btn_rect(c, i);
        bool hot = rect_contains(b, c->mouse_x, c->mouse_y);
        fb_fill_round(fb, clip, b, 6, i == 2 ? (hot ? t->danger : color_mix(t->surface_hi, t->danger, 70)) : (hot ? color_mix(t->surface_hi, t->accent, 120) : t->surface_hi));
        fb_text(fb, clip, b.x + (b.w - shell_text_width(lb[i])) / 2, b.y + 9, lb[i], t->text);
    }
}
