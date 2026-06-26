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
# iretq expects stack (bottom to top): SS, RSP, RFLAGS, CS, RIP
# Push in reverse order (top to bottom on stack)
asm_iretq_to_user:
    # Build iretq frame (push in reverse order of pop)
    pushq %rcx        # SS (user data segment)
    pushq %rsi        # RSP (user stack pointer)
    pushq %r8         # RFLAGS
    pushq %rdx        # CS (user code segment)
    pushq %rdi        # RIP (entry point)
    
    # Set data segments to user data selector BEFORE iretq
    # This prevents #GP on return to Ring 3
    mov %rcx, %rax    # SS value (already has RPL=3)
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %fs
    mov %ax, %gs
    
    # Zero out registers for clean user mode entry
    xor %rax, %rax
    xor %rbx, %rbx
    xor %rcx, %rcx
    xor %rdx, %rdx
    xor %rsi, %rsi
    xor %rdi, %rdi
    xor %r8, %r8
    xor %r9, %r9
    xor %r10, %r10
    xor %r11, %r11
    xor %r12, %r12
    xor %r13, %r13
    xor %r14, %r14
    xor %r15, %r15
    xor %rbp, %rbp
    
    iretq
.size asm_iretq_to_user, . - asm_iretq_to_user
