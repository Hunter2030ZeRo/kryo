//! Cryolite C-v1 LP64 bindings used by the experimental Rust runtime.
//! This module deliberately does not import a Linux or generic POSIX ABI.
use crate::prelude::*;
use crate::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
pub type size_t = usize;
pub type ssize_t = c_long;
pub type ptrdiff_t = isize;
pub type intptr_t = isize;
pub type uintptr_t = usize;
pub type wchar_t = c_int;
pub type off_t = c_long;
pub type time_t = c_long;
pub type suseconds_t = c_long;
pub type mode_t = c_uint;
pub type pid_t = c_int;
pub type uid_t = c_uint;
pub type gid_t = c_uint;
pub type dev_t = c_ulong;
pub type ino_t = c_ulong;
pub type nlink_t = c_ulong;
pub type blkcnt_t = c_long;
pub type blksize_t = c_long;
pub const STDIN_FILENO: c_int = 0;
pub const STDOUT_FILENO: c_int = 1;
pub const STDERR_FILENO: c_int = 2;
pub const FD_CLOEXEC: c_int = 1;
pub const O_RDONLY: c_int = 0x00000;
pub const O_WRONLY: c_int = 0x00001;
pub const O_RDWR: c_int = 0x00002;
pub const O_ACCMODE: c_int = 0x0000f;
pub const O_CREAT: c_int = 0x00010;
pub const O_EXCL: c_int = 0x00020;
pub const O_TRUNC: c_int = 0x00040;
pub const O_APPEND: c_int = 0x00080;
pub const O_CLOEXEC: c_int = 0x00800;
pub const SEEK_SET: c_int = 0;
pub const SEEK_CUR: c_int = 1;
pub const SEEK_END: c_int = 2;
pub const E2BIG: c_int = 1;
pub const EACCES: c_int = 2;
pub const EADDRINUSE: c_int = 3;
pub const EADDRNOTAVAIL: c_int = 4;
pub const EAFNOSUPPORT: c_int = 5;
pub const EAGAIN: c_int = 6;
pub const EALREADY: c_int = 7;
pub const EBADF: c_int = 8;
pub const EBADMSG: c_int = 9;
pub const EBUSY: c_int = 10;
pub const ECANCELED: c_int = 11;
pub const ECHILD: c_int = 12;
pub const ECONNABORTED: c_int = 13;
pub const ECONNREFUSED: c_int = 14;
pub const ECONNRESET: c_int = 15;
pub const EDEADLK: c_int = 16;
pub const EDESTADDRREQ: c_int = 17;
pub const EDOM: c_int = 18;
pub const EDQUOT: c_int = 19;
pub const EEXIST: c_int = 20;
pub const EFAULT: c_int = 21;
pub const EFBIG: c_int = 22;
pub const EHOSTUNREACH: c_int = 23;
pub const EIDRM: c_int = 24;
pub const EILSEQ: c_int = 25;
pub const EINPROGRESS: c_int = 26;
pub const EINTR: c_int = 27;
pub const EINVAL: c_int = 28;
pub const EIO: c_int = 29;
pub const EISCONN: c_int = 30;
pub const EISDIR: c_int = 31;
pub const ELOOP: c_int = 32;
pub const EMFILE: c_int = 33;
pub const EMLINK: c_int = 34;
pub const EMSGSIZE: c_int = 35;
pub const EMULTIHOP: c_int = 36;
pub const ENAMETOOLONG: c_int = 37;
pub const ENETDOWN: c_int = 38;
pub const ENETRESET: c_int = 39;
pub const ENETUNREACH: c_int = 40;
pub const ENFILE: c_int = 41;
pub const ENOBUFS: c_int = 42;
pub const ENODATA: c_int = 43;
pub const ENODEV: c_int = 44;
pub const ENOENT: c_int = 45;
pub const ENOEXEC: c_int = 46;
pub const ENOLCK: c_int = 47;
pub const ENOLINK: c_int = 48;
pub const ENOMEM: c_int = 49;
pub const ENOMSG: c_int = 50;
pub const ENOPROTOOPT: c_int = 51;
pub const ENOSPC: c_int = 52;
pub const ENOSR: c_int = 53;
pub const ENOSTR: c_int = 54;
pub const ENOSYS: c_int = 55;
pub const ENOTCONN: c_int = 56;
pub const ENOTDIR: c_int = 57;
pub const ENOTEMPTY: c_int = 58;
pub const ENOTRECOVERABLE: c_int = 59;
pub const ENOTSOCK: c_int = 60;
pub const ENOTSUP: c_int = 61;
pub const ENOTTY: c_int = 62;
pub const ENXIO: c_int = 63;
pub const EOVERFLOW: c_int = 64;
pub const EOWNERDEAD: c_int = 65;
pub const EPERM: c_int = 66;
pub const EPIPE: c_int = 67;
pub const EPROTO: c_int = 68;
pub const EPROTONOSUPPORT: c_int = 69;
pub const EPROTOTYPE: c_int = 70;
pub const ERANGE: c_int = 71;
pub const EROFS: c_int = 72;
pub const ESPIPE: c_int = 73;
pub const ESRCH: c_int = 74;
pub const ESTALE: c_int = 75;
pub const ETIME: c_int = 76;
pub const ETIMEDOUT: c_int = 77;
pub const ETXTBSY: c_int = 78;
pub const EXDEV: c_int = 79;
pub const EWOULDBLOCK: c_int = EAGAIN;
pub const EOPNOTSUPP: c_int = ENOTSUP;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct timespec {
    pub tv_sec: time_t,
    pub tv_nsec: c_long,
}
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct timeval {
    pub tv_sec: time_t,
    pub tv_usec: suseconds_t,
}
extern "C" {
    pub fn malloc(size: size_t) -> *mut c_void;
    pub fn calloc(count: size_t, size: size_t) -> *mut c_void;
    pub fn realloc(ptr: *mut c_void, size: size_t) -> *mut c_void;
    pub fn aligned_alloc(alignment: size_t, size: size_t) -> *mut c_void;
    pub fn free(ptr: *mut c_void);
    pub fn abort() -> !;
    pub fn exit(status: c_int) -> !;
    pub fn strlen(value: *const c_char) -> size_t;
    pub fn memchr(ptr: *const c_void, value: c_int, size: size_t) -> *mut c_void;
    pub fn memcmp(a: *const c_void, b: *const c_void, size: size_t) -> c_int;
    pub fn memcpy(dst: *mut c_void, src: *const c_void, size: size_t) -> *mut c_void;
    pub fn memmove(dst: *mut c_void, src: *const c_void, size: size_t) -> *mut c_void;
    pub fn memset(dst: *mut c_void, value: c_int, size: size_t) -> *mut c_void;
    pub fn __errno_location() -> *mut c_int;
    pub fn __cryolite_file_open(
        path: *const c_char,
        flags: c_int,
        mode: mode_t,
        output: *mut c_int,
    ) -> c_int;
    pub fn __cryolite_file_read(
        fd: c_int,
        buffer: *mut c_void,
        size: size_t,
        count: *mut ssize_t,
    ) -> c_int;
    pub fn __cryolite_file_write(
        fd: c_int,
        buffer: *const c_void,
        size: size_t,
        count: *mut ssize_t,
    ) -> c_int;
    pub fn __cryolite_file_close(fd: c_int) -> c_int;
    pub fn __cryolite_file_seek(
        fd: c_int,
        offset: off_t,
        whence: c_int,
        output: *mut off_t,
    ) -> c_int;
    pub fn __cryolite_file_dup(
        fd: c_int,
        minimum: c_int,
        flags: c_int,
        output: *mut c_int,
    ) -> c_int;
    pub fn __cryolite_file_isatty(fd: c_int) -> c_int;
}
