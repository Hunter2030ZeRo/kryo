#include <cryolite/native.h>
#include "../allocator/include/allocator.h"

#define PAGE_BYTES 4096ULL
#define WORKERS 2
#define OBJECTS 16
#define ROUNDS 16

struct barrier {
    kryo_u32 arrived;
    kryo_u32 phase;
};

struct worker {
    struct barrier *barrier;
    kryo_thread_memory memory;
    kryo_handle debug;
    unsigned number;
};

struct allocation {
    unsigned char *data;
    kryo_size size;
    kryo_size alignment;
    unsigned char pattern;
};

void __cryolite_allocator_entry(void);

static void require(int condition, kryo_handle debug, kryo_u64 stage) {
    if (!condition) {
        (void)kryo_debug_log(debug, 209, stage);
        kryo_exit(stage);
    }
}

static void succeeded(kryo_result result, kryo_handle debug, kryo_u64 stage) {
    require(result.status == KRYO_STATUS_OK && result.value == 0, debug, stage);
}

static void synchronize(struct worker *worker) {
    struct barrier *barrier = worker->barrier;
    kryo_u32 phase = __atomic_load_n(&barrier->phase, __ATOMIC_ACQUIRE);
    if (__atomic_fetch_add(&barrier->arrived, 1, __ATOMIC_ACQ_REL) == WORKERS - 1) {
        __atomic_store_n(&barrier->arrived, 0, __ATOMIC_RELAXED);
        __atomic_store_n(&barrier->phase, phase + 1, __ATOMIC_RELEASE);
        kryo_result wake = kryo_wake((kryo_u64)&barrier->phase, WORKERS - 1);
        require(wake.status == KRYO_STATUS_OK && wake.value <= WORKERS - 1,
                worker->debug, 301);
    } else {
        while (__atomic_load_n(&barrier->phase, __ATOMIC_ACQUIRE) == phase) {
            kryo_result parked = kryo_park((kryo_u64)&barrier->phase, phase);
            if (parked.status == KRYO_STATUS_OUT_OF_RESOURCES)
                require(kryo_yield().status == KRYO_STATUS_OK, worker->debug, 302);
            else
                require(parked.status == KRYO_STATUS_OK, worker->debug, 302);
        }
    }
}

static void fill(struct allocation *allocation) {
    volatile unsigned char *bytes = allocation->data;
    for (kryo_size index = 0; index < allocation->size; ++index)
        bytes[index] = allocation->pattern;
}

static void verify(struct allocation *allocation, kryo_handle debug) {
    require(allocation->data != NULL
            && ((kryo_u64)allocation->data & (allocation->alignment - 1)) == 0
            && __cryolite_allocation_size(allocation->data) == allocation->size,
            debug, 303);
    volatile unsigned char *bytes = allocation->data;
    for (kryo_size index = 0; index < allocation->size; ++index)
        require(bytes[index] == allocation->pattern, debug, 304);
}

/* Raw children have a self-pointer at FS:0, not a provider TCB. Keep this path
 * independent of malloc, errno, compiler TLS and every provider lock. */
__attribute__((noreturn)) void __cryolite_allocator_body(struct worker *worker,
                                                        kryo_u64 tls_base,
                                                        kryo_u64 features,
                                                        kryo_u64 initial_stack) {
    (void)features;
    kryo_u64 local;
    kryo_u64 self;
    __asm__ volatile("movq %%fs:0,%0" : "=r"(self) : : "memory");
    require(tls_base == worker->memory.tls_base
            && self == worker->memory.fs_base
            && initial_stack == worker->memory.stack_base + worker->memory.stack_bytes
            && (initial_stack & 15) == 0
            && (kryo_u64)&local >= worker->memory.stack_base
            && (kryo_u64)&local < initial_stack, worker->debug, 305);

    for (unsigned round = 0; round < ROUNDS; ++round) {
        struct allocation objects[OBJECTS];
        synchronize(worker);
        for (unsigned slot = 0; slot < OBJECTS; ++slot) {
            struct allocation *allocation = &objects[slot];
            allocation->size = 64 + (round * 131 + slot * 197 + worker->number * 53) % 2048;
            allocation->alignment = 16u << ((round + slot + worker->number) % 9);
            allocation->pattern = (unsigned char)(0x51 ^ worker->number * 73 ^ round * 17 ^ slot * 29);
            allocation->data = __cryolite_allocate(allocation->size, allocation->alignment);
            require(allocation->data != NULL, worker->debug, 306);
            fill(allocation);
        }
        // Both complete live sets coexist before either thread checks or frees.
        synchronize(worker);
        for (unsigned slot = 0; slot < OBJECTS; ++slot) {
            struct allocation *allocation = &objects[slot];
            verify(allocation, worker->debug);
            if ((slot + round) % 3 == 0) {
                kryo_size old_size = allocation->size;
                kryo_size new_size = old_size + 513 + worker->number * 37;
                unsigned char *replacement = __cryolite_reallocate(allocation->data, new_size);
                require(replacement != NULL, worker->debug, 307);
                allocation->data = replacement;
                // The existing bytes and original alignment survive relocation.
                require(((kryo_u64)replacement & (allocation->alignment - 1)) == 0,
                        worker->debug, 308);
                volatile unsigned char *bytes = replacement;
                for (kryo_size index = 0; index < old_size; ++index)
                    require(bytes[index] == allocation->pattern, worker->debug, 309);
                allocation->size = new_size;
                allocation->pattern ^= 0xa5;
                fill(allocation);
            }
        }
        synchronize(worker);
        for (unsigned slot = 0; slot < OBJECTS; ++slot) {
            verify(&objects[slot], worker->debug);
            __cryolite_deallocate(objects[slot].data);
        }
        synchronize(worker);
    }
    __asm__ volatile("movq %%fs:0,%0" : "=r"(self) : : "memory");
    require(self == worker->memory.fs_base, worker->debug, 310);
    kryo_thread_exit(0);
}

void check_allocator(kryo_handle debug) {
    struct barrier barrier = {0, 0};
    struct worker workers[WORKERS];
    kryo_handle threads[WORKERS];
    kryo_u64 reservations[WORKERS];

    // Execution backing is independent of Talc, so reclaiming an empty Talc
    // arena can never try to release one of these still-running entry stacks.
    for (unsigned number = 0; number < WORKERS; ++number) {
        kryo_result reserved = kryo_vm_reserve(6 * PAGE_BYTES, PAGE_BYTES);
        require(reserved.status == KRYO_STATUS_OK && reserved.value != 0, debug, 311);
        kryo_u64 base = reserved.value;
        reservations[number] = base;
        struct worker *worker = &workers[number];
        worker->barrier = &barrier;
        worker->debug = debug;
        worker->number = number;
        worker->memory.stack_base = base + PAGE_BYTES;
        worker->memory.stack_bytes = 4 * PAGE_BYTES;
        worker->memory.tls_base = base + 5 * PAGE_BYTES;
        worker->memory.tls_bytes = PAGE_BYTES;
        worker->memory.fs_base = worker->memory.tls_base + 128;
        worker->memory.guard_bytes = PAGE_BYTES;
        succeeded(kryo_vm_commit(worker->memory.stack_base, worker->memory.stack_bytes), debug, 312);
        succeeded(kryo_vm_commit(worker->memory.tls_base, worker->memory.tls_bytes), debug, 313);
        *(volatile kryo_u64 *)worker->memory.fs_base = worker->memory.fs_base;
    }
    for (unsigned number = 0; number < WORKERS; ++number) {
        kryo_result created = kryo_thread_create_native(__cryolite_allocator_entry,
                                                       (kryo_u64)&workers[number], workers[number].memory);
        require(created.status == KRYO_STATUS_OK && created.value != 0, debug, 314);
        threads[number] = created.value;
    }
    for (unsigned number = 0; number < WORKERS; ++number) {
        kryo_result start = kryo_clock();
        require(start.status == KRYO_STATUS_OK, debug, 315);
        kryo_thread_result joined;
        for (;;) {
            joined = kryo_thread_join(threads[number]);
            if (joined.status != KRYO_STATUS_OUT_OF_RESOURCES)
                break;
            kryo_result now = kryo_clock();
            require(now.status == KRYO_STATUS_OK && now.value - start.value < 10000000000ULL,
                    debug, 315);
            require(kryo_yield().status == KRYO_STATUS_OK, debug, 315);
        }
        require(kryo_thread_result_valid(joined) && joined.reason == KRYO_EXIT_EXITED
                && joined.code == 0, debug, 315);
        succeeded(kryo_close(threads[number]), debug, 316);
        // Only actual native completion authorizes stack/TLS backing reuse.
        succeeded(kryo_vm_release(reservations[number]), debug, 317);
    }
    succeeded(kryo_debug_log(debug, 203, WORKERS), debug, 318);
}
