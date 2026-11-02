#![no_std]

//! Architecture-independent kernel mechanisms. Modules are added as implemented.

extern crate alloc;
pub mod capability;
pub mod device;
pub mod exec;
pub mod ipc;
pub mod mm;
pub mod object;
pub mod process;
pub mod resource;
pub mod sched;
pub mod storage;
pub mod sync;
pub mod task;
