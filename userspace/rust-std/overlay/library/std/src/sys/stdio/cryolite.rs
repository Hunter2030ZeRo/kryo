use crate::io::{self, IoSlice, IoSliceMut};

pub struct Stdin;
pub struct Stdout;
pub struct Stderr;

fn read(fd: i32, bytes: &mut [u8]) -> io::Result<usize> {
    let mut count = 0;
    let error = unsafe {
        libc::__cryolite_file_read(fd, bytes.as_mut_ptr().cast(), bytes.len(), &mut count)
    };
    if error == 0 {
        Ok(count as usize)
    } else {
        Err(io::Error::from_raw_os_error(error))
    }
}

fn write(fd: i32, bytes: &[u8]) -> io::Result<usize> {
    let mut count = 0;
    let error =
        unsafe { libc::__cryolite_file_write(fd, bytes.as_ptr().cast(), bytes.len(), &mut count) };
    if error == 0 {
        Ok(count as usize)
    } else {
        Err(io::Error::from_raw_os_error(error))
    }
}

impl Stdin {
    pub const fn new() -> Self {
        Self
    }
}
impl io::Read for Stdin {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        read(0, bytes)
    }
    fn read_vectored(&mut self, buffers: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        self.read(
            buffers
                .iter_mut()
                .find(|b| !b.is_empty())
                .map_or(&mut [], |b| &mut **b),
        )
    }
    fn is_read_vectored(&self) -> bool {
        false
    }
}

macro_rules! writer {
    ($name:ident, $fd:literal) => {
        impl $name {
            pub const fn new() -> Self {
                Self
            }
        }
        impl io::Write for $name {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                write($fd, bytes)
            }
            fn write_vectored(&mut self, buffers: &[IoSlice<'_>]) -> io::Result<usize> {
                self.write(buffers.iter().find(|b| !b.is_empty()).map_or(&[], |b| &**b))
            }
            fn is_write_vectored(&self) -> bool {
                false
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
    };
}
writer!(Stdout, 1);
writer!(Stderr, 2);
pub const STDIN_BUF_SIZE: usize = 1024;
pub fn is_ebadf(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::EBADF)
}
pub fn panic_output() -> Option<Stderr> {
    Some(Stderr::new())
}
