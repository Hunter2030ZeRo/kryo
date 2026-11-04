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

static kryo_u64 ipc_deadline(void) {
    kryo_result now = kryo_clock();
    if (now.status != KRYO_STATUS_OK || now.value > ~(kryo_u64)0 - 1000000000ULL)
        return 0;
    return now.value + 1000000000ULL;
}

kryo_u64 c_ipc_checks(kryo_handle bootstrap, kryo_handle factory,
                     kryo_handle *server_out) {
    *server_out = 0;
    kryo_result endpoint = kryo_endpoint_create(factory);
    kryo_result notification = kryo_notification_create(factory);
    if (endpoint.status != KRYO_STATUS_OK || notification.status != KRYO_STATUS_OK)
        return 1;
    kryo_result sender = kryo_duplicate(endpoint.value, KRYO_RIGHT_SEND);
    kryo_result waiter = kryo_duplicate(notification.value, KRYO_RIGHT_WAIT);
    if (sender.status != KRYO_STATUS_OK || waiter.status != KRYO_STATUS_OK
        || kryo_duplicate(endpoint.value, KRYO_RIGHT_CREATE).status != KRYO_STATUS_PERMISSION_DENIED
        || kryo_signal(waiter.value).status != KRYO_STATUS_PERMISSION_DENIED)
        return 2;
    const kryo_u64 high = 0xfedcba9876543210ULL;
    if (!completed(kryo_signal(notification.value), 0)
        || !completed(kryo_wait_notification(waiter.value), 0)
        || kryo_wait_notification_until(waiter.value, 0).status != KRYO_STATUS_TIMED_OUT)
        return 3;
    kryo_message empty = {{0, 0, 0, 0}};
    if (!completed(kryo_endpoint_close(endpoint.value), 0))
        return 4;
    kryo_ipc_result closed = kryo_ipc_send(sender.value, empty);
    if (closed.status != KRYO_STATUS_PEER_CLOSED || kryo_ipc_accepted(closed)
        || !completed(kryo_close(sender.value), 0)
        || !completed(kryo_close(endpoint.value), 0)
        || !completed(kryo_close(waiter.value), 0)
        || !completed(kryo_close(notification.value), 0))
        return 5;

    kryo_ipc_result hello = kryo_ipc_receive_until(bootstrap, 2, ipc_deadline());
    if (hello.status != KRYO_STATUS_OK || hello.count != 1 || hello.value != 0
        || hello.message.words[0] != 1 || hello.message.words[1] != 0
        || hello.message.words[2] != 0 || hello.message.words[3] != 0)
        return 6;
    kryo_handle server = hello.handles[0];
    kryo_handle_info_result info = kryo_handle_info(server);
    if (info.status != KRYO_STATUS_OK || info.kind != KRYO_OBJECT_ENDPOINT
        || info.rights != KRYO_RIGHT_SEND
        || kryo_ipc_receive_until(server, 0, ipc_deadline()).status != KRYO_STATUS_PERMISSION_DENIED
        || kryo_endpoint_close(server).status != KRYO_STATUS_PERMISSION_DENIED)
        return 7;

    kryo_result memory = kryo_memory_create(1);
    if (memory.status != KRYO_STATUS_OK)
        return 8;
    const kryo_u64 address = 0x2000000;
    if (!completed(kryo_memory_map(memory.value, address, 0, KRYO_MEMORY_READ_WRITE), 0))
        return 9;
    volatile kryo_u64 *words = (volatile kryo_u64 *)address;
    for (kryo_size index = 0; index < 512; ++index)
        words[index] = high ^ index;
    kryo_result restricted = kryo_duplicate(memory.value, KRYO_RIGHT_READ | KRYO_RIGHT_MAP);
    if (restricted.status != KRYO_STATUS_OK)
        return 10;
    kryo_message request = {{2, 1, 41, 0}};
    kryo_grant grant = {restricted.value, KRYO_RIGHT_READ | KRYO_RIGHT_MAP};
    kryo_ipc_result denied = kryo_ipc_call_caps_until(server, request, &grant, 1, ipc_deadline());
    if (denied.status != KRYO_STATUS_PERMISSION_DENIED || kryo_ipc_accepted(denied))
        return 11;
    grant.handle = memory.value;
    kryo_ipc_result response = kryo_ipc_call_caps_until(server, request, &grant, 1, ipc_deadline());
    if (response.status != KRYO_STATUS_OK
        || response.message.words[0] != 3 || response.message.words[1] != 1
        || response.message.words[2] != 42 || response.message.words[3] != 0)
        return 12;
    // The peer acknowledges only after unmapping and closing its read-only loan.
    if (!completed(kryo_memory_unmap(address), 0)
        || !completed(kryo_close(restricted.value), 0)
        || !completed(kryo_close(memory.value), 0))
        return 13;
    kryo_message stop = {{4, 1, 0, 0}};
    if (kryo_ipc_send_until(server, stop, ipc_deadline()).status != KRYO_STATUS_OK
        || !completed(kryo_close(bootstrap), 0)
        || !completed(kryo_close(factory), 0))
        return 14;
    *server_out = server;
    return 0;
}

kryo_u64 c_ipc_peer_closed(kryo_handle server) {
    kryo_message request = {{2, 1, 41, 0}};
    kryo_ipc_result result = kryo_ipc_call_until(server, request, ipc_deadline());
    if (result.status != KRYO_STATUS_PEER_CLOSED || kryo_ipc_accepted(result)
        || !completed(kryo_close(server), 0))
        return 1;
    return 0;
}
