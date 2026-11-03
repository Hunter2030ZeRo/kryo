#include <stdint.h>
#include <stdarg.h>

__thread __attribute__((aligned(64))) volatile uint64_t c_tls_initialized = UINT64_C(0x13579bdf2468ace0);
__thread volatile uint64_t c_tls_zero[4];

uint64_t c_tls_initial(void) {
    if (c_tls_initialized != UINT64_C(0x13579bdf2468ace0)
        || (uintptr_t)&c_tls_initialized % 64 != 0)
        return 1;
    for (unsigned index = 0; index < 4; ++index)
        if (c_tls_zero[index] != 0)
            return 2;
    return 0;
}

void c_tls_store(uint64_t value) {
    c_tls_initialized = value;
    for (unsigned index = 0; index < 4; ++index)
        c_tls_zero[index] = value + index + 1;
}

uint64_t c_tls_check(uint64_t value) {
    if (c_tls_initialized != value)
        return 1;
    for (unsigned index = 0; index < 4; ++index)
        if (c_tls_zero[index] != value + index + 1)
            return 2;
    return 0;
}

uintptr_t c_tls_address(void) {
    return (uintptr_t)&c_tls_initialized;
}

extern double rust_mix(uint64_t first, double second, uint64_t third, double fourth);
extern double rust_register_pressure(double a, double b, double c, double d,
                                     double e, double f, double g, double h,
                                     double ninth, uint64_t integer);

double c_mix(uint64_t first, double second, uint64_t third, double fourth) {
    return (double)first + second * 2.0 - (double)third + fourth * 4.0;
}

uint64_t c_calls_rust(void) {
    if (rust_mix(19, 1.25, 7, 0.5) != 16.5)
        return 1;
    if (rust_register_pressure(1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10) != 55.0)
        return 2;
    return 0;
}

double c_register_pressure(double a, double b, double c, double d,
                           double e, double f, double g, double h,
                           double ninth, uint64_t integer) {
    return a + b + c + d + e + f + g + h + ninth + (double)integer;
}

double c_variadic(unsigned count, ...) {
    va_list arguments;
    va_start(arguments, count);
    double total = 0;
    for (unsigned index = 0; index < count; ++index)
        total += va_arg(arguments, double);
    va_end(arguments);
    return total;
}
