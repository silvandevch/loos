/* Stack unwinding (the Itanium _Unwind_* ABI that Objective-C and C++ exceptions use), written
 * from the published formats: Mach-O __unwind_info compact encodings (x86_64, arm64), the
 * DWARF call frame information in __eh_frame for what those cannot express, and the two-phase
 * search/cleanup protocol. C per docs/LANGUAGE.md §C.6: it calls personality routines and
 * jumps into program landing pads. */
#include "prism.h"

uint32_t _dyld_image_count(void);
const void *_dyld_get_image_header(uint32_t);
intptr_t _dyld_get_image_vmaddr_slide(uint32_t);

/* ------------------------------------------------------------------ register context */

#if defined(__x86_64__)
enum { NREG = 17, R_SP = 7, R_IP = 16, R_RA = 16 };
#else
enum { NREG = 96, R_SP = 31, R_IP = 32, R_RA = 30 };
#endif

struct _Unwind_Context {
    uint64_t r[NREG];
    uint64_t cfa;       /* of the frame being visited */
    uint64_t func;      /* its function start */
    uint64_t lsda;
    void *personality;
};

struct _Unwind_Exception {
    uint64_t exception_class;
    void (*cleanup)(int, struct _Unwind_Exception *);
    uintptr_t private_1, private_2; /* our use: 1 = stop flag, 2 = handler frame's SP */
} __attribute__((aligned(16)));

typedef enum {
    _URC_NO_REASON = 0, _URC_FOREIGN_EXCEPTION_CAUGHT = 1, _URC_FATAL_PHASE2_ERROR = 2, _URC_FATAL_PHASE1_ERROR = 3,
    _URC_NORMAL_STOP = 4, _URC_END_OF_STACK = 5, _URC_HANDLER_FOUND = 6, _URC_INSTALL_CONTEXT = 7, _URC_CONTINUE_UNWIND = 8,
} _Unwind_Reason_Code;
enum { _UA_SEARCH_PHASE = 1, _UA_CLEANUP_PHASE = 2, _UA_HANDLER_FRAME = 4, _UA_FORCE_UNWIND = 8 };
typedef _Unwind_Reason_Code (*personality_fn)(int, int, uint64_t, struct _Unwind_Exception *, struct _Unwind_Context *);

/* Capture the caller's callee-saved registers, stack pointer and return address. */
void __prism_unw_getcontext(struct _Unwind_Context *);
/* Load the context's registers and continue at its IP (does not return). */
void __prism_unw_jump(struct _Unwind_Context *) __attribute__((noreturn));

#if defined(__x86_64__)
__asm__(".text\n.globl ___prism_unw_getcontext\n___prism_unw_getcontext:\n"
        " mov %rax, 0(%rdi)\n mov %rdx, 8(%rdi)\n mov %rcx, 16(%rdi)\n mov %rbx, 24(%rdi)\n"
        " mov %rsi, 32(%rdi)\n mov %rdi, 40(%rdi)\n mov %rbp, 48(%rdi)\n"
        " lea 8(%rsp), %rax\n mov %rax, 56(%rdi)\n"
        " mov %r8, 64(%rdi)\n mov %r9, 72(%rdi)\n mov %r10, 80(%rdi)\n mov %r11, 88(%rdi)\n"
        " mov %r12, 96(%rdi)\n mov %r13, 104(%rdi)\n mov %r14, 112(%rdi)\n mov %r15, 120(%rdi)\n"
        " mov (%rsp), %rax\n mov %rax, 128(%rdi)\n ret\n"
        ".globl ___prism_unw_jump\n___prism_unw_jump:\n"
        " mov 24(%rdi), %rbx\n mov 48(%rdi), %rbp\n mov 96(%rdi), %r12\n mov 104(%rdi), %r13\n"
        " mov 112(%rdi), %r14\n mov 120(%rdi), %r15\n mov 0(%rdi), %rax\n mov 8(%rdi), %rdx\n"
        " mov 56(%rdi), %rsp\n jmp *128(%rdi)\n");
#else
__asm__(".text\n.globl ___prism_unw_getcontext\n.p2align 2\n___prism_unw_getcontext:\n"
        " stp x0, x1, [x0]\n stp x19, x20, [x0, #152]\n stp x21, x22, [x0, #168]\n stp x23, x24, [x0, #184]\n"
        " stp x25, x26, [x0, #200]\n stp x27, x28, [x0, #216]\n stp x29, x30, [x0, #232]\n"
        " mov x9, sp\n str x9, [x0, #248]\n str x30, [x0, #256]\n"
        " add x9, x0, #576\n stp d8, d9, [x9]\n stp d10, d11, [x9, #16]\n stp d12, d13, [x9, #32]\n stp d14, d15, [x9, #48]\n"
        " ret\n"
        ".globl ___prism_unw_jump\n.p2align 2\n___prism_unw_jump:\n"
        " ldp x19, x20, [x0, #152]\n ldp x21, x22, [x0, #168]\n ldp x23, x24, [x0, #184]\n"
        " ldp x25, x26, [x0, #200]\n ldp x27, x28, [x0, #216]\n ldp x29, x30, [x0, #232]\n"
        " add x9, x0, #576\n ldp d8, d9, [x9]\n ldp d10, d11, [x9, #16]\n ldp d12, d13, [x9, #32]\n ldp d14, d15, [x9, #48]\n"
        " ldr x9, [x0, #248]\n mov sp, x9\n ldr x16, [x0, #256]\n ldp x0, x1, [x0]\n br x16\n");
#endif

/* ------------------------------------------------------------------ images */

struct image { uint64_t base, text_lo, text_hi; const uint8_t *unwind_info, *eh_frame; uint64_t eh_frame_size; };
static struct image images[256];
static uint32_t nimages;

struct seg64 { uint32_t cmd, cmdsize; char segname[16]; uint64_t vmaddr, vmsize, fileoff, filesize; uint32_t maxprot, initprot, nsects, flags; };
struct sect64 { char sectname[16], segname[16]; uint64_t addr, size; uint32_t offset, align, reloff, nreloc, flags, r1, r2, r3; };

static void scan_images(void) {
    uint32_t n = _dyld_image_count();
    if (n == nimages) return;
    nimages = 0;
    for (uint32_t i = 0; i < n && nimages < 256; i++) {
        const uint32_t *h = _dyld_get_image_header(i);
        intptr_t slide = _dyld_get_image_vmaddr_slide(i);
        struct image im = {(uint64_t)h, 0, 0, 0, 0, 0};
        const char *p = (const char *)h + 32;
        for (uint32_t c = 0; c < h[4]; c++) {
            const struct seg64 *s = (const void *)p;
            if (s->cmd == 0x19 && !strncmp(s->segname, "__TEXT", 16)) {
                im.text_lo = s->vmaddr + slide;
                im.text_hi = im.text_lo + s->vmsize;
                const struct sect64 *x = (const void *)(s + 1);
                for (uint32_t j = 0; j < s->nsects; j++) {
                    if (!strncmp(x[j].sectname, "__unwind_info", 16)) im.unwind_info = (const void *)(x[j].addr + slide);
                    if (!strncmp(x[j].sectname, "__eh_frame", 16)) { im.eh_frame = (const void *)(x[j].addr + slide); im.eh_frame_size = x[j].size; }
                }
            }
            p += s->cmdsize;
        }
        images[nimages++] = im;
    }
}

static struct image *image_for(uint64_t pc) {
    scan_images();
    for (uint32_t i = 0; i < nimages; i++) if (pc >= images[i].text_lo && pc < images[i].text_hi) return &images[i];
    return 0;
}

/* ------------------------------------------------------------------ DWARF readers */

static uint64_t uleb(const uint8_t **p) {
    uint64_t v = 0; int s = 0; uint8_t b;
    do { b = *(*p)++; v |= (uint64_t)(b & 0x7f) << s; s += 7; } while (b & 0x80);
    return v;
}
static int64_t sleb(const uint8_t **p) {
    int64_t v = 0; int s = 0; uint8_t b;
    do { b = *(*p)++; v |= (int64_t)(b & 0x7f) << s; s += 7; } while (b & 0x80);
    if (s < 64 && (b & 0x40)) v |= -((int64_t)1 << s);
    return v;
}

/* A DW_EH_PE-encoded pointer at *p. */
static uint64_t read_encoded(const uint8_t **p, uint8_t enc) {
    if (enc == 0xff) return 0;
    const uint8_t *start = *p;
    uint64_t v;
    switch (enc & 0x0f) {
    case 0x00: memcpy(&v, *p, 8); *p += 8; break;
    case 0x01: v = uleb(p); break;
    case 0x02: { uint16_t x; memcpy(&x, *p, 2); *p += 2; v = x; break; }
    case 0x03: { uint32_t x; memcpy(&x, *p, 4); *p += 4; v = x; break; }
    case 0x04: memcpy(&v, *p, 8); *p += 8; break;
    case 0x09: v = (uint64_t)sleb(p); break;
    case 0x0a: { int16_t x; memcpy(&x, *p, 2); *p += 2; v = (uint64_t)(int64_t)x; break; }
    case 0x0b: { int32_t x; memcpy(&x, *p, 4); *p += 4; v = (uint64_t)(int64_t)x; break; }
    case 0x0c: memcpy(&v, *p, 8); *p += 8; break;
    default: return 0;
    }
    if (v && (enc & 0x70) == 0x10) v += (uint64_t)start; /* pcrel */
    if (v && (enc & 0x80)) v = *(uint64_t *)v;           /* indirect */
    return v;
}

/* ------------------------------------------------------------------ DWARF CFI */

enum { RULE_SAME = 0, RULE_OFFSET, RULE_VAL_OFFSET, RULE_REG, RULE_UNDEF };
struct cfi_state { uint32_t cfa_reg; int64_t cfa_off; uint8_t rule[NREG]; int64_t val[NREG]; };

struct cie { const uint8_t *insns, *end; uint64_t code_align; int64_t data_align; uint32_t ra; uint8_t fde_enc, lsda_enc; void *personality; };

static int parse_cie(const uint8_t *p, struct cie *c) {
    uint64_t len = *(const uint32_t *)p; p += 4;
    if (len == 0xffffffff) { memcpy(&len, p, 8); p += 8; }
    const uint8_t *end = p + len;
    p += 4; /* CIE id */
    uint8_t version = *p++;
    const char *aug = (const char *)p;
    p += strlen(aug) + 1;
    c->code_align = uleb(&p);
    c->data_align = sleb(&p);
    c->ra = version == 1 ? *p++ : (uint32_t)uleb(&p);
    c->fde_enc = 0; c->lsda_enc = 0xff; c->personality = 0;
    if (aug[0] == 'z') {
        uint64_t alen = uleb(&p);
        const uint8_t *aend = p + alen;
        for (const char *a = aug + 1; *a; a++) {
            if (*a == 'P') { uint8_t e = *p++; c->personality = (void *)read_encoded(&p, e); }
            else if (*a == 'L') c->lsda_enc = *p++;
            else if (*a == 'R') c->fde_enc = *p++;
        }
        p = aend;
    }
    c->insns = p; c->end = end;
    return 0;
}

static int run_cfi(const uint8_t *p, const uint8_t *end, const struct cie *c, uint64_t loc, uint64_t pc,
                   struct cfi_state *st, const struct cfi_state *init) {
    struct cfi_state stack[8]; int sp = 0;
    while (p < end && loc <= pc) {
        uint8_t op = *p++;
        uint32_t reg; int64_t off;
        switch (op >> 6) {
        case 1: loc += (op & 0x3f) * c->code_align; continue;
        case 2: reg = op & 0x3f; off = (int64_t)uleb(&p) * c->data_align;
            if (reg < NREG) { st->rule[reg] = RULE_OFFSET; st->val[reg] = off; } continue;
        case 3: reg = op & 0x3f; if (reg < NREG && init) { st->rule[reg] = init->rule[reg]; st->val[reg] = init->val[reg]; } continue;
        }
        switch (op) {
        case 0x00: break;
        case 0x01: loc = read_encoded(&p, c->fde_enc); break;
        case 0x02: loc += *p++ * c->code_align; break;
        case 0x03: { uint16_t d; memcpy(&d, p, 2); p += 2; loc += d * c->code_align; break; }
        case 0x04: { uint32_t d; memcpy(&d, p, 4); p += 4; loc += d * c->code_align; break; }
        case 0x05: reg = (uint32_t)uleb(&p); off = (int64_t)uleb(&p) * c->data_align;
            if (reg < NREG) { st->rule[reg] = RULE_OFFSET; st->val[reg] = off; } break;
        case 0x06: reg = (uint32_t)uleb(&p); if (reg < NREG && init) { st->rule[reg] = init->rule[reg]; st->val[reg] = init->val[reg]; } break;
        case 0x07: reg = (uint32_t)uleb(&p); if (reg < NREG) st->rule[reg] = RULE_UNDEF; break;
        case 0x08: reg = (uint32_t)uleb(&p); if (reg < NREG) st->rule[reg] = RULE_SAME; break;
        case 0x09: reg = (uint32_t)uleb(&p); off = (int64_t)uleb(&p); if (reg < NREG) { st->rule[reg] = RULE_REG; st->val[reg] = off; } break;
        case 0x0a: if (sp < 8) stack[sp++] = *st; break;
        case 0x0b: if (sp > 0) { uint32_t cr = st->cfa_reg; int64_t co = st->cfa_off; *st = stack[--sp]; st->cfa_reg = cr; st->cfa_off = co; } break;
        case 0x0c: st->cfa_reg = (uint32_t)uleb(&p); st->cfa_off = (int64_t)uleb(&p); break;
        case 0x0d: st->cfa_reg = (uint32_t)uleb(&p); break;
        case 0x0e: st->cfa_off = (int64_t)uleb(&p); break;
        case 0x0f: return -1; /* DW_CFA_def_cfa_expression */
        case 0x10: uleb(&p); { uint64_t n = uleb(&p); p += n; } break; /* DW_CFA_expression: not supported, skipped */
        case 0x11: reg = (uint32_t)uleb(&p); off = sleb(&p) * c->data_align;
            if (reg < NREG) { st->rule[reg] = RULE_OFFSET; st->val[reg] = off; } break;
        case 0x12: st->cfa_reg = (uint32_t)uleb(&p); st->cfa_off = sleb(&p) * c->data_align; break;
        case 0x13: st->cfa_off = sleb(&p) * c->data_align; break;
        case 0x14: reg = (uint32_t)uleb(&p); off = (int64_t)uleb(&p) * c->data_align;
            if (reg < NREG) { st->rule[reg] = RULE_VAL_OFFSET; st->val[reg] = off; } break;
        case 0x15: reg = (uint32_t)uleb(&p); off = sleb(&p) * c->data_align;
            if (reg < NREG) { st->rule[reg] = RULE_VAL_OFFSET; st->val[reg] = off; } break;
        case 0x2d: break; /* AArch64 negate_ra_state: return addresses are not signed here */
        case 0x2e: uleb(&p); break; /* GNU_args_size */
        case 0x2f: reg = (uint32_t)uleb(&p); off = -(int64_t)uleb(&p) * c->data_align;
            if (reg < NREG) { st->rule[reg] = RULE_OFFSET; st->val[reg] = off; } break;
        default: return -1;
        }
    }
    return 0;
}

/* Step `ctx` through the frame described by the FDE at `fde` (for `pc`). */
static int step_dwarf(struct _Unwind_Context *ctx, const uint8_t *fde, uint64_t pc) {
    const uint8_t *p = fde;
    uint64_t len = *(const uint32_t *)p; p += 4;
    if (len == 0xffffffff) { memcpy(&len, p, 8); p += 8; }
    const uint8_t *end = p + len;
    uint32_t cie_off = *(const uint32_t *)p;
    const uint8_t *cie_ptr = p - cie_off;
    p += 4;
    struct cie c;
    parse_cie(cie_ptr, &c);
    uint64_t start = read_encoded(&p, c.fde_enc);
    uint64_t range = read_encoded(&p, c.fde_enc & 0x0f);
    (void)range;
    ctx->lsda = 0;
    ctx->personality = c.personality;
    ctx->func = start;
    if (c.lsda_enc != 0xff || 1) {
        uint64_t alen = 0;
        /* FDE augmentation data exists only with a 'z' CIE; parse_cie consumed the CIE side. */
        const uint8_t *q = p;
        alen = uleb(&q);
        if (c.lsda_enc != 0xff) { const uint8_t *r = q; ctx->lsda = read_encoded(&r, c.lsda_enc); }
        p = q + alen;
    }
    struct cfi_state init = {0}, st;
    run_cfi(c.insns, c.end, &c, start, ~0ull, &init, 0);
    st = init;
    if (run_cfi(p, end, &c, start, pc, &st, &init) < 0) return -1;
    uint64_t cfa = ctx->r[st.cfa_reg] + st.cfa_off;
    uint64_t old[NREG];
    memcpy(old, ctx->r, sizeof old);
    for (uint32_t i = 0; i < NREG; i++) {
        switch (st.rule[i]) {
        case RULE_OFFSET: ctx->r[i] = *(uint64_t *)(cfa + st.val[i]); break;
        case RULE_VAL_OFFSET: ctx->r[i] = cfa + st.val[i]; break;
        case RULE_REG: ctx->r[i] = old[st.val[i]]; break;
        default: break;
        }
    }
    uint64_t ra = st.rule[c.ra] == RULE_UNDEF ? 0 : ctx->r[c.ra];
    ctx->cfa = cfa;
    ctx->r[R_SP] = cfa;
    ctx->r[R_IP] = ra;
    return ra ? 0 : 1;
}

/* ------------------------------------------------------------------ compact unwind */

struct ui_header { uint32_t version, common_off, common_count, pers_off, pers_count, index_off, index_count; };
struct ui_index { uint32_t func_off, second_off, lsda_off; };

/* Find the compact encoding for `pc` (image-relative lookup). 0 when there is none. */
static uint32_t compact_lookup(struct image *im, uint64_t pc, uint64_t *func, uint64_t *lsda, void **pers) {
    const uint8_t *ui = im->unwind_info;
    const struct ui_header *h = (const void *)ui;
    if (!ui || h->version != 1) return 0;
    uint32_t off = (uint32_t)(pc - im->base);
    const struct ui_index *idx = (const void *)(ui + h->index_off);
    uint32_t lo = 0, hi = h->index_count - 1; /* last entry is a sentinel */
    if (hi == 0 || off < idx[0].func_off || off >= idx[hi].func_off) return 0;
    while (hi - lo > 1) { uint32_t m = (lo + hi) / 2; if (idx[m].func_off <= off) lo = m; else hi = m; }
    const uint8_t *page = ui + idx[lo].second_off;
    uint32_t kind = *(const uint32_t *)page, enc = 0, fstart = 0, fnext = idx[lo + 1].func_off;
    if (kind == 2) {
        uint16_t eoff = *(const uint16_t *)(page + 4), n = *(const uint16_t *)(page + 6);
        const uint32_t *e = (const void *)(page + eoff);
        int k = -1;
        for (int i = 0; i < n; i++) { if (e[i * 2] <= off) k = i; else break; }
        if (k < 0) return 0;
        fstart = e[k * 2]; enc = e[k * 2 + 1];
        if (k + 1 < n) fnext = e[k * 2 + 2];
    } else if (kind == 3) {
        uint16_t eoff = *(const uint16_t *)(page + 4), n = *(const uint16_t *)(page + 6);
        uint16_t encoff = *(const uint16_t *)(page + 8);
        const uint32_t *e = (const void *)(page + eoff);
        const uint32_t *common = (const void *)(ui + h->common_off);
        const uint32_t *local = (const void *)(page + encoff);
        int k = -1;
        for (int i = 0; i < n; i++) { if (idx[lo].func_off + (e[i] & 0xffffff) <= off) k = i; else break; }
        if (k < 0) return 0;
        fstart = idx[lo].func_off + (e[k] & 0xffffff);
        uint32_t ei = e[k] >> 24;
        enc = ei < h->common_count ? common[ei] : local[ei - h->common_count];
        if (k + 1 < n) fnext = idx[lo].func_off + (e[k + 1] & 0xffffff);
    } else return 0;
    if (off >= fnext) return 0;
    *func = im->base + fstart;
    *lsda = 0;
    if (enc & 0x40000000) {
        const uint32_t *l = (const void *)(ui + idx[lo].lsda_off), *le = (const void *)(ui + idx[lo + 1].lsda_off);
        for (; l < le; l += 2) if (l[0] == fstart) { *lsda = im->base + l[1]; break; }
    }
    uint32_t pi = (enc >> 28) & 3;
    *pers = 0;
    if (pi) {
        uint32_t got = ((const uint32_t *)(ui + h->pers_off))[pi - 1];
        *pers = *(void **)(im->base + got);
    }
    return enc ? enc : 0xffffffff; /* 0 means "no unwind info": a frameless leaf */
}

#if defined(__x86_64__)
enum { MODE_MASK = 0x0f000000, M_FRAME = 0x01000000, M_IMMD = 0x02000000, M_IND = 0x03000000, M_DWARF = 0x04000000 };
static const int x86_reg[7] = {-1, 3, 12, 13, 14, 15, 6}; /* compact numbering → DWARF */

static int step_compact(struct _Unwind_Context *c, uint32_t enc, uint64_t func) {
    uint64_t *r = c->r;
    if (enc == 0xffffffff) enc = M_IMMD | (1 << 16); /* no info: return address on top */
    switch (enc & MODE_MASK) {
    case M_FRAME: {
        uint64_t bp = r[6];
        uint64_t loc = bp - ((enc >> 16) & 0xff) * 8;
        for (int i = 0; i < 5; i++) {
            int reg = (enc >> (3 * i)) & 7;
            if (reg) r[x86_reg[reg]] = *(uint64_t *)loc;
            loc += 8;
        }
        r[R_IP] = *(uint64_t *)(bp + 8);
        r[R_SP] = bp + 16;
        r[6] = *(uint64_t *)bp;
        c->cfa = bp + 16;
        return 0;
    }
    case M_IMMD:
    case M_IND: {
        uint32_t size = (enc >> 16) & 0xff;
        if ((enc & MODE_MASK) == M_IND) size = *(const uint32_t *)(func + size) + ((enc >> 13) & 7) * 8;
        else size *= 8;
        uint32_t count = (enc >> 10) & 7, perm = enc & 0x3ff;
        int pr[6] = {0};
        switch (count) {
        case 6: case 5: pr[0] = perm / 120; perm -= pr[0] * 120; pr[1] = perm / 24; perm -= pr[1] * 24;
            pr[2] = perm / 6; perm -= pr[2] * 6; pr[3] = perm / 2; perm -= pr[3] * 2; pr[4] = perm; break;
        case 4: pr[0] = perm / 60; perm -= pr[0] * 60; pr[1] = perm / 12; perm -= pr[1] * 12; pr[2] = perm / 3; perm -= pr[2] * 3; pr[3] = perm; break;
        case 3: pr[0] = perm / 20; perm -= pr[0] * 20; pr[1] = perm / 4; perm -= pr[1] * 4; pr[2] = perm; break;
        case 2: pr[0] = perm / 5; perm -= pr[0] * 5; pr[1] = perm; break;
        case 1: pr[0] = perm; break;
        }
        int regs[6], used[7] = {0};
        for (uint32_t i = 0; i < count; i++) {
            int renum = 0;
            for (int u = 1; u < 7; u++) if (!used[u]) { if (renum == pr[i]) { regs[i] = u; used[u] = 1; break; } renum++; }
        }
        uint64_t sp = r[R_SP];
        uint64_t loc = sp + size - 8 - 8 * count;
        for (uint32_t i = 0; i < count; i++) { r[x86_reg[regs[i]]] = *(uint64_t *)loc; loc += 8; }
        r[R_IP] = *(uint64_t *)(sp + size - 8);
        r[R_SP] = sp + size;
        c->cfa = sp + size;
        return 0;
    }
    }
    return -1;
}
#else
enum { MODE_MASK = 0x0f000000, M_FRAMELESS = 0x02000000, M_DWARF = 0x03000000, M_FRAME = 0x04000000 };

static int step_compact(struct _Unwind_Context *c, uint32_t enc, uint64_t func) {
    (void)func;
    uint64_t *r = c->r;
    if (enc == 0xffffffff) enc = M_FRAMELESS; /* leaf: return address still in lr */
    uint32_t mode = enc & MODE_MASK;
    if (mode != M_FRAME && mode != M_FRAMELESS) return -1;
    uint64_t loc = mode == M_FRAME ? r[29] - 8 : r[R_SP] + 16 * ((enc >> 12) & 0xfff) - 8;
    static const int pairs[5] = {19, 21, 23, 25, 27};
    for (int i = 0; i < 5; i++) if (enc & (1u << i)) {
        r[pairs[i]] = *(uint64_t *)loc; loc -= 8;
        r[pairs[i] + 1] = *(uint64_t *)loc; loc -= 8;
    }
    for (int i = 0; i < 4; i++) if (enc & (0x100u << i)) {
        r[72 + 2 * i] = *(uint64_t *)loc; loc -= 8;
        r[73 + 2 * i] = *(uint64_t *)loc; loc -= 8;
    }
    if (mode == M_FRAME) {
        uint64_t fp = r[29];
        r[R_IP] = *(uint64_t *)(fp + 8);
        r[29] = *(uint64_t *)fp;
        r[R_SP] = fp + 16;
    } else {
        r[R_IP] = r[30];
        r[R_SP] += 16 * ((enc >> 12) & 0xfff);
    }
    r[30] = r[R_IP];
    c->cfa = r[R_SP];
    return 0;
}
#endif

/* Fill in `ctx`'s function data (personality, LSDA) for its current IP. */
static int frame_info(struct _Unwind_Context *ctx, uint32_t *enc_out) {
    uint64_t pc = ctx->r[R_IP] - 1;
    struct image *im = image_for(pc);
    if (!im) return -1;
    uint64_t func = 0, lsda = 0; void *pers = 0;
    uint32_t enc = compact_lookup(im, pc, &func, &lsda, &pers);
    *enc_out = enc;
    if (enc && enc != 0xffffffff && (enc & MODE_MASK) == M_DWARF) {
        /* personality and LSDA come from the CIE/FDE */
        struct _Unwind_Context tmp = *ctx;
        step_dwarf(&tmp, im->eh_frame + (enc & 0xffffff), pc);
        ctx->func = tmp.func; ctx->lsda = tmp.lsda; ctx->personality = tmp.personality;
        return 0;
    }
    ctx->func = func; ctx->lsda = lsda; ctx->personality = pers;
    return 0;
}

/* Move `ctx` to the caller's frame. 1 at the bottom of the stack, -1 on failure. */
static int step(struct _Unwind_Context *ctx) {
    uint32_t enc;
    if (frame_info(ctx, &enc) < 0) return -1;
    uint64_t pc = ctx->r[R_IP] - 1;
    int r;
    if (enc && enc != 0xffffffff && (enc & MODE_MASK) == M_DWARF) r = step_dwarf(ctx, image_for(pc)->eh_frame + (enc & 0xffffff), pc);
    else r = step_compact(ctx, enc, ctx->func);
    if (r) return r;
    return ctx->r[R_IP] ? 0 : 1;
}

/* ------------------------------------------------------------------ the _Unwind_* ABI */

uint64_t _Unwind_GetIP(struct _Unwind_Context *c) { return c->r[R_IP]; }
uint64_t _Unwind_GetIPInfo(struct _Unwind_Context *c, int *before) { if (before) *before = 0; return c->r[R_IP]; }
void _Unwind_SetIP(struct _Unwind_Context *c, uint64_t v) { c->r[R_IP] = v; }
uint64_t _Unwind_GetGR(struct _Unwind_Context *c, int i) { return i >= 0 && i < NREG ? c->r[i] : 0; }
void _Unwind_SetGR(struct _Unwind_Context *c, int i, uint64_t v) { if (i >= 0 && i < NREG) c->r[i] = v; }
uint64_t _Unwind_GetCFA(struct _Unwind_Context *c) { return c->r[R_SP]; }
uint64_t _Unwind_GetLanguageSpecificData(struct _Unwind_Context *c) { return c->lsda; }
uint64_t _Unwind_GetRegionStart(struct _Unwind_Context *c) { return c->func; }
void _Unwind_DeleteException(struct _Unwind_Exception *e) { if (e->cleanup) e->cleanup(1, e); }

/* Phase 2 from `ctx` (already at the first frame to consider). */
static _Unwind_Reason_Code phase2(struct _Unwind_Exception *e, struct _Unwind_Context *ctx) {
    for (;;) {
        uint32_t enc;
        if (frame_info(ctx, &enc) < 0) return _URC_FATAL_PHASE2_ERROR;
        if (ctx->personality) {
            int actions = _UA_CLEANUP_PHASE | (ctx->r[R_SP] == e->private_2 ? _UA_HANDLER_FRAME : 0);
            _Unwind_Reason_Code rc = ((personality_fn)ctx->personality)(1, actions, e->exception_class, e, ctx);
            if (rc == _URC_INSTALL_CONTEXT) __prism_unw_jump(ctx);
            if (rc != _URC_CONTINUE_UNWIND) return _URC_FATAL_PHASE2_ERROR;
        }
        int s = step(ctx);
        if (s) return s > 0 ? _URC_END_OF_STACK : _URC_FATAL_PHASE2_ERROR;
    }
}

_Unwind_Reason_Code _Unwind_RaiseException(struct _Unwind_Exception *e) {
    struct _Unwind_Context start, ctx;
    __prism_unw_getcontext(&start);
    /* `start` is this function's frame; its caller is the first frame to search. */
    if (step(&start)) return _URC_END_OF_STACK;
    ctx = start;
    for (;;) {
        uint32_t enc;
        if (frame_info(&ctx, &enc) < 0) return _URC_END_OF_STACK;
        if (ctx.personality) {
            _Unwind_Reason_Code rc = ((personality_fn)ctx.personality)(1, _UA_SEARCH_PHASE, e->exception_class, e, &ctx);
            if (rc == _URC_HANDLER_FOUND) { e->private_2 = ctx.r[R_SP]; break; }
            if (rc != _URC_CONTINUE_UNWIND) return _URC_FATAL_PHASE1_ERROR;
        }
        int s = step(&ctx);
        if (s) return _URC_END_OF_STACK;
    }
    ctx = start;
    return phase2(e, &ctx);
}

void _Unwind_Resume(struct _Unwind_Exception *e) {
    struct _Unwind_Context ctx;
    __prism_unw_getcontext(&ctx);
    if (step(&ctx) == 0) phase2(e, &ctx);
    fprintf(stderr, "*** _Unwind_Resume: unwinding failed\n");
    abort();
}

_Unwind_Reason_Code _Unwind_Resume_or_Rethrow(struct _Unwind_Exception *e) {
    if (!e->private_2) return _Unwind_RaiseException(e);
    _Unwind_Resume(e);
    return _URC_FATAL_PHASE2_ERROR;
}

/* Walk the stack (for backtrace-style callers). */
typedef _Unwind_Reason_Code (*trace_fn)(struct _Unwind_Context *, void *);
_Unwind_Reason_Code _Unwind_Backtrace(trace_fn fn, void *arg) {
    struct _Unwind_Context ctx;
    __prism_unw_getcontext(&ctx);
    for (;;) {
        if (step(&ctx)) return _URC_END_OF_STACK;
        uint32_t enc;
        frame_info(&ctx, &enc);
        if (fn(&ctx, arg) != _URC_NO_REASON) return _URC_FATAL_PHASE1_ERROR;
    }
}

int backtrace(void **buf, int n) {
    struct _Unwind_Context ctx;
    __prism_unw_getcontext(&ctx);
    int k = 0;
    while (k < n) {
        buf[k++] = (void *)ctx.r[R_IP];
        if (step(&ctx)) break;
    }
    return k;
}

/* ------------------------------------------------------------------ LSDA (gcc_except_table) */

struct lsda_result { uint64_t landing_pad; int64_t selector; int found_handler; int cleanup; };

/* Scan the LSDA for `ip`; `match(typeinfo, arg)` says whether a catch clause takes the exception. */
int __prism_lsda_scan(struct _Unwind_Context *ctx, int (*match)(void *, void *), void *arg, struct lsda_result *out) {
    const uint8_t *p = (const uint8_t *)ctx->lsda;
    out->landing_pad = 0; out->selector = 0; out->found_handler = 0; out->cleanup = 0;
    if (!p) return 0;
    uint64_t func = ctx->func;
    uint64_t ip = ctx->r[R_IP] - 1;
    uint8_t lp_enc = *p++;
    uint64_t lp_start = lp_enc == 0xff ? func : read_encoded(&p, lp_enc);
    uint8_t tt_enc = *p++;
    const uint8_t *tt_end = 0;
    if (tt_enc != 0xff) { uint64_t o = uleb(&p); tt_end = p + o; }
    uint8_t cs_enc = *p++;
    uint64_t cs_len = uleb(&p);
    const uint8_t *cs_end = p + cs_len, *actions = cs_end;
    while (p < cs_end) {
        uint64_t start = read_encoded(&p, cs_enc), len = read_encoded(&p, cs_enc), lp = read_encoded(&p, cs_enc);
        uint64_t action = uleb(&p);
        if (ip < func + start) return 0; /* sorted: no entry for ip → terminate (treated as none) */
        if (ip >= func + start + len) continue;
        if (!lp) return 0;
        out->landing_pad = lp_start + lp;
        if (!action) { out->cleanup = 1; return 1; }
        const uint8_t *a = actions + action - 1;
        for (;;) {
            int64_t ti = sleb(&a);
            const uint8_t *next_pos = a;
            int64_t next = sleb(&a);
            if (ti > 0) {
                size_t sz;
                switch (tt_enc & 0x0f) { case 2: case 0x0a: sz = 2; break; case 3: case 0x0b: sz = 4; break; default: sz = 8; }
                const uint8_t *q = tt_end - ti * sz;
                void *typeinfo = (void *)read_encoded(&q, tt_enc);
                if (match(typeinfo, arg)) { out->selector = ti; out->found_handler = 1; return 1; }
            } else if (ti == 0) {
                out->cleanup = 1;
            }
            if (!next) break;
            a = next_pos + next;
        }
        return out->cleanup;
    }
    return 0;
}
