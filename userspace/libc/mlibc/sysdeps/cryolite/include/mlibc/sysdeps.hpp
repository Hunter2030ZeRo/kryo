#pragma once

#include <mlibc/sysdep-signatures.hpp>

namespace mlibc {

struct CryoliteSysdepTags :
    LibcPanic, LibcLog, Exit, TcbSet, AnonAllocate, AnonFree,
    VmMap, VmUnmap, FutexWait, FutexWake, ClockGet, Sleep, Yield,
    Open, Read, Write, Seek, Close, Isatty,
#if __MLIBC_POSIX_OPTION
    Dup2, Fcntl,
#endif
    PrepareStack, Clone, ThreadExit, FutexTid {};

template<typename Tag>
using Sysdeps = SysdepOf<CryoliteSysdepTags, Tag>;

struct SysdepTraits {
    static constexpr bool usesRtNetlink = false;
};

} // namespace mlibc
