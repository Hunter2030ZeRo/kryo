#include <stdint.h>
#include <stdarg.h>

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
