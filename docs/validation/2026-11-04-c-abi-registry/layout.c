/* Candidate layouts only; no public headers are installed. */
#include "files.h"
#include "signals.h"

#ifdef __cplusplus
#define CHECK(condition) static_assert(condition, #condition)
#define ALIGN(type) alignof(type)
#else
#define CHECK(condition) _Static_assert(condition, #condition)
#define ALIGN(type) _Alignof(type)
#endif

struct draft_timeval {
    long tv_sec;
    long tv_usec;
};

struct draft_itimerspec {
    struct draft_timespec it_interval;
    struct draft_timespec it_value;
};

struct draft_itimerval {
    struct draft_timeval it_interval;
    struct draft_timeval it_value;
};

CHECK(sizeof(struct draft_timeval) == 16);
CHECK(ALIGN(struct draft_timeval) == 8);
CHECK(__builtin_offsetof(struct draft_timeval, tv_usec) == 8);
CHECK(sizeof(struct draft_itimerspec) == 32);
CHECK(ALIGN(struct draft_itimerspec) == 8);
CHECK(__builtin_offsetof(struct draft_itimerspec, it_value) == 16);
CHECK(sizeof(struct draft_itimerval) == 32);
CHECK(ALIGN(struct draft_itimerval) == 8);
CHECK(__builtin_offsetof(struct draft_itimerval, it_value) == 16);
CHECK(sizeof(void *) == 8 && ALIGN(void *) == 8);

/* Exit-status extraction remains valid at the highest assigned signal. */
CHECK(DRAFT_WIFSIGNALED(DRAFT_SIGRTMAX));
CHECK(DRAFT_WTERMSIG(DRAFT_SIGRTMAX) == DRAFT_SIGRTMAX);
CHECK(DRAFT_WIFSTOPPED((DRAFT_SIGRTMAX << 8) | 0x7f));
CHECK(DRAFT_WSTOPSIG((DRAFT_SIGRTMAX << 8) | 0x7f) == DRAFT_SIGRTMAX);
