#ifndef CRYOLITE_ALLOCATOR_H
#define CRYOLITE_ALLOCATOR_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

void *__cryolite_allocate(size_t size, size_t alignment);
void __cryolite_deallocate(void *pointer);
void *__cryolite_reallocate(void *pointer, size_t size);
size_t __cryolite_allocation_size(void *pointer);

#ifdef __cplusplus
}
#endif

#endif
