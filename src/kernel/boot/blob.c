/* A large read-only object so the image is several MiB: guards against reintroducing a kernel size limit. */
#define BLOB (3u << 20)
const unsigned char k_blob[BLOB] = { 1, 2, 3, [BLOB - 1] = 0x5a };
const unsigned long k_blob_size = BLOB;
