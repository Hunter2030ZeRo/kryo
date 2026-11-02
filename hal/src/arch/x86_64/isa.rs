use core::arch::x86_64::__cpuid_count;

pub(super) const EXTENDED_STATE_BYTES: usize = 1024;
const STATE_HEADER_END: u32 = 576;
const USER_STATE: u64 = 7;

const BASIC_EDX: &[(u32, &str)] = &[
    (0, "FPU"),
    (8, "CMPXCHG8B"),
    (15, "CMOV"),
    (23, "MMX"),
    (24, "FXSR"),
    (25, "SSE"),
    (26, "SSE2"),
];
const BASIC_ECX: &[(u32, &str)] = &[
    (0, "SSE3"),
    (9, "SSSE3"),
    (12, "FMA"),
    (13, "CMPXCHG16B"),
    (19, "SSE4.1"),
    (20, "SSE4.2"),
    (22, "MOVBE"),
    (23, "POPCNT"),
    (26, "XSAVE"),
    (28, "AVX"),
    (29, "F16C"),
];
const STRUCTURED_EBX: &[(u32, &str)] = &[(3, "BMI1"), (5, "AVX2"), (8, "BMI2")];
const EXTENDED_ECX: &[(u32, &str)] = &[(0, "LAHF-SAHF"), (5, "LZCNT")];
const EXTENDED_EDX: &[(u32, &str)] = &[(11, "SYSCALL")];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuBaselineError {
    MissingLeaf(u32),
    MissingFeature(&'static str),
    UnsupportedXsave,
}

#[derive(Clone, Copy)]
struct Features {
    basic_ecx: u32,
    basic_edx: u32,
    structured_ebx: u32,
    extended_ecx: u32,
    extended_edx: u32,
    state_mask: u64,
    ymm_size: u32,
    ymm_offset: u32,
    ymm_flags: u32,
}

/// Checks hardware admission before this CPU can run native user programs.
/// Kernel code remains soft-float. User entry separately enables OSFXSR,
/// OSXSAVE and XCR0=7 before restoring the thread's x87/SSE/AVX state.
pub fn validate_cpu_baseline() -> Result<(), CpuBaselineError> {
    validate_with(|leaf, subleaf| {
        let result = __cpuid_count(leaf, subleaf);
        [result.eax, result.ebx, result.ecx, result.edx]
    })
}

fn validate_with(mut cpuid: impl FnMut(u32, u32) -> [u32; 4]) -> Result<(), CpuBaselineError> {
    let maximum = cpuid(0, 0)[0];
    if maximum < 0xd {
        return Err(CpuBaselineError::MissingLeaf(0xd));
    }
    if cpuid(0x8000_0000, 0)[0] < 0x8000_0001 {
        return Err(CpuBaselineError::MissingLeaf(0x8000_0001));
    }
    let basic = cpuid(1, 0);
    let structured = cpuid(7, 0);
    let extended = cpuid(0x8000_0001, 0);
    // Do not query XSAVE leaves before establishing hardware support.
    if basic[2] & (1 << 26) == 0 {
        return Err(CpuBaselineError::MissingFeature("XSAVE"));
    }
    let state = cpuid(0xd, 0);
    let ymm = cpuid(0xd, 2);
    Features {
        basic_ecx: basic[2],
        basic_edx: basic[3],
        structured_ebx: structured[1],
        extended_ecx: extended[2],
        extended_edx: extended[3],
        state_mask: u64::from(state[0]) | (u64::from(state[3]) << 32),
        ymm_size: ymm[0],
        ymm_offset: ymm[1],
        ymm_flags: ymm[2],
    }
    .validate()
}

impl Features {
    fn validate(self) -> Result<(), CpuBaselineError> {
        for (bits, required) in [
            (self.basic_edx, BASIC_EDX),
            (self.basic_ecx, BASIC_ECX),
            (self.structured_ebx, STRUCTURED_EBX),
            (self.extended_ecx, EXTENDED_ECX),
            (self.extended_edx, EXTENDED_EDX),
        ] {
            for &(bit, name) in required {
                if bits & (1 << bit) == 0 {
                    return Err(CpuBaselineError::MissingFeature(name));
                }
            }
        }
        // CPUID.OSXSAVE reflects CR4 and may be clear on bootloader entry. Its
        // hardware prerequisite is XSAVE; extended user entry enables it.
        let end = self.ymm_offset.checked_add(self.ymm_size);
        if self.state_mask & USER_STATE != USER_STATE
            || self.ymm_size != 256
            || self.ymm_offset < STATE_HEADER_END
            || end.is_none_or(|end| end as usize > EXTENDED_STATE_BYTES)
            || self.ymm_flags & !2 != 0
        {
            return Err(CpuBaselineError::UnsupportedXsave);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(required: &[(u32, &str)]) -> u32 {
        required.iter().fold(0, |mask, (bit, _)| mask | (1 << bit))
    }

    fn features() -> Features {
        Features {
            basic_ecx: mask(BASIC_ECX),
            basic_edx: mask(BASIC_EDX),
            structured_ebx: mask(STRUCTURED_EBX),
            extended_ecx: mask(EXTENDED_ECX),
            extended_edx: mask(EXTENDED_EDX),
            state_mask: USER_STATE,
            ymm_size: 256,
            ymm_offset: STATE_HEADER_END,
            ymm_flags: 0,
        }
    }

    fn leaf(cpu: Features, leaf: u32, subleaf: u32) -> [u32; 4] {
        match (leaf, subleaf) {
            (0, 0) => [0xd, 0, 0, 0],
            (0x8000_0000, 0) => [0x8000_0001, 0, 0, 0],
            (1, 0) => [0, 0, cpu.basic_ecx, cpu.basic_edx],
            (7, 0) => [0, cpu.structured_ebx, 0, 0],
            (0x8000_0001, 0) => [0, 0, cpu.extended_ecx, cpu.extended_edx],
            (0xd, 0) => [cpu.state_mask as u32, 0, 0, (cpu.state_mask >> 32) as u32],
            (0xd, 2) => [cpu.ymm_size, cpu.ymm_offset, cpu.ymm_flags, 0],
            _ => panic!("Unexpected CPUID query"),
        }
    }

    #[test]
    fn every_baseline_feature_is_required() {
        for (group, required) in [
            BASIC_EDX,
            BASIC_ECX,
            STRUCTURED_EBX,
            EXTENDED_ECX,
            EXTENDED_EDX,
        ]
        .into_iter()
        .enumerate()
        {
            for &(bit, name) in required {
                let mut cpu = features();
                let bits = match group {
                    0 => &mut cpu.basic_edx,
                    1 => &mut cpu.basic_ecx,
                    2 => &mut cpu.structured_ebx,
                    3 => &mut cpu.extended_ecx,
                    _ => &mut cpu.extended_edx,
                };
                *bits &= !(1 << bit);
                assert_eq!(cpu.validate(), Err(CpuBaselineError::MissingFeature(name)));
            }
        }
    }

    #[test]
    fn admission_does_not_depend_on_bootloader_osxsave_enablement() {
        let mut cpu = features();
        assert_eq!(cpu.basic_ecx & (1 << 27), 0);
        assert_eq!(validate_with(|a, b| leaf(cpu, a, b)), Ok(()));
        cpu.basic_ecx |= 1 << 27;
        assert_eq!(validate_with(|a, b| leaf(cpu, a, b)), Ok(()));
        cpu.state_mask |= 0xe0;
        assert_eq!(cpu.validate(), Ok(()));
    }

    #[test]
    fn unavailable_leaves_are_not_queried() {
        for maximum in [0, 1, 6, 7, 0xc] {
            assert_eq!(
                validate_with(|leaf, _| {
                    assert_eq!(leaf, 0);
                    [maximum, 0, 0, 0]
                }),
                Err(CpuBaselineError::MissingLeaf(0xd))
            );
        }
        assert_eq!(
            validate_with(|leaf, _| match leaf {
                0 => [0xd, 0, 0, 0],
                0x8000_0000 => [0x8000_0000, 0, 0, 0],
                _ => panic!("Unavailable leaf was queried"),
            }),
            Err(CpuBaselineError::MissingLeaf(0x8000_0001))
        );
        let mut cpu = features();
        cpu.basic_ecx &= !(1 << 26);
        assert_eq!(
            validate_with(|a, b| {
                assert_ne!(a, 0xd);
                leaf(cpu, a, b)
            }),
            Err(CpuBaselineError::MissingFeature("XSAVE"))
        );
    }

    #[test]
    fn xsave_admission_preserves_the_fixed_thread_storage_bound() {
        for state_mask in [0, 1, 2, 3, 4, 5, 6] {
            let mut cpu = features();
            cpu.state_mask = state_mask;
            assert_eq!(cpu.validate(), Err(CpuBaselineError::UnsupportedXsave));
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
            cpu.ymm_offset = offset;
            cpu.ymm_size = size;
            cpu.ymm_flags = flags;
            assert_eq!(cpu.validate(), Err(CpuBaselineError::UnsupportedXsave));
        }
        let mut cpu = features();
        cpu.ymm_offset = 768;
        cpu.ymm_flags = 2;
        assert_eq!(cpu.validate(), Ok(()));
    }
}
