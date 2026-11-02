const WORD_BYTES: usize = 8;
const USER_START: u64 = 4096;
const USER_END: u64 = 0x0000_8000_0000_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuxEntry {
    pub kind: u64,
    pub value: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackError {
    InvalidAddress,
    EmbeddedNul,
    InvalidAuxiliary,
    InsufficientSpace,
}

/// Writes a SysV AMD64 initial stack into caller-owned storage, returning RSP.
/// `base` names the future user mapping; no mapping is created here. Auxiliary
/// values must describe resources actually supplied by the image loader.
/// Failure preserves the entire output. Success zeroes all unused bytes.
pub fn build_initial_stack(
    output: &mut [u8],
    base: u64,
    argv: &[&[u8]],
    envp: &[&[u8]],
    aux: &[AuxEntry],
) -> Result<u64, StackError> {
    let length = u64::try_from(output.len()).map_err(|_| StackError::InvalidAddress)?;
    let end = base.checked_add(length).ok_or(StackError::InvalidAddress)?;
    if !(USER_START..USER_END).contains(&base) || end > USER_END {
        return Err(StackError::InvalidAddress);
    }
    let argc = u64::try_from(argv.len()).map_err(|_| StackError::InsufficientSpace)?;
    let strings_bytes = argv.iter().chain(envp).try_fold(0usize, |total, string| {
        if string.contains(&0) {
            return Err(StackError::EmbeddedNul);
        }
        total
            .checked_add(string.len())
            .and_then(|total| total.checked_add(1))
            .ok_or(StackError::InsufficientSpace)
    })?;
    for (index, entry) in aux.iter().enumerate() {
        if entry.kind == 0 || aux[..index].iter().any(|other| other.kind == entry.kind) {
            return Err(StackError::InvalidAuxiliary);
        }
    }
    let aux_words = aux
        .len()
        .checked_add(1)
        .and_then(|count| count.checked_mul(2))
        .ok_or(StackError::InsufficientSpace)?;
    let words_bytes = argv
        .len()
        .checked_add(envp.len())
        .and_then(|count| count.checked_add(3))
        .and_then(|count| count.checked_add(aux_words))
        .and_then(|count| count.checked_mul(WORD_BYTES))
        .ok_or(StackError::InsufficientSpace)?;
    let strings_offset = output
        .len()
        .checked_sub(strings_bytes)
        .ok_or(StackError::InsufficientSpace)?;
    let stack_pointer = (end - strings_bytes as u64)
        .checked_sub(words_bytes as u64)
        .ok_or(StackError::InsufficientSpace)?
        & !15;
    if stack_pointer < base {
        return Err(StackError::InsufficientSpace);
    }

    // The complete range and byte counts were checked before this first write.
    output.fill(0);
    let mut words = (stack_pointer - base) as usize;
    let mut strings = strings_offset;
    write_word(output, &mut words, argc);
    write_strings(output, base, &mut words, &mut strings, argv);
    write_word(output, &mut words, 0);
    write_strings(output, base, &mut words, &mut strings, envp);
    write_word(output, &mut words, 0);
    for entry in aux {
        write_word(output, &mut words, entry.kind);
        write_word(output, &mut words, entry.value);
    }
    write_word(output, &mut words, 0);
    write_word(output, &mut words, 0);
    Ok(stack_pointer)
}

fn write_strings(
    output: &mut [u8],
    base: u64,
    words: &mut usize,
    strings: &mut usize,
    values: &[&[u8]],
) {
    for value in values {
        write_word(output, words, base + *strings as u64);
        output[*strings..*strings + value.len()].copy_from_slice(value);
        *strings += value.len() + 1;
    }
}

fn write_word(output: &mut [u8], offset: &mut usize, value: u64) {
    output[*offset..*offset + WORD_BYTES].copy_from_slice(&value.to_le_bytes());
    *offset += WORD_BYTES;
}
