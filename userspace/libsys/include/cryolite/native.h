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
    KRYO_CALL_SET_TLS = 19,
    KRYO_CALL_CLOCK_READ = 20,
    KRYO_CALL_SLEEP_UNTIL = 21,
    KRYO_CALL_DEBUG_LOG = 30,
    KRYO_CALL_STARTUP_READ = 43,
    KRYO_CALL_VM_RESERVE = 54,
    KRYO_CALL_VM_COMMIT = 55,
    KRYO_CALL_VM_DECOMMIT = 56,
    KRYO_CALL_VM_RELEASE = 57,
    KRYO_CALL_PARK = 58,
    KRYO_CALL_PARK_UNTIL = 59,
    KRYO_CALL_WAKE = 60,
    KRYO_CALL_DEBUG_WRITE = 62
};

enum {
    KRYO_NATIVE_ABI_VERSION = 0,
    KRYO_RIGHT_READ = 128,
    KRYO_RIGHT_WRITE = 256,
    KRYO_DEBUG_WRITE_MAX = 256,
    KRYO_PARK_WOKEN = 0,
    KRYO_PARK_CHANGED = 1,
    KRYO_STARTUP_VERSION = 1,
    KRYO_STARTUP_HEADER_WORDS = 7,
    KRYO_STARTUP_ENTRY_WORDS = 4,
    KRYO_STARTUP_MAX_GRANTS = 32,
    KRYO_OBJECT_DEBUG_CONSOLE = 8
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

/* Caller storage must stay readable through the snapshot; output is escaped. */
static inline kryo_result kryo_debug_write(kryo_handle debug, const void *bytes,
                                          kryo_size count) {
    return kryo_call3(KRYO_CALL_DEBUG_WRITE, debug,
                      (kryo_u64)(__UINTPTR_TYPE__)bytes, (kryo_u64)count);
}

#endif
