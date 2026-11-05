//! Capability-backed Cryolite file descriptor ownership.
#![stable(feature = "os_fd", since = "1.66.0")]
use crate::{fmt, io, marker::PhantomData, mem};

#[stable(feature = "rust1", since = "1.0.0")]
pub type RawFd = i32;
type ValidRawFd = core::num::niche_types::NotAllOnes<RawFd>;
#[stable(feature = "rust1", since = "1.0.0")]
pub trait AsRawFd {
    #[stable(feature = "rust1", since = "1.0.0")]
    fn as_raw_fd(&self) -> RawFd;
}
#[stable(feature = "from_raw_os", since = "1.1.0")]
pub trait FromRawFd {
    /// Takes ownership of a descriptor in the Cryolite runtime file table.
    ///
    /// # Safety
    /// The descriptor must be open, require only the runtime close operation for
    /// cleanup, and have no other owner that can close it.
    #[stable(feature = "from_raw_os", since = "1.1.0")]
    unsafe fn from_raw_fd(fd: RawFd) -> Self;
}
#[stable(feature = "into_raw_os", since = "1.4.0")]
pub trait IntoRawFd {
    #[stable(feature = "into_raw_os", since = "1.4.0")]
    fn into_raw_fd(self) -> RawFd;
}
#[stable(feature = "io_safety", since = "1.63.0")]
pub trait AsFd {
    #[stable(feature = "io_safety", since = "1.63.0")]
    fn as_fd(&self) -> BorrowedFd<'_>;
}
#[repr(transparent)]
#[derive(Copy, Clone)]
#[rustc_nonnull_optimization_guaranteed]
#[stable(feature = "io_safety", since = "1.63.0")]
pub struct BorrowedFd<'fd> {
    fd: ValidRawFd,
    lifetime: PhantomData<&'fd OwnedFd>,
}
#[repr(transparent)]
#[rustc_nonnull_optimization_guaranteed]
#[stable(feature = "io_safety", since = "1.63.0")]
pub struct OwnedFd {
    fd: ValidRawFd,
}

const _: () = {
    assert!(mem::size_of::<OwnedFd>() == mem::size_of::<RawFd>());
    assert!(mem::size_of::<Option<OwnedFd>>() == mem::size_of::<RawFd>());
    assert!(mem::size_of::<BorrowedFd<'static>>() == mem::size_of::<RawFd>());
    assert!(mem::size_of::<Option<BorrowedFd<'static>>>() == mem::size_of::<RawFd>());
};

impl BorrowedFd<'_> {
    /// Borrows a descriptor from the Cryolite runtime file table.
    ///
    /// # Safety
    /// The descriptor must remain open for the returned borrow's lifetime.
    #[stable(feature = "io_safety", since = "1.63.0")]
    #[rustc_const_stable(feature = "io_safety", since = "1.63.0")]
    pub const unsafe fn borrow_raw(fd: RawFd) -> Self {
        assert!(fd >= 0, "invalid Cryolite file descriptor");
        Self {
            fd: ValidRawFd::new(fd).expect("fd != -1"),
            lifetime: PhantomData,
        }
    }
    #[stable(feature = "io_safety", since = "1.63.0")]
    pub fn try_clone_to_owned(&self) -> io::Result<OwnedFd> {
        let mut fd = -1;
        let error =
            unsafe { libc::__cryolite_file_dup(self.fd.as_inner(), 3, libc::FD_CLOEXEC, &mut fd) };
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}
impl OwnedFd {
    #[stable(feature = "io_safety", since = "1.63.0")]
    pub fn try_clone(&self) -> io::Result<Self> {
        self.as_fd().try_clone_to_owned()
    }
}
#[stable(feature = "io_safety", since = "1.63.0")]
impl Drop for OwnedFd {
    fn drop(&mut self) {
        // Close consumes the reference even when the service reports a close error.
        let _ = unsafe { libc::__cryolite_file_close(self.fd.as_inner()) };
    }
}
#[stable(feature = "io_safety", since = "1.63.0")]
impl AsRawFd for OwnedFd {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_inner()
    }
}
#[stable(feature = "io_safety", since = "1.63.0")]
impl AsRawFd for BorrowedFd<'_> {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_inner()
    }
}
#[stable(feature = "io_safety", since = "1.63.0")]
impl AsFd for OwnedFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(self.fd.as_inner()) }
    }
}
#[stable(feature = "io_safety", since = "1.63.0")]
impl AsFd for BorrowedFd<'_> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        *self
    }
}
#[stable(feature = "io_safety", since = "1.63.0")]
impl FromRawFd for OwnedFd {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        assert!(fd >= 0, "invalid Cryolite file descriptor");
        Self {
            fd: ValidRawFd::new(fd).expect("fd != -1"),
        }
    }
}
#[stable(feature = "io_safety", since = "1.63.0")]
impl IntoRawFd for OwnedFd {
    fn into_raw_fd(self) -> RawFd {
        let fd = self.fd.as_inner();
        mem::forget(self);
        fd
    }
}
#[stable(feature = "io_safety", since = "1.63.0")]
impl fmt::Debug for OwnedFd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnedFd").field("fd", &self.fd).finish()
    }
}
#[stable(feature = "io_safety", since = "1.63.0")]
impl fmt::Debug for BorrowedFd<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BorrowedFd").field("fd", &self.fd).finish()
    }
}
#[stable(feature = "raw_fd_reflexive_traits", since = "1.48.0")]
impl AsRawFd for RawFd {
    fn as_raw_fd(&self) -> RawFd {
        *self
    }
}
#[stable(feature = "raw_fd_reflexive_traits", since = "1.48.0")]
impl IntoRawFd for RawFd {
    fn into_raw_fd(self) -> RawFd {
        self
    }
}
#[stable(feature = "raw_fd_reflexive_traits", since = "1.48.0")]
impl FromRawFd for RawFd {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        fd
    }
}
#[stable(feature = "io_safety", since = "1.63.0")]
impl<T: AsFd + ?Sized> AsFd for &T {
    fn as_fd(&self) -> BorrowedFd<'_> {
        T::as_fd(self)
    }
}
#[stable(feature = "io_safety", since = "1.63.0")]
impl<T: AsFd + ?Sized> AsFd for &mut T {
    fn as_fd(&self) -> BorrowedFd<'_> {
        T::as_fd(self)
    }
}
#[stable(feature = "io_safety", since = "1.63.0")]
impl<T: AsRawFd + ?Sized> AsRawFd for &T {
    fn as_raw_fd(&self) -> RawFd {
        T::as_raw_fd(self)
    }
}
#[stable(feature = "io_safety", since = "1.63.0")]
impl<T: AsRawFd + ?Sized> AsRawFd for &mut T {
    fn as_raw_fd(&self) -> RawFd {
        T::as_raw_fd(self)
    }
}

macro_rules! stdio_fd {
    ($ty:ty, $fd:literal) => {
        #[stable(feature = "rust1", since = "1.0.0")]
        impl AsRawFd for $ty {
            fn as_raw_fd(&self) -> RawFd {
                $fd
            }
        }
    };
}
stdio_fd!(io::Stdin, 0);
stdio_fd!(io::StdinLock<'_>, 0);
stdio_fd!(io::Stdout, 1);
stdio_fd!(io::StdoutLock<'_>, 1);
stdio_fd!(io::Stderr, 2);
stdio_fd!(io::StderrLock<'_>, 2);
