#ifndef _DLFCN_H
#define _DLFCN_H
#include <_prism_cdefs.h>
#define RTLD_LAZY 0x1
#define RTLD_NOW 0x2
#define RTLD_LOCAL 0x4
#define RTLD_GLOBAL 0x8
#define RTLD_NOLOAD 0x10
#define RTLD_DEFAULT ((void *)-2)
typedef struct { const char *dli_fname; void *dli_fbase; const char *dli_sname; void *dli_saddr; } Dl_info;
__BEGIN_DECLS
void *dlopen(const char *, int); void *dlsym(void *, const char *); int dlclose(void *); char *dlerror(void);
int dladdr(const void *, Dl_info *);
__END_DECLS
#endif
