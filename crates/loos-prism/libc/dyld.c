/* dyld: image callbacks and dlopen. Prism maps and binds images (loader.rs); the parts that call
 * into the program (initializers, callbacks, Objective-C registration) run here. */
#include "prism.h"

uint32_t _dyld_image_count(void);
const void *_dyld_get_image_header(uint32_t);
intptr_t _dyld_get_image_vmaddr_slide(uint32_t);
int *_NSGetArgc(void);
char ***_NSGetArgv(void);
char ***_NSGetEnviron(void);
void *__prism_dlopen(const char *, int);
long __prism_dl_inits(void **, long);
void *__prism_dlsym(void *, const char *);
void _prism_objc_images_added(uint32_t first, uint32_t n);

typedef void (*image_fn)(const void *, intptr_t);
static image_fn *added;
static unsigned nadded, capadded;

void _dyld_register_func_for_add_image(image_fn f) {
    if (nadded == capadded) { capadded = capadded ? capadded * 2 : 8; added = realloc(added, capadded * sizeof *added); }
    added[nadded++] = f;
    for (uint32_t i = 0; i < _dyld_image_count(); i++) f(_dyld_get_image_header(i), _dyld_get_image_vmaddr_slide(i));
}
/* Images are never unloaded. */
void _dyld_register_func_for_remove_image(image_fn f) { (void)f; }

static char dl_message[512];
static int dl_failed;
static void dl_fail(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(dl_message, sizeof dl_message, fmt, ap);
    va_end(ap);
    dl_failed = 1;
}

void *dlopen(const char *path, int mode) {
    uint32_t first = _dyld_image_count();
    void *h = __prism_dlopen(path, mode);
    if (!h) { dl_fail("dlopen(%s, 0x%04x): image not found or not loadable", path, mode); return 0; }
    uint32_t n = _dyld_image_count();
    if (n == first) return h; /* it was loaded already */
    _prism_objc_images_added(first, n);
    for (unsigned k = 0; k < nadded; k++)
        for (uint32_t i = first; i < n; i++) added[k](_dyld_get_image_header(i), _dyld_get_image_vmaddr_slide(i));
    void *inits[64];
    long k;
    while ((k = __prism_dl_inits(inits, 64)) > 0)
        for (long i = 0; i < k; i++) ((void (*)(int, char **, char **))inits[i])(*_NSGetArgc(), *_NSGetArgv(), *_NSGetEnviron());
    return h;
}
void *dlsym(void *handle, const char *name) {
    void *p = __prism_dlsym(handle, name);
    if (!p) dl_fail("dlsym(%p, %s): symbol not found", handle, name);
    return p;
}
int dlclose(void *handle) { (void)handle; return 0; }
char *dlerror(void) {
    if (!dl_failed) return 0;
    dl_failed = 0;
    return dl_message;
}
/* 1: `path` could be loaded. */
int dlopen_preflight(const char *path) {
    int fd = open(path, O_RDONLY);
    if (fd < 0) return 0;
    close(fd);
    return 1;
}
