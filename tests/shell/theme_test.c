#include "shell.h"
#include "theme.h"
#include "backend.h"
#include "settings.h"
#include "test.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

int main(void)
{
    /* color math */
    CHECK(theme_contrast_x100(0xFF000000u, 0xFFFFFFFFu) >= 2090 && theme_contrast_x100(0xFF000000u, 0xFFFFFFFFu) <= 2110);   /* 21:1 */
    CHECK(theme_contrast_x100(0xFF777777u, 0xFFFFFFFFu) > 440 && theme_contrast_x100(0xFF777777u, 0xFFFFFFFFu) < 470);          /* ~4.48:1 */
    CHECK(theme_contrast_x100(0xFF123456u, 0xFF123456u) == 100);
    uint32_t c; CHECK(theme_parse_hex("#3B82F6", &c) && c == 0xFF3B82F6u && theme_parse_hex("3b82f6", &c) && !theme_parse_hex("#12345", &c) && !theme_parse_hex("#GGGGGG", &c));
    int h, s, l;
    theme_to_hsl(0xFFFF0000u, &h, &s, &l); CHECK(h == 0 && s == 100 && l == 50);
    theme_to_hsl(0xFF00FF00u, &h, &s, &l); CHECK(h == 120);
    CHECK(theme_hsl(240, 100, 50) == 0xFF0000FFu && theme_hsl(0, 0, 100) == 0xFFFFFFFFu && theme_hsl(0, 0, 0) == 0xFF000000u);
    for (int i = 0; i < 360; i += 17) { uint32_t col = theme_hsl(i, 70, 55); int h2; theme_to_hsl(col, &h2, NULL, NULL); CHECK(abs(h2 - i) <= 2 || abs(h2 - i) >= 358); }

    /* default spec resolves to exactly the stock dark/light palettes */
    theme_spec sp; shell_theme t, ref;
    theme_spec_default(&sp, 0xFF3B82F6u); theme_resolve(&sp, &t); shell_theme_dark(&ref, 0xFF3B82F6u);
    CHECK(!memcmp(&t, &ref, sizeof t));
    sp.base = TB_LIGHT; theme_resolve(&sp, &t); shell_theme_light(&ref, 0xFF3B82F6u); CHECK(!memcmp(&t, &ref, sizeof t));

    /* every base and preset is readable after the guard */
    for (int i = 0; i < theme_preset_count(); i++) {
        theme_spec p; CHECK(theme_preset(i, &p)); theme_resolve(&p, &t);
        CHECK(theme_contrast_x100(t.text, t.surface) >= 450);
        if (p.base != TB_HC) CHECK(theme_contrast_x100(t.text_dim, t.surface) >= 300);
        CHECK(t.surface >> 24 == 0xFF && t.accent >> 24 == 0xFF);
    }
    CHECK(!theme_preset(-1, &sp) && !theme_preset(99, &sp) && theme_preset_name(0)[0]);

    /* overrides apply, the guard repairs unreadable ones, switching the guard off keeps them verbatim */
    theme_spec_default(&sp, 0xFF3B82F6u);
    sp.ovr[TK_TEXT] = 0xFF1A2230u; sp.ovr_mask = 1u << TK_TEXT;           /* near-black text on the dark surface */
    theme_resolve(&sp, &t); CHECK(theme_contrast_x100(t.text, t.surface) >= 450);
    sp.guard = false; theme_resolve(&sp, &t); CHECK(t.text == 0xFF1A2230u);
    sp.guard = true; sp.ovr_mask = 1u << TK_SURFACE; sp.ovr[TK_SURFACE] = 0xFF204060u; theme_resolve(&sp, &t); CHECK(t.surface == 0xFF204060u);

    /* tint pulls neutrals toward the accent */
    theme_spec_default(&sp, 0xFFFF0000u); theme_resolve(&sp, &ref); sp.tint = 60; theme_resolve(&sp, &t);
    CHECK(((t.surface >> 16) & 255) > ((ref.surface >> 16) & 255));

    /* export / import round trip + hostile input */
    theme_spec a, b; theme_preset(5, &a); a.ovr[TK_BORDER] = 0xFF445566u; a.ovr_mask |= 1u << TK_BORDER;
    char txt[512]; size_t n = theme_export(&a, txt, sizeof txt); CHECK(n > 20 && n < sizeof txt);
    theme_spec_default(&b, 0xFF000001u);
    CHECK(theme_import(&b, txt) == 0 && b.base == a.base && b.accent == a.accent && b.tint == a.tint && b.radius == a.radius && b.ovr_mask == a.ovr_mask && b.ovr[TK_BORDER] == a.ovr[TK_BORDER]);
    theme_spec e; theme_spec_default(&e, 0xFF3B82F6u);
    CHECK(theme_import(&e, "radius=999;title_h=1;tint=-5;shadow=9999;base=nope;garbage;c.bogus=#fff;accent=zzz") == 4);
    CHECK(e.radius == 24 && e.title_h == 20 && e.tint == 0 && e.shadow == 150 && e.base == TB_DARK);
    CHECK(theme_import(&e, "") == 0 && theme_import(&e, "# comment only\n") == 0);
    char big[1200]; memset(big, 'x', sizeof big - 1); big[sizeof big - 1] = 0; (void)theme_import(&e, big);      /* must not overflow */

    /* persistence through the backend config */
    backend_mem mem; shell_backend be; backend_mem_init(&mem, &be);
    theme_spec_load(&b, &be, 0xFF10B981u); CHECK(b.accent == 0xFF10B981u && b.base == TB_DARK && b.radius == -1 && b.guard);
    theme_preset(9, &a); theme_spec_store(&a, &be);
    theme_spec_load(&b, &be, 0xFF10B981u);
    CHECK(b.base == a.base && b.accent == a.accent && b.radius == 16 && b.title_h == 32 && b.shadow == 140 && b.glow == 150 && b.tint == 18);
    a.ovr[TK_TEXT] = 0xFFEEEEEEu; a.ovr_mask = 1u << TK_TEXT; theme_spec_store(&a, &be); theme_spec_load(&b, &be, 0); CHECK(b.ovr_mask == (1u << TK_TEXT));
    a.ovr_mask = 0; theme_spec_store(&a, &be); theme_spec_load(&b, &be, 0); CHECK(b.ovr_mask == 0);

    /* applying a spec changes metrics and effects live */
    shell_comp *comp = calloc(1, sizeof *comp); comp_init(comp, 800, 600);
    int base_title = comp->m.title_h, base_rad = comp->m.corner_r;
    theme_spec p; theme_preset(9, &p); theme_apply(&p, comp);
    CHECK(comp->m.title_h == 32 && comp->m.corner_r == 16 && comp->style.shadow_pct == 140 && comp->theme.accent == p.accent);
    theme_preset(11, &p); theme_apply(&p, comp);
    CHECK(comp->m.corner_r == 0 && comp->style.shadow_pct == 0 && comp->style.glow_pct == 0 && comp->m.title_h == 24 && base_title == 28 && base_rad == 8);
    comp_set_scale(comp, 200); CHECK(comp->m.title_h == 48 && comp->m.border == 2);                                       /* custom metrics follow the UI scale */
    comp_set_scale(comp, 100);
    /* flat wallpaper (no glow) still paints and costs less than the glow version */
    static uint32_t px[800 * 600]; shell_fb fb = { px, 800, 600, 800, 100 };
    comp_render_full(comp, &fb); CHECK(px[300 * 800 + 400] >> 24 == 0xFF);
    free(comp);

    /* Settings integration: legacy keys still work, new keys win, the page renders and edits live */
    {
        backend_mem m2; shell_backend be2; backend_mem_init(&m2, &be2);
        shell_comp *c2 = calloc(1, sizeof *c2); comp_init(c2, 1280, 800);
        shell_settings st; settings_init(&st, &be2, c2, NULL);
        be_cfg_set_int(&be2, "ui.theme", 1); settings_apply(&st);
        CHECK(c2->theme.surface == 0xFFF7F8FAu);
        be2.cfg_set(be2.user, "ui.base", "nord"); be2.cfg_set(be2.user, "ui.accent_hex", "#88C0D0"); settings_apply(&st);
        CHECK(c2->theme.accent == 0xFF88C0D0u && st.spec.base == TB_NORD);
        be_cfg_set_bool(&be2, "ui.high_contrast", true); settings_apply(&st);
        CHECK(c2->theme.surface == 0xFF000000u && c2->high_contrast);
        be_cfg_set_bool(&be2, "ui.high_contrast", false); settings_apply(&st);
        static uint32_t spx[900 * 600]; shell_fb sfb = { spx, 900, 600, 900, 100 };
        ui_ctx ui; st.page = PG_PERSONALIZE;
        for (int f = 0; f < 3; f++) { ui_begin(&ui, &sfb, &c2->theme, 100); settings_frame(&st, &ui, 900, 600); ui_end(&ui); }
        shell_event ev;
        /* press a preset button (first grid cell) */
        int bx = 190 + 28 + 10, by = 24 + 66 + 8;
        ev = (shell_event){ EV_MOUSE_MOVE, bx, by, 0, 0, 0 }; ui_begin(&ui, &sfb, &c2->theme, 100); ui_feed(&ui, &ev); settings_frame(&st, &ui, 900, 600); ui_end(&ui);
        ev = (shell_event){ EV_MOUSE_DOWN, bx, by, 1, 0, 0 }; ui_begin(&ui, &sfb, &c2->theme, 100); ui_feed(&ui, &ev); settings_frame(&st, &ui, 900, 600); ui_end(&ui);
        ev = (shell_event){ EV_MOUSE_UP, bx, by, 1, 0, 0 }; ui_begin(&ui, &sfb, &c2->theme, 100); ui_feed(&ui, &ev); settings_frame(&st, &ui, 900, 600); ui_end(&ui);
        char v[32]; CHECK(be2.cfg_get(be2.user, "ui.base", v, sizeof v) && !strcmp(v, "dark") && c2->theme.surface == 0xFF121824u);
        for (int pg = 0; pg < PG_COUNT; pg++) { st.page = (settings_page)pg; ui_begin(&ui, &sfb, &c2->theme, 100); settings_frame(&st, &ui, 900, 600); ui_end(&ui); }
        free(c2);
    }
    DONE("theme_test");
}
