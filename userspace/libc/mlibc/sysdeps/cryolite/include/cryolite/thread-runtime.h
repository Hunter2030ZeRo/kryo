#ifndef CRYOLITE_THREAD_RUNTIME_H
#define CRYOLITE_THREAD_RUNTIME_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

struct cryolite_tcb_storage {
    void *allocation;
    void *keys;
    void **dtv;
    void *cookie;
};

int __cryolite_tcb_allocate(size_t front_bytes, size_t key_bytes, size_t dtv_count,
                            struct cryolite_tcb_storage *out);
void __cryolite_tcb_bind(void *cookie, void *tcb);
void __cryolite_tcb_discard(void *tcb);
void __cryolite_thread_published(void *tcb);
int __cryolite_thread_join(void *tcb, void *result);
int __cryolite_thread_detach(void *tcb);
size_t __cryolite_thread_records(void);

#ifdef __cplusplus
}
#endif

#endif
