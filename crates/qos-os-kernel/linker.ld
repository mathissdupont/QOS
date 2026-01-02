/* Linker script for the bare-metal kernel.
 *
 * We place the kernel in the higher half so the bootloader can map PT_LOAD
 * segments without colliding with identity-mapped low memory.
 */

ENTRY(_start)

KERNEL_OFFSET = 0xffffffff80000000;
KERNEL_PHYS_BASE = 0x00200000;

SECTIONS
{
    /* Higher-half kernel base address (bootloader/blog_os convention). */
    . = KERNEL_OFFSET;

    /* Bootloader (crate) metadata header required for loading the kernel. */
    . = ALIGN(4K);
    .bootloader_header : AT(ADDR(.bootloader_header) - KERNEL_OFFSET + KERNEL_PHYS_BASE)
    {
        KEEP(*(.bootloader_header))
    }

    . = ALIGN(4K);
    .text : AT(ADDR(.text) - KERNEL_OFFSET + KERNEL_PHYS_BASE)
    {
        *(.text .text.*)
    }

    . = ALIGN(4K);
    .rodata : AT(ADDR(.rodata) - KERNEL_OFFSET + KERNEL_PHYS_BASE)
    {
        *(.rodata .rodata.*)
    }

    . = ALIGN(4K);
    .data : AT(ADDR(.data) - KERNEL_OFFSET + KERNEL_PHYS_BASE)
    {
        /* Ensure the GOT doesn't become its own misaligned PT_LOAD segment. */
        *(.got .got.*)
        *(.got.plt .igot.plt)
        *(.data .data.*)
    }

    . = ALIGN(4K);
    .bss : AT(ADDR(.bss) - KERNEL_OFFSET + KERNEL_PHYS_BASE)
    {
        *(COMMON)
        *(.bss .bss.*)
    }

    /DISCARD/ :
    {
        *(.eh_frame .eh_frame_hdr)
        *(.note .note.*)
    }
}
