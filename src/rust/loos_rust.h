/* C ABI of the Rust crates (crates/ffi -> libloos_ffi.a). Link the `loos_rust` CMake target. Hand-maintained; the
 * Rust side is crates/ffi/src. Conventions: int functions return 0 / negative on error unless documented; every
 * pointer must be valid for the stated length; strings are NUL-terminated UTF-8; opaque handles are freed with their
 * *_free function. Functions that replace old C modules keep the old symbol names (see crypto.h, iso9660.h). */
#ifndef LOOS_RUST_H
#define LOOS_RUST_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---- AES (loos-security); SHA-256/HMAC/PBKDF2 are declared in security/crypto.h and kdf.h ---- */
int loos_xts_crypt(const uint8_t *key, size_t key_len, uint64_t sector, uint8_t *buf, size_t len, bool decrypt);          /* key 32 or 64 bytes; 0 ok */
int loos_gcm_seal(const uint8_t *key, size_t key_len, const uint8_t nonce12[12], const uint8_t *aad, size_t aad_len, uint8_t *data, size_t len, uint8_t tag16[16]);
int loos_gcm_open(const uint8_t *key, size_t key_len, const uint8_t nonce12[12], const uint8_t *aad, size_t aad_len, uint8_t *data, size_t len, const uint8_t tag16[16]);   /* -2 = auth failure */

/* ---- crash dump (loos-kdump) ---- */
typedef struct { uint64_t gpr[16]; uint64_t rip, rflags; uint64_t cr[4]; uint16_t cs, ss; } loos_kdump_context;
int64_t loos_kdump_write(uint8_t *out, size_t cap, uint32_t bugcheck_code, const uint64_t params[4], uint64_t time_ns, uint32_t cpu, uint32_t ncpus,
                         uint64_t build_id, const loos_kdump_context *ctx, const uint64_t *frames, size_t nframes, const uint8_t *log, size_t log_len);
int64_t loos_kdump_render(const uint8_t *dump, size_t len, char *out, size_t cap);                                       /* text length, -1 invalid */
size_t  loos_kdump_walk(uint64_t rip, uint64_t rbp, uint64_t stack_lo, uint64_t stack_hi, uint64_t *out, size_t max,
                        int (*read_u64)(void *opaque, uint64_t addr, uint64_t *value), void *opaque);

/* ---- task scheduler (loos-tasksched) ---- */
typedef struct loos_sched loos_sched;
enum { LOOS_TRIG_ONCE, LOOS_TRIG_INTERVAL, LOOS_TRIG_DAILY, LOOS_TRIG_WEEKLY, LOOS_TRIG_MONTHLY, LOOS_TRIG_BOOT, LOOS_TRIG_LOGON };
loos_sched *loos_sched_new(void);
void     loos_sched_free(loos_sched *s);
uint32_t loos_sched_add(loos_sched *s, const char *name, const char *command, uint32_t kind, int64_t a, int64_t b, int64_t c, bool run_missed, int64_t now);
size_t   loos_sched_poll(loos_sched *s, int64_t now, uint32_t *ids_out, size_t cap);
void     loos_sched_finished(loos_sched *s, uint32_t id);
void     loos_sched_event(loos_sched *s, bool logon);
int      loos_sched_remove(loos_sched *s, uint32_t id);
int64_t  loos_sched_next_run(const loos_sched *s, uint32_t id);
int64_t  loos_sched_command(const loos_sched *s, uint32_t id, char *out, size_t cap);

/* ---- time service (loos-timesvc) ---- */
void loos_ntp_request(int64_t t1_ns, uint8_t out48[48], uint64_t *t1_ntp);
int  loos_ntp_measure(const uint8_t *reply, size_t len, uint64_t t1_ntp, int64_t t1_ns, int64_t t4_ns, int64_t *offset_ns, int64_t *delay_ns, uint8_t *stratum);   /* 0 ok, >0 reject */
int  loos_ntp_discipline(int64_t offset_ns, int64_t over_secs, int64_t *value);                                          /* 0 none, 1 slew ppm, 2 step ns */
int  loos_tz_offset(const char *posix_tz, int64_t unix_time, int32_t *offset_seconds_east, bool *is_dst);
void loos_time_from_unix(int64_t unix_time, int64_t out7[7]);                                                           /* y m d H M S weekday */

/* ---- display class (loos-kms) ---- */
typedef struct { uint32_t clock; uint16_t hdisplay, hsync_start, hsync_end, htotal, vdisplay, vsync_start, vsync_end, vtotal; uint32_t flags, preferred; } loos_kms_mode;
typedef struct { char manufacturer[4]; uint16_t product; uint8_t digital; char name[16]; uint32_t width_mm, height_mm, nmodes; } loos_kms_edid;
int      loos_edid_parse(const uint8_t *edid, size_t len, loos_kms_edid *out);                                           /* -1 short -2 header -3 checksum */
int      loos_edid_mode(const uint8_t *edid, size_t len, uint32_t index, loos_kms_mode *out);
int      loos_cvt_rb(uint32_t w, uint32_t h, uint32_t hz, loos_kms_mode *out);
int      loos_kms_pick(const loos_kms_mode *modes, size_t n, uint16_t w, uint16_t h, uint32_t max_hz, loos_kms_mode *out);   /* w == 0: preferred */
uint32_t loos_kms_refresh_mhz(const loos_kms_mode *m);
uint32_t loos_ui_scale_for_dpi(uint32_t dpi);

/* ---- packages: dpkg, winget, msix, msi, trust, ownership ---- */
int     loos_deb_version_compare(const char *a, const char *b);                                                         /* dpkg --compare-versions order for any input: -1/0/1 */
int     loos_deb_version_cmp(const char *a, const char *b);                                                             /* -1/0/1, 2 invalid */
int64_t loos_dpkg_plan(const char *packages_index, const char *status_db, const char *want, char *out, size_t cap);    /* "name version\n"...; -2 missing -3 unsat -4 conflict */
int     loos_winget_select(const char *manifest_yaml, uint32_t arch /*0 x86 1 x64 2 arm 3 arm64*/, const char *scope, char *url, size_t url_cap, char sha256_hex[65], char *kind, size_t kind_cap);
typedef struct { char name[64], version[24], arch[12], display_name[64]; uint8_t full_trust, is_signed; uint32_t napps, nfiles; } loos_msix_info;
int     loos_msix_inspect(const uint8_t *data, size_t len, loos_msix_info *out);
typedef struct { char product[96], version[24], product_code[48], manufacturer[64]; uint32_t nfiles, nfeatures, ncustom_actions, nrisky_actions; uint64_t total_bytes; } loos_msi_info;
int     loos_msi_inspect(const uint8_t *data, size_t len, loos_msi_info *out);
int     loos_msi_file(const uint8_t *data, size_t len, uint32_t index, char *path, size_t cap, uint64_t *size);
int     loos_trust_check(const char *hosts_csv, const char *publishers_csv, bool allow_unsigned, const char *url, const char *sha256_hex, const char *signed_publisher);   /* 0 allow, 1..5 deny */
bool    loos_sha256_verify(const uint8_t *data, size_t len, const char *expected_hex);
typedef struct loos_ownership loos_ownership;
loos_ownership *loos_ownership_new(void);
void    loos_ownership_free(loos_ownership *d);
int     loos_ownership_install(loos_ownership *d, const char *pkg, const char *version, const char *paths_nul_separated);   /* paths "a\0*shared\0\0" */
int64_t loos_ownership_owners(const loos_ownership *d, const char *path, char *out, size_t cap);
int64_t loos_ownership_plan_remove(const loos_ownership *d, const char *pkg, char *out, size_t cap);
int     loos_ownership_remove(loos_ownership *d, const char *pkg);

/* ---- console host VT core (loos-vt) ---- */
typedef struct loos_vt loos_vt;
loos_vt *loos_vt_new(uint32_t cols, uint32_t rows);
void     loos_vt_free(loos_vt *t);
void     loos_vt_write(loos_vt *t, const uint8_t *data, size_t len);
int      loos_vt_cell(const loos_vt *t, uint32_t x, uint32_t y, uint32_t *ch, uint32_t *fg, uint32_t *bg, uint8_t *attrs);
void     loos_vt_cursor(const loos_vt *t, uint32_t *x, uint32_t *y, bool *visible);
int64_t  loos_vt_row(const loos_vt *t, uint32_t y, char *out, size_t cap);
int64_t  loos_vt_title(const loos_vt *t, char *out, size_t cap);

/* ---- executive: object manager, dispatcher, memory, registry, hardware tables, input (loos-kernel/ksync/kmem/registry/khal/drivers) ---- */
typedef struct loos_kobj loos_kobj;
loos_kobj *loos_kobj_new(void);
void   loos_kobj_free(loos_kobj *m);
int    loos_kobj_syscall(loos_kobj *m, uint32_t number, const uint64_t args6[6], uint64_t *value);                 /* 0 = success, else loos_kernel::Status */

typedef struct loos_disp loos_disp;
loos_disp *loos_disp_new(void);
void     loos_disp_free(loos_disp *d);
uint64_t loos_disp_create_event(loos_disp *d, bool synchronization, bool signaled);                                  /* object ids are non-zero; 0 = failure */
uint64_t loos_disp_create_semaphore(loos_disp *d, uint32_t initial, uint32_t max);
uint64_t loos_disp_create_mutex(loos_disp *d, uint64_t owner_thread);
int      loos_disp_set_event(loos_disp *d, uint64_t id);
int      loos_disp_reset_event(loos_disp *d, uint64_t id);
int64_t  loos_disp_release_semaphore(loos_disp *d, uint64_t id, uint32_t n);
int64_t  loos_disp_release_mutex(loos_disp *d, uint64_t id, uint64_t thread);
int      loos_disp_wait(loos_disp *d, uint64_t thread, const uint64_t *ids, size_t n, bool wait_all, int64_t timeout_ticks, uint32_t *index);   /* 0 signaled 1 abandoned 2 timeout 3 cancelled 4 blocked -1 error */
void     loos_disp_advance(loos_disp *d, uint64_t ticks);
void     loos_disp_thread_exit(loos_disp *d, uint64_t thread);
size_t   loos_disp_drain(loos_disp *d, uint64_t *threads, int32_t *kinds, uint32_t *indices, size_t cap);

typedef struct loos_buddy loos_buddy;
loos_buddy *loos_buddy_new(uint64_t base_frame, uint64_t frames);
void     loos_buddy_free_allocator(loos_buddy *b);
int      loos_buddy_add_free(loos_buddy *b, uint64_t start, uint64_t count);
int64_t  loos_buddy_alloc(loos_buddy *b, uint32_t order);                                                            /* first frame of 2^order frames, -1 on failure */
int      loos_buddy_release(loos_buddy *b, uint64_t frame);
uint64_t loos_buddy_free_frames(const loos_buddy *b);
typedef struct loos_vas loos_vas;
loos_vas *loos_vas_new(uint64_t lo, uint64_t hi, bool wx_enforced);
void     loos_vas_free(loos_vas *a);
uint64_t loos_vas_reserve(loos_vas *a, uint64_t at, uint64_t size, uint32_t max_protect /*0 none 1 R 2 RW 3 RX 4 RWX*/, bool top_down);
int      loos_vas_commit(loos_vas *a, uint64_t addr, uint64_t size, uint32_t protect);
int      loos_vas_decommit(loos_vas *a, uint64_t addr, uint64_t size);
int      loos_vas_release(loos_vas *a, uint64_t base);
bool     loos_vas_check_access(const loos_vas *a, uint64_t addr, uint64_t len, bool write, bool exec);
uint64_t loos_vas_committed_bytes(const loos_vas *a);
typedef struct loos_ws loos_ws;
loos_ws *loos_ws_new(size_t min_pages, size_t max_pages);
void     loos_ws_free(loos_ws *w);
size_t   loos_ws_fault_in(loos_ws *w, uint64_t page, uint64_t *evicted, size_t cap);
void     loos_ws_touch(loos_ws *w, uint64_t page);
size_t   loos_ws_size(const loos_ws *w);

typedef struct loos_hive loos_hive;
loos_hive *loos_hive_new(void);
loos_hive *loos_hive_load(const uint8_t *regf, size_t len);                                                          /* NULL if invalid */
void     loos_hive_free(loos_hive *h);
int64_t  loos_hive_save(const loos_hive *h, uint8_t *out, size_t cap);                                               /* size; nothing is written if it exceeds cap */
int      loos_hive_set(loos_hive *h, const char *key_path, const char *name, uint32_t type, const uint8_t *data, size_t len);
int64_t  loos_hive_get(const loos_hive *h, const char *key_path, const char *name, uint32_t *type, uint8_t *out, size_t cap);
int      loos_hive_delete_value(loos_hive *h, const char *key_path, const char *name);

void loos_idt_gate(uint64_t handler, uint16_t selector, bool trap, uint8_t dpl, uint8_t ist, uint8_t out16[16]);
void loos_gdt_build(uint64_t tss_base, uint8_t out56[56]);
typedef struct { uint32_t key, ch; uint8_t down, mods; } loos_key_event;
typedef struct { int32_t dx, dy, wheel; uint8_t buttons; } loos_mouse_event;
typedef struct loos_kbd loos_kbd;
typedef struct loos_mouse loos_mouse;
loos_kbd *loos_kbd_new(void);
void loos_kbd_free(loos_kbd *k);
int  loos_kbd_feed(loos_kbd *k, uint8_t scancode_set1_byte, loos_key_event *ev);                                     /* 1 = event completed */
loos_mouse *loos_mouse_new(void);
void loos_mouse_free(loos_mouse *m);
int  loos_mouse_feed(loos_mouse *m, uint8_t byte, loos_mouse_event *ev);

/* ---- storage: mount manager (FAT/ISO/NTFS) and btrfs reader (loos-vfs, loos-fs, loos-btrfs-ops) ---- */
typedef int (*loos_read_at_fn)(void *opaque, uint64_t offset, void *buf, size_t len);                                /* 0 = ok */
typedef int (*loos_write_at_fn)(void *opaque, uint64_t offset, const void *buf, size_t len);
typedef struct loos_vfs loos_vfs;
loos_vfs *loos_vfs_new(void);
void    loos_vfs_free(loos_vfs *v);
int64_t loos_vfs_mount(loos_vfs *v, uint32_t kind /*0 FAT 1 ISO 2 NTFS*/, loos_read_at_fn rd, loos_write_at_fn wr /*may be NULL*/, void *opaque, uint64_t len,
                       const char *mountpoint, const char *alias /*"C:" or NULL*/);                                    /* mount id > 0; -1 args -2 not recognized -3 conflict */
int     loos_vfs_stat(loos_vfs *v, const char *path, uint64_t *size, bool *is_dir);
int64_t loos_vfs_list(loos_vfs *v, const char *path, char *out, size_t cap);                                         /* one name per line, dirs end with '/' */
int64_t loos_vfs_read(loos_vfs *v, const char *path, uint64_t offset, uint8_t *buf, size_t len);
int     loos_vfs_write(loos_vfs *v, const char *path, uint64_t offset, const uint8_t *data, size_t len);
int     loos_vfs_unmount(loos_vfs *v, const char *mountpoint);
typedef struct loos_btrfs loos_btrfs;
loos_btrfs *loos_btrfs_open(loos_read_at_fn rd, void *opaque, uint64_t len);                                         /* NULL if not btrfs */
void    loos_btrfs_close(loos_btrfs *b);
int     loos_btrfs_stat(loos_btrfs *b, const char *path, uint64_t *size, bool *is_dir);
int64_t loos_btrfs_list(loos_btrfs *b, const char *path, char *out, size_t cap);
int64_t loos_btrfs_read(loos_btrfs *b, const char *path, uint64_t offset, uint8_t *buf, size_t len);

/* ---- Ps, timers/APC/DPC, SEH, loos-btrfs reader (crates/ffi/src/exec.rs) */
loos_kobj *loos_kobj_new_cpus(uint32_t cpus);
bool    loos_kobj_sched_query(void *kobj, uint64_t thread, uint32_t *priority, bool *runnable);          /* loos_sched_query_fn */
uint64_t loos_disp_create_timer(loos_disp *d, bool synchronization);
int     loos_disp_set_timer(loos_disp *d, uint64_t id, uint64_t delay, uint64_t period);                  /* 1 was armed, 0 not, -1 error */
int     loos_disp_cancel_timer(loos_disp *d, uint64_t id);
int     loos_disp_wait_alertable(loos_disp *d, uint64_t thread, const uint64_t *ids, size_t n, bool wait_all, int64_t timeout_ticks); /* 0 done 4 blocked -1 error */
bool    loos_disp_alert(loos_disp *d, uint64_t thread);                                                  /* drain kind 5 = alerted */
typedef struct loos_apc loos_apc;
loos_apc *loos_apc_new(void);
void    loos_apc_free(loos_apc *q);
int     loos_apc_queue(loos_apc *q, uint64_t thread, bool user, uint64_t routine, uint64_t a0, uint64_t a1, uint64_t a2); /* 1 = alert thread */
int     loos_apc_next(loos_apc *q, uint64_t thread, bool user_ok, uint64_t out4[4]);                     /* 1 = routine,a0..a2 written */
bool    loos_apc_begin_alertable(loos_apc *q, uint64_t thread);
typedef struct loos_dpc loos_dpc;
loos_dpc *loos_dpc_new(void);
void    loos_dpc_free(loos_dpc *q);
bool    loos_dpc_insert(loos_dpc *q, uint64_t id, uint64_t routine, uint64_t a0, uint64_t a1, uint32_t importance);  /* 0 low 1 medium 2 high */
size_t  loos_dpc_drain(loos_dpc *q, uint64_t *out, size_t cap);                                          /* (id,routine,a0,a1) per entry */
typedef struct loos_workq loos_workq;
loos_workq *loos_workq_new(void);
void    loos_workq_free(loos_workq *q);
void    loos_workq_push(loos_workq *q, uint32_t cls, uint64_t item);                                     /* 0 delayed 1 critical 2 hyper */
bool    loos_workq_pop(loos_workq *q, uint64_t *item);
struct loos_seh_image { uint64_t base, size; uint32_t pdata_rva, pdata_len; };
typedef bool (*loos_seh_read_fn)(void *opaque, uint64_t addr, uint8_t *out, size_t len);
typedef bool (*loos_seh_handler_fn)(void *opaque, uint32_t kind, uint64_t routine, uint64_t data, uint64_t establisher, uint64_t ctx17[17]); /* true = continue execution */
int     loos_seh_dispatch(uint64_t ctx17[17], const struct loos_seh_image *images, size_t n, const uint64_t *veh, size_t nveh,
                          const uint64_t *vch, size_t nvch, uint64_t unhandled, uint64_t stack_lo, uint64_t stack_hi,
                          loos_seh_read_fn read, loos_seh_handler_fn handler, void *opaque);              /* 0 continue 1 unhandled 2 bad stack */
typedef struct loos_btrfsr loos_btrfsr;
loos_btrfsr *loos_btrfsr_open(const uint8_t *data, size_t len);                                         /* strict validating reader, NULL if invalid */
void    loos_btrfsr_close(loos_btrfsr *r);
int     loos_btrfsr_stat(const loos_btrfsr *r, const uint8_t *path, size_t plen, uint64_t out4[4]);     /* inode,size,mode,nlink */
int64_t loos_btrfsr_read(const loos_btrfsr *r, const uint8_t *path, size_t plen, uint64_t offset, uint8_t *out, size_t cap);
int64_t loos_winpath_to_loos(const char *win, char *out, size_t cap);      /* C:\Users\u -> /home/u; -1 unmappable (docs/FILESYSTEMS.md) */
int64_t loos_winpath_to_windows(const char *path, char *out, size_t cap);  /* /home/u -> C:\Users\u; outside the map -> Z:\ */

#ifdef __cplusplus
}
#endif
#endif
