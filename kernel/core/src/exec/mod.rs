//! Validated plans for native executable images. Mapping and startup are image policy.

mod elf;

pub use elf::{ElfError, ElfImage, LoadLimits, LoadPage, LoadSegment};
