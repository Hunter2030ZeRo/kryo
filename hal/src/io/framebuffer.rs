pub struct Framebuffer32 {
    base: *mut u8,
    width: usize,
    height: usize,
    pitch: usize,
}

impl Framebuffer32 {
    /// # Safety
    /// The buffer must remain mapped and writable for its full pitch times height.
    /// Prevent concurrent writers and access through other aliases while this writer exists.
    pub unsafe fn new(base: *mut u8, width: usize, height: usize, pitch: usize) -> Option<Self> {
        if base.is_null()
            || !base.cast::<u32>().is_aligned()
            || width == 0
            || height == 0
            || pitch % 4 != 0
        {
            return None;
        }

        let row_bytes = width.checked_mul(4)?;
        if pitch < row_bytes {
            return None;
        }

        let length = pitch.checked_mul(height)?;
        (base as usize).checked_add(length)?;

        Some(Self {
            base,
            width,
            height,
            pitch,
        })
    }

    pub fn put_pixel(&mut self, x: usize, y: usize, r: u8, g: u8, b: u8) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }

        let offset = y * self.pitch + x * 4;
        let color = ((r as u32) << 16) | ((g as u32) << 8) | b as u32;

        let pixel = self.base.wrapping_add(offset).cast::<u32>();

        unsafe {
            pixel.write_volatile(color);
        }

        true
    }
}
