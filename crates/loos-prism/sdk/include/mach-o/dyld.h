#ifndef _MACH_O_DYLD_H
#define _MACH_O_DYLD_H
#include <_prism_cdefs.h>
struct mach_header;
__BEGIN_DECLS
uint32_t _dyld_image_count(void); const struct mach_header *_dyld_get_image_header(uint32_t);
const char *_dyld_get_image_name(uint32_t); intptr_t _dyld_get_image_vmaddr_slide(uint32_t);
int _NSGetExecutablePath(char *, uint32_t *);
void *NSAddressOfSymbol(void *); void *NSLookupSymbolInImage(const struct mach_header *, const char *, uint32_t);
__END_DECLS
#define NSLOOKUPSYMBOLINIMAGE_OPTION_BIND 0x0
#endif
