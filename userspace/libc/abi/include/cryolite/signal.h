#ifndef CRYOLITE_C_ABI_SIGNAL_H
#define CRYOLITE_C_ABI_SIGNAL_H

#include <cryolite/types.h>

#define SIGHUP 1
#define SIGINT 2
#define SIGQUIT 3
#define SIGILL 4
#define SIGTRAP 5
#define SIGABRT 6
#define SIGBUS 7
#define SIGFPE 8
#define SIGKILL 9
#define SIGUSR1 10
#define SIGSEGV 11
#define SIGUSR2 12
#define SIGPIPE 13
#define SIGALRM 14
#define SIGTERM 15
#define SIGCHLD 17
#define SIGCONT 18
#define SIGSTOP 19
#define SIGTSTP 20
#define SIGTTIN 21
#define SIGTTOU 22
#define SIGURG 23
#define SIGXCPU 24
#define SIGXFSZ 25
#define SIGVTALRM 26
#define SIGPROF 27
#define SIGWINCH 28
#define SIGPOLL 29
#define SIGSYS 31
#define __CRYOLITE_SIGNAL_CANCEL 32
#define __CRYOLITE_SIGNAL_TIMER 33
#define __CRYOLITE_SIGNAL_RUNTIME_RESERVED 34
#define SIGRTMIN 35
#define SIGRTMAX 64
#define _NSIG 65

#define SIG_BLOCK 0
#define SIG_UNBLOCK 1
#define SIG_SETMASK 2
#define SA_NOCLDSTOP 0x01
#define SA_NOCLDWAIT 0x02
#define SA_SIGINFO 0x04
#define SA_ONSTACK 0x08
#define SA_RESTART 0x10
#define SA_NODEFER 0x20
#define SA_RESETHAND 0x40
#define SS_ONSTACK 0x01
#define SS_DISABLE 0x02
#define SIGEV_SIGNAL 0
#define SIGEV_NONE 1
#define SIGEV_THREAD 2
#define MINSIGSTKSZ 8192UL
#define SIGSTKSZ 32768UL

#define SI_USER 0
#define SI_QUEUE (-1)
#define SI_TIMER (-2)
#define SI_MESGQ (-3)
#define SI_ASYNCIO (-4)
#define SI_TKILL (-6)

#define ILL_ILLOPC 1
#define ILL_ILLOPN 2
#define ILL_ILLADR 3
#define ILL_ILLTRP 4
#define ILL_PRVOPC 5
#define ILL_PRVREG 6
#define ILL_COPROC 7
#define ILL_BADSTK 8
#define FPE_INTDIV 1
#define FPE_INTOVF 2
#define FPE_FLTDIV 3
#define FPE_FLTOVF 4
#define FPE_FLTUND 5
#define FPE_FLTRES 6
#define FPE_FLTINV 7
#define FPE_FLTSUB 8
#define SEGV_MAPERR 1
#define SEGV_ACCERR 2
#define BUS_ADRALN 1
#define BUS_ADRERR 2
#define BUS_OBJERR 3
#define TRAP_BRKPT 1
#define TRAP_TRACE 2
#define CLD_EXITED 1
#define CLD_KILLED 2
#define CLD_DUMPED 3
#define CLD_TRAPPED 4
#define CLD_STOPPED 5
#define CLD_CONTINUED 6
#define POLL_IN 1
#define POLL_OUT 2
#define POLL_MSG 3
#define POLL_ERR 4
#define POLL_PRI 5
#define POLL_HUP 6

union sigval {
    int sival_int;
    void *sival_ptr;
};

typedef struct {
    int si_signo;
    int si_code;
    int si_errno;
    pid_t si_pid;
    uid_t si_uid;
    int si_status;
    void *si_addr;
    long si_band;
    union sigval si_value;
} siginfo_t;

#define SIG_DFL ((void (*)(int))0)
#define SIG_IGN ((void (*)(int))1)
#define SIG_HOLD ((void (*)(int))2)
#define SIG_ERR ((void (*)(int))-1)

struct sigaction {
    union {
        void (*sa_handler)(int);
        void (*sa_sigaction)(int, siginfo_t *, void *);
    };
    sigset_t sa_mask;
    int sa_flags;
    unsigned int __reserved;
};

typedef struct {
    void *ss_sp;
    size_t ss_size;
    int ss_flags;
    unsigned int __reserved;
} stack_t;

struct sigevent {
    int sigev_notify;
    int sigev_signo;
    union sigval sigev_value;
    void (*sigev_notify_function)(union sigval);
    pthread_attr_t *sigev_notify_attributes;
};

/* Experimental context view; no signal-return protocol is implemented. */
typedef struct {
    uint32_t version;
    uint32_t size;
    uint64_t gregs[18];
    uint64_t fs_base;
    uint64_t gs_base;
    const void *xstate;
    uint32_t xstate_bytes;
    uint32_t xstate_format;
    uint64_t xstate_features;
    uint64_t fault_address;
    uint64_t fault_error;
    uint32_t trap;
    uint32_t __reserved;
} mcontext_t;

typedef struct ucontext {
    unsigned long uc_flags;
    struct ucontext *uc_link;
    sigset_t uc_sigmask;
    stack_t uc_stack;
    mcontext_t uc_mcontext;
} ucontext_t;

#endif
