/* The C++ ABI runtime (what libc++abi provides on macOS), written from the Itanium C++ ABI:
 * exception objects, __cxa_throw / begin_catch / end_catch / rethrow, the __gxx_personality_v0
 * personality (over unwind.c's LSDA scan), catch matching on type_info (classes with single,
 * multiple and virtual bases; pointers with qualification conversions; void*; nullptr),
 * std::exception_ptr, __dynamic_cast, std::terminate, function-local static guards,
 * operator new / delete, and the type_info objects and std::exception classes compiled code
 * refers to. C per docs/LANGUAGE.md §C.6: it calls destructors and personality code and is
 * called from landing pads. Symbol names are the mangled C++ names, set with asm labels. */
#include "prism.h"

int sched_yield(void);
void *pthread_getspecific(unsigned long);
int pthread_setspecific(unsigned long, const void *);
int strcmp(const char *, const char *);

struct _Unwind_Context;
struct _Unwind_Exception {
    uint64_t exception_class;
    void (*cleanup)(int, struct _Unwind_Exception *);
    uintptr_t private_1, private_2;
} __attribute__((aligned(16)));
int _Unwind_RaiseException(struct _Unwind_Exception *);
void _Unwind_SetGR(struct _Unwind_Context *, int, uint64_t);
void _Unwind_SetIP(struct _Unwind_Context *, uint64_t);
struct lsda_result { uint64_t landing_pad; int64_t selector; int found_handler; int cleanup; };
int __prism_lsda_scan(struct _Unwind_Context *, int (*)(void *, void *), void *, struct lsda_result *);

#define CXX_CLASS 0x434C4E47432B2B00ull     /* "CLNGC++\0" */
#define CXX_DEPENDENT 0x434C4E47432B2B01ull /* "CLNGC++\1": rethrown exception_ptr */
#define is_cxx(c) (((c) & ~1ull) == CXX_CLASS)

/* ------------------------------------------------------------------ type_info */

/* Every type_info starts {vptr, name}; the vptr says which kind it is (it points two slots into
 * one of the __cxxabiv1 vtables below). */
struct ti { void **vptr; const char *name; };
struct si_ti { struct ti t; const struct ti *base; };
struct vmi_base { const struct ti *base; long offset_flags; };
struct vmi_ti { struct ti t; unsigned flags, nbases; struct vmi_base bases[]; };
struct ptr_ti { struct ti t; unsigned flags; const struct ti *pointee; };

static void ti_dtor(void *p) { (void)p; }
#define VT(sym, var) void *var[8] __asm__(sym) = { 0, 0, (void *)ti_dtor, (void *)ti_dtor }
VT("__ZTVN10__cxxabiv117__class_type_infoE", vt_class);
VT("__ZTVN10__cxxabiv120__si_class_type_infoE", vt_si);
VT("__ZTVN10__cxxabiv121__vmi_class_type_infoE", vt_vmi);
VT("__ZTVN10__cxxabiv119__pointer_type_infoE", vt_ptr);
VT("__ZTVN10__cxxabiv129__pointer_to_member_type_infoE", vt_pmem);
VT("__ZTVN10__cxxabiv123__fundamental_type_infoE", vt_fund);
VT("__ZTVN10__cxxabiv116__enum_type_infoE", vt_enum);
VT("__ZTVN10__cxxabiv120__function_type_infoE", vt_func);
VT("__ZTVN10__cxxabiv117__array_type_infoE", vt_array);
VT("__ZTVSt9type_info", vt_type_info);

enum { K_OTHER, K_CLASS, K_SI, K_VMI, K_PTR };
static int kind(const struct ti *t) {
    if (t->vptr == vt_class + 2) return K_CLASS;
    if (t->vptr == vt_si + 2) return K_SI;
    if (t->vptr == vt_vmi + 2) return K_VMI;
    if (t->vptr == vt_ptr + 2) return K_PTR;
    return K_OTHER;
}
/* The mangled name. On arm64 the name pointer's bit 63 marks a type whose type_info may exist in
 * several images (Apple's non-unique RTTI): those compare by name, unique ones by address. */
#define NON_UNIQUE (1ull << 63)
static const char *ti_name(const struct ti *t) { return (const char *)((uintptr_t)t->name & ~NON_UNIQUE); }
static int same(const struct ti *a, const struct ti *b) {
    if (a == b || a->name == b->name) return 1;
#if defined(__aarch64__)
    if (!(((uintptr_t)a->name | (uintptr_t)b->name) & NON_UNIQUE)) return 0;
#endif
    return strcmp(ti_name(a), ti_name(b)) == 0;
}

/* Fundamental types: the type_info objects for T, T* and const T* live in the C++ runtime. */
#define FUND(code) \
    const struct ti ti_##code __asm__("__ZTI" #code) = { vt_fund + 2, #code }; \
    const struct ptr_ti tip_##code __asm__("__ZTIP" #code) = { { vt_ptr + 2, "P" #code }, 0, &ti_##code }; \
    const struct ptr_ti tipk_##code __asm__("__ZTIPK" #code) = { { vt_ptr + 2, "PK" #code }, 1, &ti_##code };
FUND(v) FUND(b) FUND(c) FUND(a) FUND(h) FUND(s) FUND(t) FUND(i) FUND(j) FUND(l) FUND(m) FUND(x) FUND(y)
FUND(n) FUND(o) FUND(f) FUND(d) FUND(e) FUND(w) FUND(Dn) FUND(Ds) FUND(Di) FUND(Du)

/* Find base `want` of an object of class `t` at `obj`: 1 with *out set to the base subobject's
 * address. Only public bases count. */
static int find_base(const struct ti *t, void *obj, const struct ti *want, void **out) {
    if (same(t, want)) { *out = obj; return 1; }
    switch (kind(t)) {
    case K_SI: return find_base(((const struct si_ti *)t)->base, obj, want, out);
    case K_VMI: {
        const struct vmi_ti *v = (const struct vmi_ti *)t;
        for (unsigned i = 0; i < v->nbases; i++) {
            long of = v->bases[i].offset_flags;
            if (!(of & 2)) continue; /* not public */
            long off = of >> 8;
            char *p = obj;
            if (obj && (of & 1)) off = *(long *)(*(char **)obj + off); /* virtual: offset in the vtable */
            if (find_base(v->bases[i].base, obj ? p + off : 0, want, out)) return 1;
        }
        return 0;
    }
    default: return 0;
    }
}

/* Does a handler for `c` catch an object of type `t` at `obj`? Sets *adj to what the handler
 * receives (the object, a base subobject, or for pointers the converted pointer value). */
static int can_catch(const struct ti *c, const struct ti *t, void *obj, void **adj) {
    if (!c) { *adj = obj; return 1; } /* catch (...) */
    if (same(c, t)) { *adj = kind(t) == K_PTR ? *(void **)obj : obj; return 1; }
    int kc = kind(c), kt = kind(t);
    if (kc == K_PTR && t == &ti_Dn) { *adj = 0; return 1; } /* nullptr to any pointer */
    if (kc == K_PTR && kt == K_PTR) {
        const struct ptr_ti *pc = (const void *)c, *pt = (const void *)t;
        if (pt->flags & ~pc->flags & 3) return 0; /* would drop const / volatile */
        void *p = *(void **)obj;
        if (same(pc->pointee, pt->pointee)) { *adj = p; return 1; }
        if (same(pc->pointee, &ti_v) && pt->pointee->vptr != vt_func + 2) { *adj = p; return 1; }
        int kp = kind(pt->pointee);
        if ((kp == K_CLASS || kp == K_SI || kp == K_VMI) && find_base(pt->pointee, p, pc->pointee, adj)) return 1;
        return 0;
    }
    if (kt == K_CLASS || kt == K_SI || kt == K_VMI) return find_base(t, obj, c, adj);
    return 0;
}

/* ------------------------------------------------------------------ exception objects */

struct cxa_exc {
    long refs;                /* primary: exception_ptr references + the in-flight one */
    struct cxa_exc *primary;  /* itself, or the primary a dependent exception rethrows */
    void *obj;
    const struct ti *type;
    void (*dest)(void *);
    struct cxa_exc *next;     /* caught-exceptions stack */
    int handlers;             /* > 0 caught that many times; < 0 being rethrown */
    int selector;
    void *adjusted;
    uint64_t landing_pad;
    struct _Unwind_Exception ue; /* last: the object follows (primary) */
};
#define HDR (((sizeof(struct cxa_exc)) + 15) & ~(size_t)15)
static struct cxa_exc *of_ue(struct _Unwind_Exception *u) { return (struct cxa_exc *)((char *)u - __builtin_offsetof(struct cxa_exc, ue)); }
static struct cxa_exc *of_obj(void *obj) { return (struct cxa_exc *)((char *)obj - sizeof(struct cxa_exc)); }

/* Per-thread: the caught stack and the uncaught count (TSD slot 12, reserved below the keys). */
struct globals { struct cxa_exc *caught; unsigned uncaught; };
#define TSD_CXA 12
struct globals *__cxa_get_globals(void) {
    struct globals *g = pthread_getspecific(TSD_CXA);
    if (!g) { g = calloc(1, sizeof *g); pthread_setspecific(TSD_CXA, g); }
    return g;
}
struct globals *__cxa_get_globals_fast(void) { return pthread_getspecific(TSD_CXA); }

void __cxa_free_exception(void *obj) { free((char *)obj - HDR); }
void *__cxa_allocate_exception(size_t n) {
    char *m = calloc(1, HDR + n);
    if (!m) abort();
    return m + HDR;
}

static void release_primary(struct cxa_exc *p) {
    if (__atomic_sub_fetch(&p->refs, 1, __ATOMIC_ACQ_REL)) return;
    if (p->dest) p->dest(p->obj);
    __cxa_free_exception(p->obj);
}
static void finish(struct cxa_exc *e) {
    if (e->primary != e) { struct cxa_exc *p = e->primary; free(e); release_primary(p); }
    else release_primary(e);
}
static void foreign_cleanup(int why, struct _Unwind_Exception *u) { (void)why; finish(of_ue(u)); }

void __cxa_begin_catch_terminate(void) __attribute__((noreturn));
void *__cxa_begin_catch(void *u);

void __cxa_throw(void *obj, const struct ti *type, void (*dest)(void *)) __attribute__((noreturn));
void __cxa_throw(void *obj, const struct ti *type, void (*dest)(void *)) {
    struct cxa_exc *e = of_obj(obj);
    e->refs = 1;
    e->primary = e;
    e->obj = obj;
    e->type = type;
    e->dest = dest;
    e->ue.exception_class = CXX_CLASS;
    e->ue.cleanup = foreign_cleanup;
    __cxa_get_globals()->uncaught++;
    _Unwind_RaiseException(&e->ue);
    /* No handler: terminate with the exception counted as caught (as libc++abi does). */
    __cxa_begin_catch(&e->ue);
    __cxa_begin_catch_terminate();
}

/* make_exception_ptr without a throw: the object is constructed after this, and the exception_ptr
 * that libc++ builds from it takes the first reference. */
void *__cxa_init_primary_exception(void *obj, const struct ti *type, void (*dest)(void *)) {
    struct cxa_exc *e = of_obj(obj);
    e->refs = 0;
    e->primary = e;
    e->obj = obj;
    e->type = type;
    e->dest = dest;
    e->ue.exception_class = CXX_CLASS;
    e->ue.cleanup = foreign_cleanup;
    return e;
}

void *__cxa_get_exception_ptr(void *u) { return is_cxx(((struct _Unwind_Exception *)u)->exception_class) ? of_ue(u)->adjusted : (void *)((struct _Unwind_Exception *)u + 1); }

void *__cxa_begin_catch(void *uv) {
    struct _Unwind_Exception *u = uv;
    struct globals *g = __cxa_get_globals();
    struct cxa_exc *e = of_ue(u);
    if (!is_cxx(u->exception_class)) {
        /* A foreign exception (Objective-C) in catch (...): one at a time, never rethrown here. */
        if (g->caught) abort();
        e->next = 0; e->handlers = 1;
        g->caught = e;
        return u + 1;
    }
    e->handlers = e->handlers < 0 ? -e->handlers + 1 : e->handlers + 1;
    if (e != g->caught) { e->next = g->caught; g->caught = e; }
    if (g->uncaught) g->uncaught--;
    return e->adjusted;
}

void __cxa_end_catch(void) {
    struct globals *g = __cxa_get_globals();
    struct cxa_exc *e = g->caught;
    if (!e) return;
    if (!is_cxx(e->ue.exception_class)) {
        g->caught = 0;
        if (e->ue.cleanup) e->ue.cleanup(1, &e->ue);
        return;
    }
    if (e->handlers < 0) {
        if (++e->handlers == 0) g->caught = e->next; /* rethrown: the new handler owns it */
    } else if (--e->handlers == 0) {
        g->caught = e->next;
        finish(e);
    }
}

void __cxa_rethrow(void) __attribute__((noreturn));
void __cxa_rethrow(void) {
    struct globals *g = __cxa_get_globals();
    struct cxa_exc *e = g->caught;
    if (!e) __cxa_begin_catch_terminate();
    if (is_cxx(e->ue.exception_class)) { e->handlers = -e->handlers; g->uncaught++; }
    else g->caught = 0;
    _Unwind_RaiseException(&e->ue);
    __cxa_begin_catch(&e->ue);
    __cxa_begin_catch_terminate();
}

const struct ti *__cxa_current_exception_type(void) {
    struct globals *g = __cxa_get_globals_fast();
    return g && g->caught && is_cxx(g->caught->ue.exception_class) ? g->caught->type : 0;
}
int __cxa_uncaught_exceptions(void) { struct globals *g = __cxa_get_globals_fast(); return g ? (int)g->uncaught : 0; }
int uncaught_exceptions(void) __asm__("__ZSt19uncaught_exceptionsv");
int uncaught_exceptions(void) { return __cxa_uncaught_exceptions(); }
int uncaught_exception(void) __asm__("__ZSt18uncaught_exceptionv");
int uncaught_exception(void) { return __cxa_uncaught_exceptions() > 0; }

/* ------------------------------------------------------------------ personality */

struct match_arg { const struct ti *type; void *obj; void *adj; };
static int matches(void *catch_ti, void *arg) {
    struct match_arg *m = arg;
    if (!m->type) return catch_ti == 0; /* foreign: only catch (...) */
    void *adj;
    if (!can_catch(catch_ti, m->type, m->obj, &adj)) return 0;
    m->adj = adj;
    return 1;
}

int __gxx_personality_v0(int version, int actions, uint64_t cls, struct _Unwind_Exception *u, struct _Unwind_Context *ctx) {
    (void)version;
    struct cxa_exc *e = is_cxx(cls) ? of_ue(u) : 0;
    struct match_arg m = { e ? e->type : 0, e ? e->obj : 0, 0 };
    struct lsda_result r;
    if (!__prism_lsda_scan(ctx, matches, &m, &r)) return 8; /* _URC_CONTINUE_UNWIND */
    if (actions & 1) { /* search */
        if (!r.found_handler) return 8;
        if (e) { e->selector = (int)r.selector; e->landing_pad = r.landing_pad; e->adjusted = m.adj; }
        return 6; /* _URC_HANDLER_FOUND */
    }
    if (actions & 4) { /* the handler frame */
        _Unwind_SetGR(ctx, 0, (uint64_t)u);
        _Unwind_SetGR(ctx, 1, (uint64_t)(e ? e->selector : r.selector));
        _Unwind_SetIP(ctx, e ? e->landing_pad : r.landing_pad);
        return 7; /* _URC_INSTALL_CONTEXT */
    }
    if (r.cleanup && !r.found_handler) {
        _Unwind_SetGR(ctx, 0, (uint64_t)u);
        _Unwind_SetGR(ctx, 1, 0);
        _Unwind_SetIP(ctx, r.landing_pad);
        return 7;
    }
    return 8;
}

/* ------------------------------------------------------------------ terminate */

typedef void (*handler_t)(void);
static void default_terminate(void);
static handler_t terminate_handler = default_terminate;
static handler_t unexpected_handler;
handler_t set_terminate(handler_t h) __asm__("__ZSt13set_terminatePFvvE");
handler_t set_terminate(handler_t h) { handler_t o = terminate_handler; terminate_handler = h ? h : default_terminate; return o; }
handler_t get_terminate(void) __asm__("__ZSt13get_terminatev");
handler_t get_terminate(void) { return terminate_handler; }
handler_t set_unexpected(handler_t h) __asm__("__ZSt14set_unexpectedPFvvE");
handler_t set_unexpected(handler_t h) { handler_t o = unexpected_handler; unexpected_handler = h; return o; }

void terminate(void) __asm__("__ZSt9terminatev") __attribute__((noreturn));
void terminate(void) {
    terminate_handler();
    abort();
}
void __cxa_begin_catch_terminate(void) { terminate(); }
void __clang_call_terminate(void *u) __attribute__((noreturn));
void __clang_call_terminate(void *u) { __cxa_begin_catch(u); terminate(); }
void __cxa_call_unexpected(void *u) __attribute__((noreturn));
void __cxa_call_unexpected(void *u) { __cxa_begin_catch(u); if (unexpected_handler) unexpected_handler(); terminate(); }
void __cxa_pure_virtual(void) { fprintf(__stderrp, "libc++abi: Pure virtual function called!\n"); abort(); }
void __cxa_deleted_virtual(void) { fprintf(__stderrp, "libc++abi: Deleted virtual function called!\n"); abort(); }

extern const struct ti ti_exception __asm__("__ZTISt9exception");
static void default_terminate(void) {
    struct globals *g = __cxa_get_globals_fast();
    struct cxa_exc *e = g ? g->caught : 0;
    if (e && is_cxx(e->ue.exception_class)) {
        void *base;
        if (find_base(e->type, e->obj, &ti_exception, &base)) {
            const char *(*what)(void *) = (*(void ***)base)[2];
            fprintf(__stderrp, "libc++abi: terminating due to uncaught exception of type %s: %s\n", ti_name(e->type), what(base));
        } else {
            fprintf(__stderrp, "libc++abi: terminating due to uncaught exception of type %s\n", ti_name(e->type));
        }
    } else if (e) {
        fprintf(__stderrp, "libc++abi: terminating due to uncaught foreign exception\n");
    } else {
        fprintf(__stderrp, "libc++abi: terminating\n");
    }
    abort();
}

/* ------------------------------------------------------------------ exception_ptr */

/* std::exception_ptr holds the primary exception's object pointer. Functions that return one or
 * take one by value use an indirect pointer (it has a non-trivial destructor). */
static void ptr_ref(void *obj) { if (obj) __atomic_add_fetch(&of_obj(obj)->refs, 1, __ATOMIC_ACQ_REL); }
static void ptr_unref(void *obj) { if (obj) release_primary(of_obj(obj)); }

void ep_dtor(void **p) __asm__("__ZNSt13exception_ptrD1Ev");
void ep_dtor(void **p) { ptr_unref(*p); }
void ep_dtor2(void **p) __asm__("__ZNSt13exception_ptrD2Ev");
void ep_dtor2(void **p) { ptr_unref(*p); }
void ep_copy(void **d, void *const *s) __asm__("__ZNSt13exception_ptrC1ERKS_");
void ep_copy(void **d, void *const *s) { *d = *s; ptr_ref(*d); }
void ep_copy2(void **d, void *const *s) __asm__("__ZNSt13exception_ptrC2ERKS_");
void ep_copy2(void **d, void *const *s) { *d = *s; ptr_ref(*d); }
void **ep_assign(void **d, void *const *s) __asm__("__ZNSt13exception_ptraSERKS_");
void **ep_assign(void **d, void *const *s) { void *o = *d; *d = *s; ptr_ref(*d); ptr_unref(o); return d; }

void *__cxa_current_primary_exception(void) {
    struct globals *g = __cxa_get_globals_fast();
    if (!g || !g->caught || !is_cxx(g->caught->ue.exception_class)) return 0;
    void *obj = g->caught->primary->obj;
    ptr_ref(obj);
    return obj;
}
void __cxa_decrement_exception_refcount(void *obj) { ptr_unref(obj); }
void __cxa_increment_exception_refcount(void *obj) { ptr_ref(obj); }

/* current_exception() returns an exception_ptr through the hidden result pointer (rdi / x8). */
void __prism_current_exception(void **out) { *out = __cxa_current_primary_exception(); }
#if defined(__x86_64__)
__asm__(".text\n.globl __ZSt17current_exceptionv\n__ZSt17current_exceptionv:\n"
        " push %rdi\n call ___prism_current_exception\n pop %rax\n ret\n");
#else
__asm__(".text\n.globl __ZSt17current_exceptionv\n.p2align 2\n__ZSt17current_exceptionv:\n"
        " stp x29, x30, [sp, #-16]!\n mov x29, sp\n str x8, [sp, #-16]!\n mov x0, x8\n"
        " bl ___prism_current_exception\n ldr x0, [sp], #16\n ldp x29, x30, [sp], #16\n ret\n");
#endif

/* rethrow_exception(exception_ptr): a dependent exception that refers to the primary. */
void __cxa_rethrow_primary_exception(void *obj) {
    if (!obj) return;
    struct cxa_exc *p = of_obj(obj);
    struct cxa_exc *d = calloc(1, sizeof *d);
    if (!d) abort();
    ptr_ref(obj);
    d->primary = p;
    d->obj = obj;
    d->type = p->type;
    d->dest = p->dest;
    d->ue.exception_class = CXX_DEPENDENT;
    d->ue.cleanup = foreign_cleanup;
    __cxa_get_globals()->uncaught++;
    _Unwind_RaiseException(&d->ue);
    __cxa_begin_catch(&d->ue);
}
void rethrow_exception(void **p) __asm__("__ZSt17rethrow_exceptionSt13exception_ptr") __attribute__((noreturn));
void rethrow_exception(void **p) {
    __cxa_rethrow_primary_exception(*p);
    terminate();
}

/* ------------------------------------------------------------------ dynamic_cast */

/* Downcasts and cross-casts: find `dst` among the public bases of the most-derived object. */
void *__dynamic_cast(const void *sub, const struct ti *src, const struct ti *dst, long hint) {
    (void)src; (void)hint;
    if (!sub) return 0;
    void **vptr = *(void ***)sub;
    long to_top = (long)vptr[-2];
    const struct ti *whole_t = vptr[-1];
    char *whole = (char *)sub + to_top;
    void *out;
    return find_base(whole_t, whole, dst, &out) ? out : 0;
}

/* ------------------------------------------------------------------ function-local statics */

/* Guard: byte 0 = initialized, byte 1 = initialization in progress. */
int __cxa_guard_acquire(uint64_t *g) {
    uint8_t *b = (uint8_t *)g;
    for (;;) {
        if (__atomic_load_n(&b[0], __ATOMIC_ACQUIRE)) return 0;
        uint8_t z = 0;
        if (__atomic_compare_exchange_n(&b[1], &z, 1, 0, __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) {
            if (__atomic_load_n(&b[0], __ATOMIC_ACQUIRE)) { __atomic_store_n(&b[1], 0, __ATOMIC_RELEASE); return 0; }
            return 1;
        }
        sched_yield();
    }
}
void __cxa_guard_release(uint64_t *g) { uint8_t *b = (uint8_t *)g; __atomic_store_n(&b[0], 1, __ATOMIC_RELEASE); __atomic_store_n(&b[1], 0, __ATOMIC_RELEASE); }
void __cxa_guard_abort(uint64_t *g) { __atomic_store_n(&((uint8_t *)g)[1], 0, __ATOMIC_RELEASE); }

/* ------------------------------------------------------------------ operator new / delete */

static void throw_bad_alloc(void) __attribute__((noreturn));
static void *new_or_throw(size_t n) {
    void *p = malloc(n ? n : 1);
    if (!p) throw_bad_alloc();
    return p;
}
static void *aligned_new(size_t n, size_t a) {
    if (a < 16) return new_or_throw(n);
    void *p = 0;
    int posix_memalign(void **, size_t, size_t);
    if (posix_memalign(&p, a, n ? n : 1) || !p) throw_bad_alloc();
    return p;
}
#define NEW(sym, args, body) void *sym##_f args __asm__(#sym); void *sym##_f args { body }
#define DEL(sym, args) void sym##_f args __asm__(#sym); void sym##_f args { free(p); }
NEW(__Znwm, (size_t n), return new_or_throw(n);)
NEW(__Znam, (size_t n), return new_or_throw(n);)
NEW(__ZnwmRKSt9nothrow_t, (size_t n, const void *t), (void)t; return malloc(n ? n : 1);)
NEW(__ZnamRKSt9nothrow_t, (size_t n, const void *t), (void)t; return malloc(n ? n : 1);)
NEW(__ZnwmSt11align_val_t, (size_t n, size_t a), return aligned_new(n, a);)
NEW(__ZnamSt11align_val_t, (size_t n, size_t a), return aligned_new(n, a);)
DEL(__ZdlPv, (void *p))
DEL(__ZdaPv, (void *p))
DEL(__ZdlPvm, (void *p, size_t n))
DEL(__ZdaPvm, (void *p, size_t n))
DEL(__ZdlPvSt11align_val_t, (void *p, size_t a))
DEL(__ZdaPvSt11align_val_t, (void *p, size_t a))
DEL(__ZdlPvmSt11align_val_t, (void *p, size_t n, size_t a))
DEL(__ZdaPvmSt11align_val_t, (void *p, size_t n, size_t a))
DEL(__ZdlPvRKSt9nothrow_t, (void *p, const void *t))
DEL(__ZdaPvRKSt9nothrow_t, (void *p, const void *t))
const char std_nothrow __asm__("__ZSt7nothrow") = 0;

/* ------------------------------------------------------------------ std::exception and friends */

/* Each class: its type_info (single inheritance from its base), vtable {0, &type_info, D1, D0,
 * what}, constructor, destructors and what(). The objects are just the vtable pointer. */
static void exc_dtor(void *self) { (void)self; }
static void exc_delete(void *self) { free(self); }
const struct ti ti_exception __asm__("__ZTISt9exception") = { vt_class + 2, "St9exception" };

#define EXC_CLASS(M, name, text, base_ti)                                                              \
    extern void *vt_##M[5] __asm__("__ZTV" #name);                                                    \
    static const char *what_##M(void *self) { (void)self; return text; }                              \
    void *vt_##M[5] = { 0, (void *)&base_ti, (void *)exc_dtor, (void *)exc_delete, (void *)what_##M };  \
    void ctor_##M(void **self) __asm__("__ZN" #name "C1Ev");                                          \
    void ctor_##M(void **self) { *self = vt_##M + 2; }                                                \
    void ctor2_##M(void **self) __asm__("__ZN" #name "C2Ev");                                         \
    void ctor2_##M(void **self) { *self = vt_##M + 2; }                                               \
    void d1_##M(void *self) __asm__("__ZN" #name "D1Ev");                                             \
    void d1_##M(void *self) { (void)self; }                                                           \
    void d2_##M(void *self) __asm__("__ZN" #name "D2Ev");                                             \
    void d2_##M(void *self) { (void)self; }                                                           \
    void d0_##M(void *self) __asm__("__ZN" #name "D0Ev");                                             \
    void d0_##M(void *self) { free(self); }                                                           \
    const char *w_##M(void *self) __asm__("__ZNK" #name "4whatEv");                                   \
    const char *w_##M(void *self) { return what_##M(self); }

#define SI(M, mangled, base) const struct si_ti ti_##M __asm__("__ZTI" mangled) = { { vt_si + 2, mangled }, &base };
SI(bad_alloc, "St9bad_alloc", ti_exception)
SI(bad_array_new_length, "St20bad_array_new_length", ti_bad_alloc.t)
SI(bad_cast, "St8bad_cast", ti_exception)
SI(bad_typeid, "St10bad_typeid", ti_exception)
SI(bad_exception, "St13bad_exception", ti_exception)

EXC_CLASS(exception, St9exception, "std::exception", ti_exception)
EXC_CLASS(bad_alloc, St9bad_alloc, "std::bad_alloc", ti_bad_alloc)
EXC_CLASS(bad_array_new_length, St20bad_array_new_length, "bad_array_new_length", ti_bad_array_new_length)
EXC_CLASS(bad_cast, St8bad_cast, "std::bad_cast", ti_bad_cast)
EXC_CLASS(bad_typeid, St10bad_typeid, "std::bad_typeid", ti_bad_typeid)
EXC_CLASS(bad_exception, St13bad_exception, "std::bad_exception", ti_bad_exception)

static void throw_bad_alloc(void) {
    void **o = __cxa_allocate_exception(sizeof(void *));
    *o = vt_bad_alloc + 2;
    __cxa_throw(o, &ti_bad_alloc.t, d1_bad_alloc);
}
void __cxa_bad_cast(void) __attribute__((noreturn));
void __cxa_bad_cast(void) {
    void **o = __cxa_allocate_exception(sizeof(void *));
    *o = vt_bad_cast + 2;
    __cxa_throw(o, &ti_bad_cast.t, d1_bad_cast);
}
void __cxa_bad_typeid(void) __attribute__((noreturn));
void __cxa_bad_typeid(void) {
    void **o = __cxa_allocate_exception(sizeof(void *));
    *o = vt_bad_typeid + 2;
    __cxa_throw(o, &ti_bad_typeid.t, d1_bad_typeid);
}
void __cxa_throw_bad_array_new_length(void) __attribute__((noreturn));
void __cxa_throw_bad_array_new_length(void) {
    void **o = __cxa_allocate_exception(sizeof(void *));
    *o = vt_bad_array_new_length + 2;
    __cxa_throw(o, &ti_bad_array_new_length.t, d1_bad_array_new_length);
}

/* type_info members compiled code may call out of line. */
void tinfo_d1(void *t) __asm__("__ZNSt9type_infoD1Ev");
void tinfo_d1(void *t) { (void)t; }
void tinfo_d2(void *t) __asm__("__ZNSt9type_infoD2Ev");
void tinfo_d2(void *t) { (void)t; }
int ti_before(const struct ti *a, const struct ti *b) __asm__("__ZNKSt9type_info6beforeERKS_");
int ti_before(const struct ti *a, const struct ti *b) { return strcmp(ti_name(a), ti_name(b)) < 0; }

/* No demangler yet: report "invalid mangled name" (status -2), which callers handle. */
char *__cxa_demangle(const char *name, char *buf, size_t *n, int *status) {
    (void)name; (void)buf; (void)n;
    if (status) *status = -2;
    return 0;
}
