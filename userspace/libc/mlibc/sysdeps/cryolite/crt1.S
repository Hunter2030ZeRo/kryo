.section .text.start,"ax",@progbits
.global _start
.type _start,@function
_start:
    xor %ebp, %ebp
    mov %rdi, %rdx
    mov %rsp, %rdi
    lea main(%rip), %rsi
    and $-16, %rsp
    call __cryolite_entry
    ud2
.size _start, .-_start
.section .note.GNU-stack,"",@progbits
