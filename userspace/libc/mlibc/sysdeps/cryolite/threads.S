.section .text,"ax",@progbits
.global __cryolite_thread_start
.type __cryolite_thread_start,@function
__cryolite_thread_start:
    xor %ebp, %ebp
    call __cryolite_thread_body
    ud2
.size __cryolite_thread_start, .-__cryolite_thread_start

.global __cryolite_reaper_start
.type __cryolite_reaper_start,@function
__cryolite_reaper_start:
    xor %ebp, %ebp
    call __cryolite_reaper_body
    ud2
.size __cryolite_reaper_start, .-__cryolite_reaper_start
.section .note.GNU-stack,"",@progbits
