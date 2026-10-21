#include "ui.h"
#include "test.h"
#include <string.h>

#define W 400
#define H 300
static uint32_t pixels[W * H];
static shell_fb fb = { pixels, W, H, W, 100 };
static shell_theme th;
static ui_ctx u;
static bool on_a, on_b, radio_pick; static int grp, slider = 5; static char text[16]; static int caret;
static int hits_a, hits_b;

static void ev(shell_event_type t, int x, int y, int button, uint32_t key, uint32_t mods) { shell_event e = { t, x, y, button, key, mods }; ui_feed(&u, &e); }
static void frame(void)
{
    ui_begin(&u, &fb, &th, 100);
    ui_label(&u, 10, 4, "Demo form", th.text);
    if (ui_button(&u, (shell_rect){ 10, 24, 80, 28 }, "First")) hits_a++;
    if (ui_button(&u, (shell_rect){ 100, 24, 80, 28 }, "Second")) hits_b++;
    ui_toggle(&u, (shell_rect){ 10, 64, 200, 24 }, "Dark mode", &on_a);
    ui_toggle(&u, (shell_rect){ 10, 94, 200, 24 }, "Telemetry", &on_b);
    radio_pick |= ui_radio(&u, (shell_rect){ 10, 124, 120, 22 }, "Option one", &grp, 1);
    ui_radio(&u, (shell_rect){ 140, 124, 120, 22 }, "Option two", &grp, 2);
    ui_textfield(&u, (shell_rect){ 10, 154, 180, 26 }, text, sizeof text, &caret, "Name");
    ui_slider(&u, (shell_rect){ 10, 190, 180, 20 }, "Volume", &slider, 0, 10);
    ui_progress(&u, (shell_rect){ 10, 220, 100, 8 }, 40);
    ui_end(&u);
}
static void key(uint32_t k, uint32_t mods) { ev(EV_KEY_DOWN, 0, 0, 0, k, mods); frame(); }

int main(void)
{
    shell_theme_dark(&th, 0xFF3B82F6u);
    memset(&u, 0, sizeof u);
    frame();
    /* every interactive widget registered, in layout order, with role and name */
    CHECK(ui_a11y_count(&u) >= 10);
    char dump[1024]; ui_a11y_dump(&u, dump, sizeof dump);
    CHECK(strstr(dump, "label: Demo form") && strstr(dump, "button: First") && strstr(dump, "toggle: Dark mode [off]"));
    CHECK(strstr(dump, "radio: Option one [off]") && strstr(dump, "text: Name") && strstr(dump, "slider: Volume = 5") && strstr(dump, "progress: progress = 40%"));
    CHECK(u.norder == 8);                                                              /* 2 buttons, 2 toggles, 2 radios, textfield, slider */
    CHECK(u.focus == 0 && !strstr(dump, "(focused)"));

    /* Tab walks the focus ring in order, wraps, and Shift+Tab goes back */
    key(KEY_TAB, 0); CHECK(u.focus == u.order[0]);
    ui_a11y_dump(&u, dump, sizeof dump); frame(); ui_a11y_dump(&u, dump, sizeof dump);
    CHECK(strstr(dump, "button: First (focused)"));
    key(KEY_TAB, 0); frame(); ui_a11y_dump(&u, dump, sizeof dump); CHECK(strstr(dump, "button: Second (focused)"));
    key(KEY_TAB, MOD_SHIFT); frame(); ui_a11y_dump(&u, dump, sizeof dump); CHECK(strstr(dump, "button: First (focused)"));
    key(KEY_TAB, MOD_SHIFT); frame(); CHECK(u.focus == u.order[u.norder - 1]);          /* wraps backwards to the last widget */
    key(KEY_TAB, 0); frame(); CHECK(u.focus == u.order[0]);                              /* and forwards to the first */

    /* Enter and Space activate the focused control */
    key(KEY_ENTER, 0); CHECK(hits_a == 1 && hits_b == 0);
    key(KEY_SPACE, 0); CHECK(hits_a == 2);
    key(KEY_TAB, 0); frame(); key(KEY_SPACE, 0); CHECK(hits_b == 1 && hits_a == 2);
    key(KEY_TAB, 0); frame(); key(KEY_SPACE, 0); frame(); CHECK(on_a && !on_b);          /* toggle by keyboard */
    ui_a11y_dump(&u, dump, sizeof dump); CHECK(strstr(dump, "toggle: Dark mode [on] (focused)"));
    key(KEY_TAB, 0); frame(); key(KEY_ENTER, 0); frame(); CHECK(on_b);
    key(KEY_TAB, 0); frame(); key(KEY_SPACE, 0); frame(); CHECK(grp == 1 && radio_pick);
    key(KEY_TAB, 0); frame(); key(KEY_SPACE, 0); frame(); CHECK(grp == 2);

    /* Tab leaves a text field (it does not type or swallow the key), typing works while it is focused */
    key(KEY_TAB, 0); frame();
    CHECK(u.focus == ui_textfield_id("Name", (shell_rect){ 10, 154, 180, 26 }));
    ev(EV_CHAR, 0, 0, 0, 'h', 0); ev(EV_CHAR, 0, 0, 0, 'i', 0); frame();
    CHECK(!strcmp(text, "hi"));
    ev(EV_KEY_DOWN, 0, 0, 0, KEY_SPACE, 0); frame();                                    /* a bare Space key (not a char) must not activate anything else */
    CHECK(hits_a == 2 && hits_b == 1);
    key(KEY_TAB, 0); frame();
    ui_a11y_dump(&u, dump, sizeof dump); CHECK(strstr(dump, "slider: Volume = 5 (focused)"));
    key(KEY_RIGHT, 0); key(KEY_RIGHT, 0); frame(); CHECK(slider == 7);
    key(KEY_LEFT, 0); CHECK(slider == 6);
    for (int i = 0; i < 20; i++) key(KEY_RIGHT, 0);
    CHECK(slider == 10);                                                                /* clamped to the range */

    /* reduce motion keeps the caret solid; high contrast thickens the focus ring */
    key(KEY_TAB, 0); frame();
    CHECK(u.focus == u.order[0]);                                                       /* slider -> wraps to the first button */
    u.high_contrast = true; frame();
    shell_rect fr = { 10, 24, 80, 28 };
    CHECK(pixels[(fr.y - 1) * W + fr.x + 40] == th.accent && pixels[fr.y * W + fr.x + 40] == th.accent);
    u.high_contrast = false; memset(pixels, 0, sizeof pixels); frame();
    CHECK(pixels[(fr.y - 1) * W + fr.x + 40] == 0);
    u.focus = ui_textfield_id("Name", (shell_rect){ 10, 154, 180, 26 }); u.reduce_motion = true;
    int solid = 0;
    for (int f = 0; f < 60; f++) { memset(pixels, 0, sizeof pixels); frame(); solid += pixels[165 * W + 18 + shell_text_width("hi")] == th.text; }   /* caret column after "hi" */
    CHECK(solid == 60);
    u.reduce_motion = false;
    int blink = 0;
    for (int f = 0; f < 60; f++) { memset(pixels, 0, sizeof pixels); frame(); blink += pixels[165 * W + 18 + shell_text_width("hi")] == th.text; }
    CHECK(blink > 10 && blink < 50);

    /* the dump never overruns a small buffer */
    char tiny[24]; size_t need = ui_a11y_dump(&u, tiny, sizeof tiny);
    CHECK(need > sizeof tiny && strlen(tiny) < sizeof tiny);
    DONE("a11y_test");
}
