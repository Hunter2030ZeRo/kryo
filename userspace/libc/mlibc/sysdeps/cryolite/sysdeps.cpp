#include <errno.h>
#include <limits.h>
#include <stdint.h>
#include <cryolite/native.h>
#include <mlibc/all-sysdeps.hpp>

namespace {

constexpr size_t page_size = 4096;
constexpr uint64_t nanos_per_second = 1000000000;
uint64_t startup_handle;
uint64_t debug_handle;
uint64_t wait_fallbacks;

[[noreturn]] void fail() {
    kryo_exit(253);
}

int error(kryo_status status) {
    switch (status) {
        case KRYO_STATUS_OK: return 0;
        case KRYO_STATUS_OUT_OF_RESOURCES: return ENOMEM;
        case KRYO_STATUS_PERMISSION_DENIED: return EACCES;
        case KRYO_STATUS_TIMED_OUT: return ETIMEDOUT;
        case KRYO_STATUS_CANCELLED: return ECANCELED;
        case KRYO_STATUS_BUSY: return EBUSY;
        case KRYO_STATUS_PEER_CLOSED: return EPIPE;
        case KRYO_STATUS_INVALID_CALL: return ENOSYS;
        default: return EINVAL;
    }
}

int deadline_after(time_t seconds, long nanos, uint64_t *deadline) {
    if (seconds < 0 || nanos < 0 || static_cast<uint64_t>(nanos) >= nanos_per_second)
        return EINVAL;
    auto whole = static_cast<uint64_t>(seconds);
    auto fraction = static_cast<uint64_t>(nanos);
    if (whole > (UINT64_MAX - fraction) / nanos_per_second)
        return EINVAL;
    auto duration = whole * nanos_per_second + fraction;
    auto now = kryo_clock();
    if (now.status)
        return error(now.status);
    if (duration > UINT64_MAX - now.value)
        return EINVAL;
    *deadline = now.value + duration;
    return 0;
}

uint64_t read_word(uint64_t record, size_t index) {
    auto result = kryo_startup_read(record, index);
    if (result.status)
        fail();
    return result.value;
}

bool handle_matches(uint64_t handle, uint64_t kind, uint64_t rights) {
    uint64_t call = 44;
    uint64_t actual_kind;
    register uint64_t actual_rights __asm__("r8");
    __asm__ volatile("int $0x80"
        : "+a"(call), "+D"(handle), "=d"(actual_kind), "=r"(actual_rights)
        :
        : "r9", "r10", "r11", "cc", "memory");
    return call == 0 && actual_kind == kind && actual_rights == rights;
}

struct Mapping {
    Mapping *next;
    void *window;
    size_t bytes;
};

Mapping *mappings;
alignas(4) uint32_t mapping_lock;

void lock_mappings() {
    while (__atomic_exchange_n(&mapping_lock, 1, __ATOMIC_ACQUIRE)) {
        auto result = kryo_park(reinterpret_cast<uintptr_t>(&mapping_lock), 1);
        if (result.status == KRYO_STATUS_OUT_OF_RESOURCES) {
            // A full wait quota must not discard an already committed mapping.
            if (kryo_yield().status)
                fail();
        } else if (result.status) {
            fail();
        }
    }
}

void unlock_mappings() {
    __atomic_store_n(&mapping_lock, 0, __ATOMIC_RELEASE);
    if (kryo_wake(reinterpret_cast<uintptr_t>(&mapping_lock), UINT64_MAX).status)
        fail();
}

bool rounded_size(size_t size, size_t *rounded) {
    if (!size || size > SIZE_MAX - (page_size - 1))
        return false;
    *rounded = (size + page_size - 1) & ~(page_size - 1);
    return *rounded <= SIZE_MAX - page_size;
}

int map_memory(size_t size, void **out) {
    size_t bytes;
    if (!rounded_size(size, &bytes))
        return ENOMEM;
    auto reserved = kryo_vm_reserve(bytes + page_size, page_size);
    if (reserved.status)
        return error(reserved.status);
    auto committed = kryo_vm_commit(reserved.value, bytes + page_size);
    if (committed.status) {
        if (kryo_vm_release(reserved.value).status)
            fail();
        return error(committed.status);
    }

    // Metadata stays in the same owned reservation and is never exposed to libc.
    auto record = reinterpret_cast<Mapping *>(reserved.value);
    record->window = reinterpret_cast<void *>(reserved.value + page_size);
    record->bytes = bytes;
    lock_mappings();
    record->next = mappings;
    mappings = record;
    unlock_mappings();
    *out = record->window;
    return 0;
}

int unmap_memory(void *window, size_t size) {
    size_t bytes;
    if (!rounded_size(size, &bytes))
        return EINVAL;
    lock_mappings();
    auto link = &mappings;
    while (*link && (*link)->window != window)
        link = &(*link)->next;
    auto record = *link;
    if (!record || record->bytes != bytes) {
        unlock_mappings();
        return EINVAL;
    }
    auto next = record->next;
    auto result = kryo_vm_release(reinterpret_cast<uintptr_t>(record));
    if (!result.status)
        *link = next;
    unlock_mappings();
    return error(result.status);
}

} // namespace

extern "C" uint64_t __cryolite_debug_handle() {
    return debug_handle;
}

extern "C" uint64_t __cryolite_startup_handle() {
    return startup_handle;
}

extern "C" uint64_t __cryolite_wait_fallbacks() {
    return __atomic_load_n(&wait_fallbacks, __ATOMIC_RELAXED);
}

extern "C" void __cryolite_startup(uint64_t record) {
    if (!record || startup_handle || read_word(record, 0) != KRYO_STARTUP_MAGIC
        || read_word(record, 1) != KRYO_STARTUP_VERSION)
        fail();
    auto count = read_word(record, 6);
    if (count > KRYO_STARTUP_MAX_GRANTS
        || read_word(record, 2) != KRYO_STARTUP_HEADER_WORDS + count * KRYO_STARTUP_ENTRY_WORDS)
        fail();
    constexpr uint64_t allowed[] = {
        0, 1 | 2 | 4 | 1024, 128 | 256 | 512 | 4 | 1024,
        8 | 16 | 2 | 4 | 1024, 32 | 64 | 4 | 1024,
        1 | 2 | 32 | 4 | 1024, 1 | 2 | 32 | 4 | 1024,
        2048 | 4 | 1024, 256 | 4 | 1024, 128 | 256 | 2 | 4 | 1024,
        4096 | 4 | 1024, 128 | 256, 128, 128 | 8192 | 4 | 1024,
    };
    uint64_t roles[KRYO_STARTUP_MAX_GRANTS];
    uint64_t debug = 0;
    for (size_t index = 0; index < count; ++index) {
        auto base = KRYO_STARTUP_HEADER_WORDS + index * KRYO_STARTUP_ENTRY_WORDS;
        auto role = read_word(record, base);
        auto kind = read_word(record, base + 1);
        auto rights = read_word(record, base + 2);
        auto handle = read_word(record, base + 3);
        if (!role || !kind || kind >= sizeof(allowed) / sizeof(allowed[0])
            || (rights & ~allowed[kind]) || !handle || !handle_matches(handle, kind, rights))
            fail();
        for (size_t earlier = 0; earlier < index; ++earlier)
            if (roles[earlier] == role)
                fail();
        roles[index] = role;
        if (role == 1) {
            if (kind != KRYO_OBJECT_DEBUG_CONSOLE || !(rights & KRYO_RIGHT_WRITE))
                fail();
            debug = handle;
        }
    }
    startup_handle = record;
    debug_handle = debug;
}

namespace mlibc {

void Sysdeps<LibcLog>::operator()(const char *message) {
    if (!debug_handle)
        return;
    while (*message) {
        size_t size = 0;
        while (size != KRYO_DEBUG_WRITE_MAX && message[size])
            ++size;
        if (kryo_debug_write(debug_handle, message, size).status)
            fail();
        message += size;
    }
}

void Sysdeps<LibcPanic>::operator()() {
    sysdep<LibcLog>("mlibc panic");
    fail();
}

void Sysdeps<Exit>::operator()(int status) {
    kryo_exit(static_cast<uint64_t>(static_cast<int64_t>(status)));
}

int Sysdeps<TcbSet>::operator()(void *pointer) {
    return error(kryo_set_tls(reinterpret_cast<uintptr_t>(pointer)).status);
}

int Sysdeps<AnonAllocate>::operator()(size_t size, void **pointer) {
    return map_memory(size, pointer);
}

int Sysdeps<AnonFree>::operator()(void *pointer, size_t size) {
    return unmap_memory(pointer, size);
}

int Sysdeps<VmMap>::operator()(void *hint, size_t size, int prot, int flags,
                               int fd, off_t offset, void **window) {
    if (!size || hint || offset || fd != -1)
        return EINVAL;
    if (prot != (PROT_READ | PROT_WRITE) || flags != (MAP_PRIVATE | MAP_ANONYMOUS))
        return ENOTSUP;
    return map_memory(size, window);
}

int Sysdeps<VmUnmap>::operator()(void *pointer, size_t size) {
    return unmap_memory(pointer, size);
}

int Sysdeps<FutexWait>::operator()(int *pointer, int expected, const timespec *time) {
    auto address = reinterpret_cast<uintptr_t>(pointer);
    kryo_result result;
    if (!time) {
        result = kryo_park(address, static_cast<uint32_t>(expected));
    } else {
        uint64_t deadline;
        if (int e = deadline_after(time->tv_sec, time->tv_nsec, &deadline))
            return e;
        result = kryo_park_until(address, static_cast<uint32_t>(expected), deadline);
    }
    if (result.status == KRYO_STATUS_OUT_OF_RESOURCES) {
        // The caller rechecks its predicate after this permitted spurious wake.
        __atomic_fetch_add(&wait_fallbacks, 1, __ATOMIC_RELAXED);
        if (kryo_yield().status)
            fail();
        return 0;
    }
    if (result.status)
        return error(result.status);
    return result.value == KRYO_PARK_CHANGED ? EAGAIN : 0;
}

int Sysdeps<FutexWake>::operator()(int *pointer, bool all) {
    return error(kryo_wake(reinterpret_cast<uintptr_t>(pointer), all ? UINT64_MAX : 1).status);
}

int Sysdeps<ClockGet>::operator()(int clock, time_t *seconds, long *nanos) {
    if (clock != CLOCK_MONOTONIC)
        return clock >= CLOCK_REALTIME && clock <= CLOCK_THREAD_CPUTIME_ID ? ENOTSUP : EINVAL;
    auto now = kryo_clock();
    if (now.status)
        return error(now.status);
    if (now.value / nanos_per_second > LONG_MAX)
        return EOVERFLOW;
    *seconds = static_cast<time_t>(now.value / nanos_per_second);
    *nanos = static_cast<long>(now.value % nanos_per_second);
    return 0;
}

int Sysdeps<Sleep>::operator()(time_t *seconds, long *nanos) {
    uint64_t deadline;
    if (int e = deadline_after(*seconds, *nanos, &deadline))
        return e;
    auto result = kryo_sleep_until(deadline);
    if (!result.status) {
        *seconds = 0;
        *nanos = 0;
        return 0;
    }
    if (result.status == KRYO_STATUS_CANCELLED) {
        auto now = kryo_clock();
        if (!now.status) {
            auto remaining = deadline > now.value ? deadline - now.value : 0;
            *seconds = static_cast<time_t>(remaining / nanos_per_second);
            *nanos = static_cast<long>(remaining % nanos_per_second);
        }
        return EINTR;
    }
    return error(result.status);
}

void Sysdeps<Yield>::operator()() {
    if (kryo_yield().status)
        fail();
}

int Sysdeps<Open>::operator()(const char *, int, mode_t, int *) {
    return ENOSYS;
}

int Sysdeps<Read>::operator()(int, void *, size_t, ssize_t *) {
    return EBADF;
}

int Sysdeps<Write>::operator()(int, const void *, size_t, ssize_t *) {
    return EBADF;
}

int Sysdeps<Seek>::operator()(int, off_t, int, off_t *) {
    return EBADF;
}

int Sysdeps<Close>::operator()(int) {
    return EBADF;
}

int Sysdeps<Isatty>::operator()(int) {
    return EBADF;
}

} // namespace mlibc
