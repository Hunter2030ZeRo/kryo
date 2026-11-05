use crate::{fmt, io};

pub fn errno_location() -> *mut i32 {
    unsafe { libc::__errno_location() }
}
pub fn errno() -> i32 {
    unsafe { *errno_location() }
}
pub fn set_errno(code: i32) {
    unsafe {
        *errno_location() = code;
    }
}
pub fn is_interrupted(code: i32) -> bool {
    code == libc::EINTR
}

pub fn decode_error_kind(code: i32) -> io::ErrorKind {
    use io::ErrorKind::*;
    match code {
        libc::ENOENT => NotFound,
        libc::EACCES | libc::EPERM => PermissionDenied,
        libc::ECONNREFUSED => ConnectionRefused,
        libc::ECONNRESET => ConnectionReset,
        libc::ECONNABORTED => ConnectionAborted,
        libc::ENOTCONN => NotConnected,
        libc::EADDRINUSE => AddrInUse,
        libc::EADDRNOTAVAIL => AddrNotAvailable,
        libc::EPIPE => BrokenPipe,
        libc::EEXIST => AlreadyExists,
        libc::EAGAIN => WouldBlock,
        libc::EINVAL => InvalidInput,
        libc::ETIMEDOUT => TimedOut,
        libc::EINTR => Interrupted,
        libc::ENOMEM => OutOfMemory,
        libc::ENOTSUP | libc::ENOSYS => Unsupported,
        libc::ENOTDIR => NotADirectory,
        libc::EISDIR => IsADirectory,
        libc::ELOOP => FilesystemLoop,
        libc::ENOTEMPTY => DirectoryNotEmpty,
        libc::EROFS => ReadOnlyFilesystem,
        libc::ENOSPC => StorageFull,
        libc::ESPIPE => NotSeekable,
        libc::EIO => InputOutputError,
        libc::EOVERFLOW => InvalidData,
        libc::EBUSY => ResourceBusy,
        libc::EDEADLK => Deadlock,
        libc::EXDEV => CrossesDevices,
        libc::EMFILE | libc::ENFILE => TooManyOpenFiles,
        _ => Uncategorized,
    }
}

pub fn format_error(code: i32, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let description = match code {
        0 => "operation successful",
        libc::EBADF => "bad file descriptor",
        libc::EINVAL => "invalid argument",
        libc::EINTR => "interrupted operation",
        libc::EAGAIN => "operation would block",
        libc::ENOMEM => "out of memory",
        libc::ENOTSUP | libc::ENOSYS => "operation not supported",
        libc::ENOENT => "no such file or directory",
        libc::ELOOP => "too many symbolic links",
        libc::EACCES | libc::EPERM => "permission denied",
        libc::EIO => "input/output error",
        libc::ETIMEDOUT => "operation timed out",
        _ => "Cryolite error",
    };
    f.write_str(description)
}
