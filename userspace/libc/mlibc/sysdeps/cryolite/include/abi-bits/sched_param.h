#pragma once

#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2

/* Provider attributes retain a priority value; no scheduling sysdep is enabled. */
struct sched_param {
    int sched_priority;
};
