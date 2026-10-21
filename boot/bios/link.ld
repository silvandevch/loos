ENTRY(stage2)
SECTIONS
{
    . = 0x7e00;
    .text   : { *(.text) *(.text.*) }
    .rodata : { *(.rodata) *(.rodata.*) }
    .data   : { *(.data) *(.data.*) }
    . = ALIGN(16);
    __bss_start = .;
    .bss    : { *(.bss) *(.bss.*) *(COMMON) }
    __loader_end = .;
    /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame) }
}
