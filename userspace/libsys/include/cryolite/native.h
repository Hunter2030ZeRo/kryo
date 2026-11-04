#ifndef CRYOLITE_NATIVE_H
#define CRYOLITE_NATIVE_H

#if !defined(__x86_64__) || __SIZEOF_POINTER__ != 8 || __SIZEOF_LONG__ != 8
#error Cryolite native calls require the x86-64 LP64 profile.
#endif

typedef __UINT32_TYPE__ kryo_u32;
typedef __UINT64_TYPE__ kryo_u64;
typedef __INT64_TYPE__ kryo_status;
typedef __SIZE_TYPE__ kryo_size;
typedef kryo_u64 kryo_handle;

typedef struct kryo_result {
    kryo_status status;
    kryo_u64 value;
} kryo_result;

typedef struct kryo_message {
    kryo_u64 words[4];
} kryo_message;

typedef struct kryo_grant {
    kryo_handle handle;
    kryo_u64 rights;
} kryo_grant;

/* SDK aggregates describe register results, not in-memory Rust layouts. */
typedef struct kryo_ipc_result {
    kryo_status status;
    kryo_u64 value;
    kryo_message message;
    kryo_handle handles[2];
    kryo_u64 count;
} kryo_ipc_result;

typedef struct kryo_handle_info_result {
    kryo_status status;
    kryo_u64 kind;
    kryo_u64 rights;
} kryo_handle_info_result;

typedef struct kryo_thread_memory {
    kryo_u64 stack_base;
    kryo_u64 stack_bytes;
    kryo_u64 tls_base;
    kryo_u64 tls_bytes;
    kryo_u64 fs_base;
    kryo_u64 guard_bytes;
} kryo_thread_memory;

typedef struct kryo_thread_result {
    kryo_status status;
    kryo_u64 code;
    kryo_u64 reason;
} kryo_thread_result;

enum kryo_exit_reason {
    KRYO_EXIT_EXITED = 0,
    KRYO_EXIT_FAULT = 1,
    KRYO_EXIT_STOPPED = 2
};

enum kryo_status_code {
    KRYO_STATUS_OK = 0,
    KRYO_STATUS_INVALID_CALL = -1,
    KRYO_STATUS_INVALID_HANDLE = -2,
    KRYO_STATUS_PERMISSION_DENIED = -3,
    KRYO_STATUS_WRONG_TYPE = -4,
    KRYO_STATUS_OUT_OF_RESOURCES = -5,
    KRYO_STATUS_INVALID_ARGUMENT = -6,
    KRYO_STATUS_PEER_CLOSED = -7,
    KRYO_STATUS_CANCELLED = -8,
    KRYO_STATUS_INVALID_REPLY = -9,
    KRYO_STATUS_TIMED_OUT = -10,
    KRYO_STATUS_BUSY = -11
};

enum kryo_call {
    KRYO_CALL_YIELD = 0,
    KRYO_CALL_EXIT = 1,
    KRYO_CALL_CLOSE = 4,
    KRYO_CALL_DUPLICATE = 5,
    KRYO_CALL_SEND = 6,
    KRYO_CALL_RECEIVE = 7,
    KRYO_CALL_CALL = 8,
    KRYO_CALL_REPLY = 9,
    KRYO_CALL_ENDPOINT_CLOSE = 10,
    KRYO_CALL_WAIT = 11,
    KRYO_CALL_SIGNAL = 12,
    KRYO_CALL_MEMORY_CREATE = 14,
    KRYO_CALL_MEMORY_MAP = 15,
    KRYO_CALL_MEMORY_UNMAP = 16,
    KRYO_CALL_SEND_CAPS = 17,
    KRYO_CALL_CALL_CAPS = 18,
    KRYO_CALL_SET_TLS = 19,
    KRYO_CALL_CLOCK_READ = 20,
    KRYO_CALL_SLEEP_UNTIL = 21,
    KRYO_CALL_SEND_UNTIL = 22,
    KRYO_CALL_RECEIVE_UNTIL = 23,
    KRYO_CALL_CALL_UNTIL = 24,
    KRYO_CALL_WAIT_UNTIL = 25,
    KRYO_CALL_SEND_CAPS_UNTIL = 26,
    KRYO_CALL_CALL_CAPS_UNTIL = 27,
    KRYO_CALL_DEBUG_LOG = 30,
    KRYO_CALL_THREAD_EXIT = 33,
    KRYO_CALL_THREAD_JOIN = 34,
    KRYO_CALL_THREAD_JOIN_UNTIL = 35,
    KRYO_CALL_ENDPOINT_CREATE = 38,
    KRYO_CALL_NOTIFICATION_CREATE = 39,
    KRYO_CALL_STARTUP_READ = 43,
    KRYO_CALL_HANDLE_INFO = 44,
    KRYO_CALL_RECEIVE_FOR = 45,
    KRYO_CALL_VM_RESERVE = 54,
    KRYO_CALL_VM_COMMIT = 55,
    KRYO_CALL_VM_DECOMMIT = 56,
    KRYO_CALL_VM_RELEASE = 57,
    KRYO_CALL_PARK = 58,
    KRYO_CALL_PARK_UNTIL = 59,
    KRYO_CALL_WAKE = 60,
    KRYO_CALL_THREAD_CREATE_NATIVE = 61,
    KRYO_CALL_DEBUG_WRITE = 62,
    KRYO_CALL_CALL_EXCHANGE = 63,
    KRYO_CALL_CALL_EXCHANGE_UNTIL = 64,
    KRYO_CALL_REPLY_CAPS = 65
};

enum {
    KRYO_NATIVE_ABI_VERSION = 0,
    KRYO_NATIVE_THREAD_VERSION = 1,
    KRYO_RIGHT_INSPECT = 1,
    KRYO_RIGHT_CONTROL = 2,
    KRYO_RIGHT_DUP = 4,
    KRYO_RIGHT_SEND = 8,
    KRYO_RIGHT_RECEIVE = 16,
    KRYO_RIGHT_WAIT = 32,
    KRYO_RIGHT_SIGNAL = 64,
    KRYO_RIGHT_READ = 128,
    KRYO_RIGHT_WRITE = 256,
    KRYO_RIGHT_MAP = 512,
    KRYO_RIGHT_TRANSFER = 1024,
    KRYO_RIGHT_EXECUTE = 2048,
    KRYO_RIGHT_CREATE = 4096,
    KRYO_RIGHT_ASSIGN = 8192,
    KRYO_IPC_WORDS = 4,
    KRYO_IPC_CAPS = 2,
    KRYO_MEMORY_READ_ONLY = 0,
    KRYO_MEMORY_READ_WRITE = 1,
    KRYO_DEBUG_WRITE_MAX = 256,
    KRYO_PARK_WOKEN = 0,
    KRYO_PARK_CHANGED = 1,
    KRYO_STARTUP_VERSION = 1,
    KRYO_STARTUP_HEADER_WORDS = 7,
    KRYO_STARTUP_ENTRY_WORDS = 4,
    KRYO_STARTUP_MAX_GRANTS = 32,
    KRYO_OBJECT_TASK_CONTROL = 1,
    KRYO_OBJECT_MEMORY = 2,
    KRYO_OBJECT_ENDPOINT = 3,
    KRYO_OBJECT_NOTIFICATION = 4,
    KRYO_OBJECT_PROCESS_CONTROL = 5,
    KRYO_OBJECT_THREAD_CONTROL = 6,
    KRYO_OBJECT_IMAGE = 7,
    KRYO_OBJECT_DEBUG_CONSOLE = 8,
    KRYO_OBJECT_DEVICE = 9,
    KRYO_OBJECT_CHANNEL_FACTORY = 10,
    KRYO_OBJECT_LAUNCH_MANIFEST = 11,
    KRYO_OBJECT_STARTUP_RECORD = 12,
    KRYO_OBJECT_RESOURCE_BUDGET = 13
};

#define KRYO_STARTUP_MAGIC ((kryo_u64)0x545254534f59524bULL)

/* A successful value may have its high bit set. Inspect status, not value. */
static inline kryo_result kryo_call3(kryo_u64 call, kryo_u64 first,
                                     kryo_u64 second, kryo_u64 third) {
    __asm__ volatile("int $0x80"
                     : "+a"(call), "+D"(first), "+S"(second), "+d"(third)
                     :
                     : "r8", "r9", "r10", "r11", "cc", "memory");
    kryo_result result;
    result.status = (kryo_status)call;
    result.value = third;
    return result;
}

static inline kryo_result kryo_yield(void) {
    return kryo_call3(KRYO_CALL_YIELD, 0, 0, 0);
}

/* Finish runtime cleanup first; this terminates the whole process. */
__attribute__((noreturn)) static inline void kryo_exit(kryo_u64 code) {
    __asm__ volatile("int $0x80\n\tud2"
                     :
                     : "a"((kryo_u64)KRYO_CALL_EXIT), "D"(code)
                     : "cc", "memory");
    __builtin_unreachable();
}

static inline kryo_result kryo_close(kryo_handle handle) {
    return kryo_call3(KRYO_CALL_CLOSE, handle, 0, 0);
}

static inline kryo_result kryo_duplicate(kryo_handle handle, kryo_u64 rights) {
    return kryo_call3(KRYO_CALL_DUPLICATE, handle, rights, 0);
}

static inline kryo_handle_info_result kryo_handle_info(kryo_handle handle) {
    kryo_u64 call = KRYO_CALL_HANDLE_INFO;
    kryo_u64 kind = 0;
    register kryo_u64 rights __asm__("r8");
    __asm__ volatile("int $0x80"
                     : "+a"(call), "+D"(handle), "+d"(kind), "=r"(rights)
                     :
                     : "r9", "r10", "r11", "cc", "memory");
    kryo_handle_info_result result;
    result.status = (kryo_status)call;
    result.kind = kind;
    result.rights = rights;
    return result;
}

static inline kryo_result kryo_endpoint_create(kryo_handle factory) {
    return kryo_call3(KRYO_CALL_ENDPOINT_CREATE, factory, 0, 0);
}

static inline kryo_result kryo_notification_create(kryo_handle factory) {
    return kryo_call3(KRYO_CALL_NOTIFICATION_CREATE, factory, 0, 0);
}

static inline kryo_result kryo_endpoint_close(kryo_handle endpoint) {
    return kryo_call3(KRYO_CALL_ENDPOINT_CLOSE, endpoint, 0, 0);
}

static inline kryo_result kryo_signal(kryo_handle notification) {
    return kryo_call3(KRYO_CALL_SIGNAL, notification, 0, 0);
}

static inline kryo_result kryo_wait_notification(kryo_handle notification) {
    return kryo_call3(KRYO_CALL_WAIT, notification, 0, 0);
}

static inline kryo_result kryo_wait_notification_until(kryo_handle notification,
                                                       kryo_u64 deadline) {
    return kryo_call3(KRYO_CALL_WAIT_UNTIL, notification, 0, deadline);
}

static inline kryo_result kryo_memory_create(kryo_u64 pages) {
    return kryo_call3(KRYO_CALL_MEMORY_CREATE, pages, 0, 0);
}

/* Own a vacant page and preserve every concurrent mapping/reference lifetime.
 * The kernel validates the object page, address and RO/RW flag. */
static inline kryo_result kryo_memory_map(kryo_handle memory, kryo_u64 address,
                                          kryo_u64 page, kryo_u64 writable) {
    kryo_u64 call = KRYO_CALL_MEMORY_MAP;
    register kryo_u64 access __asm__("r8") = writable;
    __asm__ volatile("int $0x80"
                     : "+a"(call), "+D"(memory), "+S"(address), "+d"(page),
                       "+r"(access)
                     :
                     : "r9", "r10", "r11", "cc", "memory");
    kryo_result result;
    result.status = (kryo_status)call;
    result.value = page;
    return result;
}

/* End every access to this page before retiring the mapping. */
static inline kryo_result kryo_memory_unmap(kryo_u64 address) {
    return kryo_call3(KRYO_CALL_MEMORY_UNMAP, address, 0, 0);
}

static inline kryo_ipc_result kryo_ipc_invalid(void) {
    kryo_ipc_result result;
    result.status = KRYO_STATUS_INVALID_ARGUMENT;
    result.value = 0;
    result.message.words[0] = 0;
    result.message.words[1] = 0;
    result.message.words[2] = 0;
    result.message.words[3] = 0;
    result.handles[0] = 0;
    result.handles[1] = 0;
    result.count = 0;
    return result;
}

/* Inputs are copied into registers before entry; no user pointer crosses IPC.
 * Message data is meaningful after successful Call/Receive. Handles/count are
 * valid for Receive/CallExchange only. Failure value is delivery state; other
 * outputs never authorize handle use or backing release after an error. */
static inline kryo_ipc_result kryo_ipc_invoke_capacity(kryo_u64 call, kryo_handle endpoint,
                                              kryo_u64 argument, kryo_u64 deadline,
                                              kryo_message message,
                                              const kryo_grant *grants,
                                              kryo_size count,
                                              kryo_size reply_capacity) {
    if (count > KRYO_IPC_CAPS || reply_capacity > KRYO_IPC_CAPS
        || (count != 0 && grants == 0))
        return kryo_ipc_invalid();
    register kryo_u64 first __asm__("r8") = message.words[0];
    register kryo_u64 second __asm__("r9") = message.words[1];
    register kryo_u64 third __asm__("r10") = message.words[2];
    register kryo_u64 fourth __asm__("r11") = message.words[3];
    register kryo_u64 first_handle __asm__("r12") = count ? grants[0].handle : 0;
    register kryo_u64 second_handle __asm__("r13") = count > 1 ? grants[1].handle : 0;
    register kryo_u64 first_rights __asm__("r14") = count ? grants[0].rights : 0;
    register kryo_u64 second_rights __asm__("r15") = count > 1 ? grants[1].rights : 0;
    __asm__ volatile("int $0x80"
                     : "+a"(call), "+D"(endpoint), "+S"(argument), "+d"(deadline),
                       "+c"(reply_capacity),
                       "+r"(first), "+r"(second), "+r"(third), "+r"(fourth),
                       "+r"(first_handle), "+r"(second_handle),
                       "+r"(first_rights), "+r"(second_rights)
                     :
                     : "cc", "memory");
    kryo_ipc_result result;
    result.status = (kryo_status)call;
    result.value = deadline;
    result.message.words[0] = first;
    result.message.words[1] = second;
    result.message.words[2] = third;
    result.message.words[3] = fourth;
    result.handles[0] = first_handle;
    result.handles[1] = second_handle;
    result.count = first_rights;
    return result;
}

static inline kryo_ipc_result kryo_ipc_invoke(kryo_u64 call, kryo_handle endpoint,
                                              kryo_u64 argument, kryo_u64 deadline,
                                              kryo_message message,
                                              const kryo_grant *grants,
                                              kryo_size count) {
    return kryo_ipc_invoke_capacity(call, endpoint, argument, deadline,
                                   message, grants, count, 0);
}

/* An accepted failure can follow service-side effects; retry may repeat them. */
static inline int kryo_ipc_accepted(kryo_ipc_result result) {
    return result.status != KRYO_STATUS_OK && result.value != 0;
}

static inline kryo_ipc_result kryo_ipc_send(kryo_handle endpoint, kryo_message message) {
    return kryo_ipc_invoke(KRYO_CALL_SEND, endpoint, 0, 0, message, 0, 0);
}

static inline kryo_ipc_result kryo_ipc_send_until(kryo_handle endpoint,
                                                 kryo_message message,
                                                 kryo_u64 deadline) {
    return kryo_ipc_invoke(KRYO_CALL_SEND_UNTIL, endpoint, 0, deadline, message, 0, 0);
}

static inline kryo_ipc_result kryo_ipc_call(kryo_handle endpoint, kryo_message message) {
    return kryo_ipc_invoke(KRYO_CALL_CALL, endpoint, 0, 0, message, 0, 0);
}

static inline kryo_ipc_result kryo_ipc_call_until(kryo_handle endpoint,
                                                 kryo_message message,
                                                 kryo_u64 deadline) {
    return kryo_ipc_invoke(KRYO_CALL_CALL_UNTIL, endpoint, 0, deadline, message, 0, 0);
}

static inline kryo_ipc_result kryo_ipc_send_caps(kryo_handle endpoint,
                                                kryo_message message,
                                                const kryo_grant *grants,
                                                kryo_size count) {
    if (count == 0 || count > KRYO_IPC_CAPS || grants == 0)
        return kryo_ipc_invalid();
    return kryo_ipc_invoke(KRYO_CALL_SEND_CAPS, endpoint, count, 0, message, grants, count);
}

static inline kryo_ipc_result kryo_ipc_send_caps_until(kryo_handle endpoint,
                                                      kryo_message message,
                                                      const kryo_grant *grants,
                                                      kryo_size count,
                                                      kryo_u64 deadline) {
    if (count == 0 || count > KRYO_IPC_CAPS || grants == 0)
        return kryo_ipc_invalid();
    return kryo_ipc_invoke(KRYO_CALL_SEND_CAPS_UNTIL, endpoint, count, deadline, message, grants, count);
}

static inline kryo_ipc_result kryo_ipc_call_caps(kryo_handle endpoint,
                                                kryo_message message,
                                                const kryo_grant *grants,
                                                kryo_size count) {
    if (count == 0 || count > KRYO_IPC_CAPS || grants == 0)
        return kryo_ipc_invalid();
    return kryo_ipc_invoke(KRYO_CALL_CALL_CAPS, endpoint, count, 0, message, grants, count);
}

static inline kryo_ipc_result kryo_ipc_call_caps_until(kryo_handle endpoint,
                                                      kryo_message message,
                                                      const kryo_grant *grants,
                                                      kryo_size count,
                                                      kryo_u64 deadline) {
    if (count == 0 || count > KRYO_IPC_CAPS || grants == 0)
        return kryo_ipc_invalid();
    return kryo_ipc_invoke(KRYO_CALL_CALL_CAPS_UNTIL, endpoint, count, deadline, message, grants, count);
}

/* Reserve reply handles before request publication; zero request grants are valid. */
static inline kryo_ipc_result kryo_ipc_call_exchange(kryo_handle endpoint,
                                                      kryo_message message,
                                                      const kryo_grant *grants,
                                                      kryo_size count,
                                                      kryo_size reply_capacity) {
    return kryo_ipc_invoke_capacity(KRYO_CALL_CALL_EXCHANGE, endpoint, count, 0,
                                   message, grants, count, reply_capacity);
}

static inline kryo_ipc_result kryo_ipc_call_exchange_until(kryo_handle endpoint,
                                                            kryo_message message,
                                                            const kryo_grant *grants,
                                                            kryo_size count,
                                                            kryo_size reply_capacity,
                                                            kryo_u64 deadline) {
    return kryo_ipc_invoke_capacity(KRYO_CALL_CALL_EXCHANGE_UNTIL, endpoint, count,
                                   deadline, message, grants, count, reply_capacity);
}

/* Capacity is reserved before acceptance; zero requests inline data alone. */
static inline kryo_ipc_result kryo_ipc_receive(kryo_handle endpoint,
                                               kryo_size capacity) {
    kryo_message empty;
    empty.words[0] = 0;
    empty.words[1] = 0;
    empty.words[2] = 0;
    empty.words[3] = 0;
    if (capacity > KRYO_IPC_CAPS)
        return kryo_ipc_invalid();
    return kryo_ipc_invoke(KRYO_CALL_RECEIVE, endpoint, capacity, 0, empty, 0, 0);
}

static inline kryo_ipc_result kryo_ipc_receive_until(kryo_handle endpoint,
                                                     kryo_size capacity,
                                                     kryo_u64 deadline) {
    kryo_message empty;
    empty.words[0] = 0;
    empty.words[1] = 0;
    empty.words[2] = 0;
    empty.words[3] = 0;
    if (capacity > KRYO_IPC_CAPS)
        return kryo_ipc_invalid();
    return kryo_ipc_invoke(KRYO_CALL_RECEIVE_UNTIL, endpoint, capacity, deadline, empty, 0, 0);
}

/* Relative duration begins at kernel admission; *_until uses absolute time. */
static inline kryo_ipc_result kryo_ipc_receive_for(kryo_handle endpoint,
                                                   kryo_size capacity,
                                                   kryo_u64 duration) {
    kryo_message empty;
    empty.words[0] = 0;
    empty.words[1] = 0;
    empty.words[2] = 0;
    empty.words[3] = 0;
    if (capacity > KRYO_IPC_CAPS)
        return kryo_ipc_invalid();
    return kryo_ipc_invoke(KRYO_CALL_RECEIVE_FOR, endpoint, capacity, duration, empty, 0, 0);
}

/* Reply tokens are endpoint-local, caller-bound and single-use. */
static inline kryo_result kryo_ipc_reply(kryo_handle endpoint, kryo_u64 token,
                                         kryo_message message) {
    kryo_ipc_result reply = kryo_ipc_invoke(KRYO_CALL_REPLY, endpoint, token, 0, message, 0, 0);
    kryo_result result;
    result.status = reply.status;
    result.value = reply.value;
    return result;
}

static inline kryo_result kryo_ipc_reply_caps(kryo_handle endpoint, kryo_u64 token,
                                              kryo_message message,
                                              const kryo_grant *grants,
                                              kryo_size count) {
    if (count == 0 || count > KRYO_IPC_CAPS || grants == 0) {
        kryo_result invalid;
        invalid.status = KRYO_STATUS_INVALID_ARGUMENT;
        invalid.value = 0;
        return invalid;
    }
    kryo_ipc_result reply = kryo_ipc_invoke(KRYO_CALL_REPLY_CAPS, endpoint, token,
                                          count, message, grants, count);
    kryo_result result;
    result.status = reply.status;
    result.value = reply.value;
    return result;
}

/* Retain valid runtime TLS backing and its TCB contract across this call. */
static inline kryo_result kryo_set_tls(kryo_u64 base) {
    return kryo_call3(KRYO_CALL_SET_TLS, base, 0, 0);
}

static inline kryo_result kryo_clock(void) {
    return kryo_call3(KRYO_CALL_CLOCK_READ, 0, 0, 0);
}

static inline kryo_result kryo_sleep_until(kryo_u64 deadline) {
    return kryo_call3(KRYO_CALL_SLEEP_UNTIL, deadline, 0, 0);
}

static inline kryo_result kryo_debug_log(kryo_handle debug, kryo_u64 event,
                                        kryo_u64 value) {
    kryo_u64 call = KRYO_CALL_DEBUG_LOG;
    kryo_u64 result_value = 0;
    register kryo_u64 event_value __asm__("r8") = value;
    __asm__ volatile("int $0x80"
                     : "+a"(call), "+D"(debug), "+S"(event),
                       "+d"(result_value), "+r"(event_value)
                     :
                     : "r9", "r10", "r11", "cc", "memory");
    kryo_result result;
    result.status = (kryo_status)call;
    result.value = result_value;
    return result;
}

static inline kryo_result kryo_startup_read(kryo_handle record, kryo_u64 index) {
    return kryo_call3(KRYO_CALL_STARTUP_READ, record, index, 0);
}

static inline kryo_result kryo_vm_reserve(kryo_u64 bytes, kryo_u64 alignment) {
    return kryo_call3(KRYO_CALL_VM_RESERVE, bytes, alignment, 0);
}

static inline kryo_result kryo_vm_commit(kryo_u64 address, kryo_u64 bytes) {
    return kryo_call3(KRYO_CALL_VM_COMMIT, address, bytes, 0);
}

/* End every access before retiring backing; failure retains its ownership. */
static inline kryo_result kryo_vm_decommit(kryo_u64 address, kryo_u64 bytes) {
    return kryo_call3(KRYO_CALL_VM_DECOMMIT, address, bytes, 0);
}

static inline kryo_result kryo_vm_release(kryo_u64 base) {
    return kryo_call3(KRYO_CALL_VM_RELEASE, base, 0, 0);
}

/* Recheck an Acquire predicate after park; Release-store it before wake. */
static inline kryo_result kryo_park(kryo_u64 address, kryo_u32 expected) {
    return kryo_call3(KRYO_CALL_PARK, address, expected, 0);
}

static inline kryo_result kryo_park_until(kryo_u64 address, kryo_u32 expected,
                                         kryo_u64 deadline) {
    return kryo_call3(KRYO_CALL_PARK_UNTIL, address, expected, deadline);
}

static inline kryo_result kryo_wake(kryo_u64 address, kryo_u64 maximum) {
    return kryo_call3(KRYO_CALL_WAKE, address, maximum, 0);
}

/* Raw bootstrap: RSP has no return address. CALL C through an assembly entry.
 * Prepare TCB/TLS before creation and retain code, arguments and mappings until
 * successful native completion. The kernel borrows, but never frees, backing. */
static inline kryo_result kryo_thread_create_native(void (*entry)(void),
                                                   kryo_u64 argument,
                                                   kryo_thread_memory memory) {
    kryo_u64 call = KRYO_CALL_THREAD_CREATE_NATIVE;
    kryo_u64 version = KRYO_NATIVE_THREAD_VERSION;
    kryo_u64 entry_address = (kryo_u64)(__UINTPTR_TYPE__)entry;
    register kryo_u64 stack_base __asm__("r8") = memory.stack_base;
    register kryo_u64 stack_bytes __asm__("r9") = memory.stack_bytes;
    register kryo_u64 tls_base __asm__("r10") = memory.tls_base;
    register kryo_u64 tls_bytes __asm__("r11") = memory.tls_bytes;
    register kryo_u64 fs_base __asm__("r12") = memory.fs_base;
    register kryo_u64 guard_bytes __asm__("r13") = memory.guard_bytes;
    register kryo_u64 flags __asm__("r14") = 0;
    __asm__ volatile("int $0x80"
                     : "+a"(call), "+D"(version), "+S"(entry_address),
                       "+d"(argument), "+r"(stack_base), "+r"(stack_bytes),
                       "+r"(tls_base), "+r"(tls_bytes), "+r"(fs_base),
                       "+r"(guard_bytes), "+r"(flags)
                     :
                     : "cc", "memory");
    kryo_result result;
    result.status = (kryo_status)call;
    result.value = argument;
    return result;
}

/* Finish language cleanup first; this exits only the current thread. */
__attribute__((noreturn)) static inline void kryo_thread_exit(kryo_u64 code) {
    __asm__ volatile("int $0x80\n\tud2"
                     :
                     : "a"((kryo_u64)KRYO_CALL_THREAD_EXIT), "D"(code)
                     : "cc", "memory");
    __builtin_unreachable();
}

static inline kryo_thread_result kryo_thread_wait(kryo_u64 call,
                                                  kryo_handle thread,
                                                  kryo_u64 deadline) {
    register kryo_u64 reason __asm__("r8");
    __asm__ volatile("int $0x80"
                     : "+a"(call), "+D"(thread), "+d"(deadline), "=r"(reason)
                     :
                     : "r9", "r10", "r11", "cc", "memory");
    kryo_thread_result result;
    result.status = (kryo_status)call;
    result.code = deadline;
    result.reason = reason;
    return result;
}

static inline kryo_thread_result kryo_thread_join(kryo_handle thread) {
    return kryo_thread_wait(KRYO_CALL_THREAD_JOIN, thread, 0);
}

static inline kryo_thread_result kryo_thread_join_until(kryo_handle thread,
                                                        kryo_u64 deadline) {
    return kryo_thread_wait(KRYO_CALL_THREAD_JOIN_UNTIL, thread, deadline);
}

/* Code and reason are completion data only after a successful known response. */
static inline int kryo_thread_result_valid(kryo_thread_result result) {
    return result.status == KRYO_STATUS_OK && result.reason <= KRYO_EXIT_STOPPED;
}

/* Caller storage must stay readable through the snapshot; output is escaped. */
static inline kryo_result kryo_debug_write(kryo_handle debug, const void *bytes,
                                          kryo_size count) {
    return kryo_call3(KRYO_CALL_DEBUG_WRITE, debug,
                      (kryo_u64)(__UINTPTR_TYPE__)bytes, (kryo_u64)count);
}

#endif
