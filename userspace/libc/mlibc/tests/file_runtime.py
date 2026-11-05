"""Run verbatim provider file-backend bodies with host-only native-call doubles."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[4]
SOURCE = ROOT / "userspace/libc/mlibc/sysdeps/cryolite/files.cpp"
CASES = [
    "namespace absent", "descriptor allocation before Open", "loan creation failure",
    "loan map failure", "server Open failure", "bounded path and flag validation",
    "Open capability kind and rights", "malformed response identity and reserved words",
    "unexpected capability cleanup", "chunked transfer and access mode",
    "zero-byte operations and invalid descriptors", "server-owned seek",
    "oversized read response poisons description", "zero-progress write rejected",
    "transport failure does not close request-grant aliases", "close pins and numerical reuse",
    "RPC close and reopen preserve old operation ownership", "request id exhaustion",
    "mapping slot reuse", "unknown service status rejected",
    "close at descriptor publication does not read freed storage",
    "uncontended locks make no native waits or wakes",
    "release before park cannot lose wake", "release before contention mark avoids park",
    "wait quota failure yields and rechecks",
    "initial descriptor-table allocation failure precedes Open",
    "failed table growth preserves existing descriptors", "table growth preserves indexed lookup and hole reuse",
    "accepted write failure blocks automatic retry", "undelivered write failure remains retryable",
    "aliases retain one gate through final descriptor close",
    "descriptor flags are local and status flags exclude creation bits",
    "F_DUPFD chooses the lowest available descriptor including standard slots",
    "dup2 replaces a target atomically and clears descriptor flags",
    "dup2 self duplication preserves flags and validates inputs",
    "F_SETFL ignores immutable bits and rejects real status changes",
    "descriptor control rejects invalid and unsupported commands",
    "duplicate table growth failure preserves live descriptors",
    "duplicate table growth and hole reuse preserve shared descriptions",
    "dup2 cannot steal an unpublished Open reservation",
    "dup2 replacement retains an in-flight target operation",
    "closing aliases during RPC preserves the old description and window",
    "accepted failure poisons every alias without replay",
    "aliases share gate identity and request sequence",
    "alias operations serialize access to their scratch mapping",
    "duplicate publication never dereferences a concurrently closed description",
    "metadata uses inline object attributes without a loan",
    "unsupported metadata preserves output and healthy I/O",
    "invalid metadata kinds flags and signed sizes poison all aliases",
    "metadata error responses cannot carry values or attributes",
    "metadata rejects and closes unexpected capabilities",
    "positioned transfers encode exact offsets without seek RPCs",
    "positioned empty and invalid inputs allocate nothing",
    "positioned reads at maximum offset preserve EOF semantics",
    "positioned writes reject signed end overflow before RPC",
    "positioned completions are checked before copying output",
    "zero-progress positioned write poisons its description",
    "resize and synchronization use inline commands",
    "command inputs and resize access are checked before RPC",
    "nonzero command acknowledgements poison aliases",
    "unsupported storage barriers remain errors without poisoning",
    "accepted positioned failure blocks alias mutation replay",
    "undelivered positioned failure permits retry",
    "positioned transfer retains ownership through close and reuse",
    "metadata retains ownership through close and reuse",
    "nonmetadata responses cannot smuggle attributes",
]

PREFIX = r"""
#include <assert.h>
#include <errno.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/types.h>

static bool allocation_failure;
static unsigned records;
static unsigned tables;
static void *table_storage;
static bool table_allocation_failure;
static unsigned memories;
static unsigned maps;
static unsigned open_calls;
static unsigned calls;
static uint64_t last_rpc_handle;
static uint64_t last_rpc_id;
static uint64_t last_rpc_operation;
static uint64_t last_rpc_argument;
static uint64_t last_rpc_length;
static unsigned gate_closes;
static unsigned memory_closes;
static unsigned park_calls;
static unsigned wake_calls;
static unsigned yield_calls;
static bool park_quota_failure;
static bool park_observed_changed;
static uint64_t namespace_handle = 100;
static uint64_t next_gate = 200;
static uint64_t next_memory = 1000;
static uint64_t mapped_address;
static int create_failure;
static int map_failure;
static int transport_failure;
static int transport_status = KRYO_STATUS_PEER_CLOSED;
static bool transport_accepted = true;
static int response_fault;
static uint64_t returned_status;
static uint64_t returned_count = UINT64_MAX;
static uint64_t returned_size = 9000;
static uint32_t returned_kind = KRYO_FILE_REGULAR;
static uint32_t returned_attributes;
static uint64_t returned_command;
static void (*during_call)();
static void (*during_unlock)();
static void (*during_park)();
static void (*during_failed_cas)();
template<typename T> static T test_exchange(T *word, T value, int order) {
    auto result = __atomic_exchange_n(word, value, order);
    if (!value && during_unlock)
        during_unlock();
    return result;
}
template<typename T> static bool test_compare_exchange(T *word, T *expected, T value,
        bool weak, int success, int failure) {
    auto result = __atomic_compare_exchange_n(word, expected, value, weak, success, failure);
    if (!result && during_failed_cas) {
        auto callback = during_failed_cas;
        during_failed_cas = nullptr;
        callback();
    }
    return result;
}
static void *test_calloc(size_t count, size_t bytes) {
    if (allocation_failure)
        return nullptr;
    auto result = calloc(count, bytes);
    if (result)
        ++records;
    return result;
}
static void test_free(void *pointer) {
    if (pointer) {
        if (pointer == table_storage) {
            --tables;
            table_storage = nullptr;
        } else {
            --records;
        }
    }
    free(pointer);
}
static void *test_realloc(void *pointer, size_t bytes) {
    if (table_allocation_failure)
        return nullptr;
    auto result = realloc(pointer, bytes);
    if (result) {
        if (!pointer)
            ++tables;
        table_storage = result;
    }
    return result;
}
extern "C" uint64_t __cryolite_file_namespace() { return namespace_handle; }
extern "C" int __cryolite_file_open(const char *, int, mode_t, int *);
extern "C" int __cryolite_file_close(int);
static kryo_result kryo_park(uint64_t address, uint32_t expected) {
    ++park_calls;
    if (during_park) {
        auto callback = during_park;
        during_park = nullptr;
        callback();
    }
    park_observed_changed = __atomic_load_n(reinterpret_cast<uint32_t *>(address),
        __ATOMIC_ACQUIRE) != expected;
    return {park_quota_failure ? KRYO_STATUS_OUT_OF_RESOURCES : 0, 0};
}
static kryo_result kryo_wake(uint64_t, uint64_t) {
    ++wake_calls;
    return {0, 0};
}
static kryo_result kryo_yield() { ++yield_calls; return {0, 0}; }
[[noreturn]] static void kryo_exit(uint64_t) { abort(); }
static kryo_result kryo_close(uint64_t handle) {
    if (handle >= 1000) {
        ++memory_closes;
        --memories;
    } else {
        ++gate_closes;
    }
    return {0, 0};
}
static kryo_result kryo_memory_create(uint64_t pages) {
    assert(pages == 1);
    if (create_failure)
        return {KRYO_STATUS_OUT_OF_RESOURCES, 0};
    ++memories;
    return {0, next_memory++};
}
static kryo_result kryo_memory_map(uint64_t, uint64_t address, uint64_t page, uint64_t writable) {
    assert(page == 0 && writable == KRYO_MEMORY_READ_WRITE);
    if (map_failure)
        return {KRYO_STATUS_OUT_OF_RESOURCES, 0};
    auto result = mmap(reinterpret_cast<void *>(address), 4096, PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
    assert(result == reinterpret_cast<void *>(address));
    mapped_address = address;
    ++maps;
    return {0, 0};
}
static kryo_result kryo_memory_unmap(uint64_t address) {
    assert(munmap(reinterpret_cast<void *>(address), 4096) == 0);
    --maps;
    return {0, 0};
}
static kryo_handle_info_result kryo_handle_info(uint64_t) {
    return {0, response_fault == 4 ? KRYO_OBJECT_MEMORY : KRYO_OBJECT_ENDPOINT_GATE,
        static_cast<uint64_t>(response_fault == 5 ? KRYO_RIGHT_SEND
            : KRYO_RIGHT_SEND | KRYO_RIGHT_DUP | KRYO_RIGHT_TRANSFER)};
}
static kryo_ipc_result kryo_ipc_call_exchange(uint64_t endpoint, kryo_message message,
        const kryo_grant *grants, size_t count, size_t capacity) {
    ++calls;
    last_rpc_handle = endpoint;
    last_rpc_id = message.words[1];
    auto rpc_address = mapped_address;
    auto operation = message.words[0] & UINT32_MAX;
    last_rpc_operation = operation;
    last_rpc_argument = message.words[2];
    last_rpc_length = message.words[3];
    if (operation == KRYO_FILE_OPEN)
        ++open_calls;
    if (during_call) {
        auto callback = during_call;
        during_call = nullptr;
        callback();
    }
    kryo_ipc_result reply{};
    if (transport_failure) {
        reply.status = transport_status;
        reply.value = transport_accepted ? 1 : 0;
        // Failure registers may still contain the caller's outgoing grants.
        reply.handles[0] = count ? grants[0].handle : 0;
        reply.count = count;
        return reply;
    }
    reply.message.words[0] = message.words[1];
    reply.message.words[1] = returned_status;
    if (!returned_status) {
        if (operation == KRYO_FILE_OPEN) {
            assert(count == 1 && capacity == 1
                && grants[0].rights == (KRYO_RIGHT_READ | KRYO_RIGHT_MAP));
            reply.count = 1;
            reply.handles[0] = next_gate++;
        } else if (operation == KRYO_FILE_SEEK) {
            assert(count == 0 && capacity == 0);
            reply.message.words[2] = message.words[2];
        } else if (operation == KRYO_FILE_INFO) {
            assert(count == 0 && capacity == 0 && message.words[2] == 0 && message.words[3] == 0);
            reply.message.words[2] = returned_size;
            reply.message.words[3] = returned_kind | (uint64_t{returned_attributes} << 32);
        } else if (operation == KRYO_FILE_SET_LEN || operation == KRYO_FILE_SYNC) {
            assert(count == 0 && capacity == 0 && message.words[3] == 0);
            reply.message.words[2] = returned_command;
        } else {
            bool reading = operation == KRYO_FILE_READ || operation == KRYO_FILE_READ_AT;
            bool positioned = operation == KRYO_FILE_READ_AT || operation == KRYO_FILE_WRITE_AT;
            assert(count == 1 && capacity == 0 && (positioned || message.words[2] == 0)
                && message.words[3] <= 4096);
            uint64_t expected = KRYO_RIGHT_READ | KRYO_RIGHT_MAP
                | (reading ? KRYO_RIGHT_WRITE : 0);
            assert(grants[0].rights == expected);
            auto bytes = returned_count == UINT64_MAX ? message.words[3] : returned_count;
            reply.message.words[2] = bytes;
            if (reading)
                memset(reinterpret_cast<void *>(rpc_address), 0x5a, bytes <= 4096 ? bytes : 0);
        }
    }
    if (response_fault == 1) ++reply.message.words[0];
    if (response_fault == 2) reply.message.words[3] = 1;
    if (response_fault == 3) {
        reply.count = 2;
        reply.handles[1] = next_gate++;
    }
    if (response_fault == 6) reply.message.words[3] = uint64_t{2} << 32 | KRYO_FILE_REGULAR;
    if (response_fault == 7) reply.message.words[3] = 99;
    if (response_fault == 8) reply.message.words[2] = UINT64_MAX;
    if (response_fault == 9) reply.message.words[2] = 1;
    return reply;
}

#define calloc test_calloc
#define free test_free
#define realloc test_realloc
#define __atomic_exchange_n(word, value, order) test_exchange(word, static_cast<uint32_t>(value), order)
#define __atomic_compare_exchange_n(word, expected, value, weak, success, failure) \
    test_compare_exchange(word, expected, static_cast<uint32_t>(value), weak, success, failure)
"""

SUFFIX = r"""
#undef calloc
#undef free
#undef realloc
#undef __atomic_exchange_n
#undef __atomic_compare_exchange_n
static unsigned case_number;
static void clean() {
    assert(!files && !descriptors && !descriptor_capacity
        && !records && !tables && !table_storage && !memories && !maps);
    allocation_failure = false;
    table_allocation_failure = false;
    create_failure = map_failure = transport_failure = response_fault = 0;
    transport_status = KRYO_STATUS_PEER_CLOSED;
    transport_accepted = true;
    returned_status = 0;
    returned_count = UINT64_MAX;
    returned_size = 9000;
    returned_kind = KRYO_FILE_REGULAR;
    returned_attributes = 0;
    returned_command = 0;
    namespace_handle = 100;
    during_call = nullptr;
    during_unlock = during_park = during_failed_cas = nullptr;
    park_quota_failure = false;
    ++case_number;
}
static int opened(int flags = O_RDWR) {
    int descriptor = -1;
    assert(__cryolite_file_open("/probe", flags, 0666, &descriptor) == 0);
    assert(descriptor >= 3);
    return descriptor;
}
static int reentered_old;
static int reentered_new;
static uint64_t reentered_handle;
static void reenter() {
    auto before = gate_closes;
    assert(__cryolite_file_close(reentered_old) == 0);
    assert(gate_closes == before);
    reentered_new = opened();
    assert(reentered_new == reentered_old);
    auto newer = pin_file(reentered_new);
    assert(newer->handle != reentered_handle);
    release_file(newer);
}
static void close_published() {
    for (size_t descriptor = 0; descriptor < descriptor_capacity; ++descriptor) {
        if (descriptors[descriptor].owner && descriptors[descriptor].published) {
            during_unlock = nullptr;
            assert(__cryolite_file_close(static_cast<int>(descriptor)) == 0);
            return;
        }
    }
}
static int busy_source;
static int busy_target;
static Description *busy_owner;
static void try_reserved_target() {
    busy_target = -1;
    for (size_t index = 0; index < descriptor_capacity; ++index) {
        if (descriptors[index].owner && !descriptors[index].published) {
            busy_target = static_cast<int>(index);
            busy_owner = descriptors[index].owner;
            break;
        }
    }
    assert(busy_target >= 0);
    assert(__cryolite_file_dup2(busy_source, 0, busy_target) == EBUSY);
    int result = 19;
    assert(__cryolite_file_dup(busy_target, 0, 0, &result) == EBADF && result == 19);
    assert(descriptors[busy_target].owner == busy_owner && !descriptors[busy_target].published);
}
static int replace_source;
static int replace_target;
static uint64_t replaced_gate;
static void replace_active_target() {
    auto before = gate_closes;
    assert(__cryolite_file_dup2(replace_source, 0, replace_target) == 0);
    assert(gate_closes == before);
}
static int active_source;
static int active_alias;
static int active_replacement;
static uintptr_t active_window;
static void close_active_aliases() {
    auto before = gate_closes;
    assert(__cryolite_file_close(active_source) == 0);
    assert(__cryolite_file_close(active_alias) == 0);
    assert(gate_closes == before);
    active_replacement = opened();
    assert(active_replacement == active_source);
    auto current = pin_file(active_replacement);
    assert(current->window != active_window);
    release_file(current);
}
static uint32_t *shared_lock;
static void release_shared_operation() { unlock(shared_lock); }
static void close_all_published() {
    during_unlock = nullptr;
    for (size_t descriptor = 0; descriptor < descriptor_capacity; ++descriptor)
        if (descriptors[descriptor].owner && descriptors[descriptor].published)
            assert(__cryolite_file_close(static_cast<int>(descriptor)) == 0);
}
static uint32_t contended_word;
static void release_other_owner() { unlock(&contended_word); }
int main() {
    int descriptor = -1;
    unsigned before;
    ssize_t amount = -1;
    off_t position = -1;
    unsigned char data[9000]{};

    namespace_handle = 0;
    assert(__cryolite_file_open("/probe", O_RDWR, 0, &descriptor) == ENOSYS);
    clean();

    allocation_failure = true;
    before = open_calls;
    assert(__cryolite_file_open("/probe", O_RDWR, 0, &descriptor) == ENOMEM);
    assert(open_calls == before);
    clean();

    create_failure = 1;
    before = open_calls;
    assert(__cryolite_file_open("/probe", O_RDWR, 0, &descriptor) == ENOMEM);
    assert(open_calls == before);
    clean();

    map_failure = 1;
    before = memory_closes;
    assert(__cryolite_file_open("/probe", O_RDWR, 0, &descriptor) == ENOMEM);
    assert(memory_closes == before + 1);
    clean();

    returned_status = KRYO_FILE_NOT_FOUND;
    assert(__cryolite_file_open("/probe", O_RDWR, 0, &descriptor) == ENOENT);
    clean();

    before = open_calls;
    char path[4098];
    memset(path, 'a', sizeof(path)); path[sizeof(path) - 1] = 0;
    assert(__cryolite_file_open(path, O_RDWR, 0, &descriptor) == ENAMETOOLONG);
    assert(__cryolite_file_open("", O_RDWR, 0, &descriptor) == ENOENT);
    assert(__cryolite_file_open("/probe", 3, 0, &descriptor) == EINVAL);
    assert(__cryolite_file_open("/probe", O_RDONLY | O_TRUNC, 0, &descriptor) == EINVAL);
    assert(__cryolite_file_open("/probe", O_RDWR | O_EXCL, 0, &descriptor) == EINVAL);
    assert(__cryolite_file_open("/probe", O_RDWR | O_CREAT, 0100000, &descriptor) == EINVAL);
    assert(__cryolite_file_open("/probe", O_RDWR | O_SYNC, 0, &descriptor) == ENOTSUP);
    assert(open_calls == before);
    clean();

    for (int fault : {4, 5}) {
        response_fault = fault;
        before = gate_closes;
        assert(__cryolite_file_open("/probe", O_RDWR, 0, &descriptor) == EPROTO);
        assert(gate_closes == before + 1);
    }
    clean();

    for (int fault : {1, 2}) {
        response_fault = fault;
        assert(__cryolite_file_open("/probe", O_RDWR, 0, &descriptor) == EPROTO);
    }
    clean();

    response_fault = 3;
    before = gate_closes;
    assert(__cryolite_file_open("/probe", O_RDWR, 0, &descriptor) == EPROTO);
    assert(gate_closes == before + 2);
    clean();

    descriptor = opened();
    assert(__cryolite_file_read(descriptor, data, sizeof(data), &amount) == 0 && amount == 4096);
    assert(data[0] == 0x5a && data[4095] == 0x5a && data[4096] == 0);
    assert(__cryolite_file_write(descriptor, data, sizeof(data), &amount) == 0 && amount == 4096);
    assert(__cryolite_file_close(descriptor) == 0);
    descriptor = opened(O_RDONLY);
    assert(__cryolite_file_write(descriptor, data, 1, &amount) == EBADF);
    assert(__cryolite_file_close(descriptor) == 0);
    descriptor = opened(O_WRONLY);
    assert(__cryolite_file_read(descriptor, data, 1, &amount) == EBADF);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    before = calls;
    assert(__cryolite_file_read(0, nullptr, 0, &amount) == EBADF);
    assert(__cryolite_file_close(2) == EBADF);
    assert(__cryolite_file_isatty(-1) == EBADF);
    assert(calls == before);
    descriptor = opened();
    before = calls;
    assert(__cryolite_file_read(descriptor, nullptr, 0, &amount) == 0 && amount == 0);
    assert(__cryolite_file_write(descriptor, nullptr, 0, &amount) == 0 && amount == 0);
    assert(__cryolite_file_isatty(descriptor) == ENOTTY && calls == before);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened();
    assert(__cryolite_file_seek(descriptor, 9876543210, SEEK_SET, &position) == 0
        && position == 9876543210);
    assert(__cryolite_file_seek(descriptor, 0, 3, &position) == EINVAL);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened(); returned_count = 4097;
    assert(__cryolite_file_read(descriptor, data, 4096, &amount) == EPROTO);
    before = calls;
    assert(__cryolite_file_read(descriptor, data, 1, &amount) == EIO && calls == before);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened(); returned_count = 0;
    assert(__cryolite_file_write(descriptor, data, 1, &amount) == EPROTO);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened(); transport_failure = 1;
    before = memory_closes;
    assert(__cryolite_file_read(descriptor, data, 1, &amount) == EPIPE);
    assert(memory_closes == before + 1);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened();
    auto old = pin_file(descriptor);
    auto old_window = old->window;
    before = gate_closes;
    assert(__cryolite_file_close(descriptor) == 0 && gate_closes == before);
    auto newer_fd = opened();
    assert(newer_fd == descriptor);
    auto newer = pin_file(newer_fd);
    assert(newer != old && newer->window != old_window);
    release_file(newer);
    release_file(old);
    assert(gate_closes == before + 1);
    assert(__cryolite_file_close(newer_fd) == 0);
    clean();

    reentered_old = opened();
    old = pin_file(reentered_old);
    reentered_handle = old->handle;
    release_file(old);
    during_call = reenter;
    before = gate_closes;
    assert(__cryolite_file_read(reentered_old, data, 1, &amount) == 0 && amount == 1);
    assert(gate_closes == before + 1);
    assert(__cryolite_file_close(reentered_new) == 0);
    clean();

    descriptor = opened(); old = pin_file(descriptor); old->request = UINT64_MAX; release_file(old);
    before = calls;
    assert(__cryolite_file_read(descriptor, data, 1, &amount) == EOVERFLOW && calls == before);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened(); old = pin_file(descriptor); old_window = old->window; release_file(old);
    assert(__cryolite_file_close(descriptor) == 0);
    for (unsigned i = 0; i != 128; ++i) {
        descriptor = opened(); old = pin_file(descriptor);
        assert(old->window == old_window); release_file(old);
        assert(__cryolite_file_close(descriptor) == 0);
    }
    clean();

    descriptor = opened(); returned_status = 999;
    assert(__cryolite_file_read(descriptor, data, 1, &amount) == EPROTO);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    during_unlock = close_published;
    descriptor = opened();
    assert(__cryolite_file_close(descriptor) == EBADF);
    clean();

    before = wake_calls;
    auto before_park = park_calls;
    contended_word = 0;
    lock(&contended_word); assert(contended_word == 1); unlock(&contended_word);
    assert(contended_word == 0 && wake_calls == before && park_calls == before_park);
    clean();

    contended_word = 1;
    during_park = release_other_owner;
    before = wake_calls;
    before_park = park_calls;
    lock(&contended_word);
    assert(contended_word == 2 && park_calls == before_park + 1 && park_observed_changed);
    unlock(&contended_word);
    assert(wake_calls == before + 2);
    clean();

    contended_word = 1;
    during_failed_cas = release_other_owner;
    before_park = park_calls;
    lock(&contended_word);
    assert(contended_word == 2 && park_calls == before_park);
    unlock(&contended_word);
    clean();

    contended_word = 1;
    during_park = release_other_owner;
    park_quota_failure = true;
    before = yield_calls;
    lock(&contended_word); unlock(&contended_word);
    assert(yield_calls == before + 1);
    clean();

    table_allocation_failure = true;
    before = open_calls;
    assert(__cryolite_file_open("/probe", O_RDWR, 0, &descriptor) == ENOMEM);
    assert(open_calls == before);
    clean();

    int occupied[17];
    for (unsigned index = 0; index != 5; ++index)
        occupied[index] = opened();
    assert(descriptor_capacity == 8);
    table_allocation_failure = true;
    before = open_calls;
    assert(__cryolite_file_open("/probe", O_RDWR, 0, &descriptor) == ENOMEM);
    assert(descriptor_capacity == 8 && open_calls == before);
    for (unsigned index = 0; index != 5; ++index) {
        assert(__cryolite_file_seek(occupied[index], index, SEEK_SET, &position) == 0
            && position == static_cast<off_t>(index));
        assert(__cryolite_file_close(occupied[index]) == 0);
    }
    clean();

    for (unsigned index = 0; index != 17; ++index) {
        occupied[index] = opened();
        assert(occupied[index] == static_cast<int>(index + 3));
    }
    assert(descriptor_capacity == 32);
    for (unsigned index = 0; index != 17; ++index) {
        old = pin_file(occupied[index]);
        assert(old && descriptors[occupied[index]].owner == old
            && descriptors[occupied[index]].published);
        release_file(old);
    }
    assert(__cryolite_file_close(occupied[2]) == 0);
    occupied[2] = opened();
    assert(occupied[2] == 5);
    for (auto fd : occupied)
        assert(__cryolite_file_close(fd) == 0);
    clean();

    descriptor = opened();
    transport_failure = 1;
    transport_status = KRYO_STATUS_CANCELLED;
    assert(__cryolite_file_write(descriptor, data, 1, &amount) == ECANCELED);
    transport_failure = 0;
    before = calls;
    assert(__cryolite_file_write(descriptor, data, 1, &amount) == EIO);
    assert(__cryolite_file_read(descriptor, data, 1, &amount) == EIO);
    assert(__cryolite_file_seek(descriptor, 0, SEEK_CUR, &position) == EIO);
    assert(calls == before);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened();
    transport_failure = 1;
    transport_status = KRYO_STATUS_CANCELLED;
    transport_accepted = false;
    assert(__cryolite_file_write(descriptor, data, 1, &amount) == ECANCELED);
    transport_failure = 0;
    before = calls;
    assert(__cryolite_file_write(descriptor, data, 1, &amount) == 0 && amount == 1);
    assert(calls == before + 1);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    int alias = -1;
    int result = -1;
    descriptor = opened();
    before = gate_closes;
    assert(__cryolite_file_dup(descriptor, 3, 0, &alias) == 0 && alias == descriptor + 1);
    auto shared = pin_file(descriptor);
    auto same = pin_file(alias);
    assert(shared == same);
    release_file(same); release_file(shared);
    assert(__cryolite_file_close(descriptor) == 0 && gate_closes == before);
    assert(__cryolite_file_read(alias, data, 1, &amount) == 0 && amount == 1);
    assert(__cryolite_file_close(alias) == 0 && gate_closes == before + 1);
    clean();

    descriptor = opened(O_RDWR | O_CREAT | O_TRUNC | O_APPEND | O_CLOEXEC);
    assert(__cryolite_file_control(descriptor, F_GETFD, 0, &result) == 0 && result == FD_CLOEXEC);
    assert(__cryolite_file_control(descriptor, F_GETFL, 0, &result) == 0 && result == (O_RDWR | O_APPEND));
    assert(__cryolite_file_control(descriptor, F_DUPFD, 3, &alias) == 0);
    assert(__cryolite_file_control(alias, F_GETFD, 0, &result) == 0 && result == 0);
    assert(__cryolite_file_control(alias, F_SETFD, FD_CLOEXEC, &result) == 0 && result == 0);
    assert(__cryolite_file_control(descriptor, F_SETFD, 0, &result) == 0);
    assert(__cryolite_file_control(alias, F_GETFD, 0, &result) == 0 && result == FD_CLOEXEC);
    assert(__cryolite_file_control(descriptor, F_GETFD, 0, &result) == 0 && result == 0);
    assert(__cryolite_file_close(descriptor) == 0 && __cryolite_file_close(alias) == 0);
    clean();

    descriptor = opened();
    int standard[3];
    for (int index = 0; index != 3; ++index) {
        assert(__cryolite_file_control(descriptor, F_DUPFD, 0, &standard[index]) == 0);
        assert(standard[index] == index);
        assert(__cryolite_file_isatty(standard[index]) == ENOTTY);
    }
    assert(__cryolite_file_control(descriptor, F_DUPFD_CLOEXEC, 17, &alias) == 0 && alias == 17);
    assert(__cryolite_file_control(alias, F_GETFD, 0, &result) == 0 && result == FD_CLOEXEC);
    assert(__cryolite_file_close(standard[1]) == 0);
    assert(__cryolite_file_dup(descriptor, 0, 0, &standard[1]) == 0 && standard[1] == 1);
    for (auto fd : standard) assert(__cryolite_file_close(fd) == 0);
    assert(__cryolite_file_close(alias) == 0 && __cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened();
    auto victim = opened(O_RDONLY | O_CLOEXEC);
    auto original = pin_file(descriptor);
    before = gate_closes;
    assert(__cryolite_file_dup2(descriptor, 0, victim) == 0 && gate_closes == before + 1);
    same = pin_file(victim); assert(same == original); release_file(same); release_file(original);
    assert(__cryolite_file_control(victim, F_GETFD, 0, &result) == 0 && result == 0);
    assert(__cryolite_file_control(victim, F_GETFL, 0, &result) == 0 && result == O_RDWR);
    assert(__cryolite_file_close(descriptor) == 0);
    assert(__cryolite_file_write(victim, data, 1, &amount) == 0 && amount == 1);
    assert(__cryolite_file_close(victim) == 0);
    clean();

    descriptor = opened(O_RDWR | O_CLOEXEC);
    assert(__cryolite_file_dup2(descriptor, 0, descriptor) == 0);
    assert(__cryolite_file_control(descriptor, F_GETFD, 0, &result) == 0 && result == FD_CLOEXEC);
    assert(__cryolite_file_dup2(descriptor, O_CLOEXEC, descriptor) == EINVAL);
    assert(__cryolite_file_dup2(descriptor, 1, 4) == EINVAL);
    assert(__cryolite_file_dup2(descriptor, 0, -1) == EBADF);
    assert(__cryolite_file_dup2(-1, 0, 4) == EBADF);
    assert(__cryolite_file_dup(descriptor, -1, 0, &alias) == EINVAL);
    assert(__cryolite_file_dup(descriptor, 0, 2, &alias) == EINVAL);
    assert(__cryolite_file_dup(-1, 0, 0, &alias) == EBADF);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened(O_RDWR);
    assert(__cryolite_file_control(descriptor, F_SETFL, 3 | O_EXCL | O_DIRECTORY | O_CLOEXEC, &result) == 0);
    for (int flag : {O_APPEND, O_NONBLOCK, O_SYNC, O_DSYNC, O_RSYNC})
        assert(__cryolite_file_control(descriptor, F_SETFL, O_RDWR | flag, &result) == ENOTSUP);
    assert(__cryolite_file_control(descriptor, F_SETFL, O_RDWR | 0x20000, &result) == EINVAL);
    assert(__cryolite_file_control(descriptor, F_GETFL, 0, &result) == 0 && result == O_RDWR);
    assert(__cryolite_file_close(descriptor) == 0);
    descriptor = opened(O_RDWR | O_APPEND);
    assert(__cryolite_file_control(descriptor, F_SETFL, O_RDONLY | O_APPEND | O_TRUNC, &result) == 0);
    assert(__cryolite_file_control(descriptor, F_SETFL, O_RDWR, &result) == ENOTSUP);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened(); result = 123;
    assert(__cryolite_file_control(descriptor, F_SETFD, 2, &result) == EINVAL && result == 123);
    for (int command : {F_GETLK, F_SETLK, F_SETLKW, F_GETOWN, F_SETOWN})
        assert(__cryolite_file_control(descriptor, command, 0, &result) == ENOTSUP && result == 123);
    assert(__cryolite_file_control(descriptor, 99, 0, &result) == EINVAL && result == 123);
    assert(__cryolite_file_control(-1, F_GETFD, 0, &result) == EBADF);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened(); before = gate_closes;
    auto saved_capacity = descriptor_capacity;
    table_allocation_failure = true; alias = 123;
    assert(__cryolite_file_dup(descriptor, 17, 0, &alias) == ENOMEM && alias == 123);
    assert(__cryolite_file_dup2(descriptor, 0, 17) == ENOMEM);
    assert(descriptor_capacity == saved_capacity && gate_closes == before);
    assert(__cryolite_file_seek(descriptor, 1, SEEK_SET, &position) == 0 && position == 1);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened();
    int duplicated[17];
    for (unsigned index = 0; index != 17; ++index) {
        assert(__cryolite_file_dup(descriptor, 3, 0, &duplicated[index]) == 0);
        assert(duplicated[index] == static_cast<int>(index + 4));
    }
    assert(descriptor_capacity == 32);
    assert(__cryolite_file_close(duplicated[3]) == 0);
    assert(__cryolite_file_dup(descriptor, 3, 0, &duplicated[3]) == 0 && duplicated[3] == 7);
    for (auto fd : duplicated) assert(__cryolite_file_close(fd) == 0);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    busy_source = opened(); busy_target = -1;
    during_call = try_reserved_target;
    descriptor = opened();
    assert(descriptor == busy_target);
    original = pin_file(busy_source); same = pin_file(descriptor); assert(same != original);
    release_file(same); release_file(original);
    assert(__cryolite_file_close(busy_source) == 0 && __cryolite_file_close(descriptor) == 0);
    clean();

    replace_source = opened(); replace_target = opened();
    old = pin_file(replace_target); replaced_gate = old->handle; release_file(old);
    before = gate_closes;
    during_call = replace_active_target;
    assert(__cryolite_file_read(replace_target, data, 1, &amount) == 0 && amount == 1);
    assert(last_rpc_handle == replaced_gate && gate_closes == before + 1);
    original = pin_file(replace_source); same = pin_file(replace_target); assert(original == same);
    release_file(same); release_file(original);
    assert(__cryolite_file_close(replace_source) == 0 && __cryolite_file_close(replace_target) == 0);
    clean();

    active_source = opened(); assert(__cryolite_file_dup(active_source, 3, 0, &active_alias) == 0);
    old = pin_file(active_source); active_window = old->window; release_file(old);
    before = gate_closes; during_call = close_active_aliases;
    assert(__cryolite_file_read(active_source, data, 1, &amount) == 0 && amount == 1);
    assert(gate_closes == before + 1);
    assert(__cryolite_file_close(active_replacement) == 0);
    clean();

    descriptor = opened(); assert(__cryolite_file_dup(descriptor, 3, 0, &alias) == 0);
    transport_failure = 1; transport_status = KRYO_STATUS_CANCELLED;
    assert(__cryolite_file_write(descriptor, data, 1, &amount) == ECANCELED);
    transport_failure = 0; before = calls;
    assert(__cryolite_file_write(alias, data, 1, &amount) == EIO);
    assert(__cryolite_file_read(alias, data, 1, &amount) == EIO);
    assert(__cryolite_file_seek(alias, 0, SEEK_CUR, &position) == EIO && calls == before);
    assert(__cryolite_file_close(descriptor) == 0 && __cryolite_file_close(alias) == 0);
    clean();

    descriptor = opened(); assert(__cryolite_file_dup(descriptor, 3, 0, &alias) == 0);
    assert(__cryolite_file_read(descriptor, data, 1, &amount) == 0);
    auto first_handle = last_rpc_handle;
    auto first_id = last_rpc_id;
    assert(__cryolite_file_read(alias, data, 1, &amount) == 0);
    assert(last_rpc_handle == first_handle && last_rpc_id == first_id + 1);
    assert(__cryolite_file_close(descriptor) == 0 && __cryolite_file_close(alias) == 0);
    clean();

    descriptor = opened(); assert(__cryolite_file_dup(descriptor, 3, 0, &alias) == 0);
    old = pin_file(descriptor); shared_lock = &old->operation_lock; lock(shared_lock);
    same = pin_file(alias); before_park = park_calls; during_park = release_shared_operation;
    { Operation operation{same}; assert(park_calls == before_park + 1); }
    release_file(old);
    assert(__cryolite_file_close(descriptor) == 0 && __cryolite_file_close(alias) == 0);
    clean();

    descriptor = opened(); during_unlock = close_all_published;
    assert(__cryolite_file_dup(descriptor, 3, 0, &alias) == 0);
    assert(__cryolite_file_close(descriptor) == EBADF && __cryolite_file_close(alias) == EBADF);
    descriptor = opened(); victim = opened(); during_unlock = close_all_published;
    assert(__cryolite_file_dup2(descriptor, 0, victim) == 0);
    assert(__cryolite_file_close(descriptor) == EBADF && __cryolite_file_close(victim) == EBADF);
    clean();

    struct cryolite_file_info info{17, 19, 23};
    descriptor = opened(O_RDONLY);
    before = calls;
    auto before_memory = memories;
    returned_size = 123456789;
    returned_kind = KRYO_FILE_SYMLINK;
    returned_attributes = KRYO_FILE_READONLY;
    assert(__cryolite_file_info(descriptor, &info) == 0);
    assert(info.size == 123456789 && info.kind == KRYO_FILE_SYMLINK
        && info.flags == KRYO_FILE_READONLY);
    assert(calls == before + 1 && memories == before_memory && maps == 0
        && last_rpc_operation == KRYO_FILE_INFO && last_rpc_argument == 0 && last_rpc_length == 0);
    returned_attributes = 0;
    assert(__cryolite_file_info(descriptor, &info) == 0 && info.flags == 0);
    assert(__cryolite_file_close(descriptor) == 0);
    descriptor = opened(O_WRONLY);
    returned_kind = KRYO_FILE_DIRECTORY;
    assert(__cryolite_file_info(descriptor, &info) == 0 && info.kind == KRYO_FILE_DIRECTORY);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened();
    info = {17, 19, 23};
    returned_status = KRYO_FILE_UNSUPPORTED;
    assert(__cryolite_file_info(descriptor, &info) == ENOTSUP);
    assert(info.size == 17 && info.kind == 19 && info.flags == 23);
    returned_status = 0;
    assert(__cryolite_file_read(descriptor, data, 1, &amount) == 0 && amount == 1);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    for (int fault : {6, 7, 8}) {
        descriptor = opened();
        assert(__cryolite_file_dup(descriptor, 3, 0, &alias) == 0);
        info = {17, 19, 23};
        response_fault = fault;
        assert(__cryolite_file_info(descriptor, &info) == EPROTO);
        assert(info.size == 17 && info.kind == 19 && info.flags == 23);
        response_fault = 0; before = calls;
        assert(__cryolite_file_info(alias, &info) == EIO && calls == before);
        assert(__cryolite_file_read(alias, data, 1, &amount) == EIO && calls == before);
        assert(__cryolite_file_close(descriptor) == 0 && __cryolite_file_close(alias) == 0);
    }
    clean();

    for (int fault : {2, 9}) {
        descriptor = opened();
        returned_status = KRYO_FILE_UNSUPPORTED;
        response_fault = fault;
        info = {17, 19, 23};
        assert(__cryolite_file_info(descriptor, &info) == EPROTO);
        assert(info.size == 17 && info.kind == 19 && info.flags == 23);
        returned_status = response_fault = 0; before = calls;
        assert(__cryolite_file_read(descriptor, data, 1, &amount) == EIO && calls == before);
        assert(__cryolite_file_close(descriptor) == 0);
    }
    clean();

    descriptor = opened();
    response_fault = 3; before = gate_closes;
    assert(__cryolite_file_info(descriptor, &info) == EPROTO);
    assert(gate_closes == before + 1);
    response_fault = 0;
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened();
    before = calls;
    assert(__cryolite_file_pread(descriptor, data, sizeof(data), 345, &amount) == 0 && amount == 4096);
    assert(calls == before + 1 && last_rpc_operation == KRYO_FILE_READ_AT
        && last_rpc_argument == 345 && last_rpc_length == 4096);
    before = calls;
    assert(__cryolite_file_pwrite(descriptor, data, 19, 678, &amount) == 0 && amount == 19);
    assert(calls == before + 1 && last_rpc_operation == KRYO_FILE_WRITE_AT
        && last_rpc_argument == 678 && last_rpc_length == 19);
    assert(__cryolite_file_close(descriptor) == 0);
    descriptor = opened(O_RDWR | O_APPEND);
    assert(__cryolite_file_pwrite(descriptor, data, 1, 7, &amount) == 0);
    assert(last_rpc_operation == KRYO_FILE_WRITE_AT && last_rpc_argument == 7);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened();
    before = calls; before_memory = memories; amount = -7;
    assert(__cryolite_file_pread(descriptor, nullptr, 0, LONG_MAX, &amount) == 0 && amount == 0);
    assert(__cryolite_file_pwrite(descriptor, nullptr, 0, LONG_MAX, &amount) == 0 && amount == 0);
    amount = -7;
    assert(__cryolite_file_pread(descriptor, data, 1, -1, &amount) == EINVAL && amount == -7);
    assert(__cryolite_file_pwrite(descriptor, data, 1, -1, &amount) == EINVAL && amount == -7);
    assert(__cryolite_file_pread(descriptor, data, SIZE_MAX, 0, &amount) == EINVAL && amount == -7);
    assert(__cryolite_file_pwrite(-1, data, 1, 0, &amount) == EBADF && amount == -7);
    assert(calls == before && memories == before_memory && maps == 0);
    assert(__cryolite_file_close(descriptor) == 0);
    descriptor = opened(O_RDONLY); before = calls;
    assert(__cryolite_file_pwrite(descriptor, data, 1, 0, &amount) == EBADF && calls == before);
    assert(__cryolite_file_close(descriptor) == 0);
    descriptor = opened(O_WRONLY); before = calls;
    assert(__cryolite_file_pread(descriptor, data, 1, 0, &amount) == EBADF && calls == before);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened();
    returned_count = 0;
    assert(__cryolite_file_pread(descriptor, data, 1, LONG_MAX, &amount) == 0 && amount == 0);
    returned_count = 1; amount = -7; data[0] = 0xcc;
    assert(__cryolite_file_pread(descriptor, data, 1, LONG_MAX, &amount) == EPROTO);
    assert(amount == -7 && data[0] == 0xcc);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened(); before = calls; amount = -7;
    assert(__cryolite_file_pwrite(descriptor, data, 1, LONG_MAX, &amount) == EOVERFLOW);
    assert(__cryolite_file_pwrite(descriptor, data, 10, LONG_MAX - 9, &amount) == EOVERFLOW);
    assert(amount == -7 && calls == before && maps == 0);
    assert(__cryolite_file_pwrite(descriptor, data, 9, LONG_MAX - 9, &amount) == 0 && amount == 9);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened();
    returned_count = 3;
    assert(__cryolite_file_pread(descriptor, data, 17, 23, &amount) == 0 && amount == 3);
    assert(__cryolite_file_pwrite(descriptor, data, 17, 23, &amount) == 0 && amount == 3);
    returned_count = 18; amount = -7; memset(data, 0xcc, sizeof(data));
    assert(__cryolite_file_pread(descriptor, data, 17, 23, &amount) == EPROTO);
    assert(amount == -7 && data[0] == 0xcc && data[17] == 0xcc);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened(); returned_count = 0; amount = -7;
    assert(__cryolite_file_pwrite(descriptor, data, 1, 0, &amount) == EPROTO && amount == -7);
    before = calls;
    assert(__cryolite_file_resize(descriptor, 0) == EIO && calls == before);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened(); before = calls; before_memory = memories;
    assert(__cryolite_file_resize(descriptor, 12345) == 0);
    assert(last_rpc_operation == KRYO_FILE_SET_LEN && last_rpc_argument == 12345 && !last_rpc_length);
    assert(__cryolite_file_sync(descriptor, 0) == 0);
    assert(last_rpc_operation == KRYO_FILE_SYNC && last_rpc_argument == KRYO_FILE_SYNC_ALL);
    assert(__cryolite_file_sync(descriptor, 1) == 0);
    assert(last_rpc_operation == KRYO_FILE_SYNC && last_rpc_argument == KRYO_FILE_SYNC_DATA);
    assert(calls == before + 3 && memories == before_memory && maps == 0);
    assert(__cryolite_file_close(descriptor) == 0);
    descriptor = opened(O_RDWR | O_APPEND);
    assert(__cryolite_file_resize(descriptor, 11) == 0);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened(); before = calls;
    assert(__cryolite_file_resize(descriptor, -1) == EINVAL);
    assert(__cryolite_file_sync(descriptor, -1) == EINVAL);
    assert(__cryolite_file_sync(descriptor, 2) == EINVAL);
    assert(__cryolite_file_info(-1, &info) == EBADF);
    assert(__cryolite_file_resize(-1, 1) == EBADF);
    assert(__cryolite_file_sync(-1, 0) == EBADF);
    assert(calls == before);
    assert(__cryolite_file_close(descriptor) == 0);
    descriptor = opened(O_RDONLY); before = calls;
    assert(__cryolite_file_resize(descriptor, 0) == EBADF && calls == before);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    for (bool synchronize : {false, true}) {
        descriptor = opened(); assert(__cryolite_file_dup(descriptor, 3, 0, &alias) == 0);
        returned_command = 1;
        assert((synchronize ? __cryolite_file_sync(descriptor, 0)
                            : __cryolite_file_resize(descriptor, 0)) == EPROTO);
        returned_command = 0; before = calls;
        assert(__cryolite_file_info(alias, &info) == EIO && calls == before);
        assert(__cryolite_file_close(descriptor) == 0 && __cryolite_file_close(alias) == 0);
    }
    clean();

    descriptor = opened(); returned_status = KRYO_FILE_UNSUPPORTED;
    assert(__cryolite_file_sync(descriptor, 0) == ENOTSUP);
    assert(__cryolite_file_sync(descriptor, 1) == ENOTSUP);
    returned_status = 0;
    assert(__cryolite_file_read(descriptor, data, 1, &amount) == 0 && amount == 1);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    descriptor = opened(); assert(__cryolite_file_dup(descriptor, 3, 0, &alias) == 0);
    transport_failure = 1; transport_status = KRYO_STATUS_CANCELLED;
    assert(__cryolite_file_pwrite(descriptor, data, 1, 0, &amount) == ECANCELED);
    transport_failure = 0; before = calls;
    assert(__cryolite_file_pwrite(alias, data, 1, 0, &amount) == EIO);
    assert(__cryolite_file_resize(alias, 0) == EIO && calls == before);
    assert(__cryolite_file_close(descriptor) == 0 && __cryolite_file_close(alias) == 0);
    clean();

    descriptor = opened(); transport_failure = 1;
    transport_accepted = false;
    assert(__cryolite_file_pwrite(descriptor, data, 1, 17, &amount) == EPIPE);
    transport_failure = 0;
    assert(__cryolite_file_pwrite(descriptor, data, 1, 17, &amount) == 0 && amount == 1);
    assert(last_rpc_argument == 17);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    reentered_old = opened(); before = gate_closes; during_call = reenter;
    assert(__cryolite_file_pread(reentered_old, data, 1, 17, &amount) == 0 && amount == 1);
    assert(gate_closes == before + 1);
    assert(__cryolite_file_close(reentered_new) == 0);
    clean();

    reentered_old = opened(); before = gate_closes; during_call = reenter;
    assert(__cryolite_file_info(reentered_old, &info) == 0 && info.size == returned_size);
    assert(gate_closes == before + 1);
    assert(__cryolite_file_close(reentered_new) == 0);
    clean();

    descriptor = opened(); response_fault = 2; amount = -7; data[0] = 0xcc;
    assert(__cryolite_file_pread(descriptor, data, 1, 0, &amount) == EPROTO);
    assert(amount == -7 && data[0] == 0xcc);
    assert(__cryolite_file_close(descriptor) == 0);
    clean();

    assert(case_number == 66);
    printf("file runtime: %u cases passed\n", case_number);
}
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    arguments = parser.parse_args()
    output = arguments.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    source = SOURCE.read_text()
    body = source[source.index("namespace {"):source.rindex("namespace mlibc {")]
    native = (ROOT / "userspace/libsys/include/cryolite/native.h").read_text()
    types = native[native.index("typedef __UINT32_TYPE__"):native.index("/* A successful value")]
    helper_start = native.index("static inline int kryo_ipc_accepted(")
    accepted_helper = native[helper_start:native.index("\n}", helper_start) + 2] + "\n"
    protocol_path = ROOT / "userspace/libsys/include/cryolite/file-protocol.h"
    protocol = protocol_path.read_text()
    public = (ROOT / "userspace/libc/abi/include/cryolite/file.h").read_text()
    flags = "\n".join(line for line in public.splitlines()
                      if line.startswith(("#define O_", "#define SEEK_", "#define F_", "#define FD_")))
    runtime_path = ROOT / "userspace/libc/mlibc/sysdeps/cryolite/include/cryolite/file-runtime.h"
    runtime = runtime_path.read_text()
    info_start = runtime.index("struct cryolite_file_info {")
    info_type = runtime[info_start:runtime.index("};", info_start) + 2] + "\n"
    header_at = PREFIX.index("static bool allocation_failure;")
    test = PREFIX[:header_at] + types + accepted_helper + protocol + flags + "\n" + info_type + PREFIX[header_at:] + body + SUFFIX
    test = test.replace("#include <assert.h>", "#include <assert.h>\n#include <initializer_list>")
    fixture = output / "file-runtime.cpp"
    fixture.write_text(test)
    evidence = {
        "scope": "verbatim provider backend with host native-call/allocator doubles; not guest IPC or filesystem evidence",
        "source": str(SOURCE.relative_to(ROOT)),
        "source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
        "body_sha256": hashlib.sha256(body.encode()).hexdigest(),
        "accepted_helper_sha256": hashlib.sha256(accepted_helper.encode()).hexdigest(),
        "protocol_sha256": hashlib.sha256(protocol_path.read_bytes()).hexdigest(),
        "runtime_header_sha256": hashlib.sha256(runtime_path.read_bytes()).hexdigest(),
        "interleavings": "native wait/IPC and atomic-unlock doubles inject named cutpoints; atomic wrappers execute the real compiler atomics before callbacks",
        "cases": CASES,
        "runs": [],
    }
    environment = os.environ.copy()
    environment["ASAN_OPTIONS"] = "detect_leaks=0:abort_on_error=1"
    for optimize in ("O0", "O2"):
        binary = output / f"file-runtime-{optimize}"
        command = ["clang++", "-std=c++23", f"-{optimize}", "-g",
                   "-fsanitize=address,undefined", "-fno-omit-frame-pointer",
                   "-Wall", "-Wextra", "-Werror", str(fixture), "-o", str(binary)]
        subprocess.run(command, check=True)
        result = subprocess.run([str(binary)], text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=environment)
        (output / f"{optimize}.log").write_text(result.stdout)
        if result.returncode:
            print(result.stdout, end="")
            result.check_returncode()
        evidence["runs"].append({"optimization": optimize, "exit_code": result.returncode,
                                 "output": result.stdout.strip()})
        print(result.stdout, end="")
    (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")


if __name__ == "__main__":
    main()
