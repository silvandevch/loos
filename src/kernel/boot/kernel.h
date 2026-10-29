/* Internal interfaces of the stage-0 kernel. */
#ifndef LOOS_KBOOT_KERNEL_H
#define LOOS_KBOOT_KERNEL_H
#include <stdint.h>
#include <stddef.h>
#include "bootinfo.h"

/* serial.c */
void serial_putc(char c);
void serial_puts(const char *s);
void serial_hex(uint64_t v);
void serial_dec(uint64_t v);
static inline void outb(uint16_t p, uint8_t v) { __asm__ volatile("outb %0,%1" : : "a"(v), "Nd"(p)); }
static inline uint8_t inb(uint16_t p) { uint8_t v; __asm__ volatile("inb %1,%0" : "=a"(v) : "Nd"(p)); return v; }
static inline void outl(uint16_t p, uint32_t v) { __asm__ volatile("outl %0,%1" : : "a"(v), "Nd"(p)); }
static inline uint32_t inl(uint16_t p) { uint32_t v; __asm__ volatile("inl %1,%0" : "=a"(v) : "Nd"(p)); return v; }
static inline uint64_t rdmsr(uint32_t m) { uint32_t lo, hi; __asm__ volatile("rdmsr" : "=a"(lo), "=d"(hi) : "c"(m)); return (uint64_t)hi << 32 | lo; }
static inline void wrmsr(uint32_t m, uint64_t v) { __asm__ volatile("wrmsr" : : "c"(m), "a"((uint32_t)v), "d"((uint32_t)(v >> 32))); }

/* bugcheck.c + trap.S. Crash dump area: DUMP_AREA_SECTORS right before the LoOS data partition;
 * sector 0 = "LOOSDUMP-AREA" header (u8 version @16, u32 sectors @20, u32 len @24, u32 code @28), dump from sector 1. */
#define DUMP_AREA_SECTORS 2048u
struct video;
void trap_init(const struct loos_bootinfo *bi);         /* IDT, TSS with IST stacks, stack guard page */
uint32_t bugcheck_boot_report(void);                    /* stop code of a dump left by the last crash (reported once), or 0 */
/* bootmenu.c */
int bootmenu_run(struct video *v, int force, uint32_t wait_ms, uint32_t crash, int auto_pick);   /* 1 = safe mode */
void bugcheck_set_video(struct video *v);               /* stop screen target */
void bugcheck_selftest(const char *what);               /* loos.bugcheck=pf|ud|gp|manual */

/* crates/kffi (Rust, no_std) */
uint64_t loos_kexec_init(uint32_t cpus);                /* System pid, 0 on failure */
int loos_kexec_syscall(uint32_t number, const uint64_t args6[6], uint64_t *value);
size_t loos_kbugcheck_name(uint32_t code, char *out, size_t cap);

/* kalloc.c */
void kalloc_init(const struct loos_bootinfo *bi);
void *kalloc(uint64_t size, uint64_t align);        /* NULL when out of memory */
uint64_t kalloc_free(void);

/* ktime.c */
void time_init(void);
uint64_t time_us(void);
uint32_t time_ms(void);
void udelay(uint64_t us);
void rtc_time(int *hour, int *minute, int *second);
uint64_t rtc_unix(void);                                 /* CMOS clock as unix seconds (UTC assumed) */
void krandom(uint8_t *out, size_t n);                  /* RDRAND when present, else TSC/PIT/RTC jitter */

/* vm.c: map device memory (uncached identity mapping) into the live page tables */
int vm_map_mmio(uint64_t phys, uint64_t len);

/* pci.c */
struct pci_dev { uint8_t bus, dev, fn; uint16_t vendor, device; uint8_t class_code, subclass, prog_if; };
int pci_scan(struct pci_dev *out, int max);                               /* every function found */
uint32_t pci_read32(const struct pci_dev *d, uint8_t off);
void pci_write32(const struct pci_dev *d, uint8_t off, uint32_t v);
uint64_t pci_bar(const struct pci_dev *d, int index, uint64_t *size);     /* memory BAR base (64-bit aware); size via sizing */
void pci_enable(const struct pci_dev *d);                                 /* memory space + bus mastering */

/* blk.c: every disk the kernel can talk to, regardless of controller */
struct blkdev {
    char name[24], model[48];
    uint64_t sectors;                                                      /* 512-byte sectors */
    int sector_size_ok;                                                    /* 0 for 4Kn disks we cannot address in 512-byte units */
    int (*read)(struct blkdev *d, uint64_t lba, uint32_t count, void *buf);
    int (*write)(struct blkdev *d, uint64_t lba, uint32_t count, const void *buf);
    void *drv;
};
int blk_init(void);                                                        /* probes ATA, AHCI, NVMe; returns the disk count */
int blk_count(void);
struct blkdev *blk_get(int i);
struct blkdev *blk_add(void);                                              /* for drivers */
int ahci_probe(void);
int nvme_probe(void);

/* ata.c: legacy IDE (PIO) disks */
struct ata_dev {
    uint16_t io, ctl; int slave, channel, index;
    int lba48;
    uint64_t sectors;
    char model[41];
};
int ata_init(void);                                    /* returns the number of disks found */
int ata_count(void);
struct ata_dev *ata_dev(int i);
int ata_read(const struct ata_dev *d, uint64_t lba, uint32_t count, void *buf);          /* 0 / -1 */
int ata_write(const struct ata_dev *d, uint64_t lba, uint32_t count, const void *buf);

/* install.c */
struct shell_inst_backend;
void install_set_bootinfo(const struct loos_bootinfo *bi);       /* modules (media image, config) come from the boot protocol */
int install_read_config(char *out, size_t cap);                 /* installed-system configuration text, 0 if none */
const struct shell_inst_backend *install_backend(void);
int install_unattended(int disk, const char *user, const char *pass, const char *host);

/* ps2.c */
enum { KIN_KEY = 1, KIN_MOUSE = 2 };
struct kin { int type; uint32_t key, ch, mods; int down; int dx, dy; uint32_t buttons; };
void ps2_init(void);
int ps2_poll(struct kin *out);                /* one event; 0 when the controller is idle */
int ps2_has_mouse(void);

/* video.c: the boot framebuffer plus a back buffer; present() copies a rectangle with the right pixel format */
struct video {
    volatile uint8_t *screen;
    uint32_t pitch;
    int w, h, rgbx;
    uint32_t *back;                                 /* w*h 0xAARRGGBB */
};
int video_init(struct video *v, const struct loos_bootinfo *bi);        /* 0 on success */
void video_present(const struct video *v, int x, int y, int w, int h);
void video_fade(const struct video *v, const uint32_t *from, uint32_t ms);   /* cross-fade from `from` to v->back */

/* shellhost.c: the LoOS shell on the boot framebuffer. test_mode renders one frame and returns. */
int shellhost_run(const struct loos_bootinfo *bi, struct video *v, int test_mode, int scale_pct);   /* 0 done (test), 1 ready, <0 error */
void shellhost_loop(struct video *v);
int shellhost_login_selftest(const char *user, const char *password);      /* exercises the stored account: wrong password refused, right one accepted */
void shellhost_set_config(const char *text);                                                      /* machine configuration to apply to the desktop */
int shellhost_installer(struct video *v, int scale_pct, int test_mode);                          /* 1 = user chose live mode */
int shellhost_login(struct video *v, int scale_pct, const char *accounts_text, char *user, size_t cap, int test_mode);   /* 0 = signed in */                                                            /* never returns */

/* splash.c: boot screen with a progress bar */
void splash_start(struct video *v, int scale_pct);
void splash_progress(int percent, const char *step);

/* acpi.c */
#define ACPI_MAX_CPUS 256
struct acpi_cpus {
    uint32_t count;                         /* enabled processors found in the MADT */
    uint32_t apic_id[ACPI_MAX_CPUS];
    uint64_t lapic_phys;
    int ok;
};
int acpi_parse_madt(uint64_t rsdp_phys, struct acpi_cpus *out);

/* smp.c */
uint32_t smp_start_aps(const struct loos_bootinfo *bi, const struct acpi_cpus *c);   /* returns CPUs online incl. BSP */

/* vm.c: page-table inspection used for the W^X self check */
uint64_t vm_pte(uint64_t cr3, uint64_t vaddr, uint64_t *page_size);
int vm_unmap_page(uint64_t cr3, uint64_t vaddr);
uint64_t vm_virt_to_phys(const struct loos_bootinfo *bi, const void *p);

void kgdt_load(void);
#endif
