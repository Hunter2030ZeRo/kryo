/* Code-generation evidence only; not a library implementation or benchmark. */
#if !defined(__AVX2__) || !defined(__BMI__) || !defined(__BMI2__) \
    || !defined(__FMA__) || !defined(__F16C__) || !defined(__LZCNT__) \
    || !defined(__POPCNT__) || !defined(__MOVBE__)
#error The native compiler profile must expose the x86-64-v3 features.
#endif
#ifdef __MMX__
#error Native code generation must disable MMX.
#endif

typedef unsigned int vector_u32 __attribute__((vector_size(32)));

double add_double(double left, double right) {
    return left + right;
}

long double add_extended(long double left, long double right) {
    return left + right;
}

double fused_multiply_add(double left, double right, double addend) {
    return __builtin_elementwise_fma(left, right, addend);
}

vector_u32 add_vector(vector_u32 left, vector_u32 right) {
    return left + right;
}

unsigned long leading_zeros(unsigned long value) {
    return __builtin_clzl(value);
}

unsigned long low_bits(unsigned long value, unsigned int count) {
    return __builtin_ia32_bzhi_di(value, count);
}
