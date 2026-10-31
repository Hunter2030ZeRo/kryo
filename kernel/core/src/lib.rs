#![no_std]

//! Architecture-independent kernel mechanisms. Modules are added as implemented.
pub mod capability;
pub mod ipc;
pub mod mm;
pub mod object;
pub mod sched;
pub mod sync;
pub mod task;
