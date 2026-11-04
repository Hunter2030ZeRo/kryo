#include <cryolite/native.h>

static int completed(kryo_result result, kryo_u64 value) {
    return result.status == KRYO_STATUS_OK && result.value == value;
}

kryo_u64 c_native_checks(kryo_handle debug, kryo_handle startup,
                         kryo_handle no_write) {
    const kryo_u64 high = 0xfedcba9876543210ULL;
    if (!completed(kryo_startup_read(startup, 0), KRYO_STARTUP_MAGIC)
        || !completed(kryo_startup_read(startup, 1), KRYO_STARTUP_VERSION)
        || !completed(kryo_startup_read(startup, 5), high))
        return 1;
    if (kryo_startup_read(startup, ~(kryo_u64)0).status != KRYO_STATUS_INVALID_ARGUMENT
        || kryo_call3(~(kryo_u64)0, 0, 0, 0).status != KRYO_STATUS_INVALID_CALL
        || kryo_close(0).status != KRYO_STATUS_INVALID_HANDLE)
        return 2;
    if (kryo_debug_write(0, 0, 0).status != KRYO_STATUS_INVALID_HANDLE
        || kryo_debug_write(startup, 0, 0).status != KRYO_STATUS_WRONG_TYPE
        || kryo_debug_write(no_write, 0, 0).status != KRYO_STATUS_PERMISSION_DENIED)
        return 3;
    if (!completed(kryo_debug_write(debug, 0, 0), 0)
        || kryo_debug_write(debug, 0, 1).status != KRYO_STATUS_INVALID_ARGUMENT
        || kryo_debug_write(debug, 0, KRYO_DEBUG_WRITE_MAX + 1).status != KRYO_STATUS_INVALID_ARGUMENT
        || kryo_debug_write(debug, (const void *)0xffff800000000000ULL, 1).status != KRYO_STATUS_INVALID_ARGUMENT
        || kryo_debug_write(debug, (const void *)~(kryo_u64)0, 2).status != KRYO_STATUS_INVALID_ARGUMENT)
        return 4;
    kryo_result clock = kryo_clock();
    if (clock.status != KRYO_STATUS_OK || clock.value == 0
        || !completed(kryo_sleep_until(0), 0) || !completed(kryo_yield(), 0))
        return 5;

    kryo_u32 word = 7;
    if (!completed(kryo_park((kryo_u64)&word, 8), KRYO_PARK_CHANGED)
        || kryo_park_until((kryo_u64)&word, 7, 0).status != KRYO_STATUS_TIMED_OUT
        || !completed(kryo_wake((kryo_u64)&word, 0), 0)
        || kryo_park((kryo_u64)&word + 1, 7).status != KRYO_STATUS_INVALID_ARGUMENT)
        return 6;

    static const char greeting[] = "C native calls ready";
    static const char injection[] = "\nPANIC\r\033\\\0\177\200ABC\nABI: startup and execution resources retired";
    if (!completed(kryo_debug_write(debug, greeting, sizeof(greeting) - 1), sizeof(greeting) - 1)
        || !completed(kryo_debug_write(debug, injection, sizeof(injection) - 1), sizeof(injection) - 1))
        return 7;

    kryo_result reservation = kryo_vm_reserve(8192, 4096);
    if (reservation.status != KRYO_STATUS_OK)
        return 8;
    kryo_u64 base = reservation.value;
    if (!completed(kryo_vm_commit(base, 8192), 0))
        return 9;
    volatile unsigned char *bytes = (volatile unsigned char *)base;
    for (kryo_size index = 0; index < 8192; ++index) {
        if (bytes[index] != 0)
            return 10;
        bytes[index] = 'D';
    }
    if (!completed(kryo_debug_write(debug, (const void *)(base + 4095), KRYO_DEBUG_WRITE_MAX),
                   KRYO_DEBUG_WRITE_MAX))
        return 11;
    if (!completed(kryo_vm_decommit(base + 4096, 4096), 0)
        || kryo_debug_write(debug, (const void *)(base + 4095), 2).status != KRYO_STATUS_INVALID_ARGUMENT)
        return 12;
    if (!completed(kryo_vm_commit(base + 4096, 4096), 0))
        return 13;
    for (kryo_size index = 4096; index < 8192; ++index)
        if (bytes[index] != 0)
            return 14;
    if (!completed(kryo_vm_release(base), 0)
        || kryo_debug_write(debug, (const void *)base, 1).status != KRYO_STATUS_INVALID_ARGUMENT)
        return 15;
    if (!completed(kryo_debug_log(debug, 98, 1), 0))
        return 16;
    return 0;
}
