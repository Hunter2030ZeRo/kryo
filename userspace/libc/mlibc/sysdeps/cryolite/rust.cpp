#include <limits.h>
#include <stdint.h>
#include <threads.h>
#include <cryolite/native.h>
#include <cryolite/rust-runtime.h>
#include <mlibc/threads.hpp>

static_assert(sizeof(tss_t) == sizeof(uintptr_t));

extern "C" uint64_t __cryolite_debug_handle();

namespace {

[[gnu::cold, noreturn]] void fail(uint32_t operation, int64_t detail) {
    auto debug = __cryolite_debug_handle();
    if (debug)
        kryo_debug_log(debug, 228, (uint64_t{operation} << 32) | uint32_t(detail));
    __cryolite_rust_abort();
}

} // namespace

extern "C" void __cryolite_rust_abort() {
    kryo_exit(134);
}

extern "C" uint64_t __cryolite_rust_clock() {
    auto result = kryo_clock();
    if (result.status)
        fail(1, result.status);
    return result.value;
}

extern "C" void __cryolite_rust_sleep_until(uint64_t deadline) {
    while (__cryolite_rust_clock() < deadline) {
        auto result = kryo_sleep_until(deadline);
        if (result.status == KRYO_STATUS_OUT_OF_RESOURCES)
            __cryolite_rust_yield();
        else if (result.status && result.status != KRYO_STATUS_CANCELLED)
            fail(2, result.status);
    }
}

extern "C" void __cryolite_rust_yield() {
    auto result = kryo_yield();
    if (result.status)
        fail(3, result.status);
}

extern "C" int64_t __cryolite_rust_wait(const uint32_t *word, uint32_t expected,
                                       uint64_t deadline, int timed) {
    auto address = reinterpret_cast<uintptr_t>(word);
    auto result = timed ? kryo_park_until(address, expected, deadline)
                        : kryo_park(address, expected);
    if (result.status == KRYO_STATUS_OUT_OF_RESOURCES) {
        // The Rust primitive rechecks its predicate after this spurious wake.
        __cryolite_rust_yield();
        return 0;
    }
    // Cancellation means the private mapping retired while Rust still borrowed it.
    if (result.status && result.status != KRYO_STATUS_TIMED_OUT)
        fail(4, result.status);
    if (!result.status && result.value > KRYO_PARK_CHANGED)
        fail(5, result.value);
    return result.status;
}

extern "C" uint64_t __cryolite_rust_wake(const uint32_t *word, uint64_t maximum) {
    auto result = kryo_wake(reinterpret_cast<uintptr_t>(word), maximum);
    if (result.status)
        fail(6, result.status);
    return result.value;
}

extern "C" int __cryolite_rust_thread_create(uintptr_t *thread, size_t stack_size,
                                            int (*entry)(void *), void *argument) {
    __mlibc_threadattr attributes;
    attributes.__stacksize = stack_size < PTHREAD_STACK_MIN ? PTHREAD_STACK_MIN : stack_size;
    thrd_t created;
    auto error = mlibc::thread_create(&created, &attributes,
                                     reinterpret_cast<void *>(entry), argument, true);
    if (!error)
        *thread = reinterpret_cast<uintptr_t>(created);
    return error;
}

extern "C" int __cryolite_rust_thread_join(uintptr_t thread) {
    return mlibc::thread_join(reinterpret_cast<thrd_t>(thread), nullptr);
}

extern "C" int __cryolite_rust_thread_detach(uintptr_t thread) {
    return mlibc::thread_detach(reinterpret_cast<thrd_t>(thread));
}

extern "C" int __cryolite_rust_key_create(uintptr_t *key, void (*destructor)(void *)) {
    return tss_create(key, destructor);
}

extern "C" void __cryolite_rust_key_delete(uintptr_t key) {
    tss_delete(key);
}

extern "C" void *__cryolite_rust_key_get(uintptr_t key) {
    return tss_get(key);
}

extern "C" int __cryolite_rust_key_set(uintptr_t key, void *value) {
    return tss_set(key, value);
}
