OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY {
    APP (rwx) : ORIGIN = 0x10000, LENGTH = 0x42000
    STACK (rw) : ORIGIN = 0x100000, LENGTH = 16K
}

PHDRS {
    code PT_LOAD FLAGS(5);
    data PT_LOAD FLAGS(6);
    request PT_LOAD FLAGS(6);
    stack PT_LOAD FLAGS(6);
}

SECTIONS {
    .text ORIGIN(APP) : {
        KEEP(*(.text.start))
        *(.text .text.*)
    } > APP :code

    .rodata : {
        *(.rodata .rodata.* .srodata .srodata.*)
    } > APP :code

    .data ALIGN(4096) : {
        PROVIDE(__global_pointer$ = . + 0x800);
        *(.data .data.* .sdata .sdata.*)
    } > APP :data

    .bss (NOLOAD) : ALIGN(8) {
        __bss_start = .;
        *(.bss .bss.* .sbss .sbss.* COMMON)
        . = ALIGN(8);
        __bss_end = .;
    } > APP :data

    .request ORIGIN(APP) + LENGTH(APP) - 0x100 (NOLOAD) : {
        KEEP(*(.request))
        __request_end = .;
        . = ORIGIN(APP) + LENGTH(APP);
    } > APP :request

    .stack ORIGIN(STACK) (NOLOAD) : {
        __stack_bottom = .;
        . += LENGTH(STACK) - 16;
        __stack_top = .;
    } > STACK :stack

    .result ORIGIN(STACK) + LENGTH(STACK) - 16 (NOLOAD) : {
        KEEP(*(.result))
    } > STACK :stack

    /DISCARD/ : { *(.eh_frame .eh_frame_hdr) }
}

ASSERT(__bss_end <= ADDR(.request), "firmware image overlaps request")
ASSERT(__request_end - ADDR(.request) == 144, "unexpected request layout")
ASSERT(SIZEOF(.request) == 256, "unexpected request window")
ASSERT(SIZEOF(.result) == 16, "unexpected result layout")
