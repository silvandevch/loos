#include <_prism_cdefs.h>
#define PROT_READ 1
#define PROT_WRITE 2
#define MAP_SHARED 1
#define MAP_PRIVATE 2
#define MAP_ANON 0x1000
#define MAP_FAILED ((void *)-1)
__BEGIN_DECLS
void *mmap(void *, size_t, int, int, int, off_t); int munmap(void *, size_t);
__END_DECLS
