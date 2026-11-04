#include <cryolite/native.h>

#ifdef __cplusplus
#define CHECK(condition) static_assert(condition, #condition)
#define ALIGN(type) alignof(type)
CHECK(__is_same(decltype(((kryo_thread_result *)0)->status), kryo_status));
CHECK(__is_same(decltype(((kryo_thread_result *)0)->code), kryo_u64));
CHECK(__is_same(decltype(((kryo_thread_result *)0)->reason), kryo_u64));
#else
#define CHECK(condition) _Static_assert(condition, #condition)
#define ALIGN(type) _Alignof(type)
CHECK(__builtin_types_compatible_p(__typeof__(((kryo_thread_result *)0)->status), kryo_status));
CHECK(__builtin_types_compatible_p(__typeof__(((kryo_thread_result *)0)->code), kryo_u64));
CHECK(__builtin_types_compatible_p(__typeof__(((kryo_thread_result *)0)->reason), kryo_u64));
#endif

CHECK(sizeof(kryo_thread_memory) == 48 && ALIGN(kryo_thread_memory) == 8);
CHECK(__builtin_offsetof(kryo_thread_memory, stack_base) == 0);
CHECK(__builtin_offsetof(kryo_thread_memory, stack_bytes) == 8);
CHECK(__builtin_offsetof(kryo_thread_memory, tls_base) == 16);
CHECK(__builtin_offsetof(kryo_thread_memory, tls_bytes) == 24);
CHECK(__builtin_offsetof(kryo_thread_memory, fs_base) == 32);
CHECK(__builtin_offsetof(kryo_thread_memory, guard_bytes) == 40);
CHECK(sizeof(kryo_thread_result) == 24 && ALIGN(kryo_thread_result) == 8);
CHECK(__builtin_offsetof(kryo_thread_result, status) == 0);
CHECK(__builtin_offsetof(kryo_thread_result, code) == 8);
CHECK(__builtin_offsetof(kryo_thread_result, reason) == 16);

extern void raw_thread_entry(void);

kryo_result create_thread(kryo_thread_memory memory, kryo_u64 argument) {
    return kryo_thread_create_native(raw_thread_entry, argument, memory);
}

kryo_thread_result join_thread(kryo_handle thread) {
    return kryo_thread_join(thread);
}

kryo_thread_result join_thread_until(kryo_handle thread, kryo_u64 deadline) {
    return kryo_thread_join_until(thread, deadline);
}

int valid_completion(kryo_thread_result result) {
    return kryo_thread_result_valid(result);
}

__attribute__((noreturn)) void exit_thread(kryo_u64 code) {
    kryo_thread_exit(code);
}

/* Keep the native-call registers live to expose missing compiler constraints. */
kryo_u64 preserve_registers(kryo_thread_memory memory, kryo_u64 argument) {
    register kryo_u64 first __asm__("r12") = 0x123456789abcdef0ULL;
    register kryo_u64 second __asm__("r13") = 0xfedcba9876543210ULL;
    register kryo_u64 third __asm__("r14") = 0x5555555555555555ULL;
    __asm__ volatile("" : "+r"(first), "+r"(second), "+r"(third));
    kryo_result result = kryo_thread_create_native(raw_thread_entry, argument, memory);
    __asm__ volatile("" : "+r"(first), "+r"(second), "+r"(third));
    return first ^ second ^ third ^ result.value ^ (kryo_u64)result.status;
}
