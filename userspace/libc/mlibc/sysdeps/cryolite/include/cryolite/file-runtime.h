#ifndef CRYOLITE_FILE_RUNTIME_H
#define CRYOLITE_FILE_RUNTIME_H

#include <stddef.h>
#include <stdint.h>
#include <cryolite/file.h>

#ifdef __cplusplus
extern "C" {
#endif

struct cryolite_file_info {
    uint64_t size;
    uint32_t kind;
    uint32_t flags;
};

/* Pinned provider hooks, not the public POSIX interface. */
uint64_t __cryolite_file_namespace(void);
int __cryolite_file_open(const char *, int, mode_t, int *);
int __cryolite_file_read(int, void *, size_t, ssize_t *);
int __cryolite_file_write(int, const void *, size_t, ssize_t *);
int __cryolite_file_pread(int, void *, size_t, off_t, ssize_t *);
int __cryolite_file_pwrite(int, const void *, size_t, off_t, ssize_t *);
int __cryolite_file_info(int, struct cryolite_file_info *);
int __cryolite_file_resize(int, off_t);
int __cryolite_file_sync(int, int);
int __cryolite_file_seek(int, off_t, int, off_t *);
int __cryolite_file_close(int);
int __cryolite_file_isatty(int);
int __cryolite_file_dup(int, int, int, int *);
int __cryolite_file_dup2(int, int, int);
int __cryolite_file_control(int, int, int, int *);

#ifdef __cplusplus
}
#endif

#endif
