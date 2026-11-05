use crate::ffi::{CString, OsString};
use crate::fs::TryLockError;
use crate::io::{self, BorrowedCursor, IoSlice, IoSliceMut, SeekFrom};
use crate::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use crate::os::unix::ffi::OsStrExt;
use crate::path::{Path, PathBuf};
use crate::sys::time::SystemTime;
use crate::sys::{FromInner, IntoInner, unsupported, unsupported_err};

pub use super::unsupported::{
    Dir, DirBuilder, ExtraHomeDirs, ExtraMediaDirs, FileTimes, canonicalize, copy, exists, link,
    readlink, remove_dir_all, rename, rename_noreplace, rmdir, set_times, set_times_nofollow,
    symlink, unlink,
};

#[derive(Clone, Debug)]
pub struct FileAttr {
    size: u64,
    file_type: FileType,
    permissions: FilePermissions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileType {
    Regular,
    Directory,
    Symlink,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilePermissions(bool);

impl FileAttr {
    fn decode(info: libc::cryolite_file_info) -> io::Result<Self> {
        let file_type = match info.kind {
            libc::KRYO_FILE_REGULAR => FileType::Regular,
            libc::KRYO_FILE_DIRECTORY => FileType::Directory,
            libc::KRYO_FILE_SYMLINK => FileType::Symlink,
            _ => return Err(invalid_metadata()),
        };
        if info.size > i64::MAX as u64 || info.flags & !libc::KRYO_FILE_READONLY != 0 {
            return Err(invalid_metadata());
        }
        Ok(Self {
            size: info.size,
            file_type,
            permissions: FilePermissions(info.flags & libc::KRYO_FILE_READONLY != 0),
        })
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn perm(&self) -> FilePermissions {
        self.permissions.clone()
    }

    pub fn file_type(&self) -> FileType {
        self.file_type
    }

    pub fn modified(&self) -> io::Result<SystemTime> {
        unsupported()
    }

    pub fn accessed(&self) -> io::Result<SystemTime> {
        unsupported()
    }

    pub fn created(&self) -> io::Result<SystemTime> {
        unsupported()
    }
}

fn invalid_metadata() -> io::Error {
    io::const_error!(io::ErrorKind::InvalidData, "invalid native file metadata")
}

impl FileType {
    pub fn is_dir(&self) -> bool {
        *self == Self::Directory
    }

    pub fn is_file(&self) -> bool {
        *self == Self::Regular
    }

    pub fn is_symlink(&self) -> bool {
        *self == Self::Symlink
    }
}

impl FilePermissions {
    pub fn readonly(&self) -> bool {
        self.0
    }

    pub fn set_readonly(&mut self, readonly: bool) {
        self.0 = readonly;
    }
}

#[derive(Debug)]
pub struct ReadDir(!);

#[derive(Debug)]
pub struct DirEntry(!);

impl Drop for ReadDir {
    fn drop(&mut self) {
        self.0
    }
}

impl Drop for DirEntry {
    fn drop(&mut self) {
        self.0
    }
}

impl ReadDir {
    pub fn dir(&self) -> io::Result<Dir> {
        self.0
    }
}

impl Iterator for ReadDir {
    type Item = io::Result<DirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0
    }
}

impl DirEntry {
    pub fn path(&self) -> PathBuf {
        self.0
    }

    pub fn file_name(&self) -> OsString {
        self.0
    }

    pub fn metadata(&self) -> io::Result<FileAttr> {
        self.0
    }

    pub fn file_type(&self) -> io::Result<FileType> {
        self.0
    }
}

pub fn readdir(_path: &Path) -> io::Result<ReadDir> {
    unsupported()
}

pub fn stat(_path: &Path) -> io::Result<FileAttr> {
    unsupported()
}

pub fn lstat(_path: &Path) -> io::Result<FileAttr> {
    unsupported()
}

pub fn set_perm(_path: &Path, _permissions: FilePermissions) -> io::Result<()> {
    unsupported()
}

pub fn set_perm_nofollow(_path: &Path, _permissions: FilePermissions) -> io::Result<()> {
    unsupported()
}

#[derive(Debug)]
pub struct File(OwnedFd);

#[derive(Clone, Debug)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
    mode: u32,
    custom_flags: i32,
}

impl OpenOptions {
    pub fn new() -> Self {
        Self {
            read: false,
            write: false,
            append: false,
            truncate: false,
            create: false,
            create_new: false,
            mode: 0o666,
            custom_flags: 0,
        }
    }

    pub fn read(&mut self, read: bool) {
        self.read = read;
    }

    pub fn write(&mut self, write: bool) {
        self.write = write;
    }

    pub fn append(&mut self, append: bool) {
        self.append = append;
    }

    pub fn truncate(&mut self, truncate: bool) {
        self.truncate = truncate;
    }

    pub fn create(&mut self, create: bool) {
        self.create = create;
    }

    pub fn create_new(&mut self, create_new: bool) {
        self.create_new = create_new;
    }

    pub fn mode(&mut self, mode: u32) {
        self.mode = mode;
    }

    pub fn custom_flags(&mut self, flags: i32) {
        self.custom_flags = flags;
    }

    fn flags(&self) -> io::Result<i32> {
        let access = match (self.read, self.write, self.append) {
            (true, false, false) => libc::O_RDONLY,
            (false, true, false) => libc::O_WRONLY,
            (true, true, false) => libc::O_RDWR,
            (false, _, true) => libc::O_WRONLY | libc::O_APPEND,
            (true, _, true) => libc::O_RDWR | libc::O_APPEND,
            (false, false, false) => {
                return Err(io::const_error!(
                    io::ErrorKind::InvalidInput,
                    "must specify read, write, or append access",
                ));
            }
        };
        if !self.write && !self.append && (self.truncate || self.create || self.create_new) {
            return Err(io::const_error!(
                io::ErrorKind::InvalidInput,
                "creating or truncating a file requires write or append access",
            ));
        }
        if self.append && self.truncate && !self.create_new {
            return Err(io::const_error!(
                io::ErrorKind::InvalidInput,
                "append and truncate cannot both be enabled",
            ));
        }
        let creation = if self.create_new {
            libc::O_CREAT | libc::O_EXCL
        } else {
            (if self.create { libc::O_CREAT } else { 0 })
                | (if self.truncate { libc::O_TRUNC } else { 0 })
        };
        Ok(libc::O_CLOEXEC | access | creation | (self.custom_flags & !libc::O_ACCMODE))
    }
}

fn native_path(path: &Path) -> io::Result<CString> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() > 4096 {
        return Err(io::Error::from_raw_os_error(libc::ENAMETOOLONG));
    }
    if bytes.contains(&0) {
        return Err(io::const_error!(
            io::ErrorKind::InvalidInput,
            "path contains NUL"
        ));
    }
    let mut terminated = Vec::new();
    terminated
        .try_reserve_exact(bytes.len() + 1)
        .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
    terminated.extend_from_slice(bytes);
    terminated.push(0);
    // The bounded byte path was checked for interior NUL before appending its terminator.
    Ok(unsafe { CString::from_vec_with_nul_unchecked(terminated) })
}

fn completed(error: i32, count: libc::ssize_t, requested: usize) -> io::Result<usize> {
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error));
    }
    usize::try_from(count)
        .ok()
        .filter(|&count| count <= requested)
        .ok_or(io::const_error!(
            io::ErrorKind::InvalidData,
            "invalid native file completion count",
        ))
}

fn file_offset(offset: u64) -> io::Result<libc::off_t> {
    i64::try_from(offset).map_err(|_| {
        io::const_error!(
            io::ErrorKind::InvalidInput,
            "file offset exceeds signed 64-bit range"
        )
    })
}

fn completed_at(
    error: i32,
    count: libc::ssize_t,
    requested: usize,
    offset: u64,
) -> io::Result<usize> {
    let count = completed(error, count, requested)?;
    if offset
        .checked_add(count as u64)
        .is_none_or(|end| end > i64::MAX as u64)
    {
        return Err(io::const_error!(
            io::ErrorKind::InvalidData,
            "invalid native positioned file completion",
        ));
    }
    Ok(count)
}

fn status(error: i32) -> io::Result<()> {
    if error == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(error))
    }
}

impl File {
    pub fn open(path: &Path, options: &OpenOptions) -> io::Result<Self> {
        let flags = options.flags()?;
        let path = native_path(path)?;
        let mut fd = -1;
        let error =
            unsafe { libc::__cryolite_file_open(path.as_ptr(), flags, options.mode, &mut fd) };
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        if fd < 0 {
            return Err(io::const_error!(
                io::ErrorKind::InvalidData,
                "invalid native file descriptor",
            ));
        }
        Ok(unsafe { Self::from_raw_fd(fd) })
    }

    pub fn file_attr(&self) -> io::Result<FileAttr> {
        let mut info = libc::cryolite_file_info {
            size: 0,
            kind: 0,
            flags: 0,
        };
        status(unsafe { libc::__cryolite_file_info(self.as_raw_fd(), &mut info) })?;
        FileAttr::decode(info)
    }

    pub fn fsync(&self) -> io::Result<()> {
        status(unsafe { libc::__cryolite_file_sync(self.as_raw_fd(), libc::KRYO_FILE_SYNC_ALL) })
    }

    pub fn datasync(&self) -> io::Result<()> {
        status(unsafe { libc::__cryolite_file_sync(self.as_raw_fd(), libc::KRYO_FILE_SYNC_DATA) })
    }

    pub fn lock(&self) -> io::Result<()> {
        unsupported()
    }

    pub fn lock_shared(&self) -> io::Result<()> {
        unsupported()
    }

    pub fn try_lock(&self) -> Result<(), TryLockError> {
        Err(TryLockError::Error(unsupported_err()))
    }

    pub fn try_lock_shared(&self) -> Result<(), TryLockError> {
        self.try_lock()
    }

    pub fn unlock(&self) -> io::Result<()> {
        unsupported()
    }

    pub fn truncate(&self, size: u64) -> io::Result<()> {
        let size = file_offset(size)?;
        status(unsafe { libc::__cryolite_file_resize(self.as_raw_fd(), size) })
    }

    pub fn read(&self, bytes: &mut [u8]) -> io::Result<usize> {
        let mut count = 0;
        let error = unsafe {
            libc::__cryolite_file_read(
                self.as_raw_fd(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                &mut count,
            )
        };
        completed(error, count, bytes.len())
    }

    pub fn read_vectored(&self, buffers: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        io::default_read_vectored(|bytes| self.read(bytes), buffers)
    }

    pub fn is_read_vectored(&self) -> bool {
        false
    }

    pub fn read_at(&self, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
        let position = file_offset(offset)?;
        let mut count = 0;
        let error = unsafe {
            libc::__cryolite_file_pread(
                self.as_raw_fd(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                position,
                &mut count,
            )
        };
        completed_at(error, count, bytes.len(), offset)
    }

    pub fn read_buf(&self, cursor: BorrowedCursor<'_, u8>) -> io::Result<()> {
        io::default_read_buf(|bytes| self.read(bytes), cursor)
    }

    pub fn write(&self, bytes: &[u8]) -> io::Result<usize> {
        let mut count = 0;
        let error = unsafe {
            libc::__cryolite_file_write(
                self.as_raw_fd(),
                bytes.as_ptr().cast(),
                bytes.len(),
                &mut count,
            )
        };
        completed(error, count, bytes.len())
    }

    pub fn write_vectored(&self, buffers: &[IoSlice<'_>]) -> io::Result<usize> {
        io::default_write_vectored(|bytes| self.write(bytes), buffers)
    }

    pub fn is_write_vectored(&self) -> bool {
        false
    }

    pub fn write_at(&self, bytes: &[u8], offset: u64) -> io::Result<usize> {
        let position = file_offset(offset)?;
        let mut count = 0;
        let error = unsafe {
            libc::__cryolite_file_pwrite(
                self.as_raw_fd(),
                bytes.as_ptr().cast(),
                bytes.len(),
                position,
                &mut count,
            )
        };
        completed_at(error, count, bytes.len(), offset)
    }

    pub fn flush(&self) -> io::Result<()> {
        // File has no runtime buffer; storage durability requires a separate service operation.
        Ok(())
    }

    pub fn seek(&self, position: SeekFrom) -> io::Result<u64> {
        let (offset, whence) = match position {
            SeekFrom::Start(offset) => (file_offset(offset)?, libc::SEEK_SET),
            SeekFrom::Current(offset) => (offset, libc::SEEK_CUR),
            SeekFrom::End(offset) => (offset, libc::SEEK_END),
        };
        let mut actual = 0;
        let error =
            unsafe { libc::__cryolite_file_seek(self.as_raw_fd(), offset, whence, &mut actual) };
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        u64::try_from(actual).map_err(|_| {
            io::const_error!(io::ErrorKind::InvalidData, "negative native file position")
        })
    }

    pub fn size(&self) -> Option<io::Result<u64>> {
        Some(self.file_attr().map(|attributes| attributes.size()))
    }

    pub fn tell(&self) -> io::Result<u64> {
        self.seek(SeekFrom::Current(0))
    }

    pub fn duplicate(&self) -> io::Result<Self> {
        self.0.try_clone().map(Self)
    }

    pub fn set_permissions(&self, _permissions: FilePermissions) -> io::Result<()> {
        unsupported()
    }

    pub fn set_times(&self, _times: FileTimes) -> io::Result<()> {
        unsupported()
    }
}

impl AsRawFd for File {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl AsFd for File {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl FromRawFd for File {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

impl IntoRawFd for File {
    fn into_raw_fd(self) -> RawFd {
        self.0.into_raw_fd()
    }
}

impl FromInner<OwnedFd> for File {
    fn from_inner(fd: OwnedFd) -> Self {
        Self(fd)
    }
}

impl IntoInner<OwnedFd> for File {
    fn into_inner(self) -> OwnedFd {
        self.0
    }
}
