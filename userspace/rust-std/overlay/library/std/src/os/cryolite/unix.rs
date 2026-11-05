//! Unix API extensions supported by the experimental Cryolite runtime.
#![stable(feature = "rust1", since = "1.0.0")]
#[path = "../unix/ffi/mod.rs"]
pub mod ffi;
pub mod fs;
#[stable(feature = "rust1", since = "1.0.0")]
pub mod io {
    #[stable(feature = "rust1", since = "1.0.0")]
    pub use crate::os::fd::*;
}
#[stable(feature = "rust1", since = "1.0.0")]
pub mod prelude {
    #[stable(feature = "rust1", since = "1.0.0")]
    pub use super::ffi::{OsStrExt, OsStringExt};
    #[stable(feature = "rust1", since = "1.0.0")]
    pub use super::fs::OpenOptionsExt;
    #[stable(feature = "rust1", since = "1.0.0")]
    pub use super::io::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
}
