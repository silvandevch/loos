/* Prism's Objective-C runtime (docs/LANGUAGE.md §C.6: every message send calls into the program,
 * and compiled code reads these layouts directly). It reads the class, category, selector and
 * protocol metadata of every loaded image (found through the _dyld_* calls), realizes classes
 * eagerly, and dispatches messages through per-class caches. One thread: no locking yet. */
#include "objc.h"

uint32_t _dyld_image_count(void);
const void *_dyld_get_image_header(uint32_t);
intptr_t _dyld_get_image_vmaddr_slide(uint32_t);

/* ------------------------------------------------------------------ small hash tables */

static uint64_t hash_str(const char *s) {
    uint64_t h = 1469598103934665603ull;
    while (*s) h = (h ^ (unsigned char)*s++) * 1099511628211ull;
    return h;
}
static uint64_t hash_ptr(const void *p) {
    uint64_t h = (uintptr_t)p;
    h ^= h >> 33; h *= 0xff51afd7ed558ccdull; h ^= h >> 33;
    return h;
}

/* String-keyed table (selectors, class names, protocols). */
struct strtab { uint32_t mask, used; struct { const char *k; void *v; } *e; };

static void *strtab_get(struct strtab *t, const char *k) {
    if (!t->e) return 0;
    for (uint64_t i = hash_str(k) & t->mask;; i = (i + 1) & t->mask) {
        if (!t->e[i].k) return 0;
        if (!strcmp(t->e[i].k, k)) return t->e[i].v;
    }
}
static void strtab_put(struct strtab *t, const char *k, void *v);
static void strtab_grow(struct strtab *t) {
    uint32_t old = t->e ? t->mask + 1 : 0;
    __typeof__(t->e) e = t->e;
    t->mask = old ? old * 2 - 1 : 255;
    t->used = 0;
    t->e = calloc(t->mask + 1, sizeof *t->e);
    for (uint32_t i = 0; i < old; i++) if (e[i].k) strtab_put(t, e[i].k, e[i].v);
    free(e);
}
static void strtab_put(struct strtab *t, const char *k, void *v) {
    if (!t->e || (t->used + 1) * 4 > (t->mask + 1) * 3) strtab_grow(t);
    for (uint64_t i = hash_str(k) & t->mask;; i = (i + 1) & t->mask) {
        if (!t->e[i].k) { t->e[i].k = k; t->e[i].v = v; t->used++; return; }
        if (!strcmp(t->e[i].k, k)) { t->e[i].v = v; return; }
    }
}

/* Pointer-keyed table with deletion (refcounts, weak references, associations). */
struct ptrtab { uint32_t mask, used; struct { const void *k; uintptr_t v; } *e; };
#define TOMB ((const void *)1)

static uintptr_t *ptrtab_find(struct ptrtab *t, const void *k) {
    if (!t->e) return 0;
    for (uint64_t i = hash_ptr(k) & t->mask;; i = (i + 1) & t->mask) {
        if (!t->e[i].k) return 0;
        if (t->e[i].k == k) return &t->e[i].v;
    }
}
static uintptr_t *ptrtab_add(struct ptrtab *t, const void *k);
static void ptrtab_grow(struct ptrtab *t) {
    uint32_t old = t->e ? t->mask + 1 : 0;
    __typeof__(t->e) e = t->e;
    t->mask = old ? old * 2 - 1 : 255;
    t->used = 0;
    t->e = calloc(t->mask + 1, sizeof *t->e);
    for (uint32_t i = 0; i < old; i++) if (e[i].k && e[i].k != TOMB) *ptrtab_add(t, e[i].k) = e[i].v;
    free(e);
}
static uintptr_t *ptrtab_add(struct ptrtab *t, const void *k) {
    uintptr_t *f = ptrtab_find(t, k);
    if (f) return f;
    if (!t->e || (t->used + 1) * 2 > (t->mask + 1)) ptrtab_grow(t);
    for (uint64_t i = hash_ptr(k) & t->mask;; i = (i + 1) & t->mask) {
        if (!t->e[i].k || t->e[i].k == TOMB) { t->e[i].k = k; t->e[i].v = 0; t->used++; return &t->e[i].v; }
    }
}
static void ptrtab_del(struct ptrtab *t, const void *k) {
    if (!t->e) return;
    for (uint64_t i = hash_ptr(k) & t->mask;; i = (i + 1) & t->mask) {
        if (!t->e[i].k) return;
        if (t->e[i].k == k) { t->e[i].k = TOMB; return; }
    }
}

/* ------------------------------------------------------------------ selectors, classes */

static struct strtab selectors, classes, protocols;
static Class all_classes;
SEL _prism_sel_retain, _prism_sel_release, _prism_sel_autorelease, _prism_sel_dealloc, _prism_sel_alloc, _prism_sel_init,
    _prism_sel_initialize, _prism_sel_class, _prism_sel_copy, _prism_sel_new, _prism_sel_isKindOfClass, _prism_sel_respondsToSelector;
static SEL sel_load, sel_cxx_destruct, sel_forwardingTarget, sel_resolveInstance, sel_resolveClass, sel_dnr;

/* `name` must stay valid: image strings do; others are copied. */
static SEL sel_intern(const char *name, int copy) {
    SEL s = strtab_get(&selectors, name);
    if (s) return s;
    s = copy ? strdup(name) : name;
    strtab_put(&selectors, s, (void *)s);
    return s;
}
SEL sel_registerName(const char *name) { return name ? sel_intern(name, 1) : 0; }
SEL sel_getUid(const char *name) { return sel_registerName(name); }
const char *sel_getName(SEL s) { return s ? s : "<null selector>"; }
BOOL sel_isEqual(SEL a, SEL b) { return a == b; }

Class objc_getClass(const char *name) { return name ? strtab_get(&classes, name) : 0; }
Class objc_lookUpClass(const char *name) { return objc_getClass(name); }
id objc_getMetaClass(const char *name) { Class c = objc_getClass(name); return c ? (id)c->isa : nil; }
Class object_getClass(id o) { return o ? o->isa : 0; }
Class object_setClass(id o, Class c) { if (!o) return 0; Class old = o->isa; o->isa = c; return old; }
const char *class_getName(Class c) { return c ? ro_of(c)->name : "nil"; }
const char *object_getClassName(id o) { return class_getName(object_getClass(o)); }
Class class_getSuperclass(Class c) { return c ? c->superclass : 0; }
BOOL class_isMetaClass(Class c) { return c && (ro_of(c)->flags & RO_META); }
size_t class_getInstanceSize(Class c) { return c ? ro_of(c)->instance_size : 0; }
int class_getVersion(Class c) { (void)c; return 0; }
void class_setVersion(Class c, int v) { (void)c; (void)v; }

/* ------------------------------------------------------------------ method lists */

static uint32_t ml_entsize(const struct method_list *m) { return m->entsize_flags & 0xfffc; }
static struct method_t *ml_get(struct method_list *m, uint32_t i) {
    return (struct method_t *)((char *)(m + 1) + (size_t)i * ml_entsize(m));
}

/* A copy of `m` with absolute entries and uniqued selectors (relative lists become absolute). */
static struct method_list *ml_prepare(struct method_list *m) {
    if (!m || !m->count) return 0;
    struct method_list *out = malloc(sizeof *out + m->count * sizeof(struct method_t));
    out->entsize_flags = sizeof(struct method_t);
    out->count = m->count;
    int rel = (m->entsize_flags & 0x80000000u) != 0;
    for (uint32_t i = 0; i < m->count; i++) {
        struct method_t *d = (struct method_t *)(out + 1) + i;
        if (rel) {
            const int32_t *e = (const int32_t *)((char *)(m + 1) + (size_t)i * 12);
            /* name: offset to a selector reference (or, flag 0x40000000, to the name itself) */
            const char *nm = (m->entsize_flags & 0x40000000u) ? (const char *)e + e[0] : *(const char **)((const char *)e + e[0]);
            d->name = sel_intern(nm, 0);
            d->types = (const char *)(e + 1) + e[1];
            d->imp = (IMP)((const char *)(e + 2) + e[2]);
        } else {
            struct method_t *s = ml_get(m, i);
            d->name = sel_intern(s->name, 0);
            d->types = s->types;
            d->imp = s->imp;
        }
    }
    return out;
}

static void rw_add_list(struct rw *r, struct method_list *m) {
    if (!m) return;
    if (r->nlists == r->caplists) {
        r->caplists = r->caplists ? r->caplists * 2 : 4;
        r->lists = realloc(r->lists, r->caplists * sizeof *r->lists);
    }
    memmove(r->lists + 1, r->lists, r->nlists * sizeof *r->lists);
    r->lists[0] = m;
    r->nlists++;
}

static void rw_add_protocols(struct rw *r, struct protocol_list *p) {
    if (!p || !p->count) return;
    if (r->nprotos == r->capprotos) {
        r->capprotos = r->capprotos ? r->capprotos * 2 : 4;
        r->protos = realloc(r->protos, r->capprotos * sizeof *r->protos);
    }
    r->protos[r->nprotos++] = p;
}

static struct method_t *find_own(Class c, SEL sel) {
    struct rw *r = rw_of(c);
    if (!r) return 0;
    for (uint32_t l = 0; l < r->nlists; l++) {
        struct method_list *m = r->lists[l];
        for (uint32_t i = 0; i < m->count; i++) {
            struct method_t *e = (struct method_t *)(m + 1) + i;
            if (e->name == sel) return e;
        }
    }
    return 0;
}

/* ------------------------------------------------------------------ caches */

static void flush_all_caches(void) {
    for (Class c = all_classes; c; c = rw_of(c)->next) {
        struct rw *r = rw_of(c);
        free(r->cache); r->cache = 0;
        struct rw *m = rw_of(c->isa);
        if (m) { free(m->cache); m->cache = 0; }
    }
}

static IMP cache_get(struct rw *r, SEL sel) {
    struct cache *c = r->cache;
    if (!c) return 0;
    for (uintptr_t i = ((uintptr_t)sel >> 3) & c->mask;; i = (i + 1) & c->mask) {
        if (c->e[i].sel == sel) return c->e[i].imp;
        if (!c->e[i].sel) return 0;
    }
}

static void cache_put(struct rw *r, SEL sel, IMP imp) {
    struct cache *c = r->cache;
    if (!c || (c->used + 1) * 4 > (c->mask + 1) * 3) {
        uint32_t n = c ? (c->mask + 1) * 2 : 16;
        struct cache *nc = calloc(1, sizeof *nc + n * sizeof(struct cache_entry));
        nc->mask = n - 1;
        r->cache = nc;
        if (c) {
            for (uint32_t i = 0; i <= c->mask; i++) if (c->e[i].sel) cache_put(r, c->e[i].sel, c->e[i].imp);
            free(c);
        }
        c = r->cache;
    }
    for (uintptr_t i = ((uintptr_t)sel >> 3) & c->mask;; i = (i + 1) & c->mask) {
        if (!c->e[i].sel) { c->e[i].sel = sel; c->e[i].imp = imp; c->used++; return; }
        if (c->e[i].sel == sel) { c->e[i].imp = imp; return; }
    }
}

/* ------------------------------------------------------------------ realization */

static void realize(Class c);

static void attach_category(struct category_t *cat) {
    Class c = cat->cls;
    rw_add_list(rw_of(c), ml_prepare(cat->instance_methods));
    rw_add_list(rw_of(c->isa), ml_prepare(cat->class_methods));
    rw_add_protocols(rw_of(c), cat->protocols);
    flush_all_caches();
}

static struct category_t **pending; static uint32_t npending, cappending;

static void realize(Class c) {
    if (!c || rw_of(c)) return;
    const struct class_ro *ro = (const struct class_ro *)(c->bits & DATA_MASK);
    if (c->superclass) realize(c->superclass);
    struct rw *r = calloc(1, sizeof *r);
    /* Non-fragile ivars: slide this class's ivars past a superclass that grew. */
    if (c->superclass && !(ro->flags & RO_META)) {
        uint32_t super_size = ro_of(c->superclass)->instance_size;
        if (super_size > ro->instance_start) {
            struct class_ro *copy = malloc(sizeof *copy);
            *copy = *ro;
            uint32_t delta = super_size - ro->instance_start;
            if (ro->ivars) {
                const struct ivar_list *iv = ro->ivars;
                for (uint32_t i = 0; i < iv->count; i++) {
                    const struct ivar_t *v = (const struct ivar_t *)((const char *)&iv->first + (size_t)i * iv->entsize);
                    uint32_t align = v->alignment_raw == ~0u ? 8 : 1u << v->alignment_raw;
                    if (align > 1) delta = (delta + align - 1) & ~(align - 1);
                }
                for (uint32_t i = 0; i < iv->count; i++) {
                    const struct ivar_t *v = (const struct ivar_t *)((const char *)&iv->first + (size_t)i * iv->entsize);
                    if (v->offset) *v->offset += delta;
                }
            }
            copy->instance_start += delta;
            copy->instance_size += delta;
            ro = copy;
        }
    }
    r->ro = ro;
    rw_add_list(r, ml_prepare(ro->methods));
    rw_add_protocols(r, ro->protocols);
    c->bits = (uintptr_t)r | RW_TAG;
    if (!(ro->flags & RO_META)) {
        realize(c->isa);
        r->next = all_classes;
        all_classes = c;
        strtab_put(&classes, ro->name, c);
    }
    for (uint32_t i = 0; i < npending;) {
        if (pending[i]->cls == c) {
            attach_category(pending[i]);
            pending[i] = pending[--npending];
        } else i++;
    }
}

/* ------------------------------------------------------------------ images */

struct seg64 { uint32_t cmd, cmdsize; char segname[16]; uint64_t vmaddr, vmsize, fileoff, filesize; uint32_t maxprot, initprot, nsects, flags; };
struct sect64 { char sectname[16], segname[16]; uint64_t addr, size; uint32_t offset, align, reloff, nreloc, flags, r1, r2, r3; };

static void *section(const void *hdr, intptr_t slide, const char *name, size_t *count, size_t elem) {
    const uint32_t *h = hdr;
    const char *p = (const char *)hdr + 32;
    for (uint32_t i = 0; i < h[4]; i++) {
        const struct seg64 *s = (const void *)p;
        if (s->cmd == 0x19) {
            const struct sect64 *x = (const void *)(s + 1);
            for (uint32_t j = 0; j < s->nsects; j++)
                if (!strncmp(x[j].sectname, name, 16) && (!strncmp(x[j].segname, "__DATA", 6))) {
                    *count = x[j].size / elem;
                    return (void *)(x[j].addr + slide);
                }
        }
        p += s->cmdsize;
    }
    *count = 0;
    return 0;
}

static Class protocol_class;

static void map_image(const void *hdr, intptr_t slide) {
    size_t n;
    SEL *refs = section(hdr, slide, "__objc_selrefs", &n, sizeof(SEL));
    for (size_t i = 0; i < n; i++) refs[i] = sel_intern(refs[i], 0);
    struct protocol_t **pl = section(hdr, slide, "__objc_protolist", &n, sizeof(void *));
    for (size_t i = 0; i < n; i++) {
        if (!strtab_get(&protocols, pl[i]->name)) strtab_put(&protocols, pl[i]->name, pl[i]);
        if (protocol_class) pl[i]->isa = (id)protocol_class;
    }
    struct protocol_t **pr = section(hdr, slide, "__objc_protorefs", &n, sizeof(void *));
    for (size_t i = 0; i < n; i++) { struct protocol_t *c = strtab_get(&protocols, pr[i]->name); if (c) pr[i] = c; }
    Class *cl = section(hdr, slide, "__objc_classlist", &n, sizeof(Class));
    for (size_t i = 0; i < n; i++) realize(cl[i]);
    struct category_t **cats = section(hdr, slide, "__objc_catlist", &n, sizeof(void *));
    for (size_t i = 0; i < n; i++) {
        struct category_t *cat = cats[i];
        if (!cat->cls) continue; /* weak-linked class that is not there */
        if (rw_of(cat->cls)) attach_category(cat);
        else {
            if (npending == cappending) { cappending = cappending ? cappending * 2 : 16; pending = realloc(pending, cappending * sizeof *pending); }
            pending[npending++] = cat;
        }
    }
}

static void call_load(Class c) {
    if (!c) return;
    struct rw *r = rw_of(c);
    if (r->flags & RW_LOADED) return;
    call_load(c->superclass);
    r->flags |= RW_LOADED;
    struct method_t *m = find_own(c->isa, sel_load);
    if (m) ((void (*)(Class, SEL))m->imp)(c, sel_load);
}

static void load_image(const void *hdr, intptr_t slide) {
    size_t n;
    Class *cl = section(hdr, slide, "__objc_nlclslist", &n, sizeof(Class));
    for (size_t i = 0; i < n; i++) call_load(cl[i]);
    struct category_t **cats = section(hdr, slide, "__objc_nlcatlist", &n, sizeof(void *));
    for (size_t i = 0; i < n; i++) {
        struct method_list *m = cats[i]->class_methods;
        if (!m || !cats[i]->cls) continue;
        struct method_list *p = ml_prepare(m);
        for (uint32_t j = 0; j < p->count; j++) {
            struct method_t *e = (struct method_t *)(p + 1) + j;
            if (e->name == sel_load) ((void (*)(Class, SEL))e->imp)(cats[i]->cls, sel_load);
        }
        free(p);
    }
}

extern uintptr_t _NSConcreteStackBlock[], _NSConcreteMallocBlock[], _NSConcreteGlobalBlock[],
    _NSConcreteAutoBlock[], _NSConcreteFinalizingBlock[], _NSConcreteWeakBlockVariable[];

__attribute__((constructor)) static void objc_init(void) {
    _prism_sel_retain = sel_intern("retain", 0);
    _prism_sel_release = sel_intern("release", 0);
    _prism_sel_autorelease = sel_intern("autorelease", 0);
    _prism_sel_dealloc = sel_intern("dealloc", 0);
    _prism_sel_alloc = sel_intern("alloc", 0);
    _prism_sel_init = sel_intern("init", 0);
    _prism_sel_initialize = sel_intern("initialize", 0);
    _prism_sel_class = sel_intern("class", 0);
    _prism_sel_copy = sel_intern("copy", 0);
    _prism_sel_new = sel_intern("new", 0);
    _prism_sel_isKindOfClass = sel_intern("isKindOfClass:", 0);
    _prism_sel_respondsToSelector = sel_intern("respondsToSelector:", 0);
    sel_load = sel_intern("load", 0);
    sel_cxx_destruct = sel_intern(".cxx_destruct", 0);
    sel_forwardingTarget = sel_intern("forwardingTargetForSelector:", 0);
    sel_resolveInstance = sel_intern("resolveInstanceMethod:", 0);
    sel_resolveClass = sel_intern("resolveClassMethod:", 0);
    sel_dnr = sel_intern("doesNotRecognizeSelector:", 0);
    uint32_t n = _dyld_image_count();
    for (uint32_t i = 0; i < n; i++) map_image(_dyld_get_image_header(i), _dyld_get_image_vmaddr_slide(i));
    protocol_class = objc_getClass("Protocol");
    for (uint32_t i = 0; i < n; i++) {
        size_t k;
        struct protocol_t **pl = section(_dyld_get_image_header(i), _dyld_get_image_vmaddr_slide(i), "__objc_protolist", &k, sizeof(void *));
        for (size_t j = 0; j < k; j++) pl[j]->isa = (id)protocol_class;
    }
    /* Blocks are objects: their isa words become copies of the block classes. */
    struct { uintptr_t *isa; const char *cls; } blocks[] = {
        {_NSConcreteStackBlock, "__NSStackBlock__"}, {_NSConcreteMallocBlock, "__NSMallocBlock__"}, {_NSConcreteGlobalBlock, "__NSGlobalBlock__"},
        {_NSConcreteAutoBlock, "__NSAutoBlock__"}, {_NSConcreteFinalizingBlock, "__NSFinalizingBlock__"}, {_NSConcreteWeakBlockVariable, "__NSBlockVariable__"},
    };
    for (size_t i = 0; i < sizeof blocks / sizeof blocks[0]; i++) {
        Class c = objc_getClass(blocks[i].cls);
        if (c) memcpy(blocks[i].isa, c, sizeof *c);
    }
    for (uint32_t i = 0; i < n; i++) load_image(_dyld_get_image_header(i), _dyld_get_image_vmaddr_slide(i));
}

/* ------------------------------------------------------------------ lookup and dispatch */

static int is_meta(Class c) { return (ro_of(c)->flags & RO_META) != 0; }

static IMP lookup(Class cls, SEL sel);

static void initialize_class(Class c) {
    struct rw *r = rw_of(c);
    if (!r || (r->flags & (RW_INITIALIZED | RW_INITIALIZING))) return;
    if (c->superclass) initialize_class(c->superclass);
    r->flags |= RW_INITIALIZING;
    IMP imp = lookup(c->isa, _prism_sel_initialize);
    if (imp) ((void (*)(Class, SEL))imp)(c, _prism_sel_initialize);
    r->flags = (r->flags & ~RW_INITIALIZING) | RW_INITIALIZED;
}

/* IMP for `sel` in `cls` and its superclasses (0 if none), cached in `cls`. */
static IMP lookup(Class cls, SEL sel) {
    struct rw *r = rw_of(cls);
    if (!r) { realize(cls); r = rw_of(cls); if (!r) return 0; }
    IMP imp = cache_get(r, sel);
    if (imp) return imp;
    for (Class c = cls; c; c = c->superclass) {
        struct method_t *m = find_own(c, sel);
        if (m) { cache_put(r, sel, m->imp); return m->imp; }
    }
    return 0;
}

static void unrecognized_imp(id self, SEL sel) { _prism_objc_unrecognized(self, sel); }

/* The class a message to `self` initializes: the class itself, or the receiver of a class message. */
static void ensure_initialized(id self, Class cls) {
    Class c = is_meta(cls) ? (Class)self : cls;
    struct rw *r = rw_of(c);
    if (r && !(r->flags & RW_INITIALIZED)) initialize_class(c);
}

static IMP resolve_or_forward(id self, Class cls, SEL sel, id *slot) {
    /* +resolveInstanceMethod: / +resolveClassMethod: may add the method. */
    if (sel != sel_resolveInstance && sel != sel_resolveClass) {
        Class meta = is_meta(cls) ? cls : cls->isa;
        id target = is_meta(cls) ? self : (id)cls;
        SEL rs = is_meta(cls) ? sel_resolveClass : sel_resolveInstance;
        IMP r = lookup(meta, rs);
        if (r && ((BOOL(*)(id, SEL, SEL))r)(target, rs, sel)) {
            IMP imp = lookup(cls, sel);
            if (imp) return imp;
        }
    }
    if (sel != sel_forwardingTarget && slot) {
        IMP f = lookup(cls, sel_forwardingTarget);
        id t = f ? ((id(*)(id, SEL, SEL))f)(self, sel_forwardingTarget, sel) : nil;
        if (t && t != self) {
            *slot = t;
            ensure_initialized(t, t->isa);
            IMP imp = lookup(t->isa, sel);
            if (imp) return imp;
            self = t; cls = t->isa;
        }
    }
    IMP dnr = lookup(cls, sel_dnr);
    if (dnr) ((void (*)(id, SEL, SEL))dnr)(self, sel_dnr, sel);
    return (IMP)unrecognized_imp;
}

/* Called by objc_msgSend with the receiver's saved slot (forwarding may replace the receiver). */
IMP _prism_objc_lookup(id self, SEL sel, id *slot) {
    Class cls = self->isa;
    struct rw *r = rw_of(cls);
    IMP imp = r ? cache_get(r, sel) : 0;
    if (imp) return imp;
    ensure_initialized(self, cls);
    imp = lookup(cls, sel);
    return imp ? imp : resolve_or_forward(self, cls, sel, slot);
}

struct objc_super { id receiver; Class cls; };
IMP _prism_objc_lookup_super(struct objc_super *s, SEL sel) {
    IMP imp = lookup(s->cls, sel);
    return imp ? imp : resolve_or_forward(s->receiver, s->cls, sel, 0);
}
IMP _prism_objc_lookup_super2(struct objc_super *s, SEL sel) {
    Class c = s->cls->superclass;
    IMP imp = c ? lookup(c, sel) : 0;
    return imp ? imp : resolve_or_forward(s->receiver, c ? c : s->cls, sel, 0);
}

IMP class_getMethodImplementation(Class c, SEL sel) {
    if (!c) return 0;
    IMP imp = lookup(c, sel);
    return imp ? imp : (IMP)unrecognized_imp;
}
IMP objc_msg_lookup(id self, SEL sel) { return self ? _prism_objc_lookup(self, sel, 0) : 0; }
BOOL class_respondsToSelector(Class c, SEL sel) { return c && sel && lookup(c, sel) != 0; }

typedef struct method_t *Method;
Method class_getInstanceMethod(Class c, SEL sel) {
    for (; c; c = c->superclass) { Method m = find_own(c, sel); if (m) return m; }
    return 0;
}
Method class_getClassMethod(Class c, SEL sel) { return c ? class_getInstanceMethod(c->isa, sel) : 0; }
IMP method_getImplementation(Method m) { return m ? m->imp : 0; }
SEL method_getName(Method m) { return m ? m->name : 0; }
const char *method_getTypeEncoding(Method m) { return m ? m->types : 0; }
IMP method_setImplementation(Method m, IMP imp) {
    if (!m) return 0;
    IMP old = m->imp; m->imp = imp;
    flush_all_caches();
    return old;
}
void method_exchangeImplementations(Method a, Method b) {
    if (!a || !b) return;
    IMP t = a->imp; a->imp = b->imp; b->imp = t;
    flush_all_caches();
}
BOOL class_addMethod(Class c, SEL sel, IMP imp, const char *types) {
    if (!c || find_own(c, sel)) return NO;
    struct method_list *m = malloc(sizeof *m + sizeof(struct method_t));
    m->entsize_flags = sizeof(struct method_t);
    m->count = 1;
    struct method_t *e = (struct method_t *)(m + 1);
    e->name = sel; e->types = types; e->imp = imp;
    rw_add_list(rw_of(c), m);
    flush_all_caches();
    return YES;
}
IMP class_replaceMethod(Class c, SEL sel, IMP imp, const char *types) {
    Method m = find_own(c, sel);
    if (m) return method_setImplementation(m, imp);
    class_addMethod(c, sel, imp, types);
    return 0;
}
Method *class_copyMethodList(Class c, unsigned *out) {
    struct rw *r = c ? rw_of(c) : 0;
    unsigned n = 0;
    if (r) for (uint32_t l = 0; l < r->nlists; l++) n += r->lists[l]->count;
    Method *v = n ? malloc((n + 1) * sizeof *v) : 0;
    unsigned k = 0;
    if (v) {
        for (uint32_t l = 0; l < r->nlists; l++)
            for (uint32_t i = 0; i < r->lists[l]->count; i++) v[k++] = (struct method_t *)(r->lists[l] + 1) + i;
        v[k] = 0;
    }
    if (out) *out = n;
    return v;
}

/* Ivars */
typedef struct ivar_t *Ivar;
Ivar class_getInstanceVariable(Class c, const char *name) {
    for (; c; c = c->superclass) {
        const struct ivar_list *iv = ro_of(c)->ivars;
        if (!iv) continue;
        for (uint32_t i = 0; i < iv->count; i++) {
            struct ivar_t *v = (struct ivar_t *)((const char *)&iv->first + (size_t)i * iv->entsize);
            if (!strcmp(v->name, name)) return v;
        }
    }
    return 0;
}
ptrdiff_t ivar_getOffset(Ivar v) { return v && v->offset ? *v->offset : 0; }
const char *ivar_getName(Ivar v) { return v ? v->name : 0; }
const char *ivar_getTypeEncoding(Ivar v) { return v ? v->type : 0; }
id object_getIvar(id o, Ivar v) { return o && v ? *(id *)((char *)o + ivar_getOffset(v)) : nil; }
void object_setIvar(id o, Ivar v, id val) { if (o && v) *(id *)((char *)o + ivar_getOffset(v)) = val; }

/* Protocols */
typedef struct protocol_t Protocol;
Protocol *objc_getProtocol(const char *name) { return strtab_get(&protocols, name); }
const char *protocol_getName(Protocol *p) { return p ? p->name : 0; }
static BOOL proto_inherits(Protocol *p, Protocol *want) {
    if (!p) return NO;
    if (p == want || !strcmp(p->name, want->name)) return YES;
    if (p->protocols) for (uintptr_t i = 0; i < p->protocols->count; i++) if (proto_inherits(p->protocols->list[i], want)) return YES;
    return NO;
}
BOOL protocol_conformsToProtocol(Protocol *p, Protocol *q) { return p && q && proto_inherits(p, q); }
BOOL protocol_isEqual(Protocol *p, Protocol *q) { return p == q || (p && q && !strcmp(p->name, q->name)); }
BOOL class_conformsToProtocol(Class c, Protocol *p) {
    if (!c || !p) return NO;
    struct rw *r = rw_of(c);
    if (r) for (uint32_t i = 0; i < r->nprotos; i++)
        for (uintptr_t j = 0; j < r->protos[i]->count; j++)
            if (proto_inherits(r->protos[i]->list[j], p)) return YES;
    return NO;
}

/* Dynamic classes */
Class objc_allocateClassPair(Class super, const char *name, size_t extra) {
    (void)extra;
    if (objc_getClass(name)) return 0;
    Class c = calloc(1, sizeof *c * 2), m = c + 1;
    struct class_ro *ro = calloc(2, sizeof *ro), *mro = ro + 1;
    ro->name = mro->name = strdup(name);
    ro->instance_start = ro->instance_size = super ? ro_of(super)->instance_size : sizeof(Class);
    mro->flags = RO_META | (super ? 0 : RO_ROOT);
    mro->instance_start = mro->instance_size = sizeof(struct objc_class);
    if (!super) ro->flags = RO_ROOT;
    c->isa = m;
    c->superclass = super;
    Class root = super; while (root && root->superclass) root = root->superclass;
    m->isa = root ? root->isa : m;
    m->superclass = super ? super->isa : c;
    c->bits = (uintptr_t)ro;
    m->bits = (uintptr_t)mro;
    return c;
}
void objc_registerClassPair(Class c) { realize(c); }
void objc_disposeClassPair(Class c) { (void)c; }

Class *objc_copyClassList(unsigned *out) {
    unsigned n = 0;
    for (Class c = all_classes; c; c = rw_of(c)->next) n++;
    Class *v = malloc((n + 1) * sizeof *v);
    unsigned k = 0;
    for (Class c = all_classes; c; c = rw_of(c)->next) v[k++] = c;
    v[k] = 0;
    if (out) *out = n;
    return v;
}
int objc_getClassList(Class *buf, int len) {
    int n = 0;
    for (Class c = all_classes; c; c = rw_of(c)->next) { if (buf && n < len) buf[n] = c; n++; }
    return n;
}

/* ------------------------------------------------------------------ memory management */

static struct ptrtab refcounts; /* object → extra references */
static struct ptrtab weak_refs; /* object → heap array of id* (count in [0]) */
static struct ptrtab associations; /* object → heap list */

id _prism_objc_rootRetain(id o) { if (o) (*ptrtab_add(&refcounts, o))++; return o; }
BOOL _prism_objc_rootRelease(id o) {
    if (!o) return NO;
    uintptr_t *n = ptrtab_find(&refcounts, o);
    if (n && *n) { (*n)--; return NO; }
    return YES; /* the caller deallocates */
}
uintptr_t _prism_objc_rootRetainCount(id o) { uintptr_t *n = ptrtab_find(&refcounts, o); return 1 + (n ? *n : 0); }

id objc_retain(id o) { return o ? ((id(*)(id, SEL))_prism_objc_lookup(o, _prism_sel_retain, 0))(o, _prism_sel_retain) : nil; }
void objc_release(id o) { if (o) ((void (*)(id, SEL))_prism_objc_lookup(o, _prism_sel_release, 0))(o, _prism_sel_release); }
id objc_autorelease(id o) { return o ? ((id(*)(id, SEL))_prism_objc_lookup(o, _prism_sel_autorelease, 0))(o, _prism_sel_autorelease) : nil; }
id objc_retainAutorelease(id o) { return objc_autorelease(objc_retain(o)); }
id objc_autoreleaseReturnValue(id o) { return objc_autorelease(o); }
id objc_retainAutoreleaseReturnValue(id o) { return objc_autorelease(objc_retain(o)); }
id objc_retainAutoreleasedReturnValue(id o) { return objc_retain(o); }
id objc_unsafeClaimAutoreleasedReturnValue(id o) { return o; }
id objc_claimAutoreleasedReturnValue(id o) { return objc_retain(o); }
id objc_retainBlock(id b) { return (id)_Block_copy(b); }
void objc_storeStrong(id *loc, id v) {
    id old = *loc;
    if (old == v) return;
    *loc = objc_retain(v);
    objc_release(old);
}
/* objc_retain_x0 etc. are arm64e register variants; plain objc_retain covers the rest. */

/* Weak references */
static void weak_unregister(id obj, id *loc) {
    uintptr_t *e = obj ? ptrtab_find(&weak_refs, obj) : 0;
    if (!e || !*e) return;
    id **v = (id **)*e;
    uintptr_t n = (uintptr_t)v[0];
    for (uintptr_t i = 1; i <= n; i++) if (v[i] == loc) { v[i] = v[n]; v[0] = (id *)(n - 1); return; }
}
static void weak_register(id obj, id *loc) {
    uintptr_t *e = ptrtab_add(&weak_refs, obj);
    id **v = (id **)*e;
    uintptr_t n = v ? (uintptr_t)v[0] : 0;
    v = realloc(v, (n + 2) * sizeof *v);
    v[0] = (id *)(n + 1);
    v[n + 1] = loc;
    *e = (uintptr_t)v;
}
id objc_storeWeak(id *loc, id v) {
    weak_unregister(*loc, loc);
    if (v) weak_register(v, loc);
    *loc = v;
    return v;
}
id objc_initWeak(id *loc, id v) { *loc = nil; return objc_storeWeak(loc, v); }
id objc_initWeakOrNil(id *loc, id v) { return objc_initWeak(loc, v); }
id objc_storeWeakOrNil(id *loc, id v) { return objc_storeWeak(loc, v); }
void objc_destroyWeak(id *loc) { objc_storeWeak(loc, nil); }
id objc_loadWeakRetained(id *loc) { return objc_retain(*loc); }
id objc_loadWeak(id *loc) { return objc_autorelease(objc_retain(*loc)); }
void objc_copyWeak(id *to, id *from) { objc_initWeak(to, *from); }
void objc_moveWeak(id *to, id *from) { objc_initWeak(to, *from); objc_destroyWeak(from); }

/* Associated objects: per object a heap array {count, (key, value, policy)...}. */
enum { ASSIGN = 0, RETAIN_NONATOMIC = 1, COPY_NONATOMIC = 3, RETAIN = 01401, COPY = 01403 };
void objc_setAssociatedObject(id o, const void *key, id value, uintptr_t policy) {
    uintptr_t *e = ptrtab_add(&associations, o);
    uintptr_t *v = (uintptr_t *)*e;
    uintptr_t n = v ? v[0] : 0, i;
    for (i = 0; i < n && v[1 + i * 3] != (uintptr_t)key; i++) {}
    if (value && (policy & 3) == 3) value = ((id(*)(id, SEL))_prism_objc_lookup(value, _prism_sel_copy, 0))(value, _prism_sel_copy);
    else if (value && (policy & 1)) value = objc_retain(value);
    if (i < n) {
        if (v[3 + i * 3] & 1) objc_release((id)v[2 + i * 3]);
    } else {
        v = realloc(v, (1 + (n + 1) * 3) * sizeof *v);
        v[0] = n + 1;
        v[1 + i * 3] = (uintptr_t)key;
        *e = (uintptr_t)v;
    }
    v[2 + i * 3] = (uintptr_t)value;
    v[3 + i * 3] = policy;
}
id objc_getAssociatedObject(id o, const void *key) {
    uintptr_t *e = ptrtab_find(&associations, o);
    uintptr_t *v = e ? (uintptr_t *)*e : 0;
    for (uintptr_t i = 0; v && i < v[0]; i++) if (v[1 + i * 3] == (uintptr_t)key) return (id)v[2 + i * 3];
    return nil;
}
void objc_removeAssociatedObjects(id o) {
    uintptr_t *e = ptrtab_find(&associations, o);
    if (!e) return;
    uintptr_t *v = (uintptr_t *)*e;
    ptrtab_del(&associations, o);
    for (uintptr_t i = 0; v && i < v[0]; i++) if (v[3 + i * 3] & 1) objc_release((id)v[2 + i * 3]);
    free(v);
}

/* Instances */
id class_createInstance(Class c, size_t extra) {
    if (!c) return nil;
    id o = calloc(1, ro_of(c)->instance_size + extra);
    if (o) o->isa = c;
    return o;
}
id object_dispose(id o) { _prism_objc_dispose(o); return nil; }

static void cxx_destruct(id o) {
    for (Class c = o->isa; c; c = c->superclass) {
        if (!(ro_of(c)->flags & RO_HAS_CXX_STRUCTORS)) continue;
        struct method_t *m = find_own(c, sel_cxx_destruct);
        if (m) ((void (*)(id, SEL))m->imp)(o, sel_cxx_destruct);
    }
}

void *objc_destructInstance(id o) {
    if (!o) return 0;
    cxx_destruct(o);
    objc_removeAssociatedObjects(o);
    uintptr_t *e = ptrtab_find(&weak_refs, o);
    if (e && *e) {
        id **v = (id **)*e;
        for (uintptr_t i = 1; i <= (uintptr_t)v[0]; i++) *v[i] = nil;
        free(v);
    }
    ptrtab_del(&weak_refs, o);
    ptrtab_del(&refcounts, o);
    return o;
}
void _prism_objc_dispose(id o) { if (o) { objc_destructInstance(o); free(o); } }

/* Autorelease pools: one stack of objects; a pool token is the stack depth + 1. */
static id *pool; static size_t npool, cappool;
void *objc_autoreleasePoolPush(void) { return (void *)(npool + 1); }
void objc_autoreleasePoolPop(void *token) {
    size_t to = (size_t)token - 1;
    while (npool > to) objc_release(pool[--npool]);
}
id _prism_objc_rootAutorelease(id o) {
    if (npool == cappool) { cappool = cappool ? cappool * 2 : 256; pool = realloc(pool, cappool * sizeof *pool); }
    pool[npool++] = o;
    return o;
}
void *_objc_autoreleasePoolPush(void) { return objc_autoreleasePoolPush(); }
void _objc_autoreleasePoolPop(void *t) { objc_autoreleasePoolPop(t); }

/* ------------------------------------------------------------------ fast paths the compiler calls */

static id send0(id o, SEL s) { return o ? ((id(*)(id, SEL))_prism_objc_lookup(o, s, 0))(o, s) : nil; }
id objc_alloc(Class c) { return send0((id)c, _prism_sel_alloc); }
id objc_allocWithZone(Class c) { return send0((id)c, _prism_sel_alloc); }
id objc_alloc_init(Class c) { return send0(send0((id)c, _prism_sel_alloc), _prism_sel_init); }
id objc_opt_new(Class c) { return send0((id)c, _prism_sel_new); }
Class objc_opt_class(id o) { return (Class)send0(o, _prism_sel_class); }
id objc_opt_self(id o) { return o; }
BOOL objc_opt_isKindOfClass(id o, Class c) {
    return o ? ((BOOL(*)(id, SEL, Class))_prism_objc_lookup(o, _prism_sel_isKindOfClass, 0))(o, _prism_sel_isKindOfClass, c) : NO;
}
BOOL objc_opt_respondsToSelector(id o, SEL s) {
    return o ? ((BOOL(*)(id, SEL, SEL))_prism_objc_lookup(o, _prism_sel_respondsToSelector, 0))(o, _prism_sel_respondsToSelector, s) : NO;
}

/* ------------------------------------------------------------------ errors */

void _prism_objc_unrecognized(id self, SEL sel) {
    Class c = self->isa;
    fprintf(stderr, "*** %c[%s %s]: unrecognized selector sent to %s %p\n", is_meta(c) ? '+' : '-', class_getName(c), sel, is_meta(c) ? "class" : "instance", (void *)self);
    abort();
}
void objc_enumerationMutation(id o) {
    fprintf(stderr, "*** Collection <%s: %p> was mutated while being enumerated.\n", object_getClassName(o), (void *)o);
    abort();
}
void objc_terminate(void) { abort(); }
int objc_sync_enter(id o) { (void)o; return 0; }
int objc_sync_exit(id o) { (void)o; return 0; }
void objc_setEnumerationMutationHandler(void (*h)(id)) { (void)h; }
void *objc_getProperty(id self, SEL s, ptrdiff_t off, BOOL atomic) {
    (void)s; (void)atomic;
    id v = *(id *)((char *)self + off);
    return objc_autorelease(objc_retain(v));
}
void objc_setProperty(id self, SEL s, ptrdiff_t off, id v, BOOL atomic, signed char copy) {
    (void)s; (void)atomic;
    if (copy && v) v = ((id(*)(id, SEL))_prism_objc_lookup(v, _prism_sel_copy, 0))(v, _prism_sel_copy);
    else v = objc_retain(v);
    id *slot = (id *)((char *)self + off);
    id old = *slot; *slot = v; objc_release(old);
}
void objc_setProperty_nonatomic(id self, SEL s, id v, ptrdiff_t off) { objc_setProperty(self, s, off, v, NO, 0); }
void objc_setProperty_atomic(id self, SEL s, id v, ptrdiff_t off) { objc_setProperty(self, s, off, v, YES, 0); }
void objc_setProperty_nonatomic_copy(id self, SEL s, id v, ptrdiff_t off) { objc_setProperty(self, s, off, v, NO, 1); }
void objc_setProperty_atomic_copy(id self, SEL s, id v, ptrdiff_t off) { objc_setProperty(self, s, off, v, YES, 1); }
void objc_copyStruct(void *dst, const void *src, ptrdiff_t size, BOOL atomic, BOOL strong) { (void)atomic; (void)strong; memmove(dst, src, size); }

/* Data the compiler references. */
uintptr_t _objc_empty_cache[4];
void *_objc_empty_vtable[1];

/* ------------------------------------------------------------------ exceptions */

struct _Unwind_Exception {
    uint64_t exception_class;
    void (*cleanup)(int, struct _Unwind_Exception *);
    uintptr_t private_1, private_2;
} __attribute__((aligned(16)));
struct _Unwind_Context;
int _Unwind_RaiseException(struct _Unwind_Exception *);
void _Unwind_SetGR(struct _Unwind_Context *, int, uint64_t);
void _Unwind_SetIP(struct _Unwind_Context *, uint64_t);
struct lsda_result { uint64_t landing_pad; int64_t selector; int found_handler; int cleanup; };
int __prism_lsda_scan(struct _Unwind_Context *, int (*)(void *, void *), void *, struct lsda_result *);

#define OBJC_EXCEPTION_CLASS 0x474e55434f424a43ull /* "GNUCOBJC" */
struct objc_exception { struct _Unwind_Exception u; id obj; struct objc_exception *next_caught; };

/* The type info @catch clauses name: {vtable, class name, class}; a nil class is `@catch (id)`. */
struct objc_ehtype { const void *vtable; const char *name; Class cls; };
void *objc_ehtype_vtable[3];
struct objc_ehtype OBJC_EHTYPE_id = {objc_ehtype_vtable + 2, "id", 0};

static void (*uncaught_handler)(id);
void (*objc_setUncaughtExceptionHandler(void (*h)(id)))(id) { void (*old)(id) = uncaught_handler; uncaught_handler = h; return old; }

static struct objc_exception *caught; /* innermost first */

static void exception_cleanup(int reason, struct _Unwind_Exception *u) {
    (void)reason;
    struct objc_exception *e = (struct objc_exception *)u;
    objc_release(e->obj);
    free(e);
}

void objc_exception_throw(id obj) {
    struct objc_exception *e = calloc(1, sizeof *e);
    e->u.exception_class = OBJC_EXCEPTION_CLASS;
    e->u.cleanup = exception_cleanup;
    e->obj = objc_retain(obj);
    _Unwind_RaiseException(&e->u);
    /* Nothing caught it. */
    if (uncaught_handler) uncaught_handler(obj);
    fprintf(stderr, "*** Terminating app due to uncaught exception of class '%s'\n", object_getClassName(obj));
    abort();
}

id objc_begin_catch(void *u) {
    struct objc_exception *e = u;
    if (e->u.exception_class != OBJC_EXCEPTION_CLASS) return nil; /* a foreign (C++) exception */
    e->next_caught = caught;
    caught = e;
    return e->obj;
}

void objc_end_catch(void) {
    struct objc_exception *e = caught;
    if (!e) return;
    caught = e->next_caught;
    if (e->u.private_1 != 1) exception_cleanup(0, &e->u); /* private_1 = 1: being rethrown */
}

void objc_exception_rethrow(void) {
    struct objc_exception *e = caught;
    if (!e) abort();
    caught = e->next_caught;
    e->u.private_2 = 0;
    _Unwind_RaiseException(&e->u);
    if (uncaught_handler) uncaught_handler(e->obj);
    fprintf(stderr, "*** Terminating app due to uncaught exception of class '%s'\n", object_getClassName(e->obj));
    abort();
}

static int catches(void *typeinfo, void *arg) {
    if (!typeinfo) return 1; /* catch (...) */
    struct objc_ehtype *t = typeinfo;
    if (!arg) return 0;      /* foreign exception: only catch-all */
    if (!t->cls) return 1;   /* @catch (id) */
    for (Class c = ((id)arg)->isa; c; c = c->superclass) if (c == t->cls) return 1;
    return 0;
}

int __objc_personality_v0(int version, int actions, uint64_t cls, struct _Unwind_Exception *u, struct _Unwind_Context *ctx) {
    (void)version;
    id obj = cls == OBJC_EXCEPTION_CLASS ? ((struct objc_exception *)u)->obj : nil;
    struct lsda_result r;
    if (!__prism_lsda_scan(ctx, catches, obj, &r)) return 8; /* _URC_CONTINUE_UNWIND */
    if (actions & 1) return r.found_handler ? 6 : 8; /* search: _URC_HANDLER_FOUND */
    if ((actions & 4) || (r.cleanup && !r.found_handler)) {
        _Unwind_SetGR(ctx, 0, (uint64_t)u);
        _Unwind_SetGR(ctx, 1, (uint64_t)((actions & 4) ? r.selector : 0));
        _Unwind_SetIP(ctx, r.landing_pad);
        return 7; /* _URC_INSTALL_CONTEXT */
    }
    return 8;
}
