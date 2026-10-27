/* LoOS UEFI loader (BOOTX64.EFI): the firmware-dependent half of the UEFI boot path.
 *
 * Reads \LOOS\BOOT.CFG from the ESP (kernel=..., module=NAME:PATH, cmdline=...), loads the kernel ELF and modules into
 * RAM, picks a GOP framebuffer, finds the ACPI RSDP, allocates the loader's scratch arena, leaves boot services with
 * the final memory map converted to E820 form, and hands over to the shared loader core (boot/common). */
#include "efi.h"
#include "../common/loader.h"
#include "../common/rt.h"
#define LADER_MAX LOADER_MAX_FILES

static EFI_SYSTEM_TABLE *ST;
static EFI_BOOT_SERVICES *BS;
static struct loader_env env;
static struct loos_mem_entry e820[LOADER_MAX_RAW_MMAP];

static void out(const char *s)
{
    CHAR16 buf[96]; int n = 0;
    for (; *s; s++) {
        if (n > 92) { buf[n] = 0; ST->ConOut->OutputString(ST->ConOut, buf); n = 0; }
        if (*s == '\n') buf[n++] = '\r';
        buf[n++] = (CHAR16)(unsigned char)*s;
    }
    buf[n] = 0;
    ST->ConOut->OutputString(ST->ConOut, buf);
}

static void serial(const char *s)                    /* after ExitBootServices the console is gone */
{
    for (; *s; s++) {
        uint8_t st;
        do { __asm__ volatile("inb %1,%0" : "=a"(st) : "Nd"((uint16_t)0x3fd)); } while (!(st & 0x20));
        __asm__ volatile("outb %0,%1" : : "a"((uint8_t)*s), "Nd"((uint16_t)0x3f8));
    }
}

static __attribute__((noreturn)) void die(const char *msg)
{
    out("LoOS UEFI loader: "); out(msg); out("\n");
    BS->Stall(5000000);
    for (;;) __asm__ volatile("hlt");
}

static int guid_eq(const EFI_GUID *a, const EFI_GUID *b) { return memcmp(a, b, sizeof *a) == 0; }

static void *alloc_pages(UINTN bytes, int type, EFI_PHYSICAL_ADDRESS max)
{
    if (bytes == 0 || bytes > UINT64_MAX - 4095) return 0;
    EFI_PHYSICAL_ADDRESS a = max;
    UINTN pages = (bytes + 4095) / 4096;
    if (pages == 0) return 0;
    if (EFI_ERROR(BS->AllocatePages(max ? AllocateMaxAddress : AllocateAnyPages, type, pages, &a))) return 0;
    return (void *)(uintptr_t)a;
}

/* ---- ESP file access ---- */
static EFI_FILE_PROTOCOL *root;

static CHAR16 *widen(const char *s, CHAR16 *buf, size_t cap)
{
    size_t n = 0;
    for (; s[n] && n + 1 < cap; n++) buf[n] = (CHAR16)(unsigned char)s[n];
    buf[n] = 0;
    return buf;
}

/* Reads a whole file into freshly allocated pages. */
static void *read_file(const char *path, uint64_t *size_out)
{
    CHAR16 w[256];
    EFI_FILE_PROTOCOL *f;
    if (EFI_ERROR(root->Open(root, &f, widen(path, w, 256), EFI_FILE_MODE_READ, 0))) return 0;
    uint64_t size = 0;
    if (EFI_ERROR(f->SetPosition(f, ~0ull)) || EFI_ERROR(f->GetPosition(f, &size)) || EFI_ERROR(f->SetPosition(f, 0)) || size == 0) {
        f->Close(f);
        return 0;
    }
    if (size == UINT64_MAX) {
        f->Close(f);
        return 0;
    }
    void *buf = alloc_pages(size + 1, EfiLoaderData, 0);          /* +1: callers may NUL-terminate text */
    if (!buf) { f->Close(f); return 0; }
    UINTN got = size;
    EFI_STATUS st = f->Read(f, &got, buf);
    f->Close(f);
    if (EFI_ERROR(st) || got != size) return 0;
    *size_out = size;
    return buf;
}

/* ---- BOOT.CFG ---- */
struct cfg_module { char name[16]; char path[128]; };
static char cfg_kernel[128];
static struct cfg_module cfg_modules[LOOS_BOOT_MAX_MODULES];
static uint32_t cfg_nmodules;

static void cfg_parse(char *text, uint64_t len)
{
    for (uint64_t i = 0; i < len;) {
        uint64_t start = i;
        char *line = text + start;
        while (i < len && text[i] != '\n') i++;
        uint64_t end = i - start;                       /* length of this line */
        if (i < len) i++;
        while (end > 0 && (line[end - 1] == '\r' || line[end - 1] == ' ')) end--;
        line[end] = 0;
        if (line[0] == '#' || !line[0]) continue;
        if (!memcmp(line, "kernel=", 7)) {
            size_t n = strlen(line + 7);
            if (n < sizeof cfg_kernel) memcpy(cfg_kernel, line + 7, n + 1);
        } else if (!memcmp(line, "module=", 7) && cfg_nmodules < LOOS_BOOT_MAX_MODULES) {
            char *colon = line + 7;
            while (*colon && *colon != ':') colon++;
            if (*colon != ':' || colon - (line + 7) >= 16 || strlen(colon + 1) >= sizeof cfg_modules[0].path) continue;
            struct cfg_module *m = &cfg_modules[cfg_nmodules++];
            memcpy(m->name, line + 7, (size_t)(colon - (line + 7)));
            m->name[colon - (line + 7)] = 0;
            memcpy(m->path, colon + 1, strlen(colon + 1) + 1);
        } else if (!memcmp(line, "cmdline=", 8)) {
            size_t n = strlen(line + 8);
            if (n < sizeof env.cmdline) memcpy(env.cmdline, line + 8, n + 1);
        }
    }
}

/* ---- graphics ---- */
/* "video=WxH" from the command line (default 1920x1080) */
static void wanted_video(uint32_t *w, uint32_t *h)
{
    *w = 1920; *h = 1080;
    for (const char *p = env.cmdline; *p; p++) {
        if ((p != env.cmdline && p[-1] != ' ') || memcmp(p, "video=", 6) != 0) continue;
        uint32_t a = 0, b = 0; const char *q = p + 6;
        while (*q >= '0' && *q <= '9') a = a * 10 + (uint32_t)(*q++ - '0');
        if (*q == 'x') q++;
        while (*q >= '0' && *q <= '9') b = b * 10 + (uint32_t)(*q++ - '0');
        if (a >= 640 && b >= 400) { *w = a; *h = b; }
    }
}

static void setup_gop(void)
{
    static const EFI_GUID gg = EFI_GOP_GUID;
    EFI_GRAPHICS_OUTPUT_PROTOCOL *g;
    if (EFI_ERROR(BS->LocateProtocol(&gg, 0, (void **)&g)) || !g || !g->Mode) return;
    uint32_t ww, wh;
    wanted_video(&ww, &wh);
    uint64_t want = (uint64_t)ww * wh, best_diff = ~0ull; uint32_t best = g->Mode->Mode; int have = 0;
    for (uint32_t m = 0; m < g->Mode->MaxMode; m++) {
        EFI_GRAPHICS_OUTPUT_MODE_INFORMATION *info; UINTN sz;
        if (EFI_ERROR(g->QueryMode(g, m, &sz, &info))) continue;
        if (info->PixelFormat > PixelBlueGreenRedReserved8BitPerColor) continue;       /* need a direct 32-bpp format */
        uint64_t area = (uint64_t)info->HorizontalResolution * info->VerticalResolution;
        uint64_t diff = area > want ? area - want : want - area;
        if (info->HorizontalResolution == ww && info->VerticalResolution == wh) diff = 0;
        if (diff < best_diff) { best_diff = diff; best = m; have = 1; }
    }
    if (have && best != g->Mode->Mode) g->SetMode(g, best);
    if (!g->Mode->Info || g->Mode->Info->PixelFormat > PixelBlueGreenRedReserved8BitPerColor || !g->Mode->FrameBufferBase) return;
    env.fb_phys = g->Mode->FrameBufferBase;
    env.fb_width = g->Mode->Info->HorizontalResolution;
    env.fb_height = g->Mode->Info->VerticalResolution;
    env.fb_pitch = g->Mode->Info->PixelsPerScanLine * 4;
    env.fb_bpp = 32;
    env.fb_format = g->Mode->Info->PixelFormat == PixelRedGreenBlueReserved8BitPerColor ? LOOS_FB_RGBX : LOOS_FB_BGRX;
}

static uint64_t find_rsdp(void)
{
    static const EFI_GUID a20 = EFI_ACPI20_GUID, a10 = EFI_ACPI10_GUID;
    uint64_t v1 = 0;
    for (UINTN i = 0; i < ST->NumberOfTableEntries; i++) {
        if (guid_eq(&ST->ConfigurationTable[i].VendorGuid, &a20)) return (uint64_t)(uintptr_t)ST->ConfigurationTable[i].VendorTable;
        if (guid_eq(&ST->ConfigurationTable[i].VendorGuid, &a10)) v1 = (uint64_t)(uintptr_t)ST->ConfigurationTable[i].VendorTable;
    }
    return v1;
}

static uint32_t e820_type(uint32_t t)
{
    switch (t) {
    case EfiLoaderCode: case EfiLoaderData: case EfiBootServicesCode: case EfiBootServicesData: case EfiConventionalMemory:
        return LOOS_MEM_USABLE;                              /* boot services are gone once we convert */
    case EfiACPIReclaimMemory: return LOOS_MEM_ACPI_RECLAIM;
    case EfiACPIMemoryNVS: return LOOS_MEM_ACPI_NVS;
    case EfiUnusableMemory: return LOOS_MEM_BAD;
    default: return LOOS_MEM_RESERVED;                       /* runtime services, MMIO, reserved, persistent */
    }
}

EFI_STATUS efi_main(EFI_HANDLE image, EFI_SYSTEM_TABLE *st)
{
    ST = st; BS = st->BootServices;
    BS->SetWatchdogTimer(0, 0, 0, 0);
    out("LoOS UEFI loader\n");

    static const EFI_GUID gli = EFI_LOADED_IMAGE_GUID, gfs = EFI_SIMPLE_FS_GUID;
    EFI_LOADED_IMAGE_PROTOCOL *li;
    EFI_SIMPLE_FILE_SYSTEM_PROTOCOL *fs;
    if (EFI_ERROR(BS->HandleProtocol(image, &gli, (void **)&li))) die("no loaded-image protocol");
    if (EFI_ERROR(BS->HandleProtocol(li->DeviceHandle, &gfs, (void **)&fs))) die("boot volume has no file system");
    if (EFI_ERROR(fs->OpenVolume(fs, &root))) die("cannot open the boot volume");

    memset(&env, 0, sizeof env);
    env.magic = LOADER_ENV_MAGIC; env.version = LOADER_ENV_VERSION; env.flags = LOADER_ENV_UEFI | LOADER_ENV_NO_BIOS_SCAN;

    uint64_t cfg_size;
    char *cfg = read_file("\\LOOS\\BOOT.CFG", &cfg_size);
    if (!cfg) die("\\LOOS\\BOOT.CFG not found");
    char *cfg_copy = alloc_pages(cfg_size + 1, EfiLoaderData, 0);           /* cfg_parse edits its buffer: keep the original text */
    if (!cfg_copy) die("out of memory");
    memcpy(cfg_copy, cfg, cfg_size); cfg_copy[cfg_size] = 0;
    cfg_parse(cfg, cfg_size);
    if (!cfg_kernel[0]) die("BOOT.CFG has no kernel= line");

    uint64_t ksize;
    void *kernel = read_file(cfg_kernel, &ksize);
    if (!kernel) die("cannot read the kernel image");
    memcpy(env.files[0].name, "kernel", 7);
    env.files[0].addr = (uint64_t)(uintptr_t)kernel; env.files[0].size = ksize;
    env.nfiles = 1;
    uint64_t total = ksize;
    for (uint32_t i = 0; i < cfg_nmodules; i++) {
        uint64_t sz;
        void *m = read_file(cfg_modules[i].path, &sz);
        if (!m) die("cannot read a boot module");
        struct loader_file *f = &env.files[env.nfiles++];
        memcpy(f->name, cfg_modules[i].name, sizeof f->name);
        f->addr = (uint64_t)(uintptr_t)m; f->size = sz;
        total += sz;
    }

    /* installer media: hand the installer the files it has to copy onto the target disk (the kernel has no file system driver),
     * and tell it which partition we booted from so it never offers that disk as a target */
    int installer = 0;
    for (const char *p = env.cmdline; *p; p++) if ((p == env.cmdline || p[-1] == ' ') && !memcmp(p, "installer", 9) && (p[9] == 0 || p[9] == ' ')) installer = 1;
    if (installer) {
        uint64_t esz, kz;
        void *efi = read_file("\\EFI\\BOOT\\BOOTX64.EFI", &esz), *k2 = read_file(cfg_kernel, &kz);
        if (!efi || !k2) die("cannot read the files the installer needs");
        if (env.nfiles + 3 > LADER_MAX) die("too many boot modules for an installer");
        struct loader_file *f = &env.files[env.nfiles++];
        memcpy(f->name, "efi", 4); f->addr = (uint64_t)(uintptr_t)efi; f->size = esz;
        f = &env.files[env.nfiles++];
        memcpy(f->name, "kernel.elf", 11); f->addr = (uint64_t)(uintptr_t)k2; f->size = kz;
        f = &env.files[env.nfiles++];
        memcpy(f->name, "boot.cfg", 9); f->addr = (uint64_t)(uintptr_t)cfg_copy; f->size = cfg_size;
        static const EFI_GUID gpi = EFI_PARTITION_INFO_GUID;
        EFI_PARTITION_INFO_PROTOCOL *pi;
        if (!EFI_ERROR(BS->HandleProtocol(li->DeviceHandle, &gpi, (void **)&pi)) && pi->Type == 2) {
            const EFI_GUID *g = &pi->Info.Gpt.UniquePartitionGUID;
            static const char hex[] = "0123456789abcdef";
            char t[48]; int n = 0;
            uint32_t a = g->a; uint16_t b = g->b, c = g->c;
            for (int sh = 28; sh >= 0; sh -= 4) t[n++] = hex[(a >> sh) & 15];
            t[n++] = '-';
            for (int sh = 12; sh >= 0; sh -= 4) t[n++] = hex[(b >> sh) & 15];
            t[n++] = '-';
            for (int sh = 12; sh >= 0; sh -= 4) t[n++] = hex[(c >> sh) & 15];
            t[n++] = '-';
            for (int i = 0; i < 8; i++) { t[n++] = hex[g->d[i] >> 4]; t[n++] = hex[g->d[i] & 15]; if (i == 1) t[n++] = '-'; }
            t[n] = 0;
            size_t cl = strlen(env.cmdline);
            if (cl + 15 + n < sizeof env.cmdline) { memcpy(env.cmdline + cl, " loos.espguid=", 14); memcpy(env.cmdline + cl + 14, t, (size_t)n + 1); }
        }
    }

    /* scratch arena below 4 GiB (CR3 must be 32-bit addressable for AP startup): bootinfo + page tables + kernel image */
    uint64_t scratch = 3 * ksize + (16ull << 20);
    scratch = (scratch + 4095) & ~4095ull;
    void *sp = alloc_pages(scratch, EfiLoaderData, 0xffffffffull);
    if (!sp) die("cannot allocate the loader scratch area below 4 GiB");
    env.scratch_base = (uint64_t)(uintptr_t)sp; env.scratch_size = scratch;

    env.image = (struct loader_range){ (uint64_t)(uintptr_t)li->ImageBase, li->ImageSize };
    uint64_t sptr = (uint64_t)(uintptr_t)__builtin_frame_address(0);
    env.reserved[0] = (struct loader_range){ (sptr & ~4095ull) - 256 * 1024, 320 * 1024 };      /* the firmware stack we run on */
    env.nreserved = 1;
    env.rsdp_phys = find_rsdp();
    setup_gop();
    out("LoOS UEFI loader: leaving boot services\n");

    /* final memory map -> exit boot services (retry: any allocation changes the map key) */
    UINTN msize = 0, key, dsize; uint32_t dver;
    BS->GetMemoryMap(&msize, 0, &key, &dsize, &dver);
    msize += 16 * dsize;
    EFI_MEMORY_DESCRIPTOR *map = alloc_pages(msize, EfiLoaderData, 0);
    if (!map) die("cannot allocate the memory-map buffer");
    for (int tries = 0;; tries++) {
        UINTN sz = msize;
        EFI_STATUS s = BS->GetMemoryMap(&sz, map, &key, &dsize, &dver);
        if (EFI_ERROR(s)) die("GetMemoryMap failed");
        s = BS->ExitBootServices(image, key);
        if (!EFI_ERROR(s)) { msize = sz; break; }
        if (tries > 4) die("ExitBootServices failed");
    }

    /* convert to E820 form, merging neighbours of equal type */
    uint32_t n = 0;
    for (UINTN off = 0; off + sizeof(EFI_MEMORY_DESCRIPTOR) <= msize; off += dsize) {
        const EFI_MEMORY_DESCRIPTOR *d = (const EFI_MEMORY_DESCRIPTOR *)((const uint8_t *)map + off);
        uint32_t t = e820_type(d->Type);
        uint64_t len = d->NumberOfPages * 4096;
        if (n && e820[n - 1].type == t && e820[n - 1].base + e820[n - 1].length == d->PhysicalStart) { e820[n - 1].length += len; continue; }
        if (n == LOADER_MAX_RAW_MMAP) { serial("LoOS UEFI loader: memory map too large\n"); for (;;) __asm__ volatile("hlt"); }
        e820[n++] = (struct loos_mem_entry){ d->PhysicalStart, len, t, 1 };
    }
    env.mmap_phys = (uint64_t)(uintptr_t)e820;
    env.mmap_count = n;

    int rc = loader_main(&env);                              /* returns only on failure */
    struct loader_result r;
    loader_prepare(&env, &r);
    serial("LoOS UEFI loader: "); serial(r.status != LOADER_OK ? loader_strerror(&r) : "kernel entry returned"); serial("\n");
    (void)rc; (void)total;
    for (;;) __asm__ volatile("cli; hlt");
}
