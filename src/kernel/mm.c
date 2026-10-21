#include "mm.h"

#include <stdlib.h>
#include <string.h>

static uint64_t align_up(uint64_t value, uint64_t alignment)
{
    return (value + alignment - 1U) & ~(alignment - 1U);
}

int loos_mm_section_init(struct loos_mm_section *section,
                         uint64_t size,
                         uint32_t protection,
                         bool file_backed,
                         int (*read_page)(void *, uint64_t, uint8_t *),
                         void *opaque)
{
    if (section == NULL || size == 0 || (protection & ~7U) != 0 ||
        (file_backed && read_page == NULL)) {
        return LOOS_MM_EINVAL;
    }
    section->size = size;
    section->protection = protection;
    section->file_backed = file_backed;
    section->read_page = read_page;
    section->opaque = opaque;
    return LOOS_MM_OK;
}

int loos_mm_map_view(struct loos_mm_view *view,
                     struct loos_mm_section *section,
                     uint64_t requested_base,
                     uint64_t size,
                     uint32_t protection,
                     void *page_storage,
                     size_t page_storage_size)
{
    if (view == NULL || section == NULL || size == 0 ||
        (protection & ~section->protection) != 0 ||
        requested_base % LOOS_MM_VIEW_ALIGNMENT != 0) {
        return LOOS_MM_EINVAL;
    }
    const uint64_t view_size = align_up(size, LOOS_MM_PAGE_SIZE);
    if (view_size < size || view_size > section->size) {
        return LOOS_MM_EINVAL;
    }
    const size_t pages = (size_t)(view_size / LOOS_MM_PAGE_SIZE);
    if (pages > SIZE_MAX / sizeof(struct loos_mm_page) ||
        page_storage == NULL ||
        page_storage_size < pages * sizeof(struct loos_mm_page)) {
        return LOOS_MM_ENOMEM;
    }
    memset(view, 0, sizeof(*view));
    view->section = section;
    view->base = requested_base;
    view->size = view_size;
    view->pages = page_storage;
    view->page_count = pages;
    for (size_t i = 0; i < pages; ++i) {
        view->pages[i].virtual_address = requested_base +
                                         (uint64_t)i * LOOS_MM_PAGE_SIZE;
        view->pages[i].protection = protection;
        view->pages[i].copy_on_write = (protection & LOOS_MM_WRITE) == 0;
        view->pages[i].references = NULL;
    }
    return LOOS_MM_OK;
}

void loos_mm_unmap_view(struct loos_mm_view *view)
{
    if (view == NULL) {
        return;
    }
    for (size_t i = 0; i < view->page_count; ++i) {
        if (view->pages[i].references != NULL &&
            --*view->pages[i].references == 0) {
            free(view->pages[i].physical_page);
            free(view->pages[i].references);
        }
        view->pages[i].physical_page = NULL;
        view->pages[i].present = false;
    }
    memset(view, 0, sizeof(*view));
}

int loos_mm_fault(struct loos_mm_view *view,
                  uint64_t address,
                  bool write,
                  uint8_t **page_out)
{
    if (view == NULL || page_out == NULL || address < view->base ||
        address - view->base >= view->size) {
        return LOOS_MM_EFAULT;
    }
    const size_t index = (size_t)((address - view->base) / LOOS_MM_PAGE_SIZE);
    struct loos_mm_page *page = &view->pages[index];
    if (write && (page->protection & LOOS_MM_WRITE) == 0 &&
        !page->copy_on_write) {
        return LOOS_MM_EACCES;
    }
    if (!page->present) {
        page->physical_page = calloc(1, LOOS_MM_PAGE_SIZE);
        if (page->physical_page == NULL) {
            return LOOS_MM_ENOMEM;
        }
        if (view->section->file_backed &&
            view->section->read_page(view->section->opaque,
                                     (uint64_t)index * LOOS_MM_PAGE_SIZE,
                                     page->physical_page) != 0) {
            free(page->physical_page);
            page->physical_page = NULL;
            return LOOS_MM_EFAULT;
        }
        page->present = true;
        page->references = malloc(sizeof(*page->references));
        if (page->references == NULL) {
            free(page->physical_page);
            page->physical_page = NULL;
            page->present = false;
            return LOOS_MM_ENOMEM;
        }
        *page->references = 1;
    } else if (write && page->copy_on_write) {
        uint8_t *copy = malloc(LOOS_MM_PAGE_SIZE);
        if (copy == NULL) {
            return LOOS_MM_ENOMEM;
        }
        memcpy(copy, page->physical_page, LOOS_MM_PAGE_SIZE);
        uint32_t *new_references = malloc(sizeof(*new_references));
        if (new_references == NULL) {
            free(copy);
            return LOOS_MM_ENOMEM;
        }
        *new_references = 1;
        if (page->references != NULL && --*page->references == 0) {
            free(page->physical_page);
            free(page->references);
        }
        page->physical_page = copy;
        page->copy_on_write = false;
        page->protection |= LOOS_MM_WRITE;
        page->references = new_references;
    }
    *page_out = page->physical_page;
    return LOOS_MM_OK;
}

int loos_mm_clone_cow(struct loos_mm_view *child,
                      struct loos_mm_view *parent,
                      void *page_storage,
                      size_t page_storage_size)
{
    if (child == NULL || parent == NULL || parent->section == NULL) {
        return LOOS_MM_EINVAL;
    }
    int status = loos_mm_map_view(child, parent->section, parent->base,
                                  parent->size, parent->pages[0].protection,
                                  page_storage, page_storage_size);
    if (status != LOOS_MM_OK) {
        return status;
    }
    for (size_t i = 0; i < parent->page_count; ++i) {
        child->pages[i].physical_page = parent->pages[i].physical_page;
        child->pages[i].present = parent->pages[i].present;
        child->pages[i].copy_on_write = parent->pages[i].present;
        if (child->pages[i].present) {
            if (parent->pages[i].references == NULL) {
                loos_mm_unmap_view(child);
                return LOOS_MM_EFAULT;
            }
            ++*parent->pages[i].references;
            child->pages[i].references = parent->pages[i].references;
            child->pages[i].protection &= ~LOOS_MM_WRITE;
            parent->pages[i].copy_on_write = true;
            parent->pages[i].protection &= ~LOOS_MM_WRITE;
        }
    }
    return LOOS_MM_OK;
}
