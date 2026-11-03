/* Candidate layout checks; this is not an installed public ABI header. */
typedef __SIZE_TYPE__ draft_size_t;
typedef __PTRDIFF_TYPE__ draft_ptrdiff_t;
typedef __INT64_TYPE__ draft_time_t;
typedef __INT32_TYPE__ draft_pid_t;
typedef __UINT64_TYPE__ draft_u64;
typedef __UINT32_TYPE__ draft_u32;

struct draft_timespec {
    draft_time_t tv_sec;
    long tv_nsec;
};

struct native_buffer {
    draft_u64 offset;
    draft_u32 length;
    draft_u32 flags;
};

#ifdef __cplusplus
#define CHECK static_assert
#define ALIGNOF alignof
CHECK(__is_same(draft_size_t, unsigned long), "size_t type identity");
CHECK(__is_same(draft_ptrdiff_t, long), "ptrdiff_t type identity");
CHECK(sizeof(bool) == 1 && ALIGNOF(bool) == 1, "boolean");
CHECK(sizeof(wchar_t) == 4 && (wchar_t)-1 < 0, "C++ wchar_t");
#else
#define CHECK _Static_assert
#define ALIGNOF _Alignof
CHECK(sizeof(_Bool) == 1 && ALIGNOF(_Bool) == 1, "boolean");
CHECK(__builtin_types_compatible_p(draft_size_t, unsigned long), "size_t type identity");
CHECK(__builtin_types_compatible_p(draft_ptrdiff_t, long), "ptrdiff_t type identity");
#endif

CHECK(__CHAR_BIT__ == 8, "byte");
CHECK(__BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__, "byte order");
CHECK((char)-1 < 0, "plain char");
CHECK(sizeof(short) == 2 && sizeof(int) == 4, "small integers");
CHECK(sizeof(long) == 8 && sizeof(long long) == 8, "LP64 integers");
CHECK(sizeof(void *) == 8 && ALIGNOF(void *) == 8, "pointer");
CHECK(sizeof(draft_size_t) == 8 && (draft_size_t)-1 > 0, "size_t");
CHECK(sizeof(draft_ptrdiff_t) == 8 && (draft_ptrdiff_t)-1 < 0, "ptrdiff_t");
CHECK(sizeof(draft_pid_t) == 4 && (draft_pid_t)-1 < 0, "signed PID");
CHECK(sizeof(__WCHAR_TYPE__) == 4 && (__WCHAR_TYPE__)-1 < 0, "wchar_t");
CHECK(sizeof(float) == 4 && sizeof(double) == 8, "float storage");
CHECK(__FLT_RADIX__ == 2 && __FLT_MANT_DIG__ == 24, "binary32 precision");
CHECK(__FLT_MIN_EXP__ == -125 && __FLT_MAX_EXP__ == 128, "binary32 exponent");
CHECK(__DBL_MANT_DIG__ == 53, "binary64 precision");
CHECK(__DBL_MIN_EXP__ == -1021 && __DBL_MAX_EXP__ == 1024, "binary64 exponent");
CHECK(sizeof(long double) == 16 && ALIGNOF(long double) == 16, "long double storage");
CHECK(__LDBL_MANT_DIG__ == 64, "x87 precision");
CHECK(__LDBL_MIN_EXP__ == -16381 && __LDBL_MAX_EXP__ == 16384, "x87 exponent");
CHECK(sizeof(__int128) == 16 && ALIGNOF(__int128) == 16, "extended integer");
CHECK(sizeof(struct draft_timespec) == 16, "timespec size");
CHECK(ALIGNOF(struct draft_timespec) == 8, "timespec alignment");
CHECK(__builtin_offsetof(struct draft_timespec, tv_sec) == 0, "seconds offset");
CHECK(__builtin_offsetof(struct draft_timespec, tv_nsec) == 8, "nanoseconds offset");
CHECK(sizeof(struct native_buffer) == 16 && ALIGNOF(struct native_buffer) == 8, "buffer layout");
CHECK(__builtin_offsetof(struct native_buffer, offset) == 0, "buffer offset");
CHECK(__builtin_offsetof(struct native_buffer, length) == 8, "buffer length");
CHECK(__builtin_offsetof(struct native_buffer, flags) == 12, "buffer flags");
