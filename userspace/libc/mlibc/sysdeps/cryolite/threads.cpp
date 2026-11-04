#include <errno.h>
#include <fenv.h>
#include <limits.h>
#include <stdint.h>
#include <stdlib.h>
#include <cryolite/native.h>
#include <cryolite/thread-runtime.h>
#include <mlibc/all-sysdeps.hpp>
#include <mlibc/tcb.hpp>
#include <mlibc/thread.hpp>

#if !MLIBC_STATIC_BUILD || __MLIBC_POSIX_OPTION || MLIBC_BUILDING_RTLD
#error The Cryolite thread backend supports only the static ANSI profile.
#endif

extern "C" void __cryolite_thread_start();
extern "C" void __cryolite_reaper_start();
extern "C" uint64_t __cryolite_debug_handle();

namespace {

constexpr size_t page_size = 4096;
constexpr uint32_t last_id = (uint32_t{1} << 30) - 1;

struct ThreadRecord {
    ThreadRecord *next;
    Tcb *tcb;
    void *keys;
    void **dtv;
    uintptr_t payload;
    size_t payload_bytes;
    size_t front_bytes;
    uintptr_t stack;
    uintptr_t stack_base;
    size_t stack_bytes;
    size_t guard_bytes;
    void *entry;
    void *argument;
    fenv_t environment;
    kryo_handle control;
    uint32_t id;
    alignas(4) uint32_t creator_pin;
    bool main;
    bool claimed;
    bool prepared;
    bool detached;
    bool active;
    bool completed;
    bool destroyed;
    bool control_closed;
    bool poisoned;
    TcbThreadReturnValue result_type;
    mlibc::thread_exit_return result;
};

static_assert(sizeof(ThreadRecord) <= page_size);
ThreadRecord *records;
ThreadRecord *main_record;
size_t child_records;
size_t active_threads;
uint32_t next_id = 2;
alignas(4) uint32_t registry_lock;
alignas(4) uint32_t reaper_state;
alignas(4) uint32_t reaper_sequence;

[[noreturn]] void fail(const char *message) {
    auto debug = __cryolite_debug_handle();
    if (debug) {
        size_t length = 0;
        while (length != KRYO_DEBUG_WRITE_MAX && message[length])
            ++length;
        kryo_debug_write(debug, message, length);
    }
    kryo_exit(254);
}

int error(kryo_status status) {
    switch (status) {
        case KRYO_STATUS_OK: return 0;
        case KRYO_STATUS_OUT_OF_RESOURCES: return EAGAIN;
        case KRYO_STATUS_TIMED_OUT: return ETIMEDOUT;
        case KRYO_STATUS_CANCELLED: return ECANCELED;
        case KRYO_STATUS_BUSY: return EBUSY;
        case KRYO_STATUS_PERMISSION_DENIED: return EACCES;
        default: return EINVAL;
    }
}

void yield() {
    if (kryo_yield().status)
        fail("thread runtime: yield failed");
}

void wait_word(uint32_t *word, uint32_t expected) {
    auto result = kryo_park(reinterpret_cast<uintptr_t>(word), expected);
    if (result.status == KRYO_STATUS_OUT_OF_RESOURCES)
        yield();
    else if (result.status)
        fail("thread runtime: private wait failed");
}

void wake_word(uint32_t *word) {
    if (kryo_wake(reinterpret_cast<uintptr_t>(word), UINT64_MAX).status)
        fail("thread runtime: private wake failed");
}

void lock() {
    while (__atomic_exchange_n(&registry_lock, 1, __ATOMIC_ACQUIRE))
        wait_word(&registry_lock, 1);
}

void unlock() {
    __atomic_store_n(&registry_lock, 0, __ATOMIC_RELEASE);
    wake_word(&registry_lock);
}

void notify_reaper() {
    __atomic_fetch_add(&reaper_sequence, 1, __ATOMIC_RELEASE);
    wake_word(&reaper_sequence);
}

bool add(size_t first, size_t second, size_t *out) {
    return !__builtin_add_overflow(first, second, out);
}

bool aligned(size_t size, size_t alignment, size_t *out) {
    size_t total;
    if (!add(size, alignment - 1, &total))
        return false;
    *out = total & ~(alignment - 1);
    return true;
}

ThreadRecord *find(void *tcb) {
    for (auto record = records; record; record = record->next)
        if (record->tcb == tcb)
            return record;
    return nullptr;
}

uint32_t allocate_id() {
    auto first = next_id;
    do {
        auto candidate = next_id;
        next_id = next_id == last_id ? 2 : next_id + 1;
        bool used = false;
        for (auto record = records; record; record = record->next) {
            if (record->id == candidate) {
                used = true;
                break;
            }
        }
        if (!used)
            return candidate;
    } while (next_id != first);
    return 0;
}

void release_failed_reservation(uintptr_t base) {
    if (kryo_vm_release(base).status)
        fail("thread runtime: reservation rollback failed");
}

int reserve_committed(size_t bytes, uintptr_t *base) {
    auto reserved = kryo_vm_reserve(bytes, page_size);
    if (reserved.status)
        return error(reserved.status);
    auto committed = kryo_vm_commit(reserved.value, bytes);
    if (committed.status) {
        release_failed_reservation(reserved.value);
        return error(committed.status);
    }
    *base = reserved.value;
    return 0;
}

void wait_creator(ThreadRecord *record) {
    while (__atomic_load_n(&record->creator_pin, __ATOMIC_ACQUIRE))
        wait_word(&record->creator_pin, 1);
}

void unclaim(ThreadRecord *record) {
    lock();
    record->claimed = false;
    unlock();
}

int observe_completion(ThreadRecord *record) {
    if (record->completed)
        return 0;
    auto result = kryo_thread_join(record->control);
    if (result.status)
        return error(result.status);
    if (!kryo_thread_result_valid(result) || result.reason != KRYO_EXIT_EXITED || result.code != 0) {
        lock();
        record->poisoned = true;
        unlock();
        return EIO;
    }
    wait_creator(record);
    if (__atomic_load_n(&record->tcb->didExit, __ATOMIC_ACQUIRE) != 1) {
        lock();
        record->poisoned = true;
        unlock();
        return EIO;
    }
    record->result_type = record->tcb->returnValueType;
    record->result = record->tcb->returnValue;
    record->completed = true;
    return 0;
}

int retire(ThreadRecord *record) {
    if (record->control && !record->control_closed) {
        auto result = kryo_close(record->control);
        if (result.status)
            return error(result.status);
        record->control_closed = true;
    }
    if (record->stack) {
        auto result = kryo_vm_release(record->stack);
        if (result.status)
            return error(result.status);
        record->stack = 0;
    }
    if (!record->destroyed && record->tcb) {
        using Keys = frg::array<Tcb::LocalKey, PTHREAD_KEYS_MAX>;
        static_cast<Keys *>(record->keys)->~Keys();
        record->tcb->~Tcb();
        record->destroyed = true;
    }

    lock();
    auto link = &records;
    while (*link && *link != record)
        link = &(*link)->next;
    if (!*link || record->main)
        fail("thread runtime: retirement owner missing");
    auto next = record->next;
    auto result = kryo_vm_release(reinterpret_cast<uintptr_t>(record));
    if (!result.status) {
        *link = next;
        --child_records;
    }
    unlock();
    return error(result.status);
}

int start_reaper() {
    constexpr size_t stack_bytes = 64 * 1024;
    constexpr size_t total_bytes = page_size + stack_bytes + page_size;
    auto reserved = kryo_vm_reserve(total_bytes, page_size);
    if (reserved.status)
        return error(reserved.status);
    auto committed = kryo_vm_commit(reserved.value + page_size, stack_bytes + page_size);
    if (committed.status) {
        release_failed_reservation(reserved.value);
        return error(committed.status);
    }
    auto tls = reserved.value + page_size + stack_bytes;
    *reinterpret_cast<uintptr_t *>(tls) = tls;
    kryo_thread_memory memory = {
        reserved.value + page_size, stack_bytes,
        tls, page_size, tls, page_size,
    };
    auto created = kryo_thread_create_native(__cryolite_reaper_start, 0, memory);
    if (created.status) {
        release_failed_reservation(reserved.value);
        return error(created.status);
    }
    // The process owns the permanent raw reaper and its private stack/TLS.
    return 0;
}

int ensure_reaper() {
    for (;;) {
        lock();
        auto state = __atomic_load_n(&reaper_state, __ATOMIC_RELAXED);
        if (state == 2) {
            unlock();
            return 0;
        }
        if (!state) {
            __atomic_store_n(&reaper_state, 1, __ATOMIC_RELEASE);
            unlock();
            auto result = start_reaper();
            lock();
            __atomic_store_n(&reaper_state, result ? 0 : 2, __ATOMIC_RELEASE);
            wake_word(&reaper_state);
            unlock();
            return result;
        }
        unlock();
        wait_word(&reaper_state, 1);
    }
}

} // namespace

extern "C" int __cryolite_tcb_allocate(size_t front_bytes, size_t key_bytes,
                                        size_t dtv_count, cryolite_tcb_storage *out) {
    size_t keys_offset;
    size_t dtv_offset;
    size_t dtv_bytes;
    size_t payload_bytes;
    size_t allocation_bytes;
    if (front_bytes < sizeof(Tcb)
        || !aligned(front_bytes, 16, &keys_offset)
        || !add(keys_offset, key_bytes, &dtv_offset)
        || !aligned(dtv_offset, 16, &dtv_offset)
        || __builtin_mul_overflow(dtv_count, sizeof(void *), &dtv_bytes)
        || !add(dtv_offset, dtv_bytes, &payload_bytes)
        || !aligned(payload_bytes, page_size, &payload_bytes)
        || !add(payload_bytes, page_size, &allocation_bytes))
        return ENOMEM;
    uintptr_t base;
    if (int e = reserve_committed(allocation_bytes, &base))
        return e;

    auto record = new (reinterpret_cast<void *>(base)) ThreadRecord{};
    record->payload = base + page_size;
    record->payload_bytes = payload_bytes;
    record->front_bytes = front_bytes;
    record->keys = reinterpret_cast<void *>(record->payload + keys_offset);
    record->dtv = reinterpret_cast<void **>(record->payload + dtv_offset);
    record->creator_pin = 1;
    lock();
    record->main = !main_record;
    record->id = record->main ? 1 : allocate_id();
    if (!record->id) {
        unlock();
        release_failed_reservation(base);
        return EAGAIN;
    }
    record->next = records;
    records = record;
    if (record->main)
        main_record = record;
    else
        ++child_records;
    unlock();
    *out = {reinterpret_cast<void *>(record->payload), record->keys, record->dtv, record};
    return 0;
}

extern "C" void __cryolite_tcb_bind(void *cookie, void *pointer) {
    lock();
    auto record = records;
    while (record && record != cookie)
        record = record->next;
    auto address = reinterpret_cast<uintptr_t>(pointer);
    if (!record || record->tcb || address < record->payload
        || address - record->payload > record->front_bytes - sizeof(Tcb))
        fail("thread runtime: invalid TCB binding");
    record->tcb = static_cast<Tcb *>(pointer);
    if (record->main) {
        record->active = true;
        ++active_threads;
    }
    unlock();
}

extern "C" void __cryolite_tcb_discard(void *tcb) {
    lock();
    auto record = find(tcb);
    if (!record || record->main || record->control || record->claimed)
        fail("thread runtime: invalid unpublished TCB");
    record->claimed = true;
    unlock();
    if (retire(record))
        fail("thread runtime: unpublished cleanup failed");
}

extern "C" void __cryolite_thread_published(void *tcb) {
    lock();
    auto record = find(tcb);
    if (!record || !record->control || !record->creator_pin)
        fail("thread runtime: invalid publication owner");
    __atomic_store_n(&record->creator_pin, 0, __ATOMIC_RELEASE);
    wake_word(&record->creator_pin);
    unlock();
}

extern "C" int __cryolite_thread_join(void *tcb, void *result) {
    lock();
    auto record = find(tcb);
    if (!record || record->main || !record->control) {
        unlock();
        return ESRCH;
    }
    if (record->poisoned) {
        unlock();
        return EIO;
    }
    if (record->detached || record->claimed) {
        unlock();
        return EINVAL;
    }
    if (record->tcb == mlibc::get_current_tcb()) {
        unlock();
        return EDEADLK;
    }
    record->claimed = true;
    unlock();
    if (int e = observe_completion(record)) {
        unclaim(record);
        return e;
    }
    auto value = record->result;
    auto type = record->result_type;
    if (int e = retire(record)) {
        unclaim(record);
        return e;
    }
    if (result && type == TcbThreadReturnValue::Integer)
        *static_cast<int *>(result) = value.integer;
    else if (result)
        *static_cast<void **>(result) = value.voidPtr;
    return 0;
}

extern "C" int __cryolite_thread_detach(void *tcb) {
    lock();
    auto record = find(tcb);
    if (!record || record->main || !record->control) {
        unlock();
        return ESRCH;
    }
    if (record->poisoned) {
        unlock();
        return EIO;
    }
    if (record->detached || record->claimed) {
        unlock();
        return EINVAL;
    }
    record->claimed = true;
    unlock();
    if (int e = ensure_reaper()) {
        unclaim(record);
        return e;
    }
    lock();
    record->detached = true;
    if (!record->destroyed)
        __atomic_store_n(&record->tcb->isJoinable, 0, __ATOMIC_RELEASE);
    record->claimed = false;
    unlock();
    notify_reaper();
    return 0;
}

extern "C" size_t __cryolite_thread_records() {
    lock();
    auto count = child_records;
    unlock();
    return count;
}

extern "C" [[noreturn]] void __cryolite_thread_body(ThreadRecord *record) {
    auto tcb = record->tcb;
    while (!__atomic_load_n(&tcb->tid, __ATOMIC_ACQUIRE))
        wait_word(reinterpret_cast<uint32_t *>(&tcb->tid), 0);
    if (fesetenv(&record->environment))
        fail("thread runtime: floating-point environment restore failed");
    tcb->invokeThreadFunc(record->entry, record->argument);
    mlibc::thread_exit(tcb->returnValue);
}

extern "C" [[noreturn]] void __cryolite_reaper_body() {
    for (;;) {
        auto sequence = __atomic_load_n(&reaper_sequence, __ATOMIC_ACQUIRE);
        lock();
        ThreadRecord *candidate = nullptr;
        for (auto record = records; record; record = record->next) {
            if (record->detached && record->control && !record->claimed
                && !record->poisoned
                && (record->completed
                    || __atomic_load_n(&record->tcb->didExit, __ATOMIC_ACQUIRE) == 1)) {
                record->claimed = true;
                candidate = record;
                break;
            }
        }
        unlock();
        if (candidate) {
            if (int e = observe_completion(candidate)) {
                if (candidate->poisoned || e != EAGAIN)
                    fail("thread runtime: detached thread failed");
                unclaim(candidate);
                yield();
            } else if (retire(candidate)) {
                fail("thread runtime: detached cleanup failed");
            }
            continue;
        }
        wait_word(&reaper_sequence, sequence);
    }
}

namespace mlibc {

pid_t Sysdeps<FutexTid>::operator()() {
    if (tcb_available_flag) {
        auto id = __atomic_load_n(&get_current_tcb()->tid, __ATOMIC_RELAXED);
        if (id > 0)
            return id;
    }
    return 1;
}

int Sysdeps<PrepareStack>::operator()(void **stack, void *entry, void *argument,
                                      void *tcb, size_t *stack_size,
                                      size_t *guard_size, void **stack_base) {
    if (*stack || !entry || *stack_size < PTHREAD_STACK_MIN)
        return EINVAL;
    size_t usable;
    size_t guard;
    size_t total;
    if (!aligned(*stack_size, page_size, &usable)
        || !aligned(*guard_size, page_size, &guard)
        || !add(usable, guard, &total))
        return EINVAL;
    fenv_t environment;
    if (fegetenv(&environment))
        return EINVAL;
    lock();
    auto record = find(tcb);
    if (!record || record->main || record->claimed || record->control || record->prepared) {
        unlock();
        return EINVAL;
    }
    record->claimed = true;
    unlock();
    auto reserved = kryo_vm_reserve(total, page_size);
    if (reserved.status) {
        unclaim(record);
        return error(reserved.status);
    }
    auto committed = kryo_vm_commit(reserved.value + guard, usable);
    if (committed.status) {
        release_failed_reservation(reserved.value);
        unclaim(record);
        return error(committed.status);
    }
    lock();
    record->stack = reserved.value;
    record->stack_base = reserved.value + guard;
    record->stack_bytes = usable;
    record->guard_bytes = guard;
    record->entry = entry;
    record->argument = argument;
    record->environment = environment;
    record->prepared = true;
    record->claimed = false;
    unlock();
    *stack = reinterpret_cast<void *>(record->stack_base + usable);
    *stack_base = reinterpret_cast<void *>(record->stack_base);
    *stack_size = usable;
    *guard_size = guard;
    return 0;
}

int Sysdeps<Clone>::operator()(void *tcb, pid_t *pid_out, void *stack) {
    lock();
    auto record = find(tcb);
    if (!record || record->main || record->claimed || record->control || !record->prepared
        || reinterpret_cast<uintptr_t>(stack) != record->stack_base + record->stack_bytes) {
        unlock();
        return EINVAL;
    }
    record->claimed = true;
    auto detached = !__atomic_load_n(&record->tcb->isJoinable, __ATOMIC_RELAXED);
    unlock();
    if (detached) {
        if (int e = ensure_reaper()) {
            unclaim(record);
            return e;
        }
    }
    kryo_thread_memory memory = {
        record->stack_base, record->stack_bytes,
        record->payload, record->payload_bytes,
        reinterpret_cast<uintptr_t>(record->tcb), record->guard_bytes,
    };
    auto created = kryo_thread_create_native(__cryolite_thread_start,
                                            reinterpret_cast<uintptr_t>(record), memory);
    if (created.status) {
        unclaim(record);
        return error(created.status);
    }
    lock();
    record->control = created.value;
    record->detached = detached;
    record->active = true;
    record->claimed = false;
    ++active_threads;
    *pid_out = static_cast<pid_t>(record->id);
    unlock();
    return 0;
}

void Sysdeps<ThreadExit>::operator()() {
    auto tcb = get_current_tcb();
    lock();
    auto record = find(tcb);
    if (!record || !record->active || !active_threads)
        fail("thread runtime: active thread owner missing");
    record->active = false;
    --active_threads;
    auto last = !active_threads;
    auto detached = record->detached;
    unlock();
    if (last)
        ::exit(0);
    if (detached)
        notify_reaper();
    kryo_thread_exit(0);
}

} // namespace mlibc
