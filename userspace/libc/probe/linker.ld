OUTPUT_FORMAT(elf64-x86-64)
OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

PHDRS {
    phdr PT_PHDR PHDRS FLAGS(4);
    headers PT_LOAD FILEHDR PHDRS FLAGS(4);
    text PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data PT_LOAD FLAGS(6);
    tls PT_TLS FLAGS(4);
}

SECTIONS {
    PROVIDE_HIDDEN(__ehdr_start = 0x400000);
    . = 0x400000 + SIZEOF_HEADERS;
    .headers : { BYTE(0); } :headers
    ASSERT(SIZEOF_HEADERS < 0x1000, "Native headers exceed their page")

    . = ALIGN(0x1000);
    .text : { KEEP(*(.text.start)) *(.text .text.*) } :text
    . = ALIGN(0x1000);
    .rodata : { *(.rodata .rodata.*) } :rodata
    .tdata : ALIGN(64) { *(.tdata .tdata.*) } :rodata :tls
    .tbss (NOLOAD) : { *(.tbss .tbss.*) } :rodata :tls
    . = ADDR(.tbss) + SIZEOF(.tbss);
    .tls_end (NOLOAD) : { BYTE(0); } :rodata

    . = ALIGN(0x1000);
    .data : { *(.data .data.*) *(.got .got.*) } :data
    .preinit_array : {
        PROVIDE_HIDDEN(__preinit_array_start = .);
        KEEP(*(.preinit_array))
        PROVIDE_HIDDEN(__preinit_array_end = .);
    } :data
    .init_array : {
        PROVIDE_HIDDEN(__init_array_start = .);
        KEEP(*(SORT_BY_INIT_PRIORITY(.init_array.*))) KEEP(*(.init_array))
        PROVIDE_HIDDEN(__init_array_end = .);
    } :data
    .fini_array : {
        PROVIDE_HIDDEN(__fini_array_start = .);
        KEEP(*(SORT_BY_INIT_PRIORITY(.fini_array.*))) KEEP(*(.fini_array))
        PROVIDE_HIDDEN(__fini_array_end = .);
    } :data
    . = ALIGN(0x1000);
    .bss (NOLOAD) : { *(.bss .bss.*) *(COMMON) } :data
    /DISCARD/ : { *(.eh_frame*) *(.note .note.*) *(.comment) }
}
