#include <cryolite/native.h>

#define PAGE_BYTES 4096ULL
#define EXIT_CODE 0xfedcba9876543210ULL

struct thread_probe {
    kryo_u32 ready;
    kryo_u32 release;
    kryo_thread_memory memory;
    kryo_u64 marker;
    kryo_u64 code;
};

void c_thread_start(void);
__asm__(".text\n"
        ".global c_thread_start\n"
        ".type c_thread_start,@function\n"
        "c_thread_start:\n"
        "cld\n"
        "xor %ebp,%ebp\n"
        "mov %rsp,%rcx\n"
        "call c_thread_body\n"
        "ud2\n"
        ".size c_thread_start,.-c_thread_start\n");

static void check(int valid, kryo_u64 failure) {
    if (!valid)
        kryo_exit(failure);
}

static void succeeded(kryo_result result, kryo_u64 failure) {
    check(result.status == KRYO_STATUS_OK && result.value == 0, failure);
}

/* This fixture uses one POD word at FS:0, not a libc TCB or compiler TLS. */
__attribute__((noreturn)) void c_thread_body(struct thread_probe *probe,
                                            kryo_u64 tls_base,
                                            kryo_u64 features,
                                            kryo_u64 initial_stack) {
    (void)features;
    kryo_u64 local = 0;
    kryo_u64 fs_word;
    __asm__ volatile("movq %%fs:0,%0" : "=r"(fs_word) : : "memory");
    check(tls_base == probe->memory.tls_base
          && initial_stack == probe->memory.stack_base + probe->memory.stack_bytes
          && (initial_stack & 15) == 0
          && (kryo_u64)&local >= probe->memory.stack_base
          && (kryo_u64)&local < initial_stack
          && fs_word == probe->marker, 451);
    __atomic_store_n(&probe->ready, 1, __ATOMIC_RELEASE);
    while (__atomic_load_n(&probe->release, __ATOMIC_ACQUIRE) == 0) {
        kryo_result parked = kryo_park((kryo_u64)&probe->release, 0);
        check(parked.status == KRYO_STATUS_OK, 452);
    }
    fs_word += 1;
    __asm__ volatile("movq %0,%%fs:0" : : "r"(fs_word) : "memory");
    kryo_thread_exit(probe->code);
}

void c_thread_checks(kryo_handle debug, kryo_handle wrong_type) {
    kryo_result reservation = kryo_vm_reserve(6 * PAGE_BYTES, PAGE_BYTES);
    check(reservation.status == KRYO_STATUS_OK, 430);
    kryo_u64 arena = reservation.value;
    kryo_thread_memory memory = {
        arena + PAGE_BYTES, 4 * PAGE_BYTES,
        arena + 5 * PAGE_BYTES, PAGE_BYTES,
        arena + 5 * PAGE_BYTES + 128, PAGE_BYTES
    };
    succeeded(kryo_vm_commit(memory.stack_base, memory.stack_bytes), 431);
    succeeded(kryo_vm_commit(memory.tls_base, memory.tls_bytes), 432);
    check(kryo_thread_join(0).status == KRYO_STATUS_INVALID_HANDLE
          && kryo_thread_join(wrong_type).status == KRYO_STATUS_WRONG_TYPE, 433);

    kryo_thread_memory invalid = memory;
    invalid.fs_base = memory.tls_base + memory.tls_bytes;
    check(kryo_thread_create_native(c_thread_start, 0, invalid).status
          == KRYO_STATUS_INVALID_ARGUMENT, 434);
    check(kryo_thread_create_native((void (*)(void))0, 0, memory).status
          == KRYO_STATUS_INVALID_ARGUMENT, 435);

    kryo_handle previous = 0;
    for (kryo_u64 round = 0; round < 2; ++round) {
        struct thread_probe probe = {0, 0, memory, 0x123456789abcdef0ULL + round,
                                     EXIT_CODE + round};
        volatile kryo_u64 *fs_word = (volatile kryo_u64 *)memory.fs_base;
        *fs_word = probe.marker;
        kryo_result created = kryo_thread_create_native(c_thread_start,
                                                       (kryo_u64)&probe, memory);
        check(created.status == KRYO_STATUS_OK && created.value != 0, 436);
        kryo_handle thread = created.value;
        while (__atomic_load_n(&probe.ready, __ATOMIC_ACQUIRE) == 0)
            succeeded(kryo_yield(), 437);

        kryo_result clock = kryo_clock();
        check(clock.status == KRYO_STATUS_OK && clock.value < ~(kryo_u64)0 - 500000, 438);
        check(kryo_thread_join_until(thread, clock.value + 500000).status
              == KRYO_STATUS_TIMED_OUT, 439);
        check(kryo_vm_decommit(memory.stack_base, memory.stack_bytes).status
              == KRYO_STATUS_BUSY
              && kryo_vm_release(arena).status == KRYO_STATUS_BUSY, 440);

        __atomic_store_n(&probe.release, 1, __ATOMIC_RELEASE);
        kryo_result wake = kryo_wake((kryo_u64)&probe.release, 1);
        check(wake.status == KRYO_STATUS_OK && wake.value <= 1, 441);
        kryo_thread_result joined = kryo_thread_join(thread);
        check(kryo_thread_result_valid(joined) && joined.reason == KRYO_EXIT_EXITED
              && joined.code == probe.code && *fs_word == probe.marker + 1, 442);
        if (previous != 0) {
            joined = kryo_thread_join(previous);
            check(kryo_thread_result_valid(joined) && joined.reason == KRYO_EXIT_EXITED
                  && joined.code == EXIT_CODE, 443);
            succeeded(kryo_close(previous), 444);
        }
        previous = thread;
        /* Reuse after actual join while the old control handle stays open. */
        succeeded(kryo_vm_decommit(memory.stack_base, memory.stack_bytes), 445);
        succeeded(kryo_vm_commit(memory.stack_base, memory.stack_bytes), 446);
    }
    succeeded(kryo_close(previous), 447);
    check(kryo_thread_join(previous).status == KRYO_STATUS_INVALID_HANDLE, 448);
    succeeded(kryo_vm_release(arena), 449);
    succeeded(kryo_debug_log(debug, 99, 2), 450);
}
