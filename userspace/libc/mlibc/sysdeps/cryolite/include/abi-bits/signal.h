#pragma once
#include <cryolite/signal.h>
typedef void (*__sighandler)(int);

#if defined(__MLIBC_BUILDING_MLIBC)
#include <bits/sigset_t.h>
#endif
