#include <errno.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <threads.h>
#include <time.h>
#include <cryolite/native.h>

extern kryo_handle __cryolite_debug_handle(void);
extern kryo_handle __cryolite_startup_handle(void);
extern uint64_t __cryolite_wait_fallbacks(void);
extern void check_allocator(kryo_handle debug);
extern void check_provider_threads(kryo_handle debug);
extern void check_provider_stdio(kryo_handle debug);
extern void check_provider_files(kryo_handle debug);

static _Thread_local volatile uint64_t tls_value = UINT64_C(0x2197acde4568);
static _Thread_local volatile uint64_t tls_zero;
static unsigned constructed;
static int (*volatile compare_text)(const char *, const char *) = strcmp;
static size_t (*volatile text_length)(const char *) = strlen;

static void event(uint64_t number, uint64_t value) {
    if (kryo_debug_log(__cryolite_debug_handle(), number, value).status)
        kryo_exit(252);
}

static void require(int condition, unsigned stage) {
    if (!condition) {
        event(209, stage);
        kryo_exit(stage);
    }
}

__attribute__((constructor(200))) static void initialize(void) {
    require(tls_value == UINT64_C(0x2197acde4568) && tls_zero == 0, 1);
    constructed = 1;
    tls_zero = UINT64_C(0x817a37);
}

static void at_exit(void) {
    require(constructed == 1 && tls_zero == UINT64_C(0x817a37), 2);
    event(201, 1);
}

__attribute__((destructor(200))) static void finalize(void) {
    event(202, 1);
}

int main(int argc, char **argv, char **envp) {
    require(constructed == 1 && argc == 2 && !argv[2], 3);
    require(!strcmp(argv[0], "libc-probe") && !strcmp(argv[1], "cryolite-v3"), 4);
    require(envp && envp[0] && !strcmp(envp[0], "KRYO_LIBC=1") && !envp[1], 5);
    require(!strcmp(getenv("KRYO_LIBC"), "1"), 6);
    require(tls_value == UINT64_C(0x2197acde4568) && tls_zero == UINT64_C(0x817a37), 7);
    errno = EOVERFLOW;
    require(errno == EOVERFLOW && atexit(at_exit) == 0, 8);

    unsigned char *memory = calloc(128, 1);
    require(memory != NULL && ((uintptr_t)memory & 15) == 0, 9);
    for (unsigned i = 0; i < 128; ++i) {
        require(memory[i] == 0, 10);
        memory[i] = (unsigned char)i;
    }
    unsigned char *larger = realloc(memory, 32768);
    require(larger != NULL, 11);
    for (unsigned i = 0; i < 128; ++i)
        require(larger[i] == (unsigned char)i, 12);
    memory = larger;
    errno = 0;
    // The probe's explicit account cannot commit this backing request.
    volatile size_t too_large = 128u * 1024u * 1024u;
    require(realloc(memory, too_large) == NULL && errno == ENOMEM, 13);
    for (unsigned i = 0; i < 128; ++i)
        require(memory[i] == (unsigned char)i, 14);
    free(memory);
    free(NULL);

    errno = 0;
    volatile size_t overflow = SIZE_MAX;
    require(calloc(overflow, 2) == NULL && errno == ENOMEM, 15);
    void *aligned = aligned_alloc(65536, 65536);
    require(aligned != NULL && ((uintptr_t)aligned & 65535) == 0, 16);
    memset(aligned, 0x53, 65536);
    free(aligned);
    errno = 0;
    require(aligned_alloc(3, 12) == NULL && errno == EINVAL, 17);

    char text[128];
    int length = snprintf(text, sizeof(text), "mlibc: %s value=%d hex=%x float=%.2f", "Cryolite", 42, 42, 3.25);
    static const char expected[] = "mlibc: Cryolite value=42 hex=2a float=3.25";
    require(length == (int)sizeof(expected) - 1 && !compare_text(text, expected), 18);
    require(text_length(text) == (size_t)length, 19);
    char short_text[4];
    require(snprintf(short_text, sizeof(short_text), "%s", "abcdef") == 6
            && !strcmp(short_text, "abc"), 20);

    struct timespec before, after;
    require(clock_gettime(CLOCK_MONOTONIC, &before) == 0, 21);
    thrd_yield();
    struct timespec pause = {.tv_sec = 0, .tv_nsec = 1000000};
    require(thrd_sleep(&pause, NULL) == 0 && clock_gettime(CLOCK_MONOTONIC, &after) == 0, 22);
    require(after.tv_sec > before.tv_sec || (after.tv_sec == before.tv_sec && after.tv_nsec >= before.tv_nsec), 23);
    uint64_t before_ns = (uint64_t)before.tv_sec * UINT64_C(1000000000) + (uint64_t)before.tv_nsec;
    uint64_t after_ns = (uint64_t)after.tv_sec * UINT64_C(1000000000) + (uint64_t)after.tv_nsec;
    require(after_ns - before_ns >= UINT64_C(1000000), 33);
    errno = 0;
    require(clock_gettime(CLOCK_REALTIME, &after) == -1 && errno == ENOTSUP, 24);
    errno = 0;
    require(fopen("/missing", "r") == NULL && errno == ENOENT, 25);
    kryo_result reserved = kryo_vm_reserve(8192, 4096);
    require(reserved.status == 0, 27);
    require(kryo_vm_commit(reserved.value, 4096).status == 0, 28);
    for (size_t size = 0; size < 64; ++size) {
        char *boundary = (char *)(uintptr_t)(reserved.value + 4095 - size);
        char expected_text[64];
        for (size_t index = 0; index < size; ++index)
            boundary[index] = expected_text[index] = (char)('a' + index % 26);
        boundary[size] = expected_text[size] = 0;
        require(text_length(boundary) == size && compare_text(boundary, expected_text) == 0, 29);
        if (size) {
            expected_text[size - 1] ^= 1;
            require(compare_text(boundary, expected_text) != 0, 30);
        }
    }
    require(kryo_vm_release(reserved.value).status == 0, 31);
    require(kryo_debug_write(__cryolite_debug_handle(), text, (size_t)length).status == 0, 26);
    check_provider_stdio(__cryolite_debug_handle());
    check_provider_files(__cryolite_debug_handle());
    check_provider_threads(__cryolite_debug_handle());
    check_allocator(__cryolite_debug_handle());
    kryo_result instance = kryo_startup_read(__cryolite_startup_handle(), 4);
    require(instance.status == KRYO_STATUS_OK && instance.value >= 1 && instance.value <= 3, 34);
    if (instance.value == 3)
        require(__cryolite_wait_fallbacks() > 0, 35);
    event(200, 1);
    if (instance.value == 2)
        thrd_exit(42);
    return 42;
}
