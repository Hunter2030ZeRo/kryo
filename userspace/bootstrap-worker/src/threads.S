.section .text.thread_start,"ax"
.global thread_start
.type thread_start,@function
thread_start:
    cld
    xor ebp, ebp
    call {main}
    ud2
.size thread_start, .-thread_start

.section .text.state_probe,"ax"
.global state_probe
.type state_probe,@function
state_probe:
    sub rsp, 64
    mov [rsp], rdi
    mov [rsp + 8], rsi
    mov [rsp + 16], rdx
    fnstcw [rsp + 24]
    stmxcsr [rsp + 28]
    mov word ptr [rsp + 32], 0x077f
    mov dword ptr [rsp + 36], 0x3f80
    test dil, 1
    jnz 1f
    mov word ptr [rsp + 32], 0x0b7f
    mov dword ptr [rsp + 36], 0x5f80
1:
    fldcw [rsp + 32]
    ldmxcsr [rsp + 36]
    fild qword ptr [rsp]
    movq xmm15, rdi
    test qword ptr [rsp + 16], 1
    jz 2f
    vxorps ymm14, ymm14, ymm14
    vinsertf128 ymm14, ymm14, xmm15, 1
2:
    xor eax, eax
    int 0x80
    mov [rsp + 56], rax
    mov ecx, 20000000
3:
    dec ecx
    jnz 3b
    fistp qword ptr [rsp + 40]
    xor eax, eax
    cmp qword ptr [rsp + 56], 0
    jne 5f
    mov rcx, [rsp]
    cmp [rsp + 40], rcx
    jne 5f
    movq rdx, xmm15
    cmp rdx, rcx
    jne 5f
    fnstcw [rsp + 40]
    mov dx, [rsp + 32]
    cmp [rsp + 40], dx
    jne 5f
    stmxcsr [rsp + 40]
    mov edx, [rsp + 36]
    cmp [rsp + 40], edx
    jne 5f
    mov rdx, fs:[0]
    cmp [rsp + 8], rdx
    jne 5f
    test qword ptr [rsp + 16], 1
    jz 4f
    vextractf128 xmm13, ymm14, 1
    movq rdx, xmm13
    cmp rdx, rcx
    jne 5f
4:
    mov eax, 1
5:
    test qword ptr [rsp + 16], 1
    jz 6f
    vzeroupper
6:
    fldcw [rsp + 24]
    ldmxcsr [rsp + 28]
    add rsp, 64
    ret
.size state_probe, .-state_probe
