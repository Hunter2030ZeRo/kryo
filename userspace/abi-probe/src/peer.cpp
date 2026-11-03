#include <stdint.h>

alignas(64) thread_local volatile uint64_t cpp_tls_initialized = UINT64_C(0xfedcba9876543210);
thread_local volatile uint64_t cpp_tls_zero[4];

extern "C" uint64_t cpp_tls_initial() {
    if (cpp_tls_initialized != UINT64_C(0xfedcba9876543210)
        || reinterpret_cast<uintptr_t>(&cpp_tls_initialized) % 64 != 0)
        return 1;
    for (unsigned index = 0; index < 4; ++index)
        if (cpp_tls_zero[index] != 0)
            return 2;
    return 0;
}

extern "C" void cpp_tls_store(uint64_t value) {
    cpp_tls_initialized = value;
    for (unsigned index = 0; index < 4; ++index)
        cpp_tls_zero[index] = value + index + 1;
}

extern "C" uint64_t cpp_tls_check(uint64_t value) {
    if (cpp_tls_initialized != value)
        return 1;
    for (unsigned index = 0; index < 4; ++index)
        if (cpp_tls_zero[index] != value + index + 1)
            return 2;
    return 0;
}

extern "C" uintptr_t cpp_tls_address() {
    return reinterpret_cast<uintptr_t>(&cpp_tls_initialized);
}

extern "C" double rust_mix(uint64_t, double, uint64_t, double);
extern "C" double rust_register_pressure(double, double, double, double, double,
                                        double, double, double, double, uint64_t);

struct Mixed {
    uint64_t integer;
    double floating;
};

extern "C" Mixed rust_aggregate(Mixed value);

extern "C" double cpp_mix(uint64_t first, double second, uint64_t third, double fourth) {
    return static_cast<double>(first) + second * 2.0 - static_cast<double>(third) + fourth * 4.0;
}

extern "C" Mixed cpp_aggregate(Mixed value) {
    return {value.integer + 7, value.floating * 2.0};
}

extern "C" uint64_t cpp_calls_rust() {
    if (rust_mix(19, 1.25, 7, 0.5) != 16.5)
        return 1;
    if (rust_register_pressure(1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10) != 55.0)
        return 2;
    const auto aggregate = rust_aggregate({11, 3.25});
    if (aggregate.integer != 18 || aggregate.floating != 6.5)
        return 3;
    return 0;
}
