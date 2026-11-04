#include <cryolite/native.h>

#ifdef __cplusplus
#define CHECK(condition) static_assert(condition, #condition)
#define ALIGN(type) alignof(type)
CHECK(__is_same(decltype(((kryo_ipc_result *)0)->status), kryo_status));
CHECK(__is_same(decltype(((kryo_ipc_result *)0)->value), kryo_u64));
CHECK(__is_same(decltype(((kryo_grant *)0)->handle), kryo_handle));
#else
#define CHECK(condition) _Static_assert(condition, #condition)
#define ALIGN(type) _Alignof(type)
CHECK(__builtin_types_compatible_p(__typeof__(((kryo_ipc_result *)0)->status), kryo_status));
CHECK(__builtin_types_compatible_p(__typeof__(((kryo_ipc_result *)0)->value), kryo_u64));
CHECK(__builtin_types_compatible_p(__typeof__(((kryo_grant *)0)->handle), kryo_handle));
#endif

CHECK(sizeof(kryo_message) == 32 && ALIGN(kryo_message) == 8);
CHECK(sizeof(kryo_grant) == 16 && ALIGN(kryo_grant) == 8);
CHECK(__builtin_offsetof(kryo_grant, handle) == 0);
CHECK(__builtin_offsetof(kryo_grant, rights) == 8);
CHECK(sizeof(kryo_ipc_result) == 72 && ALIGN(kryo_ipc_result) == 8);
CHECK(__builtin_offsetof(kryo_ipc_result, status) == 0);
CHECK(__builtin_offsetof(kryo_ipc_result, value) == 8);
CHECK(__builtin_offsetof(kryo_ipc_result, message) == 16);
CHECK(__builtin_offsetof(kryo_ipc_result, handles) == 48);
CHECK(__builtin_offsetof(kryo_ipc_result, count) == 64);
CHECK(sizeof(kryo_handle_info_result) == 24 && ALIGN(kryo_handle_info_result) == 8);
CHECK(__builtin_offsetof(kryo_handle_info_result, status) == 0);
CHECK(__builtin_offsetof(kryo_handle_info_result, kind) == 8);
CHECK(__builtin_offsetof(kryo_handle_info_result, rights) == 16);

__attribute__((noinline)) kryo_ipc_result exercise_ipc(
        kryo_handle endpoint, kryo_message message, const kryo_grant *grants,
        kryo_size count, kryo_u64 deadline, unsigned operation) {
    switch (operation) {
    case 0: return kryo_ipc_send(endpoint, message);
    case 1: return kryo_ipc_send_until(endpoint, message, deadline);
    case 2: return kryo_ipc_call(endpoint, message);
    case 3: return kryo_ipc_call_until(endpoint, message, deadline);
    case 4: return kryo_ipc_send_caps(endpoint, message, grants, count);
    case 5: return kryo_ipc_send_caps_until(endpoint, message, grants, count, deadline);
    case 6: return kryo_ipc_call_caps(endpoint, message, grants, count);
    case 7: return kryo_ipc_call_caps_until(endpoint, message, grants, count, deadline);
    case 8: return kryo_ipc_receive(endpoint, count);
    case 9: return kryo_ipc_receive_until(endpoint, count, deadline);
    case 11: return kryo_ipc_call_exchange(endpoint, message, grants, count, 2);
    case 12: return kryo_ipc_call_exchange_until(endpoint, message, grants, count, 2, deadline);
    default: return kryo_ipc_receive_for(endpoint, count, deadline);
    }
}

kryo_result exercise_reply_caps(kryo_handle endpoint, kryo_u64 token,
                               kryo_message message, const kryo_grant *grants,
                               kryo_size count) {
    return kryo_ipc_reply_caps(endpoint, token, message, grants, count);
}

__attribute__((noinline)) kryo_result exercise_scalar(
        kryo_handle handle, kryo_message message, kryo_u64 first,
        kryo_u64 second, unsigned operation) {
    switch (operation) {
    case 0: return kryo_duplicate(handle, first);
    case 1: return kryo_endpoint_create(handle);
    case 2: return kryo_notification_create(handle);
    case 3: return kryo_endpoint_close(handle);
    case 4: return kryo_signal(handle);
    case 5: return kryo_wait_notification(handle);
    case 6: return kryo_wait_notification_until(handle, first);
    case 7: return kryo_memory_create(first);
    case 8: return kryo_memory_map(handle, first, second, KRYO_MEMORY_READ_WRITE);
    case 9: return kryo_memory_unmap(first);
    default: return kryo_ipc_reply(handle, first, message);
    }
}

kryo_handle_info_result describe_handle(kryo_handle handle) {
    return kryo_handle_info(handle);
}

/* Aggregate return storage and live callee-saved values exercise register pressure. */
kryo_u64 preserve_ipc_registers(kryo_handle endpoint, kryo_message message,
                               const kryo_grant *grants, kryo_size count) {
    register kryo_u64 first __asm__("r12") = 0x123456789abcdef0ULL;
    register kryo_u64 second __asm__("r13") = 0xfedcba9876543210ULL;
    register kryo_u64 third __asm__("r14") = 0x5555555555555555ULL;
    register kryo_u64 fourth __asm__("r15") = 0xaaaaaaaaaaaaaaaaULL;
    __asm__ volatile("" : "+r"(first), "+r"(second), "+r"(third), "+r"(fourth));
    kryo_ipc_result result = kryo_ipc_call_exchange(endpoint, message, grants, count, 2);
    __asm__ volatile("" : "+r"(first), "+r"(second), "+r"(third), "+r"(fourth));
    return first ^ second ^ third ^ fourth ^ (kryo_u64)result.status ^ result.value
        ^ result.message.words[0] ^ result.message.words[1] ^ result.message.words[2]
        ^ result.message.words[3] ^ result.handles[0] ^ result.handles[1] ^ result.count;
}

#ifdef KRYO_HOST_CHECK
static int invalid(kryo_ipc_result result) {
    return result.status == KRYO_STATUS_INVALID_ARGUMENT && result.value == 0
        && result.message.words[0] == 0 && result.message.words[1] == 0
        && result.message.words[2] == 0 && result.message.words[3] == 0
        && result.handles[0] == 0 && result.handles[1] == 0 && result.count == 0;
}

int main(void) {
    const kryo_message message = {{0x8000000000000000ULL, 2, 3, 4}};
    const kryo_grant grant = {0xfedcba9876543210ULL, KRYO_RIGHT_READ | KRYO_RIGHT_MAP};
    if (!invalid(kryo_ipc_send_caps(0, message, &grant, 0))
        || !invalid(kryo_ipc_send_caps_until(0, message, &grant, 3, 0))
        || !invalid(kryo_ipc_call_caps(0, message, 0, 1))
        || !invalid(kryo_ipc_call_caps_until(0, message, 0, 2, 0))
        || !invalid(kryo_ipc_receive(0, 3))
        || !invalid(kryo_ipc_receive_until(0, (kryo_size)-1, 0))
        || !invalid(kryo_ipc_receive_for(0, 3, 0))
        || !invalid(kryo_ipc_call_exchange(0, message, &grant, 1, 3))
        || !invalid(kryo_ipc_call_exchange_until(0, message, 0, 1, 2, 0))
        || !invalid(kryo_ipc_invoke(KRYO_CALL_SEND_CAPS, 0, 3, 0, message, &grant, 3)))
        return 1;
    kryo_result reply = kryo_ipc_reply_caps(0, 0, message, &grant, 0);
    if (reply.status != KRYO_STATUS_INVALID_ARGUMENT || reply.value != 0)
        return 5;
    kryo_ipc_result result = kryo_ipc_invalid();
    result.status = KRYO_STATUS_OK;
    result.value = 0xfedcba9876543210ULL;
    if (kryo_ipc_accepted(result))
        return 2;
    result.status = KRYO_STATUS_TIMED_OUT;
    result.value = 0;
    if (kryo_ipc_accepted(result))
        return 3;
    result.value = 1;
    return kryo_ipc_accepted(result) ? 0 : 4;
}
#endif
