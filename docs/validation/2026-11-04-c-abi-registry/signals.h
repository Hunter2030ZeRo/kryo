#ifndef CRYOLITE_DRAFT_SIGNALS_H
#define CRYOLITE_DRAFT_SIGNALS_H

#ifdef __cplusplus
#define SIGNAL_CHECK(condition) static_assert(condition, #condition)
#define SIGNAL_ALIGN(type) alignof(type)
#else
#define SIGNAL_CHECK(condition) _Static_assert(condition, #condition)
#define SIGNAL_ALIGN(type) _Alignof(type)
#endif
#define SIGNAL_OFFSET(type, field) __builtin_offsetof(type, field)

#define DRAFT_SIGHUP 1
#define DRAFT_SIGINT 2
#define DRAFT_SIGQUIT 3
#define DRAFT_SIGILL 4
#define DRAFT_SIGTRAP 5
#define DRAFT_SIGABRT 6
#define DRAFT_SIGBUS 7
#define DRAFT_SIGFPE 8
#define DRAFT_SIGKILL 9
#define DRAFT_SIGUSR1 10
#define DRAFT_SIGSEGV 11
#define DRAFT_SIGUSR2 12
#define DRAFT_SIGPIPE 13
#define DRAFT_SIGALRM 14
#define DRAFT_SIGTERM 15
#define DRAFT_SIGCHLD 17
#define DRAFT_SIGCONT 18
#define DRAFT_SIGSTOP 19
#define DRAFT_SIGTSTP 20
#define DRAFT_SIGTTIN 21
#define DRAFT_SIGTTOU 22
#define DRAFT_SIGURG 23
#define DRAFT_SIGXCPU 24
#define DRAFT_SIGXFSZ 25
#define DRAFT_SIGVTALRM 26
#define DRAFT_SIGPROF 27
#define DRAFT_SIGWINCH 28
#define DRAFT_SIGPOLL 29
#define DRAFT_SIGSYS 31
#define DRAFT_SIGNAL_CANCEL 32
#define DRAFT_SIGNAL_TIMER 33
#define DRAFT_SIGNAL_RUNTIME_RESERVED 34
#define DRAFT_SIGRTMIN 35
#define DRAFT_SIGRTMAX 64
#define DRAFT_NSIG 65

#define DRAFT_SIG_BLOCK 0
#define DRAFT_SIG_UNBLOCK 1
#define DRAFT_SIG_SETMASK 2
#define DRAFT_SA_NOCLDSTOP 0x01
#define DRAFT_SA_NOCLDWAIT 0x02
#define DRAFT_SA_SIGINFO 0x04
#define DRAFT_SA_ONSTACK 0x08
#define DRAFT_SA_RESTART 0x10
#define DRAFT_SA_NODEFER 0x20
#define DRAFT_SA_RESETHAND 0x40
#define DRAFT_SA_MASK 0x7f
#define DRAFT_SS_ONSTACK 0x01
#define DRAFT_SS_DISABLE 0x02
#define DRAFT_SIGEV_SIGNAL 0
#define DRAFT_SIGEV_NONE 1
#define DRAFT_SIGEV_THREAD 2
#define DRAFT_MINSIGSTKSZ 8192UL
#define DRAFT_SIGSTKSZ 32768UL

#define DRAFT_SI_USER 0
#define DRAFT_SI_QUEUE (-1)
#define DRAFT_SI_TIMER (-2)
#define DRAFT_SI_MESGQ (-3)
#define DRAFT_SI_ASYNCIO (-4)
#define DRAFT_SI_TKILL (-6)

typedef int draft_sig_atomic;

typedef struct {
    unsigned long __sig[1];
} draft_sigset;

union draft_sigval {
    int sival_int;
    void *sival_ptr;
};

struct draft_siginfo {
    int si_signo;
    int si_code;
    int si_errno;
    int si_pid;
    unsigned int si_uid;
    int si_status;
    void *si_addr;
    long si_band;
    union draft_sigval si_value;
};

typedef void (*draft_sighandler)(int);
#define DRAFT_SIG_DFL ((draft_sighandler)0)
#define DRAFT_SIG_IGN ((draft_sighandler)1)
#define DRAFT_SIG_HOLD ((draft_sighandler)2)
#define DRAFT_SIG_ERR ((draft_sighandler)-1)

struct draft_sigaction {
    union {
        draft_sighandler sa_handler;
        void (*sa_sigaction)(int, struct draft_siginfo *, void *);
    } handler;
    draft_sigset sa_mask;
    int sa_flags;
    unsigned int reserved;
};

struct draft_signal_stack {
    void *ss_sp;
    __SIZE_TYPE__ ss_size;
    int ss_flags;
    unsigned int reserved;
};

struct draft_pthread_attr;
struct draft_sigevent {
    int sigev_notify;
    int sigev_signo;
    union draft_sigval sigev_value;
    void (*sigev_notify_function)(union draft_sigval);
    struct draft_pthread_attr *sigev_notify_attributes;
};

struct draft_mcontext {
    unsigned int version;
    unsigned int size;
    unsigned long gregs[18];
    unsigned long fs_base;
    unsigned long gs_base;
    const void *xstate;
    unsigned int xstate_bytes;
    unsigned int xstate_format;
    unsigned long xstate_features;
    unsigned long fault_address;
    unsigned long fault_error;
    unsigned int trap;
    unsigned int reserved;
};

struct draft_ucontext {
    unsigned long uc_flags;
    struct draft_ucontext *uc_link;
    draft_sigset uc_sigmask;
    struct draft_signal_stack uc_stack;
    struct draft_mcontext uc_mcontext;
};

SIGNAL_CHECK(sizeof(draft_sig_atomic) == 4);
SIGNAL_CHECK((draft_sig_atomic)-1 < 0);
SIGNAL_CHECK(sizeof(draft_sigset) == 8);
SIGNAL_CHECK(SIGNAL_ALIGN(draft_sigset) == 8);
SIGNAL_CHECK(SIGNAL_OFFSET(draft_sigset, __sig) == 0);
SIGNAL_CHECK(sizeof(union draft_sigval) == 8);
SIGNAL_CHECK(SIGNAL_ALIGN(union draft_sigval) == 8);
SIGNAL_CHECK(sizeof(struct draft_siginfo) == 48);
SIGNAL_CHECK(SIGNAL_ALIGN(struct draft_siginfo) == 8);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_siginfo, si_signo) == 0);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_siginfo, si_code) == 4);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_siginfo, si_errno) == 8);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_siginfo, si_pid) == 12);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_siginfo, si_uid) == 16);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_siginfo, si_status) == 20);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_siginfo, si_addr) == 24);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_siginfo, si_band) == 32);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_siginfo, si_value) == 40);
SIGNAL_CHECK(sizeof(DRAFT_SIG_DFL) == 8);
SIGNAL_CHECK(sizeof(DRAFT_SIG_IGN) == 8);
SIGNAL_CHECK(sizeof(DRAFT_SIG_HOLD) == 8);
SIGNAL_CHECK(sizeof(DRAFT_SIG_ERR) == 8);
SIGNAL_CHECK(sizeof(struct draft_sigaction) == 24);
SIGNAL_CHECK(SIGNAL_ALIGN(struct draft_sigaction) == 8);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_sigaction, handler) == 0);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_sigaction, sa_mask) == 8);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_sigaction, sa_flags) == 16);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_sigaction, reserved) == 20);
SIGNAL_CHECK(sizeof(struct draft_signal_stack) == 24);
SIGNAL_CHECK(SIGNAL_ALIGN(struct draft_signal_stack) == 8);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_signal_stack, ss_sp) == 0);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_signal_stack, ss_size) == 8);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_signal_stack, ss_flags) == 16);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_signal_stack, reserved) == 20);
SIGNAL_CHECK(sizeof(struct draft_sigevent) == 32);
SIGNAL_CHECK(SIGNAL_ALIGN(struct draft_sigevent) == 8);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_sigevent, sigev_notify) == 0);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_sigevent, sigev_signo) == 4);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_sigevent, sigev_value) == 8);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_sigevent, sigev_notify_function) == 16);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_sigevent, sigev_notify_attributes) == 24);
SIGNAL_CHECK(sizeof(struct draft_mcontext) == 216);
SIGNAL_CHECK(SIGNAL_ALIGN(struct draft_mcontext) == 8);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_mcontext, version) == 0);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_mcontext, size) == 4);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_mcontext, gregs) == 8);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_mcontext, fs_base) == 152);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_mcontext, gs_base) == 160);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_mcontext, xstate) == 168);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_mcontext, xstate_bytes) == 176);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_mcontext, xstate_format) == 180);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_mcontext, xstate_features) == 184);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_mcontext, fault_address) == 192);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_mcontext, fault_error) == 200);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_mcontext, trap) == 208);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_mcontext, reserved) == 212);
SIGNAL_CHECK(sizeof(struct draft_ucontext) == 264);
SIGNAL_CHECK(SIGNAL_ALIGN(struct draft_ucontext) == 8);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_ucontext, uc_flags) == 0);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_ucontext, uc_link) == 8);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_ucontext, uc_sigmask) == 16);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_ucontext, uc_stack) == 24);
SIGNAL_CHECK(SIGNAL_OFFSET(struct draft_ucontext, uc_mcontext) == 48);
SIGNAL_CHECK(DRAFT_SIGRTMIN > DRAFT_SIGNAL_RUNTIME_RESERVED);
SIGNAL_CHECK(DRAFT_SIGRTMAX - DRAFT_SIGRTMIN + 1 == 30);
SIGNAL_CHECK(DRAFT_SIGRTMAX == sizeof(draft_sigset) * 8);
SIGNAL_CHECK((1UL << (DRAFT_SIGRTMAX - 1)) == 0x8000000000000000UL);
SIGNAL_CHECK((DRAFT_SA_NOCLDSTOP | DRAFT_SA_NOCLDWAIT | DRAFT_SA_SIGINFO |
              DRAFT_SA_ONSTACK | DRAFT_SA_RESTART | DRAFT_SA_NODEFER |
              DRAFT_SA_RESETHAND) == DRAFT_SA_MASK);
SIGNAL_CHECK(DRAFT_SIGSTKSZ >= DRAFT_MINSIGSTKSZ);

#undef SIGNAL_OFFSET
#undef SIGNAL_ALIGN
#undef SIGNAL_CHECK

#endif
