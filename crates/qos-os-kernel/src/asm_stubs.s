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

.p2align 4
.global asm_iretq_to_user
.type asm_iretq_to_user, @function
# Arguments (SysV):
#   rdi = rip, rsi = rsp, rdx = cs, rcx = ss, r8 = rflags
asm_iretq_to_user:
    # Stack on entry: 16-byte aligned in caller, misaligned here by call's return address.
    # We deliberately keep the return address; pushing 5 qwords realigns the stack
    # before the hardware pops state via iretq.
    pushq %rcx        # SS
    pushq %rsi        # RSP
    pushq %r8         # RFLAGS
    pushq %rdx        # CS
    pushq %rdi        # RIP
    iretq
.size asm_iretq_to_user, . - asm_iretq_to_user
