pub const FRAME_SIZE: u64 = 4096;
const FRAME_MASK: u64 = FRAME_SIZE - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysRange {
    start: u64,
    end: u64,
}

impl PhysRange {
    pub const EMPTY: Self = Self { start: 0, end: 0 };

    pub fn from_base_len(base: u64, length: u64) -> Option<Self> {
        Some(Self {
            start: base,
            end: base.checked_add(length)?,
        })
    }

    pub const fn start(self) -> u64 {
        self.start
    }

    pub const fn end(self) -> u64 {
        self.end
    }

    pub const fn len(self) -> u64 {
        self.end - self.start
    }

    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub fn page_interior(self) -> Option<Self> {
        let start = self.start.checked_add(FRAME_MASK)? & !FRAME_MASK;
        let end = self.end & !FRAME_MASK;

        (start < end).then_some(Self { start, end })
    }

    pub fn page_cover(self) -> Option<Self> {
        if self.is_empty() {
            return Some(Self::EMPTY);
        }

        Some(Self {
            start: self.start & !FRAME_MASK,
            end: self.end.checked_add(FRAME_MASK)? & !FRAME_MASK,
        })
    }
}
