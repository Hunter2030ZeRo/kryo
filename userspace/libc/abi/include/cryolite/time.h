#ifndef CRYOLITE_C_ABI_TIME_H
#define CRYOLITE_C_ABI_TIME_H

#include <cryolite/types.h>

#define CLOCK_REALTIME 0
#define CLOCK_MONOTONIC 1
#define CLOCK_PROCESS_CPUTIME_ID 2
#define CLOCK_THREAD_CPUTIME_ID 3
#define TIMER_ABSTIME 0x01
#define TIME_UTC 1
#define CLOCKS_PER_SEC ((clock_t)1000000)

struct timespec {
    time_t tv_sec;
    long tv_nsec;
};

struct timeval {
    time_t tv_sec;
    suseconds_t tv_usec;
};

struct itimerspec {
    struct timespec it_interval;
    struct timespec it_value;
};

struct itimerval {
    struct timeval it_interval;
    struct timeval it_value;
};

#endif
