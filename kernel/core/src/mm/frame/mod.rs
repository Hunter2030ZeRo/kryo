mod map;
mod normalize;
mod pool;
mod range;

pub use map::{MemoryRegion, RegionKind};
pub use normalize::{MemoryPlan, NormalizeError, normalize_memory};
pub use pool::{
    FrameAllocError, FrameFreeError, FrameInitError, FrameMetadata, FrameMetadataError,
    FrameMetadataLayout, FramePool, FrameStats, OwnedFrame, OwnedRun, metadata_layout,
};
pub use range::{FRAME_SIZE, PhysRange};
