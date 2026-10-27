/* Calls every Rust-backed C API from C: proves the ABI, the headers and the linkage, and spot-checks behavior. */
#include "loos_rust.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int fails, checks;
#define CHECK(c) do { checks++; if (!(c)) { fails++; printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); } } while (0)

/* ---- helpers ---- */
static int mem_read(void *op, uint64_t off, void *buf, size_t len)
{
    const struct { const uint8_t *p; size_t n; } *m = op;
    if (off + len > m->n) return -1;
    memcpy(buf, m->p + off, len);
    return 0;
}
static int read_u64_cb(void *op, uint64_t a, uint64_t *v)
{
    const uint64_t *mem = op;                 /* fake stack: addresses 0x1000.. map to mem[(a-0x1000)/8] */
    if (a < 0x1000 || a >= 0x1000 + 8 * 8) return 0;
    *v = mem[(a - 0x1000) / 8];
    return 1;
}

static void test_kdump(void)
{
    uint8_t dump[4096]; char text[2048];
    uint64_t params[4] = { 0xdead, 1, 0xfff00010, 0 }, frames[2] = { 0xfff00010, 0xfff00200 };
    loos_kdump_context ctx; memset(&ctx, 0, sizeof ctx); ctx.rip = 0xfff00010; ctx.cr[2] = 0xdead;
    int64_t n = loos_kdump_write(dump, sizeof dump, 0x50, params, 42, 1, 4, 0xabc, &ctx, frames, 2, (const uint8_t *)"oops\n", 5);
    CHECK(n > 112 && n < 1024);
    int64_t t = loos_kdump_render(dump, (size_t)n, text, sizeof text);
    CHECK(t > 0 && strstr(text, "PAGE_FAULT_IN_NONPAGED_AREA") && strstr(text, "oops"));
    dump[(size_t)n - 3] ^= 1;
    CHECK(loos_kdump_render(dump, (size_t)n, text, sizeof text) == -1);
    CHECK(loos_kdump_write(dump, 50, 0x50, params, 0, 0, 1, 0, NULL, NULL, 0, NULL, 0) == -2);
    /* frame walk: rbp=0x1000 -> [0x1010, 0xAA]; 0x1010 -> [0x1020, 0xBB]; 0x1020 -> [0, 0] ends */
    uint64_t mem[8] = { 0x1010, 0xAA, 0x1020, 0xBB, 0, 0, 0, 0 };
    uint64_t out[8];
    size_t k = loos_kdump_walk(0x99, 0x1000, 0x1000, 0x1100, out, 8, read_u64_cb, mem);
    CHECK(k == 3 && out[0] == 0x99 && out[1] == 0xAA && out[2] == 0xBB);
}

static void test_sched_time(void)
{
    loos_sched *s = loos_sched_new();
    uint32_t a = loos_sched_add(s, "a", "a.exe /x", LOOS_TRIG_INTERVAL, 0, 100, 0, false, 0);
    uint32_t b = loos_sched_add(s, "b", "b.exe", LOOS_TRIG_INTERVAL, 0, 100, 0, true, 0);
    CHECK(a && b && a != b && loos_sched_add(s, "a", "x", LOOS_TRIG_BOOT, 0, 0, 0, false, 0) == 0);
    uint32_t ids[4];
    CHECK(loos_sched_poll(s, 1000, ids, 4) == 1 && ids[0] == b);            /* a skipped the missed runs, b catches up once */
    loos_sched_finished(s, b);
    CHECK(loos_sched_next_run(s, a) == 1100);
    CHECK(loos_sched_poll(s, 1100, ids, 4) == 2);
    char cmd[32]; CHECK(loos_sched_command(s, a, cmd, sizeof cmd) == 8 && !strcmp(cmd, "a.exe /x"));
    CHECK(loos_sched_remove(s, a) == 0 && loos_sched_remove(s, a) == -1);
    loos_sched_free(s);

    int64_t t1 = 1700000000000000000LL, off, delay; uint8_t req[48], rep[48]; uint64_t t1n; uint8_t st;
    loos_ntp_request(t1, req, &t1n);
    CHECK(req[0] == 0x23);
    memset(rep, 0, sizeof rep); rep[0] = 0x24; rep[1] = 2;
    memcpy(rep + 24, req + 40, 8);
    CHECK(loos_ntp_measure(rep, 47, t1n, t1, t1 + 1000, &off, &delay, &st) == 1);        /* length */
    CHECK(loos_ntp_measure(rep, 48, t1n, t1, t1 + 1000, &off, &delay, &st) == 6);        /* no transmit timestamp */
    int64_t v; CHECK(loos_ntp_discipline(500000000LL, 600, &v) == 2 && v == 500000000LL);
    CHECK(loos_ntp_discipline(10000000LL, 100, &v) == 1 && v == 100);
    int32_t o; bool dst;
    CHECK(loos_tz_offset("CET-1CEST,M3.5.0,M10.5.0/3", 1719828000 /* 2024-07-01 10:00 UTC */, &o, &dst) == 0 && o == 7200 && dst);
    CHECK(loos_tz_offset("CET-1CEST,M3.5.0,M10.5.0/3", 1704103200 /* 2024-01-01 10:00 UTC */, &o, &dst) == 0 && o == 3600 && !dst);
    CHECK(loos_tz_offset("bogus", 0, &o, &dst) == -1);
    int64_t d[7]; loos_time_from_unix(1709210096, d);
    CHECK(d[0] == 2024 && d[1] == 2 && d[2] == 29 && d[3] == 12 && d[6] == 4);
}

static void test_kms(void)
{
    loos_kms_mode m;
    CHECK(loos_cvt_rb(1920, 1080, 60, &m) == 0 && m.htotal == 2080 && m.vtotal == 1111);
    uint32_t r = loos_kms_refresh_mhz(&m); CHECK(r > 59900 && r < 60100);
    CHECK(loos_cvt_rb(10, 10, 60, &m) == -1);
    loos_kms_mode modes[2]; loos_cvt_rb(1280, 720, 60, &modes[0]); loos_cvt_rb(1280, 720, 75, &modes[1]);
    loos_kms_mode pick;
    CHECK(loos_kms_pick(modes, 2, 1280, 720, 60, &pick) == 0 && loos_kms_refresh_mhz(&pick) < 61000);
    CHECK(loos_kms_pick(modes, 2, 1280, 720, 0, &pick) == 0 && loos_kms_refresh_mhz(&pick) > 74000);
    CHECK(loos_kms_pick(modes, 2, 3840, 2160, 0, &pick) == -1);
    CHECK(loos_ui_scale_for_dpi(96) == 100 && loos_ui_scale_for_dpi(192) == 200);
    uint8_t edid[128] = { 0 };
    loos_kms_edid info;
    CHECK(loos_edid_parse(edid, 100, &info) == -1 && loos_edid_parse(edid, 128, &info) == -2);
}

static void test_pkg(void)
{
    CHECK(loos_deb_version_cmp("1.0~rc1", "1.0") == -1 && loos_deb_version_cmp("1:0.1", "9.9") == 1 && loos_deb_version_cmp("2.0-0", "2.0") == 0);
    CHECK(loos_deb_version_cmp("", "1") == 2);
    char out[256];
    const char *avail = "Package: app\nVersion: 1\nDepends: libfoo (>= 2)\n\nPackage: libfoo\nVersion: 1\n\nPackage: libfoo\nVersion: 3\n";
    CHECK(loos_dpkg_plan(avail, "", "app", out, sizeof out) > 0 && !strcmp(out, "libfoo 3\napp 1\n"));
    CHECK(loos_dpkg_plan(avail, "", "ghost", out, sizeof out) == -2);
    CHECK(loos_dpkg_plan("Package: a\nVersion: 1\nDepends: nothere\n", "", "a", out, sizeof out) == -3);

    const char *wg = "PackageIdentifier: Contoso.Tool\nPackageVersion: 1.0\nInstallerType: exe\nInstallers:\n"
                     "- Architecture: x86\n  InstallerUrl: https://e.com/x86.exe\n  InstallerSha256: A1B2C3D4E5F60718293A4B5C6D7E8F90A1B2C3D4E5F60718293A4B5C6D7E8F90\n"
                     "- Architecture: x64\n  InstallerUrl: https://e.com/x64.exe\n  InstallerSha256: A1B2C3D4E5F60718293A4B5C6D7E8F90A1B2C3D4E5F60718293A4B5C6D7E8F90\n";
    char url[128], sha[65], kind[16];
    CHECK(loos_winget_select(wg, 1, "", url, sizeof url, sha, kind, sizeof kind) == 0 && !strcmp(url, "https://e.com/x64.exe") && !strcmp(kind, "exe") && sha[0] == 'a');
    CHECK(loos_winget_select(wg, 2, "", url, sizeof url, sha, kind, sizeof kind) == -2 && loos_winget_select("junk: [", 1, "", url, sizeof url, sha, kind, sizeof kind) == -1);

    const char *h = "dl.example.com,*.cdn.test", *pubs = "CN=Contoso";
    const char *hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    CHECK(loos_trust_check(h, pubs, false, "https://dl.example.com/a", hash, "cn=contoso") == 0);
    CHECK(loos_trust_check(h, pubs, false, "http://dl.example.com/a", hash, "CN=Contoso") == 1);
    CHECK(loos_trust_check(h, pubs, false, "https://evil.io/a", hash, "CN=Contoso") == 3);
    CHECK(loos_trust_check(h, pubs, false, "https://dl.example.com/a", "zz", "CN=Contoso") == 4);
    CHECK(loos_trust_check(h, pubs, false, "https://dl.example.com/a", hash, NULL) == 5);
    CHECK(loos_trust_check(h, pubs, true, "https://dl.example.com/a", hash, NULL) == 0);
    CHECK(loos_sha256_verify((const uint8_t *)"abc", 3, hash) && !loos_sha256_verify((const uint8_t *)"abd", 3, hash));

    loos_ownership *db = loos_ownership_new();
    CHECK(loos_ownership_install(db, "app", "1", "C:\\App\\app.exe\0*sys/vc.dll\0\0") == 0);
    CHECK(loos_ownership_install(db, "other", "1", "*sys/VC.dll\0\0") == 0);
    CHECK(loos_ownership_install(db, "bad", "1", "c:/app/APP.EXE\0\0") == -2);
    CHECK(loos_ownership_owners(db, "c:/app/app.exe", out, sizeof out) == 3 && !strcmp(out, "app"));
    CHECK(loos_ownership_plan_remove(db, "app", out, sizeof out) > 0 && strstr(out, "c:/app/app.exe") && !strstr(out, "vc.dll"));
    CHECK(loos_ownership_remove(db, "app") == 0 && loos_ownership_remove(db, "app") == -1);
    loos_ownership_free(db);

    uint8_t garbage[64] = { 0 }; loos_msix_info mi; loos_msi_info qi; uint64_t sz; char path[64];
    CHECK(loos_msix_inspect(garbage, sizeof garbage, &mi) < 0 && loos_msi_inspect(garbage, sizeof garbage, &qi) < 0 && loos_msi_file(garbage, 64, 0, path, sizeof path, &sz) < 0);
}

static void test_vt(void)
{
    loos_vt *t = loos_vt_new(10, 3);
    const char *s = "hello\x1b[2;3H\x1b[1;31mX\x1b]0;title\x07";
    loos_vt_write(t, (const uint8_t *)s, strlen(s));
    char row[16]; uint32_t ch, fg, bg; uint8_t at; uint32_t x, y; bool vis;
    CHECK(loos_vt_row(t, 0, row, sizeof row) == 5 && !strcmp(row, "hello"));
    CHECK(loos_vt_cell(t, 2, 1, &ch, &fg, &bg, &at) == 0 && ch == 'X' && fg == (0x01000000u | 1) && (at & 1));
    loos_vt_cursor(t, &x, &y, &vis); CHECK(x == 3 && y == 1 && vis);
    char title[16]; CHECK(loos_vt_title(t, title, sizeof title) == 5 && !strcmp(title, "title"));
    CHECK(loos_vt_cell(t, 99, 0, &ch, &fg, &bg, &at) == -1);
    loos_vt_free(t);
}

static void test_kernel_memory(void)
{
    loos_buddy *b = loos_buddy_new(0, 1024);
    CHECK(b && loos_buddy_add_free(b, 0, 1024) == 0 && loos_buddy_free_frames(b) == 1024);
    int64_t f = loos_buddy_alloc(b, 3);
    CHECK(f >= 0 && loos_buddy_free_frames(b) == 1016 && loos_buddy_release(b, (uint64_t)f) == 0 && loos_buddy_free_frames(b) == 1024);
    CHECK(loos_buddy_release(b, (uint64_t)f) != 0);                                /* double free is refused */
    loos_buddy_free_allocator(b);

    loos_vas *a = loos_vas_new(0x10000, 0x7fff0000, true);
    uint64_t base = loos_vas_reserve(a, 0, 0x10000, 2, false);
    CHECK(base && loos_vas_commit(a, base, 0x4000, 2) == 0 && loos_vas_check_access(a, base, 0x100, true, false));
    CHECK(!loos_vas_check_access(a, base, 0x100, false, true) && loos_vas_committed_bytes(a) == 0x4000);
    CHECK(loos_vas_commit(a, base, 0x1000, 4) != 0);                               /* W^X refused */
    CHECK(loos_vas_release(a, base) == 0);
    loos_vas_free(a);

    loos_ws *w = loos_ws_new(1, 2);
    uint64_t ev[4];
    CHECK(w && loos_ws_fault_in(w, 1, ev, 4) == 0 && loos_ws_fault_in(w, 2, ev, 4) == 0 && loos_ws_fault_in(w, 3, ev, 4) == 1 && loos_ws_size(w) == 2);
    loos_ws_free(w);
}

static void test_kernel_objects(void)
{
    loos_kobj *m = loos_kobj_new();
    uint64_t args[6] = { 1, 0, 0, 0, 0, 0 }, ev = 0, v = 0;       /* CreateEvent(kind=synchronization, initial=false) */
    CHECK(loos_kobj_syscall(m, 2, args, &ev) == 0 && ev != 0);
    uint64_t a2[6] = { ev, 0, 0, 0, 0, 0 };
    CHECK(loos_kobj_syscall(m, 3, a2, &v) == 0);                  /* SetEvent */
    CHECK(loos_kobj_syscall(m, 0, a2, &v) == 0 && loos_kobj_syscall(m, 0, a2, &v) != 0);   /* Close twice: second is refused */
    CHECK(loos_kobj_syscall(m, 999, a2, &v) != 0);
    loos_kobj_free(m);

    loos_disp *d = loos_disp_new();
    uint64_t e = loos_disp_create_event(d, true, false), ids[1] = { e };
    uint32_t idx = 9;
    CHECK(e && loos_disp_wait(d, 7, ids, 1, false, -1, &idx) == 4);                     /* blocks */
    CHECK(loos_disp_set_event(d, e) == 0);
    uint64_t th[2]; int32_t kind[2]; uint32_t ix[2];
    CHECK(loos_disp_drain(d, th, kind, ix, 2) == 1 && th[0] == 7 && kind[0] == 0 && ix[0] == 0);
    CHECK(loos_disp_wait(d, 8, ids, 1, false, 5, &idx) == 4);
    loos_disp_advance(d, 5);
    CHECK(loos_disp_drain(d, th, kind, ix, 2) == 1 && th[0] == 8 && kind[0] == 2);        /* timeout */
    uint64_t sem = loos_disp_create_semaphore(d, 1, 2);
    CHECK(sem && loos_disp_release_semaphore(d, sem, 5) == -1);                          /* exceeds max */
    loos_disp_free(d);
}

static void test_hive_hw(void)
{
    loos_hive *h = loos_hive_new();
    CHECK(loos_hive_set(h, "Software\\LoOS", "Version", 1, (const uint8_t *)"1.0\0", 4) == 0);
    uint32_t dw = 42; CHECK(loos_hive_set(h, "Software\\LoOS", "Count", 4, (const uint8_t *)&dw, 4) == 0);
    uint8_t buf[16]; uint32_t kind = 0;
    CHECK(loos_hive_get(h, "Software\\LoOS", "Count", &kind, buf, sizeof buf) == 4 && kind == 4 && buf[0] == 42);
    int64_t n = loos_hive_save(h, NULL, 0);
    CHECK(n > 4096);
    uint8_t *img = malloc((size_t)n);
    CHECK(loos_hive_save(h, img, (size_t)n) == n);
    loos_hive *h2 = loos_hive_load(img, (size_t)n);
    CHECK(h2 && loos_hive_get(h2, "Software\\LoOS", "Version", &kind, buf, sizeof buf) == 4 && !memcmp(buf, "1.0", 3));
    CHECK(loos_hive_delete_value(h2, "Software\\LoOS", "Version") == 0 && loos_hive_get(h2, "Software\\LoOS", "Version", &kind, buf, sizeof buf) == -1);
    img[0] = 'X'; CHECK(loos_hive_load(img, (size_t)n) == NULL);
    free(img); loos_hive_free(h); loos_hive_free(h2);

    uint8_t gate[16]; loos_idt_gate(0xffffffff80001234ull, 0x08, false, 0, 1, gate);
    CHECK(gate[0] == 0x34 && gate[1] == 0x12 && gate[2] == 0x08 && gate[4] == 1 && gate[5] == 0x8e);
    uint8_t gdt[56]; loos_gdt_build(0xffff800000010000ull, gdt);
    CHECK(gdt[8 + 5] == 0x9b || gdt[8 + 5] == 0x9a);                    /* kernel code descriptor present */

    loos_kbd *k = loos_kbd_new(); loos_key_event ke;
    CHECK(loos_kbd_feed(k, 0x1e, &ke) == 1 && ke.down && ke.key == 'a');
    CHECK(loos_kbd_feed(k, 0x9e, &ke) == 1 && !ke.down);
    loos_kbd_free(k);
    loos_mouse *ms = loos_mouse_new(); loos_mouse_event me = {0};
    CHECK(loos_mouse_feed(ms, 0x09, &me) == 0 && loos_mouse_feed(ms, 5, &me) == 0 && loos_mouse_feed(ms, 3, &me) == 1 && me.dx == 5 && me.buttons == 1);
    loos_mouse_free(ms);
}

static void test_storage(void)
{
    /* an empty buffer is neither FAT nor ISO nor btrfs: every opener must refuse it cleanly */
    static uint8_t zero[1 << 16];
    struct { const uint8_t *p; size_t n; } img = { zero, sizeof zero };
    loos_vfs *v = loos_vfs_new();
    CHECK(loos_vfs_mount(v, 0, mem_read, NULL, &img, sizeof zero, "/mnt", "") < 0);
    CHECK(loos_vfs_mount(v, 1, mem_read, NULL, &img, sizeof zero, "/mnt", "") < 0);
    CHECK(loos_vfs_mount(v, 9, mem_read, NULL, &img, sizeof zero, "/mnt", "") == -1);
    uint64_t sz; bool dir; CHECK(loos_vfs_stat(v, "/nothing", &sz, &dir) == -1);
    loos_vfs_free(v);
    CHECK(loos_btrfs_open(mem_read, &img, sizeof zero) == NULL);
    uint8_t xts_key[64] = { 1 }, sec[512] = { 7 }, orig[512]; memcpy(orig, sec, sizeof sec);
    CHECK(loos_xts_crypt(xts_key, 64, 5, sec, sizeof sec, false) == 0 && memcmp(sec, orig, sizeof sec) != 0);
    CHECK(loos_xts_crypt(xts_key, 64, 5, sec, sizeof sec, true) == 0 && !memcmp(sec, orig, sizeof sec));
    CHECK(loos_xts_crypt(xts_key, 7, 5, sec, sizeof sec, false) == -1);
    uint8_t key[32] = { 9 }, nonce[12] = { 1 }, data[40] = "attack at dawn", tag[16];
    CHECK(loos_gcm_seal(key, 32, nonce, (const uint8_t *)"hdr", 3, data, sizeof data, tag) == 0);
    CHECK(loos_gcm_open(key, 32, nonce, (const uint8_t *)"hdr", 3, data, sizeof data, tag) == 0 && !memcmp(data, "attack at dawn", 14));
    tag[0] ^= 1; loos_gcm_seal(key, 32, nonce, (const uint8_t *)"hdr", 3, data, sizeof data, tag); tag[0] ^= 1;
    CHECK(loos_gcm_open(key, 32, nonce, (const uint8_t *)"hdr", 3, data, sizeof data, tag) == -2);
}

int main(void)
{
    test_kdump(); test_sched_time(); test_kms(); test_pkg(); test_vt(); test_kernel_memory(); test_kernel_objects(); test_hive_hw(); test_storage();
    printf("ffi_test: %d checks, %d failed\n", checks, fails);
    return fails != 0;
}
