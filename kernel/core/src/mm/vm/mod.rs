mod backend;
mod space;

pub use backend::{FrameAllocator, Permissions, VmBackend, VmError};
pub use space::AddressSpace;
