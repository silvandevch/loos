/* dyld callbacks (they call into the program). The images themselves come from Prism. */
#include "prism.h"

uint32_t _dyld_image_count(void);
const void *_dyld_get_image_header(uint32_t);
intptr_t _dyld_get_image_vmaddr_slide(uint32_t);

void _dyld_register_func_for_add_image(void (*f)(const void *, intptr_t)) {
    for (uint32_t i = 0; i < _dyld_image_count(); i++) f(_dyld_get_image_header(i), _dyld_get_image_vmaddr_slide(i));
}
void _dyld_register_func_for_remove_image(void (*f)(const void *, intptr_t)) { (void)f; }
