//! Unix file options translated by the Cryolite runtime.
#![stable(feature = "rust1", since = "1.0.0")]

use crate::fs::{File, OpenOptions};
use crate::io;
use crate::sys::{AsInner, AsInnerMut};

/// Positional file I/O that preserves the shared open-description cursor.
#[stable(feature = "file_offset", since = "1.15.0")]
pub trait FileExt {
    /// Reads at an explicit offset, returning a possibly short completion.
    #[stable(feature = "file_offset", since = "1.15.0")]
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize>;

    /// Fills the buffer from an explicit offset or reports premature EOF.
    #[stable(feature = "rw_exact_all_at", since = "1.33.0")]
    fn read_exact_at(&self, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
        while !buf.is_empty() {
            match self.read_at(buf, offset) {
                Ok(0) => break,
                Ok(n) => {
                    let remaining = buf;
                    buf = &mut remaining[n..];
                    offset += n as u64;
                }
                Err(ref error) if error.is_interrupted() => {}
                Err(error) => return Err(error),
            }
        }
        if buf.is_empty() {
            Ok(())
        } else {
            Err(io::Error::READ_EXACT_EOF)
        }
    }

    /// Writes at an explicit offset without moving the current file cursor.
    /// Append status does not override the offset; gaps are zero-filled.
    #[stable(feature = "file_offset", since = "1.15.0")]
    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<usize>;

    /// Writes the whole buffer using successive positional completions.
    #[stable(feature = "rw_exact_all_at", since = "1.33.0")]
    fn write_all_at(&self, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
        while !buf.is_empty() {
            match self.write_at(buf, offset) {
                Ok(0) => return Err(io::Error::WRITE_ALL_EOF),
                Ok(n) => {
                    buf = &buf[n..];
                    offset += n as u64;
                }
                Err(ref error) if error.is_interrupted() => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

#[stable(feature = "file_offset", since = "1.15.0")]
impl FileExt for File {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
        self.as_inner().read_at(buf, offset)
    }

    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<usize> {
        self.as_inner().write_at(buf, offset)
    }
}

#[stable(feature = "fs_ext", since = "1.1.0")]
pub trait OpenOptionsExt {
    /// Sets creation mode bits passed to the namespace service.
    /// Permission enforcement belongs to that service; this profile has no umask contract.
    #[stable(feature = "fs_ext", since = "1.1.0")]
    fn mode(&mut self, mode: u32) -> &mut Self;

    /// Adds Cryolite C open flags, excluding access-mode bits.
    /// Unsupported or unknown flags return an error when the file is opened.
    #[stable(feature = "fs_ext", since = "1.1.0")]
    fn custom_flags(&mut self, flags: i32) -> &mut Self;
}

#[stable(feature = "fs_ext", since = "1.1.0")]
impl OpenOptionsExt for OpenOptions {
    fn mode(&mut self, mode: u32) -> &mut Self {
        self.as_inner_mut().mode(mode);
        self
    }

    fn custom_flags(&mut self, flags: i32) -> &mut Self {
        self.as_inner_mut().custom_flags(flags);
        self
    }
}
