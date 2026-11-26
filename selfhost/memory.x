OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY {
    RAM (rwx) : ORIGIN = 0x10000, LENGTH = 128K
}

PHDRS {
    code PT_LOAD FLAGS(5);
    data PT_LOAD FLAGS(6);
    stack PT_LOAD FLAGS(6);
}

SECTIONS {
    .text ORIGIN(RAM) : {
        KEEP(*(.text.start))
        *(.text .text.*)
    } > RAM :code

    .rodata : {
        *(.rodata .rodata.* .srodata .srodata.*)
    } > RAM :code

    .data ALIGN(4096) : {
        PROVIDE(__global_pointer$ = . + 0x800);
        *(.data .data.* .sdata .sdata.*)
    } > RAM :data

    .bss (NOLOAD) : ALIGN(8) {
        __bss_start = .;
        *(.bss .bss.* .sbss .sbss.* COMMON)
        . = ALIGN(8);
        __bss_end = .;
    } > RAM :data

    .request ORIGIN(RAM) + LENGTH(RAM) - 0x4100 (NOLOAD) : {
        KEEP(*(.request))
    } > RAM :stack

    .stack ORIGIN(RAM) + LENGTH(RAM) - 0x4000 (NOLOAD) : {
        __stack_bottom = .;
        . += 0x3ff0;
        __stack_top = .;
    } > RAM :stack

    .result ORIGIN(RAM) + LENGTH(RAM) - 16 (NOLOAD) : {
        KEEP(*(.result))
    } > RAM :stack

    /DISCARD/ : { *(.eh_frame .eh_frame_hdr) }
}

ASSERT(__bss_end <= ADDR(.request), "firmware image overlaps request")
ASSERT(SIZEOF(.request) == 144, "unexpected request layout")
ASSERT(SIZEOF(.result) == 16, "unexpected result layout")
