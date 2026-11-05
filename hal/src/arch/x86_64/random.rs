use core::arch::{asm, x86_64::__cpuid_count};

pub const CPU_RANDOM_MAX: usize = 16;
const ATTEMPTS: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RandomError {
    Unsupported,
    Busy,
    InvalidLength,
}

/// Reports the current CPU only; this is not a system-wide migration guarantee.
pub fn cpu_random_available() -> bool {
    available_with(|leaf| {
        let value = __cpuid_count(leaf, 0);
        (value.eax, value.ecx)
    })
}

fn available_with(mut cpuid: impl FnMut(u32) -> (u32, u32)) -> bool {
    cpuid(0).0 >= 1 && cpuid(1).1 & (1 << 30) != 0
}

/// Fills a bounded buffer from the current CPU's hardware random-byte source.
/// RDRAND output is not a claim of independently sampled entropy per bit.
/// Exhaustion clears the buffer; unsupported or oversized requests leave it unchanged.
///
/// # Safety
/// The caller must prevent CPU migration throughout feature detection and use.
/// Kernel native calls hold serialized BSP ownership across this operation.
pub unsafe fn fill_cpu_random(output: &mut [u8]) -> Result<(), RandomError> {
    let supported = output.is_empty() || cpu_random_available();
    fill_with(output, supported, || {
        let value: u64;
        let ready: u8;
        // The feature check above and the caller's CPU ownership admit this opcode.
        unsafe {
            asm!(
                "rdrand {value}",
                "setc {ready}",
                value = out(reg) value,
                ready = out(reg_byte) ready,
                options(nomem, nostack),
            );
        }
        (ready != 0).then_some(value)
    })
}

fn fill_with(
    output: &mut [u8],
    supported: bool,
    mut next: impl FnMut() -> Option<u64>,
) -> Result<(), RandomError> {
    if output.len() > CPU_RANDOM_MAX {
        return Err(RandomError::InvalidLength);
    }
    if output.is_empty() {
        return Ok(());
    }
    if !supported {
        return Err(RandomError::Unsupported);
    }
    for bytes in output.chunks_mut(8) {
        let mut word = None;
        for _ in 0..ATTEMPTS {
            if let Some(value) = next() {
                word = Some(value);
                break;
            }
        }
        let Some(word) = word else {
            clear(output);
            return Err(RandomError::Busy);
        };
        bytes.copy_from_slice(&word.to_ne_bytes()[..bytes.len()]);
    }
    Ok(())
}

fn clear(bytes: &mut [u8]) {
    for byte in bytes {
        // Failed fills must not retain the successfully sampled prefix.
        unsafe { core::ptr::write_volatile(byte, 0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_queries_only_an_admitted_feature_leaf() {
        assert!(!available_with(|leaf| {
            assert_eq!(leaf, 0);
            (0, 0)
        }));
        for present in [false, true] {
            assert_eq!(
                available_with(|leaf| match leaf {
                    0 => (1, 0),
                    1 => (0, if present { 1 << 30 } else { 0 }),
                    _ => panic!("unexpected leaf"),
                }),
                present
            );
        }
    }

    #[test]
    fn empty_fill_does_not_require_a_source() {
        assert_eq!(fill_with(&mut [], false, || panic!("sampled")), Ok(()));
    }

    #[test]
    fn unavailable_and_excessive_requests_do_not_sample_or_change_output() {
        let mut bytes = [0xa5; 8];
        assert_eq!(
            fill_with(&mut bytes, false, || panic!("sampled")),
            Err(RandomError::Unsupported)
        );
        assert_eq!(bytes, [0xa5; 8]);
        let mut bytes = [0xa5; CPU_RANDOM_MAX + 1];
        assert_eq!(
            fill_with(&mut bytes, true, || panic!("sampled")),
            Err(RandomError::InvalidLength)
        );
        assert!(bytes.iter().all(|byte| *byte == 0xa5));
    }

    #[test]
    fn carry_success_accepts_zero_and_copies_a_partial_final_word() {
        let mut word = 0;
        let mut bytes = [0xff; 11];
        assert_eq!(
            fill_with(&mut bytes, true, || {
                let value = word;
                word = 0x8877_6655_4433_2211;
                Some(value)
            }),
            Ok(())
        );
        assert_eq!(&bytes[..8], &[0; 8]);
        assert_eq!(&bytes[8..], &[0x11, 0x22, 0x33]);
    }

    #[test]
    fn carry_failure_has_a_finite_retry_budget() {
        let mut attempts = 0;
        let mut bytes = [0xa5; 8];
        assert_eq!(
            fill_with(&mut bytes, true, || {
                attempts += 1;
                None
            }),
            Err(RandomError::Busy)
        );
        assert_eq!(attempts, ATTEMPTS);
        assert_eq!(bytes, [0; 8]);
    }

    #[test]
    fn a_later_failure_clears_every_sampled_byte() {
        let mut attempts = 0;
        let mut bytes = [0xa5; 16];
        assert_eq!(
            fill_with(&mut bytes, true, || {
                attempts += 1;
                (attempts == 1).then_some(u64::MAX)
            }),
            Err(RandomError::Busy)
        );
        assert_eq!(attempts, 1 + ATTEMPTS);
        assert_eq!(bytes, [0; 16]);
    }

    #[test]
    fn each_word_can_succeed_on_the_last_attempt() {
        let mut attempts: usize = 0;
        let mut bytes = [0; CPU_RANDOM_MAX];
        assert_eq!(
            fill_with(&mut bytes, true, || {
                attempts += 1;
                attempts.is_multiple_of(ATTEMPTS).then_some(u64::MAX)
            }),
            Ok(())
        );
        assert_eq!(attempts, CPU_RANDOM_MAX / 8 * ATTEMPTS);
        assert_eq!(bytes, [0xff; CPU_RANDOM_MAX]);
    }
}
