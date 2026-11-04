#ifndef _MLIBC_INTERNAL_PTHREAD_ATTR_T_H
#define _MLIBC_INTERNAL_PTHREAD_ATTR_T_H
#include <cryolite/types.h>

/* Complete the owned tag with this pinned provider's real representation. */
struct __cryolite_pthread_attr {
    void *__heap_ptr;
};
#endif
