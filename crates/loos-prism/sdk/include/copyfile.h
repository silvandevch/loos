#ifndef _COPYFILE_H
#define _COPYFILE_H
#include <_prism_cdefs.h>
typedef struct _copyfile_state *copyfile_state_t;
typedef uint32_t copyfile_flags_t;
#define COPYFILE_ACL (1 << 0)
#define COPYFILE_STAT (1 << 1)
#define COPYFILE_XATTR (1 << 2)
#define COPYFILE_DATA (1 << 3)
#define COPYFILE_SECURITY (COPYFILE_STAT | COPYFILE_ACL)
#define COPYFILE_METADATA (COPYFILE_SECURITY | COPYFILE_XATTR)
#define COPYFILE_ALL (COPYFILE_METADATA | COPYFILE_DATA)
#define COPYFILE_EXCL (1 << 17)
#define COPYFILE_NOFOLLOW_SRC (1 << 18)
#define COPYFILE_NOFOLLOW_DST (1 << 19)
#define COPYFILE_NOFOLLOW (COPYFILE_NOFOLLOW_SRC | COPYFILE_NOFOLLOW_DST)
#define COPYFILE_UNLINK (1 << 21)
#define COPYFILE_CLONE (1 << 24)
__BEGIN_DECLS
int copyfile(const char *, const char *, copyfile_state_t, copyfile_flags_t);
int fcopyfile(int, int, copyfile_state_t, copyfile_flags_t);
copyfile_state_t copyfile_state_alloc(void); int copyfile_state_free(copyfile_state_t);
__END_DECLS
#endif
