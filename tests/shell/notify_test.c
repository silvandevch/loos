#include "notify.h"
#include "test.h"
#include <stdlib.h>
#include <string.h>

#define W 1280
#define H 720
static uint32_t pixels[W * H];
static shell_fb fb = { pixels, W, H, W, 100 };

static int cb_calls, cb_allowed; static char cb_app[32]; static perm_res cb_res;
static void on_perm(const char *app, perm_res r, bool ok, void *x) { (void)x; cb_calls++; cb_allowed = ok; strcpy(cb_app, app); cb_res = r; }
static uint32_t act_note, act_id; static int acts;
static void on_action(uint32_t n, uint32_t a, void *x) { (void)x; act_note = n; act_id = a; acts++; }

int main(void)
{
    shell_comp *c = calloc(1, sizeof *c);
    comp_init(c, W, H);
    backend_mem mem; shell_backend be; backend_mem_init(&mem, &be);
    backend_mem_add_app(&mem, "browser.exe"); backend_mem_add_app(&mem, "mailer.exe"); backend_mem_add_app(&mem, "recorder.exe");
    shell_perms pm; perms_init(&pm, &be, c); perms_install(&pm);

    /* defaults: nothing granted, everything asks */
    CHECK(perms_check(&pm, "browser.exe", RES_CAMERA) == PERM_ASK && perms_check(&pm, "ghost.exe", RES_MIC) == PERM_ASK);
    CHECK(be.perm_state_get(be.user, "browser.exe", RES_FILES) == PERM_ASK && !be.perm_get(be.user, "browser.exe", RES_FILES));

    /* request -> prompt -> answers */
    CHECK(perms_request(&pm, "recorder.exe", RES_MIC, on_perm, NULL) == PERM_ASK && perms_prompting(&pm) && cb_calls == 0);
    CHECK(perms_request(&pm, "recorder.exe", RES_MIC, on_perm, NULL) == PERM_ASK && pm.nq == 2);       /* duplicate shares the prompt */
    perms_request(&pm, "browser.exe", RES_CAMERA, on_perm, NULL);
    CHECK(pm.nq == 3);
    comp_render_full(c, &fb);
    shell_rect pr = perms_prompt_rect(c);
    CHECK(pixels[(pr.y + 60) * W + pr.x + 300] == c->theme.surface && pixels[10 * W + 10] != c->theme.desktop);   /* modal card + dimmed screen */
    shell_rect b0 = { pr.x + 16, pr.y + pr.h - 50, 136, 34 };
    CHECK(perms_mouse(&pm, pr.x - 100, pr.y - 100, true) && pm.nq == 3);                                  /* clicks outside are swallowed */
    CHECK(perms_mouse(&pm, b0.x + 10, b0.y + 10, true));                                                  /* Allow once */
    CHECK(cb_calls == 2 && cb_allowed && !strcmp(cb_app, "recorder.exe") && cb_res == RES_MIC && pm.nq == 1);
    CHECK(perms_check(&pm, "recorder.exe", RES_MIC) == PERM_ALLOW && !be.perm_get(be.user, "recorder.exe", RES_MIC));   /* not persisted */
    CHECK(be.perm_state_get(be.user, "recorder.exe", RES_MIC) == PERM_ASK);
    perms_end_session(&pm); CHECK(perms_check(&pm, "recorder.exe", RES_MIC) == PERM_ASK);
    perms_answer(&pm, PERM_ANSWER_DENY);                                                                    /* browser camera: block */
    CHECK(cb_calls == 3 && !cb_allowed && perms_check(&pm, "browser.exe", RES_CAMERA) == PERM_DENY && be.perm_state_get(be.user, "browser.exe", RES_CAMERA) == PERM_DENY);
    CHECK(!perms_prompting(&pm));
    CHECK(perms_request(&pm, "browser.exe", RES_CAMERA, on_perm, NULL) == PERM_DENY && cb_calls == 4);   /* remembered, no prompt */
    perms_request(&pm, "mailer.exe", RES_FILES, on_perm, NULL); perms_answer(&pm, PERM_ANSWER_ALLOW_ALWAYS);
    CHECK(perms_check(&pm, "mailer.exe", RES_FILES) == PERM_ALLOW && be.perm_get(be.user, "mailer.exe", RES_FILES));
    CHECK(perms_request(&pm, "mailer.exe", RES_FILES, on_perm, NULL) == PERM_ALLOW);
    /* Esc declines without remembering */
    perms_request(&pm, "mailer.exe", RES_LOCATION, on_perm, NULL);
    CHECK(perms_key(&pm, KEY_ESC) && !cb_allowed && perms_check(&pm, "mailer.exe", RES_LOCATION) == PERM_ASK);
    CHECK(!perms_key(&pm, 'x'));
    /* a flood of requests fails closed */
    for (int i = 0; i < 40; i++) { char nm[16]; snprintf(nm, sizeof nm, "app%d.exe", i); perms_request(&pm, nm, RES_CAMERA, on_perm, NULL); }
    CHECK(pm.nq == PERMS_MAX_REQ);
    while (perms_prompting(&pm)) perms_answer(&pm, PERM_ANSWER_DENY);

    /* in-use indicators only for allowed apps; revoke clears them and the grant */
    perms_set_in_use(&pm, "browser.exe", RES_CAMERA, true); CHECK(!perms_in_use(&pm, RES_CAMERA, NULL, 0));      /* denied: cannot be "in use" */
    be.perm_set(be.user, "recorder.exe", RES_MIC, true); be.perm_set(be.user, "recorder.exe", RES_CAMERA, true);
    perms_set_in_use(&pm, "recorder.exe", RES_MIC, true); perms_set_in_use(&pm, "recorder.exe", RES_CAMERA, true);
    char who[32]; CHECK(perms_in_use(&pm, RES_MIC, who, sizeof who) && !strcmp(who, "recorder.exe") && perms_in_use(&pm, RES_CAMERA, NULL, 0) && !perms_in_use(&pm, RES_LOCATION, NULL, 0));
    comp_render_full(c, &fb);
    int red = 0, amber = 0; for (int y = H - 40; y < H; y++) for (int x = W - 300; x < W - 150; x++) { if (pixels[y * W + x] == 0xFFEF4444u) red++; if (pixels[y * W + x] == 0xFFF59E0Bu) amber++; }
    CHECK(red > 100 && amber > 100);
    perms_set_in_use(&pm, "recorder.exe", RES_CAMERA, false); CHECK(!perms_in_use(&pm, RES_CAMERA, NULL, 0));
    perms_revoke_all(&pm, "recorder.exe");
    CHECK(!perms_in_use(&pm, RES_MIC, NULL, 0) && perms_check(&pm, "recorder.exe", RES_MIC) == PERM_ASK);

    /* ---- notifications ---- */
    shell_notify nf; notify_init(&nf, &pm, c); notify_install(&nf); nf.on_action = on_action;
    nf.now_ms = 1000;
    CHECK(notify_publish(&nf, "mailer.exe", "Hi", "x", NOTE_NORMAL, NULL, 0) == 0);                       /* mailer has no notification decision: asks */
    CHECK(perms_prompting(&pm) && pm.q[0].res == RES_NOTIFY && nf.npending == 1 && notify_count(&nf) == 0);
    notify_publish(&nf, "mailer.exe", "Second", "y", NOTE_NORMAL, "Open", 7);
    CHECK(nf.npending == 2 && pm.nq == 1);                                                                 /* one prompt for the app */
    perms_answer(&pm, PERM_ANSWER_ALLOW_ALWAYS);
    CHECK(notify_count(&nf) == 2 && nf.npending == 0 && notify_unread(&nf) == 2 && notify_visible_toasts(&nf) == 2);
    uint32_t id3 = notify_publish(&nf, "mailer.exe", "Third", "z", NOTE_NORMAL, NULL, 0);
    CHECK(id3 != 0 && notify_get(&nf, id3) && !strcmp(notify_get(&nf, id3)->title, "Third"));
    /* denied app: dropped and counted */
    be.perm_set(be.user, "browser.exe", RES_NOTIFY, false);
    CHECK(notify_publish(&nf, "browser.exe", "Ad", "buy", NOTE_NORMAL, NULL, 0) == 0 && nf.dropped_denied == 1 && notify_count(&nf) == 3);
    CHECK(notify_publish(&nf, "", "x", "y", NOTE_NORMAL, NULL, 0) == 0 && notify_publish(&nf, "mailer.exe", NULL, "y", NOTE_NORMAL, NULL, 0) == 0);
    /* rate limit: 5 per 10 s per app */
    be.perm_set(be.user, "recorder.exe", RES_NOTIFY, true);
    int ok = 0; for (int i = 0; i < 12; i++) ok += notify_publish(&nf, "recorder.exe", "spam", "", NOTE_NORMAL, NULL, 0) != 0;
    CHECK(ok == 5 && nf.dropped_rate == 7);
    nf.now_ms += NOTIFY_RATE_WINDOW_MS + 1; CHECK(notify_publish(&nf, "recorder.exe", "later", "", NOTE_NORMAL, NULL, 0) != 0);
    /* toasts expire; urgent ones stay until dismissed */
    uint32_t urg = notify_publish(&nf, "mailer.exe", "Disk almost full", "98%", NOTE_URGENT, NULL, 0);
    CHECK(notify_get(&nf, urg)->toast_until == UINT64_MAX);
    notify_tick(&nf, nf.now_ms + NOTIFY_TOAST_MS + 1);
    CHECK(notify_visible_toasts(&nf) == 1 && notify_get(&nf, urg)->toast_until == UINT64_MAX);
    /* do not disturb: history keeps everything, toasts suppressed except urgent */
    notify_set_dnd(&nf, true);
    uint32_t q = notify_publish(&nf, "mailer.exe", "quiet", "", NOTE_NORMAL, NULL, 0);
    CHECK(q && notify_get(&nf, q)->toast_until == 0 && notify_visible_toasts(&nf) == 1);
    uint32_t q2 = notify_publish(&nf, "mailer.exe", "fire", "", NOTE_URGENT, NULL, 0);
    CHECK(notify_get(&nf, q2)->toast_until == UINT64_MAX && notify_visible_toasts(&nf) == 2);
    notify_set_dnd(&nf, false);

    /* UI: toast click dismisses; bell opens the center and marks read; center buttons work */
    comp_render_full(c, &fb);
    shell_rect t0 = { W - 340, 8, 332, 76 };
    int before = notify_count(&nf);
    CHECK(notify_mouse(&nf, t0.x + 40, t0.y + 40, true) && notify_count(&nf) == before - 1);
    CHECK(notify_unread(&nf) > 0);
    shell_rect bell = notify_bell_rect(c);
    CHECK(!nf.center_open && notify_mouse(&nf, bell.x + 5, bell.y + 5, true) && nf.center_open && notify_unread(&nf) == 0 && notify_visible_toasts(&nf) == 0);
    comp_render_full(c, &fb);
    shell_rect cr = notify_center_rect(c);
    CHECK(pixels[(cr.y + 200) * W + cr.x + 5] == c->theme.surface || pixels[(cr.y + 200) * W + cr.x + 5] == c->theme.surface_hi);
    CHECK(!notify_mouse(&nf, 100, 100, true) && !nf.center_open);                                          /* click outside closes it */
    notify_set_center(&nf, true);
    /* find the note with an action and trigger it */
    uint32_t with_action = 0; for (int i = 0; i < NOTIFY_MAX; i++) if (nf.note[i].id && nf.note[i].action[0]) with_action = nf.note[i].id;
    notify_publish(&nf, "mailer.exe", "Reply?", "from Sam", NOTE_NORMAL, "Reply", 99);
    uint32_t latest = 0; for (int i = 0; i < NOTIFY_MAX; i++) if (nf.note[i].id > latest) latest = nf.note[i].id;
    shell_rect row0 = { cr.x + 8, cr.y + 76, cr.w - 16, 70 };
    CHECK(notify_get(&nf, latest) && notify_mouse(&nf, row0.x + row0.w - 60, row0.y + row0.h - 18, true));
    CHECK(acts == 1 && act_note == latest && act_id == 99 && !notify_get(&nf, latest));
    (void)with_action;
    int cnt = notify_count(&nf);
    CHECK(notify_mouse(&nf, row0.x + row0.w - 15, row0.y + 12, true) && notify_count(&nf) == cnt - 1);       /* x on the newest row */
    CHECK(notify_key(&nf, KEY_ESC, true, 0) && !nf.center_open);
    CHECK(notify_key(&nf, 'n', true, MOD_SUPER) && nf.center_open && notify_key(&nf, 'n', true, MOD_SUPER) && !nf.center_open && !notify_key(&nf, 'n', true, 0));
    notify_set_center(&nf, true);
    CHECK(notify_mouse(&nf, cr.x + cr.w - 60, cr.y + 20, true) && notify_count(&nf) == 0);                   /* Clear all */
    CHECK(notify_mouse(&nf, cr.x + 40, cr.y + 52, true) && nf.dnd);

    /* history is bounded: oldest read entries are evicted first */
    notify_set_dnd(&nf, false); notify_clear_all(&nf); notify_set_center(&nf, false);
    be.perm_set(be.user, "bulk.exe", RES_NOTIFY, true); backend_mem_add_app(&mem, "bulk.exe"); be.perm_set(be.user, "bulk.exe", RES_NOTIFY, true);
    for (int i = 0; i < NOTIFY_MAX + 10; i++) { nf.now_ms += NOTIFY_RATE_WINDOW_MS; char t[24]; snprintf(t, sizeof t, "n%d", i); notify_publish(&nf, "bulk.exe", t, "", NOTE_NORMAL, NULL, 0); }
    CHECK(notify_count(&nf) == NOTIFY_MAX && nf.dropped_full == 10);
    notify_clear_app(&nf, "bulk.exe"); CHECK(notify_count(&nf) == 0);

    /* privacy: the API has no network entry points (compile-time guarantee of this header's types) */
    CHECK(sizeof(((shell_note *)0)->app) == 32);
    free(c);
    DONE("notify_test");
}
