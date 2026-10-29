/* Locks the C layout of the structures shared with crates/loos-shell (compositor.rs/wm.rs assert the same numbers). */
#include "shell.h"
#include "test.h"
#include <stddef.h>

int main(void)
{
    CHECK(sizeof(shell_event) == 24);
    CHECK(sizeof(shell_window) == 1720);
    CHECK(offsetof(shell_window, title) == 36);
    CHECK(offsetof(shell_window, flags) == 132);
    CHECK(offsetof(shell_window, client) == 152);
    CHECK(offsetof(shell_window, q) == 172);
    CHECK(offsetof(shell_window, dpi) == 1716);
    CHECK(sizeof(shell_metrics) == 20);
    CHECK(sizeof(shell_style) == 32);
    CHECK(sizeof(shell_comp) == 221272);
    CHECK(offsetof(shell_comp, win) == 8);
    CHECK(offsetof(shell_comp, order) == 220168);
    CHECK(offsetof(shell_comp, nwin) == 220680);
    CHECK(offsetof(shell_comp, theme) == 220696);
    CHECK(offsetof(shell_comp, m) == 220736);
    CHECK(offsetof(shell_comp, style) == 220756);
    CHECK(offsetof(shell_comp, work_area) == 220788);
    CHECK(offsetof(shell_comp, taskbar) == 220816);
    CHECK(offsetof(shell_comp, wall_cache) == 220824);
    CHECK(offsetof(shell_comp, damage) == 220836);
    CHECK(offsetof(shell_comp, hover) == 221108);
    CHECK(offsetof(shell_comp, capture) == 221144);
    CHECK(offsetof(shell_comp, on_change) == 221152);
    CHECK(offsetof(shell_comp, overlay) == 221168);
    CHECK(offsetof(shell_comp, scale_pct) == 221232);
    CHECK(offsetof(shell_comp, kbd_mods) == 221240);
    CHECK(offsetof(shell_comp, stat_culled) == 221256);
    CHECK(offsetof(shell_comp, now_ms) == 221268);
    CHECK(sizeof(shell_monitor) == 36);
    CHECK(sizeof(shell_wm) == 192);
    CHECK(offsetof(shell_wm, nmon) == 152);
    CHECK(offsetof(shell_wm, focus_mode) == 156);
    CHECK(offsetof(shell_wm, tiling) == 160);
    CHECK(offsetof(shell_wm, gap) == 164);
    CHECK(offsetof(shell_wm, preview) == 172);
    CHECK(offsetof(shell_wm, master_pct) == 184);
    DONE("layout_test");
}
