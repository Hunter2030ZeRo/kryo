#ifndef CRYOLITE_C_ABI_TYPES_H
#define CRYOLITE_C_ABI_TYPES_H

#if !defined(__x86_64__) || __CHAR_BIT__ != 8 \
    || __SIZEOF_SHORT__ != 2 || __SIZEOF_INT__ != 4 \
    || __SIZEOF_LONG__ != 8 || __SIZEOF_LONG_LONG__ != 8 \
    || __SIZEOF_POINTER__ != 8 || __BYTE_ORDER__ != __ORDER_LITTLE_ENDIAN__
#error Cryolite C-v1 requires little-endian x86-64 LP64.
#endif
#if defined(__CHAR_UNSIGNED__) || __SIZEOF_WCHAR_T__ != 4 \
    || defined(__WCHAR_UNSIGNED__)
#error Cryolite C-v1 requires signed plain char and 32-bit signed wchar_t.
#endif
#if __SIZEOF_FLOAT__ != 4 || __FLT_MANT_DIG__ != 24 \
    || __SIZEOF_DOUBLE__ != 8 || __DBL_MANT_DIG__ != 53
#error Cryolite C-v1 requires binary32 float and binary64 double.
#endif
#if __SIZEOF_LONG_DOUBLE__ != 16 || __LDBL_MANT_DIG__ != 64 \
    || __LDBL_MAX_EXP__ != 16384
#error Cryolite C-v1 requires the selected 80-bit long double format.
#endif

typedef __INT8_TYPE__ int8_t;
typedef __UINT8_TYPE__ uint8_t;
typedef __INT16_TYPE__ int16_t;
typedef __UINT16_TYPE__ uint16_t;
typedef __INT32_TYPE__ int32_t;
typedef __UINT32_TYPE__ uint32_t;
typedef __INT64_TYPE__ int64_t;
typedef __UINT64_TYPE__ uint64_t;
typedef __INTPTR_TYPE__ intptr_t;
typedef __UINTPTR_TYPE__ uintptr_t;
typedef __SIZE_TYPE__ size_t;
typedef __PTRDIFF_TYPE__ ptrdiff_t;
#ifndef __cplusplus
typedef __WCHAR_TYPE__ wchar_t;
#endif

typedef long ssize_t;
typedef long off_t;
typedef long time_t;
typedef long clock_t;
typedef long suseconds_t;
typedef int pid_t;
typedef int clockid_t;
typedef unsigned int uid_t;
typedef unsigned int gid_t;
typedef unsigned int mode_t;
typedef unsigned int id_t;
typedef unsigned long ino_t;
typedef unsigned long dev_t;
typedef unsigned long nlink_t;
typedef long blkcnt_t;
typedef long blksize_t;
typedef unsigned long fsblkcnt_t;
typedef unsigned long fsfilcnt_t;
typedef unsigned long nfds_t;
typedef void *timer_t;
typedef int sig_atomic_t;
typedef struct __cryolite_pthread_attr pthread_attr_t;

typedef struct {
    unsigned long __sig[1];
} sigset_t;

#endif
