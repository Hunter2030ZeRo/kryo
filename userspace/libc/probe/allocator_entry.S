.section .text.allocator_entry,"ax",@progbits
.global __cryolite_allocator_entry
.type __cryolite_allocator_entry,@function
__cryolite_allocator_entry:
    cld
    xor %ebp, %ebp
    mov %rsp, %rcx
    call __cryolite_allocator_body
    ud2
.size __cryolite_allocator_entry, .-__cryolite_allocator_entry

.section .note.GNU-stack,"",@progbits
