/* Browser app integration: navigation model, local pages, bookmarks/tabs and a painted frame. */
#include "backend.h"
#include "test.h"
#include "ui.h"
#include <stdlib.h>
#include <string.h>

#define W 900
#define H 600
static uint32_t pixels[W * H];
static shell_fb fb = { pixels, W, H, W, 100 };
static shell_theme th;
static ui_ctx u;
static void *browser;

/* crates/loos-shell/src/apps/browser.rs (C ABI) */
void *loos_browser_new(const shell_backend *be, const char *url);
void loos_browser_frame(void *app, ui_ctx *ui, int w, int h);
bool loos_browser_navigate_to(void *app, const char *url);
bool loos_browser_url(const void *app, char *out, size_t cap);
int loos_browser_tab_count(const void *app);
int loos_browser_bookmark_count(const void *app);

static void run(void) { ui_begin(&u, &fb, &th, 100); loos_browser_frame(browser, &u, W, H); ui_end(&u); }
static void ev(shell_event_type t, uint32_t key, uint32_t mods) { shell_event e = { t, 0, 0, 0, key, mods }; ui_feed(&u, &e); }
static void key(uint32_t k, uint32_t mods) { ev(EV_KEY_DOWN, k, mods); run(); ev(EV_KEY_UP, k, mods); run(); }
static void typestr(const char *s) { for (; *s; s++) { ev(EV_CHAR, (uint32_t)*s, 0); run(); } }
static void click(int x, int y)
{
    u.now_ms += 1000;
    { shell_event e = { EV_MOUSE_MOVE, x, y, 0, 0, 0 }; ui_feed(&u, &e); } run();
    { shell_event e = { EV_MOUSE_DOWN, x, y, BTN_LEFT, 0, 0 }; ui_feed(&u, &e); } run();
    { shell_event e = { EV_MOUSE_UP, x, y, BTN_LEFT, 0, 0 }; ui_feed(&u, &e); } run();
}
static bool url_is(const char *want)
{
    char got[256];
    return loos_browser_url(browser, got, sizeof got) && !strcmp(got, want);
}

int main(void)
{
    shell_theme_dark(&th, 0xFF3B82F6u);
    backend_mem mem; shell_backend be; backend_mem_init(&mem, &be);
    backend_mem_add_volume(&mem, "/", "LoOS", "btrfs", 500ull << 30, 320ull << 30, false);
    backend_mem_add_node(&mem, "/home", true, 0, 0, 0);
    backend_mem_add_node(&mem, "/home/user", true, 0, 0, 0);
    backend_mem_add_node(&mem, "/home/user/.config", true, 0, 0, 0);
    backend_mem_add_node(&mem, "/home/user/Web", true, 0, 0, 0);
    static const char page[] =
        "<html><head><title>Local Test</title></head><body><h1>Local Test</h1>"
        "<p>Hello <b>LoOS</b>.</p><ul><li>one</li><li>two</li></ul>"
        "<p><a href=\"guide.html\">Guide</a></p></body></html>";
    backend_mem_add_node(&mem, "/home/user/Web/index.html", false, sizeof page - 1, 0, 0);
    CHECK(be.fs_write(be.user, "/home/user/Web/index.html", page, sizeof page - 1));
    { static const char guide[] = "<title>Guide</title><p>Guide page</p>"; CHECK(be.fs_write(be.user, "/home/user/Web/guide.html", guide, sizeof guide - 1)); }

    browser = loos_browser_new(&be, NULL);
    CHECK(browser != NULL);
    CHECK(loos_browser_tab_count(browser) == 1);
    CHECK(loos_browser_bookmark_count(browser) == 3);
    CHECK(url_is("loos://home"));
    run();

    /* Local files, relative links and internal pages. */
    CHECK(loos_browser_navigate_to(browser, "/home/user/Web/index.html"));
    CHECK(url_is("/home/user/Web/index.html"));
    run();
    { const char *pp = getenv("LOOS_BROWSER_PPM");                        /* optional visual review frame */
      if (pp) { FILE *f = fopen(pp, "wb");
        if (f) { fprintf(f, "P6\n%d %d\n255\n", W, H);
          for (int i = 0; i < W * H; i++) { unsigned char rgb[3] = { (unsigned char)(pixels[i] >> 16), (unsigned char)(pixels[i] >> 8), (unsigned char)pixels[i] }; fwrite(rgb, 1, 3, f); }
          fclose(f); } } }
    CHECK(loos_browser_navigate_to(browser, "/home/user/Web/guide.html"));
    CHECK(url_is("/home/user/Web/guide.html"));
    CHECK(loos_browser_navigate_to(browser, "loos://history"));
    CHECK(url_is("loos://history"));
    CHECK(loos_browser_navigate_to(browser, "loos://bookmarks"));
    CHECK(url_is("loos://bookmarks"));

    /* Web addresses are deliberately offline: the tab stays on the requested URL and the
       browser paints an explanation page instead of making a request. */
    CHECK(!loos_browser_navigate_to(browser, "https://example.com"));
    CHECK(url_is("https://example.com"));

    /* Back/forward over the tab history. */
    CHECK(loos_browser_navigate_to(browser, "loos://home"));
    run();
    click(36, 52);                                              /* Back button: home -> web request */
    CHECK(url_is("https://example.com"));
    click(104, 52);                                             /* Forward button */
    CHECK(url_is("loos://home"));

    /* Bookmarks toggle and persist through the backend. */
    CHECK(loos_browser_navigate_to(browser, "loos://about"));
    CHECK(loos_browser_bookmark_count(browser) == 3);
    key('d', MOD_CTRL);
    CHECK(loos_browser_bookmark_count(browser) == 4);
    key('d', MOD_CTRL);
    CHECK(loos_browser_bookmark_count(browser) == 3);
    key('d', MOD_CTRL);
    CHECK(loos_browser_bookmark_count(browser) == 4);
    { void *second = loos_browser_new(&be, NULL); CHECK(second && loos_browser_bookmark_count(second) == 4); }

    /* Tabs: the + button and Ctrl+W. */
    CHECK(loos_browser_tab_count(browser) == 1);
    click(210, 16);                                             /* "+" after the first 190 px tab */
    CHECK(loos_browser_tab_count(browser) == 2);
    CHECK(url_is("loos://home"));
    key('w', MOD_CTRL);
    CHECK(loos_browser_tab_count(browser) == 1);

    /* Ctrl+L focuses the address bar, typing replaces it and Enter navigates. */
    key('l', MOD_CTRL);
    key(KEY_END, 0);
    for (int i = 0; i < 40; i++) key(KEY_BACKSPACE, 0);
    typestr("loos://about");
    key(KEY_ENTER, 0);
    CHECK(url_is("loos://about"));

    /* The frame actually paints content beyond the window background. */
    int lit = 0;
    for (int i = 0; i < W * H; i += 17) if (pixels[i] != th.surface) lit++;
    CHECK(lit > 200);

    DONE("browser_test");
}
