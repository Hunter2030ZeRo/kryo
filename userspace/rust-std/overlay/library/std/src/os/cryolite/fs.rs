//! Unix file options translated by the Cryolite runtime.
#![stable(feature = "rust1", since = "1.0.0")]

use crate::fs::OpenOptions;
use crate::sys::AsInnerMut;

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
