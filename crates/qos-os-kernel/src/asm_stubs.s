# Assembly stubs for QOS
# These are compiled separately to avoid LLVM inline asm issues

.section .text

.global asm_triple_fault
.type asm_triple_fault, @function
asm_triple_fault:
    cli
    # Load null IDT
    xor %eax, %eax
    lidt (%rax)
    int3
    # Should never reach here
    hlt
.size asm_triple_fault, . - asm_triple_fault
