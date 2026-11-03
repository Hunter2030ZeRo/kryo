mod backend;
mod private;
mod space;

pub use backend::{FrameAllocator, Permissions, VmBackend, VmError};
pub use private::{PrivateError, PrivateVm};
pub use space::{AddressSpace, ExecutionLease, PrivateWordKey};
