#include "session.h"
#include "test.h"
#include <string.h>

#define W 900
#define H 600
static uint32_t pixels[W * H];
static shell_fb fb = { pixels, W, H, W, 100 };
static shell_theme th;
static ui_ctx u;
static int events[16], nev; static char last_user[32];
static void on_event(sess_event e, const char *user, void *c) { (void)c; if (nev < 16) events[nev++] = e; strcpy(last_user, user); }
static unsigned rng_calls;
static void rng(uint8_t *out, size_t n, void *x) { (void)x; for (size_t i = 0; i < n; i++) out[i] = (uint8_t)(0x40 + i * 7 + rng_calls * 31); rng_calls++; }
static void ev(shell_event_type t, int x, int y, int button, uint32_t key) { shell_event e = { t, x, y, button, key, 0 }; ui_feed(&u, &e); }
static void run(shell_login *l) { ui_begin(&u, &fb, &th, 100); login_frame(l, &u, W, H); ui_end(&u); }
static void click(shell_login *l, int x, int y) { u.now_ms += 1000; ev(EV_MOUSE_MOVE, x, y, 0, 0); run(l); ev(EV_MOUSE_DOWN, x, y, BTN_LEFT, 0); run(l); ev(EV_MOUSE_UP, x, y, BTN_LEFT, 0); run(l); }
static void typestr(shell_login *l, const char *s) { for (; *s; s++) ev(EV_CHAR, 0, 0, 0, (uint32_t)*s); run(l); }
static void key(shell_login *l, uint32_t k) { ev(EV_KEY_DOWN, 0, 0, 0, k); run(l); }

int main(void)
{
    shell_theme_dark(&th, 0xFF3B82F6u);
    shell_accounts db; accounts_init(&db, 2000);
    db.random = rng;
    uint32_t retry;

    /* names */
    CHECK(account_valid_name("silvan") && account_valid_name("a_b-9") && !account_valid_name("") && !account_valid_name("Bob") && !account_valid_name("a b") && !account_valid_name("../x"));
    char longname[40]; memset(longname, 'a', 39); longname[39] = 0; CHECK(!account_valid_name(longname));

    /* creation: first is admin, salts are random per account, hashes differ for equal passwords */
    CHECK(account_create(&db, "owner", "Owner", "hunter22", NULL, false));
    CHECK(db.acc[0].admin && db.acc[0].has_password && db.acc[0].iters == 2000);
    CHECK(account_create(&db, "guest", "Guest", "hunter22", NULL, false) && !db.acc[1].admin);
    CHECK(memcmp(db.acc[0].salt, db.acc[1].salt, 16) != 0 && memcmp(db.acc[0].hash, db.acc[1].hash, 32) != 0);
    CHECK(!account_create(&db, "owner", "dup", "x", NULL, false) && !account_create(&db, "BAD", "x", "x", NULL, false));
    CHECK(account_create(&db, "kiosk", "Kiosk", "", NULL, false) && !db.acc[2].has_password);
    shell_accounts nornd; accounts_init(&nornd, 1000);
    CHECK(!account_create(&nornd, "a", "A", "pw", NULL, true));                       /* refuses to invent a salt */
    uint8_t salt[16] = { 1, 2, 3 };
    CHECK(account_create(&nornd, "a", "A", "pw", salt, true) && !memcmp(nornd.acc[0].salt, salt, 16));

    /* verification */
    CHECK(account_verify(&db, "owner", "hunter22", 1000, &retry) == AUTH_OK);
    CHECK(account_verify(&db, "owner", "nope", 1000, &retry) == AUTH_BAD_PASSWORD && retry == 0);
    CHECK(account_verify(&db, "nobody", "x", 1000, &retry) == AUTH_NO_SUCH_USER);
    CHECK(account_verify(&db, "kiosk", "", 1000, &retry) == AUTH_OK && account_verify(&db, "kiosk", "x", 1000, &retry) == AUTH_BAD_PASSWORD);

    /* lockout: free tries, then 30 s, 60 s, ... capped; success resets; locked accounts do not even check the password */
    uint64_t now = 100000;
    for (int i = 0; i < ACC_FREE_TRIES - 1; i++) CHECK(account_verify(&db, "guest", "wrong", now, &retry) == AUTH_BAD_PASSWORD && retry == 0);
    CHECK(account_verify(&db, "guest", "wrong", now, &retry) == AUTH_BAD_PASSWORD && retry == ACC_LOCK_BASE_MS);
    CHECK(account_verify(&db, "guest", "hunter22", now + 1000, &retry) == AUTH_LOCKED_OUT && retry == ACC_LOCK_BASE_MS - 1000);   /* correct password still refused */
    CHECK(account_verify(&db, "guest", "wrong", now + ACC_LOCK_BASE_MS, &retry) == AUTH_BAD_PASSWORD && retry == 2 * ACC_LOCK_BASE_MS);
    now += ACC_LOCK_BASE_MS;
    CHECK(account_verify(&db, "guest", "hunter22", now + 2 * ACC_LOCK_BASE_MS, &retry) == AUTH_OK && db.acc[1].failures == 0);
    for (int i = 0; i < 40; i++) account_verify(&db, "guest", "wrong", now + 10000000ull * (unsigned)(i + 1), &retry);
    CHECK(retry == ACC_LOCK_MAX_MS);

    /* passwords */
    CHECK(!account_set_password(&db, "owner", "wrong", "new", NULL) && !account_set_password(&db, "owner", NULL, "new", NULL));
    CHECK(account_set_password(&db, "owner", "hunter22", "correct horse", NULL));
    CHECK(account_verify(&db, "owner", "correct horse", 5000000, &retry) == AUTH_OK && account_verify(&db, "owner", "hunter22", 5000000, &retry) == AUTH_BAD_PASSWORD);
    CHECK(account_set_password(&db, "kiosk", NULL, "now-protected", NULL) && db.acc[2].has_password);
    CHECK(account_set_password(&db, "kiosk", "now-protected", "", NULL) && !db.acc[2].has_password);
    char big[ACC_PW_MAX + 8]; memset(big, 'x', sizeof big - 1); big[sizeof big - 1] = 0;
    CHECK(!account_set_password(&db, "kiosk", NULL, big, NULL));

    /* deletion and the last administrator */
    CHECK(!account_delete(&db, "owner") && account_delete(&db, "guest") && db.n == 2 && !account_delete(&db, "ghost"));
    CHECK(account_create(&db, "admin2", "Second admin", "pw-pw", NULL, true) && account_delete(&db, "owner") && db.n == 2 && !account_delete(&db, "admin2"));
    CHECK(account_create(&db, "owner", "Owner", "hunter22", NULL, false));

    /* autologin */
    CHECK(accounts_set_autologin(&db, "kiosk") && !accounts_set_autologin(&db, "ghost") && !strcmp(db.autologin, "kiosk"));
    CHECK(accounts_set_autologin(&db, NULL) && !db.autologin[0]);
    accounts_set_autologin(&db, "kiosk");
    CHECK(account_create(&db, "temp", "Temp", "t", NULL, false) && accounts_set_autologin(&db, "temp") && account_delete(&db, "temp") && !db.autologin[0]);
    accounts_set_autologin(&db, "kiosk");

    /* persistence round trip + tamper resistance */
    static char saved[4096]; size_t n = accounts_save(&db, saved, sizeof saved);
    CHECK(n > 100 && strstr(saved, "autologin kiosk") && !strstr(saved, "hunter22") && !strstr(saved, "correct horse"));   /* no plaintext, ever */
    shell_accounts db2; accounts_init(&db2, 2000);
    CHECK(accounts_load(&db2, saved) == db.n && !strcmp(db2.autologin, "kiosk"));
    CHECK(account_verify(&db2, "admin2", "pw-pw", 1, &retry) == AUTH_OK && account_verify(&db2, "owner", "hunter22", 1, &retry) == AUTH_OK);
    CHECK(db2.acc[0].admin == db.acc[0].admin && !strcmp(db2.acc[0].display, db.acc[0].display));
    shell_accounts db3; accounts_init(&db3, 2000);
    CHECK(accounts_load(&db3, "account bad!name 1000 1 1 00 00 x\naccount ok 0 1 1 00112233445566778899aabbccddeeff 00 x\naccount short 1000 1 1 0011 00 x\ngarbage\n") == 0 && db3.n == 0);

    /* session state machine */
    shell_session s; session_init(&s, &db); s.on_event = on_event;
    CHECK(session_boot(&s, 10) && s.state == SESS_ACTIVE && !strcmp(s.user, "kiosk") && events[0] == SESS_EV_LOGIN);
    session_logout(&s); CHECK(s.state == SESS_SIGNED_OUT && !s.user[0] && events[1] == SESS_EV_LOGOUT);
    session_logout(&s); CHECK(nev == 2);
    CHECK(session_login(&s, "owner", "bad", 100, &retry) == AUTH_BAD_PASSWORD && s.state == SESS_SIGNED_OUT && nev == 2);
    CHECK(session_login(&s, "owner", "hunter22", 100, &retry) == AUTH_OK && s.state == SESS_ACTIVE && s.login_count == 2);
    session_lock(&s); CHECK(s.state == SESS_LOCKED && events[nev - 1] == SESS_EV_LOCK);
    CHECK(session_unlock(&s, "bad", 200, &retry) == AUTH_BAD_PASSWORD && s.state == SESS_LOCKED);
    CHECK(session_unlock(&s, "hunter22", 300, &retry) == AUTH_OK && s.state == SESS_ACTIVE && events[nev - 1] == SESS_EV_UNLOCK);
    CHECK(session_unlock(&s, "hunter22", 300, &retry) == AUTH_NO_SUCH_USER);
    s.idle_lock_ms = 60000; session_activity(&s, 1000);
    session_tick(&s, 30000); CHECK(s.state == SESS_ACTIVE);
    session_activity(&s, 50000); session_tick(&s, 100000); CHECK(s.state == SESS_ACTIVE);
    session_tick(&s, 111000); CHECK(s.state == SESS_LOCKED && events[nev - 1] == SESS_EV_AUTOLOCK);
    session_tick(&s, 999999); CHECK(nev < 16);
    CHECK(session_unlock(&s, "hunter22", 1000000, &retry) == AUTH_OK);
    shell_session s0; shell_accounts none; accounts_init(&none, 2000); session_init(&s0, &none);
    CHECK(!session_boot(&s0, 0) && s0.state == SESS_SIGNED_OUT);

    /* login screen: first-run setup */
    shell_accounts fresh; accounts_init(&fresh, 2000); fresh.random = rng;
    shell_session fs; session_init(&fs, &fresh);
    shell_login l; login_init(&l, &fs);
    memset(&u, 0, sizeof u);
    run(&l);
    int cx = W / 2 - 200, cy = H / 2 - 150;
    click(&l, cx + 100, cy + 85); typestr(&l, "me");
    click(&l, cx + 100, cy + 125); typestr(&l, "My Name");
    click(&l, cx + 100, cy + 165); typestr(&l, "secret1");
    click(&l, cx + 100, cy + 205); typestr(&l, "secret2");
    click(&l, cx + 100, cy + 257);
    CHECK(fresh.n == 0 && strstr(l.msg, "do not match"));
    click(&l, cx + 100, cy + 205); key(&l, KEY_END);
    for (int i = 0; i < 8; i++) key(&l, KEY_BACKSPACE);
    typestr(&l, "secret1");
    click(&l, cx + 100, cy + 257);
    CHECK(fresh.n == 1 && fresh.acc[0].admin && !strcmp(fresh.acc[0].display, "My Name") && account_verify(&fresh, "me", "secret1", 1, &retry) == AUTH_OK);
    CHECK(!l.su_pw[0] && !l.su_pw2[0]);                                              /* the setup form wipes the password buffers */

    /* login screen: wrong password, lockout message, success */
    memset(&u, 0, sizeof u);
    account_create(&fresh, "pal", "Pal", "palpass", NULL, false);
    l.now_ms = 10000; l.hour = 9; l.minute = 5;
    run(&l);
    CHECK(l.sel == 0);
    int tx = W / 2 - (2 * 132 - 12) / 2;
    click(&l, tx + 132 + 60, 170 + 50); CHECK(l.sel == 1);                           /* pick the second tile */
    click(&l, tx + 60, 170 + 50); CHECK(l.sel == 0);
    int py = 170 + 130;
    click(&l, W / 2, py + 17); typestr(&l, "wrongpw"); key(&l, KEY_ENTER);
    CHECK(fs.state == SESS_SIGNED_OUT && !strcmp(l.msg, "Wrong password.") && !l.pw[0]);
    for (int i = 0; i < 4; i++) { click(&l, W / 2, py + 17); typestr(&l, "again"); key(&l, KEY_ENTER); }
    CHECK(strstr(l.msg, "Try again in 30 s") != NULL);
    click(&l, W / 2, py + 17); typestr(&l, "secret1"); key(&l, KEY_ENTER);
    CHECK(fs.state == SESS_SIGNED_OUT && strstr(l.msg, "Too many attempts"));
    l.now_ms += 31000;
    click(&l, W / 2, py + 17); typestr(&l, "secret1"); key(&l, KEY_ENTER);
    CHECK(fs.state == SESS_ACTIVE && !strcmp(fs.user, "me") && !l.msg[0]);
    /* locked: only the current user is offered */
    session_lock(&fs); run(&l);
    CHECK(l.sel == 0);
    click(&l, W / 2, py + 17); typestr(&l, "secret1"); click(&l, W / 2, py + 44 + 17);
    CHECK(fs.state == SESS_ACTIVE);
    int lit = 0; for (int i = 0; i < W * H; i += 19) if (pixels[i] != th.desktop) lit++;
    session_lock(&fs); run(&l);
    lit = 0; for (int i = 0; i < W * H; i += 19) if (pixels[i] != th.desktop) lit++;
    CHECK(lit > 300);
    DONE("session_test");
}
