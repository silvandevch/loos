#include <_prism_cdefs.h>
struct iovec { void *iov_base; size_t iov_len; };
__BEGIN_DECLS
ssize_t readv(int, const struct iovec *, int); ssize_t writev(int, const struct iovec *, int);
__END_DECLS
