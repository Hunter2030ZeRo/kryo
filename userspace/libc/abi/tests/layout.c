#include <cryolite/types.h>
#include <cryolite/errno.h>
#include <cryolite/memory.h>
#include <cryolite/time.h>
#include <cryolite/file.h>
#include <cryolite/signal.h>
#include <cryolite/native.h>

#include "../../../../docs/validation/2026-11-04-c-abi-registry/files.h"
#include "../../../../docs/validation/2026-11-04-c-abi-registry/signals.h"

#ifdef __cplusplus
#define CHECK(condition) static_assert(condition, #condition)
#define ALIGN(type) alignof(type)
CHECK(__is_same(size_t, unsigned long));
CHECK(__is_same(ptrdiff_t, long));
CHECK(__is_same(intptr_t, long));
CHECK(__is_same(uintptr_t, unsigned long));
CHECK(sizeof(bool) == 1 && ALIGN(bool) == 1);
CHECK(__is_same(decltype(((struct sigaction *)0)->sa_handler), void (*)(int)));
CHECK(__is_same(decltype(((struct sigaction *)0)->sa_sigaction), void (*)(int, siginfo_t *, void *)));
CHECK(__is_same(decltype(((struct sigevent *)0)->sigev_notify_attributes), pthread_attr_t *));
CHECK(__is_same(decltype(((kryo_result *)0)->status), kryo_status));
CHECK(__is_same(decltype(((kryo_result *)0)->value), kryo_u64));
#else
#define CHECK(condition) _Static_assert(condition, #condition)
#define ALIGN(type) _Alignof(type)
CHECK(__builtin_types_compatible_p(size_t, unsigned long));
CHECK(__builtin_types_compatible_p(ptrdiff_t, long));
CHECK(__builtin_types_compatible_p(intptr_t, long));
CHECK(__builtin_types_compatible_p(uintptr_t, unsigned long));
CHECK(sizeof(_Bool) == 1 && ALIGN(_Bool) == 1);
CHECK(__builtin_types_compatible_p(__typeof__(((struct sigaction *)0)->sa_handler), void (*)(int)));
CHECK(__builtin_types_compatible_p(__typeof__(((struct sigaction *)0)->sa_sigaction), void (*)(int, siginfo_t *, void *)));
CHECK(__builtin_types_compatible_p(__typeof__(((struct sigevent *)0)->sigev_notify_attributes), pthread_attr_t *));
CHECK(__builtin_types_compatible_p(__typeof__(((kryo_result *)0)->status), kryo_status));
CHECK(__builtin_types_compatible_p(__typeof__(((kryo_result *)0)->value), kryo_u64));
#endif

#define SAME(public_type, fixture_type) \
    CHECK(sizeof(public_type) == sizeof(fixture_type)); \
    CHECK(ALIGN(public_type) == ALIGN(fixture_type))
#define FIELD(public_type, public_field, fixture_type, fixture_field) \
    CHECK(__builtin_offsetof(public_type, public_field) \
          == __builtin_offsetof(fixture_type, fixture_field))

CHECK(sizeof(wchar_t) == 4 && (wchar_t)-1 < 0);
CHECK(sizeof(pid_t) == 4 && (pid_t)-1 < 0);
CHECK(sizeof(off_t) == 8 && (off_t)-1 < 0);
CHECK(sizeof(time_t) == 8 && (time_t)-1 < 0);
CHECK(sizeof(long double) == 16 && ALIGN(long double) == 16);
CHECK(__LDBL_MANT_DIG__ == 64 && __LDBL_MAX_EXP__ == 16384);

SAME(struct timespec, struct draft_timespec);
FIELD(struct timespec, tv_sec, struct draft_timespec, tv_sec);
FIELD(struct timespec, tv_nsec, struct draft_timespec, tv_nsec);
CHECK(sizeof(struct timeval) == 16 && ALIGN(struct timeval) == 8);
CHECK(__builtin_offsetof(struct timeval, tv_usec) == 8);
CHECK(sizeof(struct itimerspec) == 32 && ALIGN(struct itimerspec) == 8);
CHECK(__builtin_offsetof(struct itimerspec, it_value) == 16);
CHECK(sizeof(struct itimerval) == 32 && ALIGN(struct itimerval) == 8);
CHECK(__builtin_offsetof(struct itimerval, it_value) == 16);

SAME(struct stat, struct draft_stat);
FIELD(struct stat, st_dev, struct draft_stat, st_dev);
FIELD(struct stat, st_ino, struct draft_stat, st_ino);
FIELD(struct stat, st_nlink, struct draft_stat, st_nlink);
FIELD(struct stat, st_mode, struct draft_stat, st_mode);
FIELD(struct stat, st_uid, struct draft_stat, st_uid);
FIELD(struct stat, st_gid, struct draft_stat, st_gid);
FIELD(struct stat, __reserved0, struct draft_stat, __reserved0);
FIELD(struct stat, st_rdev, struct draft_stat, st_rdev);
FIELD(struct stat, st_size, struct draft_stat, st_size);
FIELD(struct stat, st_blksize, struct draft_stat, st_blksize);
FIELD(struct stat, st_blocks, struct draft_stat, st_blocks);
FIELD(struct stat, st_atim, struct draft_stat, st_atim);
FIELD(struct stat, st_mtim, struct draft_stat, st_mtim);
FIELD(struct stat, st_ctim, struct draft_stat, st_ctim);
FIELD(struct stat, __reserved1, struct draft_stat, __reserved1);
SAME(struct flock, struct draft_flock);
FIELD(struct flock, l_type, struct draft_flock, l_type);
FIELD(struct flock, l_whence, struct draft_flock, l_whence);
FIELD(struct flock, __reserved0, struct draft_flock, __reserved0);
FIELD(struct flock, l_start, struct draft_flock, l_start);
FIELD(struct flock, l_len, struct draft_flock, l_len);
FIELD(struct flock, l_pid, struct draft_flock, l_pid);
FIELD(struct flock, __reserved1, struct draft_flock, __reserved1);
SAME(struct pollfd, struct draft_pollfd);
FIELD(struct pollfd, fd, struct draft_pollfd, fd);
FIELD(struct pollfd, events, struct draft_pollfd, events);
FIELD(struct pollfd, revents, struct draft_pollfd, revents);
SAME(nfds_t, draft_nfds_t);
SAME(idtype_t, enum draft_idtype);

SAME(sigset_t, draft_sigset);
FIELD(sigset_t, __sig, draft_sigset, __sig);
SAME(union sigval, union draft_sigval);
SAME(siginfo_t, struct draft_siginfo);
FIELD(siginfo_t, si_signo, struct draft_siginfo, si_signo);
FIELD(siginfo_t, si_code, struct draft_siginfo, si_code);
FIELD(siginfo_t, si_errno, struct draft_siginfo, si_errno);
FIELD(siginfo_t, si_pid, struct draft_siginfo, si_pid);
FIELD(siginfo_t, si_uid, struct draft_siginfo, si_uid);
FIELD(siginfo_t, si_status, struct draft_siginfo, si_status);
FIELD(siginfo_t, si_addr, struct draft_siginfo, si_addr);
FIELD(siginfo_t, si_band, struct draft_siginfo, si_band);
FIELD(siginfo_t, si_value, struct draft_siginfo, si_value);
SAME(struct sigaction, struct draft_sigaction);
FIELD(struct sigaction, sa_handler, struct draft_sigaction, handler);
FIELD(struct sigaction, sa_sigaction, struct draft_sigaction, handler);
FIELD(struct sigaction, sa_mask, struct draft_sigaction, sa_mask);
FIELD(struct sigaction, sa_flags, struct draft_sigaction, sa_flags);
FIELD(struct sigaction, __reserved, struct draft_sigaction, reserved);
SAME(stack_t, struct draft_signal_stack);
FIELD(stack_t, ss_sp, struct draft_signal_stack, ss_sp);
FIELD(stack_t, ss_size, struct draft_signal_stack, ss_size);
FIELD(stack_t, ss_flags, struct draft_signal_stack, ss_flags);
FIELD(stack_t, __reserved, struct draft_signal_stack, reserved);
SAME(struct sigevent, struct draft_sigevent);
FIELD(struct sigevent, sigev_notify, struct draft_sigevent, sigev_notify);
FIELD(struct sigevent, sigev_signo, struct draft_sigevent, sigev_signo);
FIELD(struct sigevent, sigev_value, struct draft_sigevent, sigev_value);
FIELD(struct sigevent, sigev_notify_function, struct draft_sigevent, sigev_notify_function);
FIELD(struct sigevent, sigev_notify_attributes, struct draft_sigevent, sigev_notify_attributes);
SAME(mcontext_t, struct draft_mcontext);
FIELD(mcontext_t, version, struct draft_mcontext, version);
FIELD(mcontext_t, size, struct draft_mcontext, size);
FIELD(mcontext_t, gregs, struct draft_mcontext, gregs);
FIELD(mcontext_t, fs_base, struct draft_mcontext, fs_base);
FIELD(mcontext_t, gs_base, struct draft_mcontext, gs_base);
FIELD(mcontext_t, xstate, struct draft_mcontext, xstate);
FIELD(mcontext_t, xstate_bytes, struct draft_mcontext, xstate_bytes);
FIELD(mcontext_t, xstate_format, struct draft_mcontext, xstate_format);
FIELD(mcontext_t, xstate_features, struct draft_mcontext, xstate_features);
FIELD(mcontext_t, fault_address, struct draft_mcontext, fault_address);
FIELD(mcontext_t, fault_error, struct draft_mcontext, fault_error);
FIELD(mcontext_t, trap, struct draft_mcontext, trap);
FIELD(mcontext_t, __reserved, struct draft_mcontext, reserved);
SAME(ucontext_t, struct draft_ucontext);
FIELD(ucontext_t, uc_flags, struct draft_ucontext, uc_flags);
FIELD(ucontext_t, uc_link, struct draft_ucontext, uc_link);
FIELD(ucontext_t, uc_sigmask, struct draft_ucontext, uc_sigmask);
FIELD(ucontext_t, uc_stack, struct draft_ucontext, uc_stack);
FIELD(ucontext_t, uc_mcontext, struct draft_ucontext, uc_mcontext);

CHECK(sizeof(kryo_result) == 16 && ALIGN(kryo_result) == 8);
CHECK(__builtin_offsetof(kryo_result, status) == 0);
CHECK(__builtin_offsetof(kryo_result, value) == 8);
CHECK(sizeof(kryo_status) == 8 && (kryo_status)-1 < 0);
CHECK(sizeof(kryo_u64) == 8 && (kryo_u64)-1 > 0);
CHECK(WIFSIGNALED(SIGRTMAX) && WTERMSIG(SIGRTMAX) == SIGRTMAX);
