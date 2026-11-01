.section .text
.p2align 4
.global kryo_enter_user
.type kryo_enter_user, @function
kryo_enter_user:
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    mov rax, r8
    mov [rax + {kernel_rsp}], rsp
    mov r9, cr3
    mov [rax + {kernel_cr3}], r9
    pushfq
    pop r9
    mov [rax + {kernel_flags}], r9
    mov cr3, rdi
    mov ax, cx
    mov ds, ax
    mov es, ax
    push rcx
    push qword ptr [rsi + {context_rsp}]
    push qword ptr [rsi + {context_flags}]
    push rdx
    push qword ptr [rsi + {context_rip}]
    mov r9, r8
    mov r8, [r9 + {fp_area}]
    test r8, r8
    jz 1f
    mov rax, cr0
    and rax, -9
    mov cr0, rax
    cmp qword ptr [r9 + {fp_format}], 2
    jne 2f
    mov eax, [r9 + {fp_mask}]
    xor edx, edx
    xrstor64 [r8]
    jmp 1f
2:
    fxrstor64 [r8]
1:
    mov rax, [rsi]
    mov rbx, [rsi + 8]
    mov rcx, [rsi + 16]
    mov rdx, [rsi + 24]
    mov rdi, [rsi + 40]
    mov rbp, [rsi + 48]
    mov r8, [rsi + 56]
    mov r9, [rsi + 64]
    mov r10, [rsi + 72]
    mov r11, [rsi + 80]
    mov r12, [rsi + 88]
    mov r13, [rsi + 96]
    mov r14, [rsi + 104]
    mov r15, [rsi + 112]
    mov rsi, [rsi + 32]
    iretq
.size kryo_enter_user, .-kryo_enter_user

.global kryo_leave_user
.type kryo_leave_user, @function
kryo_leave_user:
    cli
    cld
    mov r10, rdi
    mov rcx, [r10 + {kernel_cr3}]
    mov cr3, rcx
    cmp qword ptr [r10 + {fp_area}], 0
    je 3f
    mov rax, cr0
    and rax, -9
    mov cr0, rax
    mov r8, [r10 + {kernel_fp}]
    cmp qword ptr [r10 + {fp_format}], 2
    jne 4f
    mov eax, [r10 + {fp_mask}]
    xor edx, edx
    xrstor64 [r8]
    mov rax, [r10 + {kernel_xcr0}]
    mov rdx, rax
    shr rdx, 32
    xor ecx, ecx
    xsetbv
    jmp 3f
4:
    fxrstor64 [r8]
3:
    # Restore EFER after FP state: AMD FFXSR can otherwise skip kernel XMM restore.
    mov rax, [r10 + {kernel_efer}]
    mov rdx, rax
    shr rdx, 32
    mov ecx, 0xc0000080
    wrmsr
    mov rcx, [r10 + {kernel_cr4}]
    mov cr4, rcx
    mov rcx, [r10 + {kernel_cr0}]
    mov cr0, rcx
    push qword ptr [r10 + {kernel_flags}]
    popfq
    mov rsp, [r10 + {kernel_rsp}]
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret
.size kryo_leave_user, .-kryo_leave_user

.macro save_registers
    push r15
    push r14
    push r13
    push r12
    push r11
    push r10
    push r9
    push r8
    push rbp
    push rdi
    push rsi
    push rdx
    push rcx
    push rbx
    push rax
.endm

.macro restore_registers
    pop rax
    pop rbx
    pop rcx
    pop rdx
    pop rsi
    pop rdi
    pop rbp
    pop r8
    pop r9
    pop r10
    pop r11
    pop r12
    pop r13
    pop r14
    pop r15
.endm

.macro prepare_rust
    cld
    pushfq
    # Clear user AC without requiring SMAP/CLAC support; keep saved user flags intact.
    and qword ptr [rsp], -262145
    popfq
.endm

.macro save_extended
    test byte ptr [rsp + {trap_code}], 3
    jz 9f
    mov rbx, rsp
    and rsp, -16
    call {trap_state}
    mov rsp, rbx
    mov r9, rax
    test r9, r9
    jz 9f
    mov r8, [r9 + {fp_area}]
    test r8, r8
    jz 9f
    mov rax, cr0
    and rax, -9
    mov cr0, rax
    cmp qword ptr [r9 + {fp_format}], 2
    jne 8f
    mov eax, [r9 + {fp_mask}]
    xor edx, edx
    xsave64 [r8]
    jmp 7f
8:
    fxsave64 [r8]
7:
    # State save precedes dispatch; the lookup helper is strictly soft-float.
    mov rax, cr0
    or rax, 8
    mov cr0, rax
9:
.endm

.global kryo_user_exit
.type kryo_user_exit, @function
kryo_user_exit:
    push 0
    push 0x80
    save_registers
    prepare_rust
    save_extended
    mov rdi, rsp
    and rsp, -16
    call {exit_dispatch}
    ud2
.size kryo_user_exit, .-kryo_user_exit

.macro irq_stub name, vector, dispatcher
.global \name
.type \name, @function
\name:
    push 0
    push \vector
    save_registers
    prepare_rust
    save_extended
    mov rdi, rsp
    mov rbx, rsp
    and rsp, -16
    call \dispatcher
    mov rsp, rbx
    restore_registers
    add rsp, 16
    iretq
.size \name, .-\name
.endm

irq_stub kryo_device_interrupt, 0xd0, {device_dispatch}
irq_stub kryo_timer_interrupt, 0xe0, {timer_dispatch}
irq_stub kryo_reschedule_interrupt, 0xe1, {reschedule_dispatch}
irq_stub kryo_shootdown_interrupt, 0xe2, {shootdown_dispatch}

.global kryo_spurious_interrupt
.type kryo_spurious_interrupt, @function
kryo_spurious_interrupt:
    iretq
.size kryo_spurious_interrupt, .-kryo_spurious_interrupt

.macro fault_no_error number
.global kryo_fault_\number
.type kryo_fault_\number, @function
kryo_fault_\number:
    push 0
    push \number
    jmp kryo_fault_common
.endm

.macro fault_with_error number
.global kryo_fault_\number
.type kryo_fault_\number, @function
kryo_fault_\number:
    push \number
    jmp kryo_fault_common
.endm

fault_no_error 0
fault_no_error 1
fault_no_error 4
fault_no_error 5
fault_no_error 6
fault_no_error 7
fault_with_error 8
fault_with_error 10
fault_with_error 11
fault_with_error 12
fault_with_error 13
fault_with_error 14
fault_no_error 16
fault_with_error 17
fault_no_error 19
fault_no_error 20
fault_with_error 21
fault_with_error 29
fault_with_error 30

kryo_fault_common:
    save_registers
    prepare_rust
    save_extended
    mov rdi, rsp
    mov rsi, cr2
    and rsp, -16
    call {fault_dispatch}
    ud2
