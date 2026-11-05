#ifndef CRYOLITE_RUST_RUNTIME_H
#define CRYOLITE_RUST_RUNTIME_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

__attribute__((noreturn)) void __cryolite_rust_abort(void);
int __cryolite_rust_random(void *buffer, size_t bytes);
uint64_t __cryolite_rust_clock(void);
void __cryolite_rust_sleep_until(uint64_t deadline);
void __cryolite_rust_yield(void);
int64_t __cryolite_rust_wait(const uint32_t *word, uint32_t expected,
                           uint64_t deadline, int timed);
uint64_t __cryolite_rust_wake(const uint32_t *word, uint64_t maximum);

int __cryolite_rust_thread_create(uintptr_t *thread, size_t stack_size,
                                 int (*entry)(void *), void *argument);
int __cryolite_rust_thread_join(uintptr_t thread);
int __cryolite_rust_thread_detach(uintptr_t thread);
int __cryolite_rust_key_create(uintptr_t *key, void (*destructor)(void *));
void __cryolite_rust_key_delete(uintptr_t key);
void *__cryolite_rust_key_get(uintptr_t key);
int __cryolite_rust_key_set(uintptr_t key, void *value);

#ifdef __cplusplus
}
#endif
#endif
