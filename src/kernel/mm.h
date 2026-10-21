#ifndef LOOS_KERNEL_MM_H
#define LOOS_KERNEL_MM_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define LOOS_MM_VIEW_ALIGNMENT UINT64_C(65536)
#define LOOS_MM_PAGE_SIZE UINT64_C(4096)

enum loos_mm_status {
    LOOS_MM_OK = 0,
    LOOS_MM_EINVAL = -1,
    LOOS_MM_ENOMEM = -2,
    LOOS_MM_EFAULT = -3,
    LOOS_MM_EEXIST = -4,
    LOOS_MM_ENOTFOUND = -5,
    LOOS_MM_EACCES = -6,
};

enum loos_mm_protection {
    LOOS_MM_READ = 1U,
    LOOS_MM_WRITE = 2U,
    LOOS_MM_EXECUTE = 4U,
};

struct loos_mm_page {
    uint64_t virtual_address;
    uint8_t *physical_page;
    uint32_t protection;
    bool present;
    bool copy_on_write;
    uint32_t *references;
};

struct loos_mm_section {
    uint64_t size;
    uint32_t protection;
    bool file_backed;
    void *opaque;
    int (*read_page)(void *opaque, uint64_t file_offset, uint8_t *page);
};

struct loos_mm_view {
    struct loos_mm_section *section;
    uint64_t base;
    uint64_t size;
    struct loos_mm_page *pages;
    size_t page_count;
};

int loos_mm_section_init(struct loos_mm_section *section,
                         uint64_t size,
                         uint32_t protection,
                         bool file_backed,
                         int (*read_page)(void *, uint64_t, uint8_t *),
                         void *opaque);
int loos_mm_map_view(struct loos_mm_view *view,
                     struct loos_mm_section *section,
                     uint64_t requested_base,
                     uint64_t size,
                     uint32_t protection,
                     void *page_storage,
                     size_t page_storage_size);
void loos_mm_unmap_view(struct loos_mm_view *view);
int loos_mm_fault(struct loos_mm_view *view,
                  uint64_t address,
                  bool write,
                  uint8_t **page_out);
int loos_mm_clone_cow(struct loos_mm_view *child,
                      struct loos_mm_view *parent,
                      void *page_storage,
                      size_t page_storage_size);

#endif
