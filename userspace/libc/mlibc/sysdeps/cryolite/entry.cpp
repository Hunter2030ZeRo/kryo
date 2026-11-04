#include <stdint.h>
#include <stdlib.h>
#include <mlibc/elf/startup.h>

extern "C" void __dlapi_enter(uintptr_t *);
extern "C" void __cryolite_startup(uint64_t);
extern char **environ;

extern "C" [[noreturn]] void __cryolite_entry(
    uintptr_t *stack, int (*main_fn)(int, char **, char **), uint64_t startup) {
    __cryolite_startup(startup);
    __dlapi_enter(stack);
    exit(main_fn(mlibc::entry_stack.argc, mlibc::entry_stack.argv, environ));
}
