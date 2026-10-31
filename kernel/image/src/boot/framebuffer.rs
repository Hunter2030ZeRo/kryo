use super::{FRAMEBUFFER_REQUEST, check_protocol};

/// # Safety
/// Call once and retain the boot framebuffer mapping while the returned writer exists.
pub(crate) unsafe fn init_framebuffer() -> Option<kryo_hal::Framebuffer32> {
    check_protocol();

    let Some(response) = FRAMEBUFFER_REQUEST.response() else {
        kryo_hal::emergency_write_str("\nFrame buffer: no response\n");
        return None;
    };

    let Some(fb) = response.framebuffers().first() else {
        kryo_hal::emergency_write_str("\nFrame buffer: empty response\n");
        return None;
    };

    let _ = kryo_hal::print(format_args!(
        "\nFB: addr={:p} {}x{} pitch={} bpp={} model={}\n",
        fb.address(),
        fb.width,
        fb.height,
        fb.pitch,
        fb.bpp,
        fb.memory_model,
    ));

    let _ = kryo_hal::print(format_args!(
        "FB masks: R={}@{} G={}@{} B={}@{}\n",
        fb.red_mask_size,
        fb.red_mask_shift,
        fb.green_mask_size,
        fb.green_mask_shift,
        fb.blue_mask_size,
        fb.blue_mask_shift,
    ));

    if fb.memory_model != limine::framebuffer::FRAMEBUFFER_RGB
        || fb.bpp != 32
        || (fb.red_mask_size, fb.red_mask_shift) != (8, 16)
        || (fb.green_mask_size, fb.green_mask_shift) != (8, 8)
        || (fb.blue_mask_size, fb.blue_mask_shift) != (8, 0)
    {
        kryo_hal::emergency_write_str("FB: unsupported pixel format\n");
        return None;
    }

    unsafe {
        kryo_hal::Framebuffer32::new(
            fb.address().cast::<u8>(),
            fb.width as usize,
            fb.height as usize,
            fb.pitch as usize,
        )
    }
}
