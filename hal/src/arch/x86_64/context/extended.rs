use super::super::cpu::{CpuId, MAX_CPUS};
use super::UserEntryError;
use core::arch::{asm, x86_64::__cpuid, x86_64::__cpuid_count};
use core::cell::UnsafeCell;
use x86_64::registers::control::{Cr0, Cr0Flags, Cr4, Cr4Flags};
use x86_64::registers::xcontrol::XCr0;

const AREA_SIZE: usize = super::super::isa::EXTENDED_STATE_BYTES;
const HEADER: usize = 512;
const HEADER_END: usize = 576;
const LEGACY_MASK: u64 = 3;
const AVX_MASK: u64 = 7;
const REQUIRED: u32 = 1 | (1 << 24) | (1 << 25) | (1 << 26);
const XSAVE: u32 = 1 << 26;
const AVX: u32 = 1 << 28;

#[repr(u64)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Format {
    Fxsave = 1,
    Xsave = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Layout {
    pub format: Format,
    pub mask: u64,
    bytes: usize,
}

struct Features {
    maximum: u32,
    ecx: u32,
    edx: u32,
    supported: u64,
    avx_size: u32,
    avx_offset: u32,
    avx_flags: u32,
}

#[repr(C, align(64))]
struct StateArea([u8; AREA_SIZE]);

/// Private per-thread x87, SSE, and optional AVX state. Never map this into user space.
/// Its exclusive run borrow keeps the save address stable until execution returns.
#[repr(C)]
pub struct ExtendedState {
    area: StateArea,
    layout: Layout,
}

impl core::fmt::Debug for ExtendedState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExtendedState")
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl ExtendedState {
    pub fn new() -> Result<Self, UserEntryError> {
        Ok(Self::with_layout(Layout::detect()?))
    }

    pub fn uses_xsave(&self) -> bool {
        self.layout.format == Format::Xsave
    }

    pub fn avx_enabled(&self) -> bool {
        self.uses_xsave() && self.layout.mask & 4 != 0
    }

    fn with_layout(layout: Layout) -> Self {
        let mut area = StateArea([0; AREA_SIZE]);
        area.0[..2].copy_from_slice(&0x037f_u16.to_le_bytes());
        area.0[24..28].copy_from_slice(&0x1f80_u32.to_le_bytes());
        // A zero XSAVE bitmap requests architectural initialization. Standard
        // XRSTOR still loads MXCSR from memory when SSE or AVX is requested.
        Self { area, layout }
    }

    pub(super) fn layout(&self) -> Layout {
        self.layout
    }

    pub(super) fn area_ptr(&mut self) -> *mut u8 {
        self.area.0.as_mut_ptr()
    }
}

impl Layout {
    fn detect() -> Result<Self, UserEntryError> {
        let maximum = __cpuid(0).eax;
        if maximum < 1 {
            return Err(UserEntryError::UnsupportedCpuState);
        }
        let basic = __cpuid(1);
        let mut features = Features {
            maximum,
            ecx: basic.ecx,
            edx: basic.edx,
            supported: 0,
            avx_size: 0,
            avx_offset: 0,
            avx_flags: 0,
        };
        if maximum >= 0xd && basic.ecx & XSAVE != 0 {
            let state = __cpuid_count(0xd, 0);
            features.supported = u64::from(state.eax) | (u64::from(state.edx) << 32);
            if features.supported & 4 != 0 {
                let avx = __cpuid_count(0xd, 2);
                features.avx_size = avx.eax;
                features.avx_offset = avx.ebx;
                features.avx_flags = avx.ecx;
            }
        }
        Self::from_features(features)
    }

    fn from_features(cpu: Features) -> Result<Self, UserEntryError> {
        if cpu.edx & REQUIRED != REQUIRED {
            return Err(UserEntryError::UnsupportedCpuState);
        }
        if cpu.ecx & XSAVE == 0 {
            return Ok(Self {
                format: Format::Fxsave,
                mask: LEGACY_MASK,
                bytes: HEADER,
            });
        }
        if cpu.maximum < 0xd || cpu.supported & LEGACY_MASK != LEGACY_MASK {
            return Err(UserEntryError::UnsupportedCpuState);
        }
        let mut layout = Self {
            format: Format::Xsave,
            mask: LEGACY_MASK,
            bytes: HEADER_END,
        };
        if cpu.ecx & AVX != 0 {
            let end = cpu
                .avx_offset
                .checked_add(cpu.avx_size)
                .ok_or(UserEntryError::UnsupportedCpuState)?;
            if cpu.supported & 4 == 0
                || cpu.avx_size != 256
                || cpu.avx_offset < HEADER_END as u32
                || end as usize > AREA_SIZE
                || cpu.avx_flags & !2 != 0
            {
                return Err(UserEntryError::UnsupportedCpuState);
            }
            layout.mask = AVX_MASK;
            layout.bytes = end as usize;
        }
        Ok(layout)
    }
}

#[repr(transparent)]
struct KernelState(UnsafeCell<StateArea>);

// The indexed CPU owns this state with IRQs off; its run state excludes nesting.
unsafe impl Sync for KernelState {}

static KERNEL_STATES: [KernelState; MAX_CPUS] =
    [const { KernelState(UnsafeCell::new(StateArea([0; AREA_SIZE]))) }; MAX_CPUS];

pub(super) fn kernel_area(cpu: CpuId) -> *mut u8 {
    KERNEL_STATES[cpu.index()].0.get().cast::<u8>()
}

pub(super) unsafe fn validate_cpu(state: &ExtendedState) -> Result<u64, UserEntryError> {
    if state.layout != Layout::detect()? {
        return Err(UserEntryError::UnsupportedCpuState);
    }
    if !state.uses_xsave() {
        return Ok(0);
    }
    let original = Cr4::read_raw();
    let enabled = original | Cr4Flags::OSXSAVE.bits();
    if original != enabled {
        unsafe { Cr4::write_raw(enabled) };
    }
    // XCR0 may retain a value even when the bootloader left OSXSAVE disabled.
    let xcr0 = XCr0::read_raw();
    if original != enabled {
        unsafe { Cr4::write_raw(original) };
    }
    validate_xcr0(xcr0, state.layout.mask)?;
    Ok(xcr0)
}

fn validate_xcr0(original: u64, allowed: u64) -> Result<(), UserEntryError> {
    if original & 1 == 0 || original & !allowed != 0 || (original & 4 != 0 && original & 2 == 0) {
        return Err(UserEntryError::UnsupportedCpuState);
    }
    Ok(())
}

/// Save under the selected mask even if original XCR0 omitted SSE. CR4.OSFXSR
/// can enable SSE independently of XCR0, so the kernel's XMM state still matters.
pub(super) unsafe fn prepare(layout: Layout, cpu: CpuId) {
    unsafe {
        Cr4::update(|flags| {
            flags.insert(Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT_ENABLE);
            if layout.format == Format::Xsave {
                flags.insert(Cr4Flags::OSXSAVE);
            }
        });
        Cr0::update(|flags| {
            flags.remove(Cr0Flags::EMULATE_COPROCESSOR | Cr0Flags::TASK_SWITCHED);
            flags.insert(Cr0Flags::MONITOR_COPROCESSOR | Cr0Flags::NUMERIC_ERROR);
        });
        let area = &mut *KERNEL_STATES[cpu.index()].0.get();
        if layout.format == Format::Xsave {
            XCr0::write_raw(layout.mask);
            area.0[HEADER..HEADER_END].fill(0);
            asm!("xsave64 [{area}]", area = in(reg) area.0.as_mut_ptr(),
                in("eax") layout.mask as u32, in("edx") 0_u32,
                options(nostack, preserves_flags));
        } else {
            asm!("fxsave64 [{area}]", area = in(reg) area.0.as_mut_ptr(),
                options(nostack, preserves_flags));
        }
        // Bare-metal Rust is soft-float. Any unintended kernel FP use must fault.
        Cr0::update(|flags| flags.insert(Cr0Flags::TASK_SWITCHED));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, offset_of, size_of};

    #[test]
    fn kernel_fp_areas_are_private_and_aligned_for_every_cpu() {
        for (index, state) in KERNEL_STATES.iter().enumerate() {
            let pointer = state.0.get() as usize;
            assert_eq!(pointer % 64, 0);
            for earlier in &KERNEL_STATES[..index] {
                assert_ne!(pointer, earlier.0.get() as usize);
            }
        }
    }

    fn features() -> Features {
        Features {
            maximum: 0xd,
            ecx: XSAVE | AVX,
            edx: REQUIRED,
            supported: AVX_MASK,
            avx_size: 256,
            avx_offset: 576,
            avx_flags: 0,
        }
    }

    #[test]
    fn state_storage_is_aligned_and_initial_registers_are_deterministic() {
        assert_eq!(align_of::<ExtendedState>(), 64);
        assert_eq!(offset_of!(ExtendedState, area), 0);
        assert_eq!(size_of::<StateArea>(), AREA_SIZE);
        assert!(size_of::<ExtendedState>() <= 1152);
        let state = ExtendedState::with_layout(Layout::from_features(features()).unwrap());
        assert_eq!(&state.area.0[..2], &0x037f_u16.to_le_bytes());
        assert_eq!(&state.area.0[24..28], &0x1f80_u32.to_le_bytes());
        for (index, byte) in state.area.0.iter().enumerate() {
            if ![0, 1, 24, 25].contains(&index) {
                assert_eq!(*byte, 0);
            }
        }
        assert!(state.uses_xsave());
        assert!(state.avx_enabled());
    }

    #[test]
    fn selection_supports_legacy_sse_and_bounded_avx() {
        let avx = Layout::from_features(features()).unwrap();
        assert_eq!((avx.format, avx.mask, avx.bytes), (Format::Xsave, 7, 832));
        let mut cpu = features();
        cpu.ecx = XSAVE;
        let sse = Layout::from_features(cpu).unwrap();
        assert_eq!((sse.format, sse.mask, sse.bytes), (Format::Xsave, 3, 576));
        let mut cpu = features();
        cpu.ecx = 0;
        let legacy = Layout::from_features(cpu).unwrap();
        assert_eq!(
            (legacy.format, legacy.mask, legacy.bytes),
            (Format::Fxsave, 3, 512)
        );
        let state = ExtendedState::with_layout(legacy);
        assert!(!state.uses_xsave());
        assert!(!state.avx_enabled());
    }

    #[test]
    fn incomplete_cpu_support_and_unbounded_components_are_rejected() {
        for bit in [1, 1 << 24, 1 << 25, 1 << 26] {
            let mut cpu = features();
            cpu.edx &= !bit;
            assert_eq!(
                Layout::from_features(cpu),
                Err(UserEntryError::UnsupportedCpuState)
            );
        }
        for (offset, size, flags) in [
            (0, 256, 0),
            (575, 256, 0),
            (769, 256, 0),
            (u32::MAX, 256, 0),
            (576, 0, 0),
            (576, 255, 0),
            (576, 256, 1),
            (576, 256, 4),
        ] {
            let mut cpu = features();
            cpu.avx_offset = offset;
            cpu.avx_size = size;
            cpu.avx_flags = flags;
            assert_eq!(
                Layout::from_features(cpu),
                Err(UserEntryError::UnsupportedCpuState)
            );
        }
        for supported in [0, 1, 2, 3] {
            let mut cpu = features();
            cpu.supported = supported;
            assert_eq!(
                Layout::from_features(cpu),
                Err(UserEntryError::UnsupportedCpuState)
            );
        }
        let mut cpu = features();
        cpu.maximum = 0xc;
        assert_eq!(
            Layout::from_features(cpu),
            Err(UserEntryError::UnsupportedCpuState)
        );
    }

    #[test]
    fn latent_xcr0_must_fit_the_preserved_feature_set() {
        for original in [1, 3, 7] {
            assert_eq!(validate_xcr0(original, AVX_MASK), Ok(()));
        }
        for original in [0, 2, 4, 5, 6, 15, 0xe7, u64::MAX] {
            assert_eq!(
                validate_xcr0(original, AVX_MASK),
                Err(UserEntryError::UnsupportedCpuState)
            );
        }
        assert_eq!(
            validate_xcr0(7, LEGACY_MASK),
            Err(UserEntryError::UnsupportedCpuState)
        );
    }
}
