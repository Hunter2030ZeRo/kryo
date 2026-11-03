//! Validated plans for native executable images. Mapping and startup are image policy.

mod elf;
mod stack;

pub use elf::{ElfError, ElfImage, LoadLimits, LoadPage, LoadSegment, ProgramHeaders, TlsSegment};
pub use stack::{AuxEntry, StackError, build_initial_stack};
