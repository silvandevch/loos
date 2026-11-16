/* Prism's Objective-C runtime: types and the 64-bit Apple ABI layouts compiled code uses. */
#pragma once
#include "prism.h"

typedef struct objc_class *Class;
struct objc_object { Class isa; };
typedef struct objc_object *id;
#ifndef __OBJC__
typedef const char *SEL; /* a uniqued selector name */
#endif
typedef id (*IMP)(id, SEL, ...);
typedef signed char BOOL;
#define YES ((BOOL)1)
#define NO ((BOOL)0)
#define nil ((id)0)

/* class_t as the compiler emits it. `bits` holds the class_ro_t pointer until Prism realizes the
 * class, then a pointer to its `struct rw` with RW_TAG set. */
struct objc_class {
    Class isa;
    Class superclass;
    void *cache;
    void *vtable;
    uintptr_t bits;
};
#define RW_TAG 4
#define DATA_MASK 0x00007ffffffffff8ul

struct method_t { SEL name; const char *types; IMP imp; };
struct method_list { uint32_t entsize_flags; uint32_t count; /* entries follow */ };
struct ivar_t { int32_t *offset; const char *name; const char *type; uint32_t alignment_raw; uint32_t size; };
struct ivar_list { uint32_t entsize; uint32_t count; struct ivar_t first; };
struct protocol_list { uintptr_t count; struct protocol_t *list[]; };
struct class_ro {
    uint32_t flags, instance_start, instance_size, reserved;
    const uint8_t *ivar_layout;
    const char *name;
    struct method_list *methods;
    struct protocol_list *protocols;
    struct ivar_list *ivars;
    const uint8_t *weak_ivar_layout;
    void *properties;
};
#define RO_META 1
#define RO_ROOT 2
#define RO_HAS_CXX_STRUCTORS 4

struct protocol_t {
    id isa;
    const char *name;
    struct protocol_list *protocols;
    struct method_list *instance_methods, *class_methods, *opt_instance_methods, *opt_class_methods;
    void *instance_properties;
    uint32_t size, flags;
};

struct category_t {
    const char *name;
    Class cls;
    struct method_list *instance_methods, *class_methods;
    struct protocol_list *protocols;
    void *instance_properties;
};

/* Prism's per-class runtime data. */
struct cache_entry { SEL sel; IMP imp; };
struct cache { uint32_t mask, used; struct cache_entry e[]; };
struct rw {
    uint32_t flags;
    const struct class_ro *ro;
    struct method_list **lists; /* newest (categories, added methods) first */
    uint32_t nlists, caplists;
    struct protocol_list **protos;
    uint32_t nprotos, capprotos;
    struct cache *cache;
    Class next; /* all realized classes */
    uintptr_t init_thread; /* the thread running +initialize */
};
#define RW_INITIALIZED 1
#define RW_INITIALIZING 2
#define RW_LOADED 4

static inline struct rw *rw_of(Class c) { return (c->bits & RW_TAG) ? (struct rw *)(c->bits & DATA_MASK) : 0; }
static inline const struct class_ro *ro_of(Class c) {
    struct rw *r = rw_of(c);
    return r ? r->ro : (const struct class_ro *)(c->bits & DATA_MASK);
}

/* objc.c */
SEL sel_registerName(const char *);
const char *sel_getName(SEL);
Class objc_getClass(const char *);
IMP class_getMethodImplementation(Class, SEL);
id objc_msgSend(id, SEL, ...);
id objc_retain(id);
void objc_release(id);
id objc_autorelease(id);
void _prism_objc_dispose(id);
id _prism_objc_rootRetain(id);
BOOL _prism_objc_rootRelease(id);
uintptr_t _prism_objc_rootRetainCount(id);
void _prism_objc_unrecognized(id, SEL);
extern SEL _prism_sel_retain, _prism_sel_release, _prism_sel_autorelease, _prism_sel_dealloc, _prism_sel_alloc, _prism_sel_init,
    _prism_sel_initialize, _prism_sel_class, _prism_sel_copy, _prism_sel_new, _prism_sel_isKindOfClass, _prism_sel_respondsToSelector;
void *_Block_copy(const void *);
void _Block_release(const void *);

BOOL _prism_objc_rootTryRetain(id);
BOOL _prism_objc_rootIsDeallocating(id);
/* Tagged pointers (objc.c): tags 0-6, extended tags 8-263. */
id _prism_objc_tagged_make(unsigned tag, intptr_t value);
intptr_t _prism_objc_tagged_value(id);
unsigned _prism_objc_tagged_tag(id);
BOOL _prism_objc_tagged_fits(unsigned tag, intptr_t value);
BOOL _objc_isTaggedPointer(const void *);
void _objc_registerTaggedPointerClass(unsigned tag, Class);

/* Declarations nsobject.m uses. */
id class_createInstance(Class, size_t);
Class class_getSuperclass(Class);
Class object_getClass(id);
const char *object_getClassName(id);
const char *class_getName(Class);
BOOL class_respondsToSelector(Class, SEL);
BOOL class_conformsToProtocol(Class, struct protocol_t *);
