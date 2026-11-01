const TABLE_LIMIT: usize = 4096;
const ROOT_LIMIT: usize = 256;
const HEADER_SIZE: usize = 36;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcpiError {
    Truncated,
    InvalidSignature,
    InvalidChecksum,
    InvalidLength,
    InvalidAddress,
    UnsupportedAddress,
    Capacity,
    MissingHpet,
    DuplicateHpet,
    Read,
}

/// Finds a bounded, memory-mapped HPET block through checked firmware tables.
/// The reader must copy exactly the requested physical bytes or return an error.
/// Firmware pages must remain reserved throughout the operation.
pub fn find_hpet(
    rsdp: &[u8],
    mut read: impl FnMut(u64, &mut [u8]) -> Result<(), AcpiError>,
) -> Result<u64, AcpiError> {
    let (root_address, width) = rsdp_root(rsdp)?;
    let mut root = [0; TABLE_LIMIT];
    let length = read_table(root_address, &mut root, &mut read)?;
    let signature = if width == 8 { b"XSDT" } else { b"RSDT" };
    if root[..4] != *signature {
        return Err(AcpiError::InvalidSignature);
    }
    let entries = &root[HEADER_SIZE..length];
    if !entries.len().is_multiple_of(width) {
        return Err(AcpiError::InvalidLength);
    }
    if entries.len() / width > ROOT_LIMIT {
        return Err(AcpiError::Capacity);
    }
    let mut selected = None;
    let mut table = [0; TABLE_LIMIT];
    for entry in entries.chunks_exact(width) {
        let address = if width == 8 {
            u64_at(entry, 0)?
        } else {
            u32_at(entry, 0)? as u64
        };
        check_physical(address, HEADER_SIZE)?;
        read(address, &mut table[..HEADER_SIZE])?;
        if table[..4] != *b"HPET" {
            continue;
        }
        let length = read_table(address, &mut table, &mut read)?;
        if selected.is_some() {
            return Err(AcpiError::DuplicateHpet);
        }
        selected = Some(hpet_address(&table[..length])?);
    }
    selected.ok_or(AcpiError::MissingHpet)
}

fn rsdp_root(bytes: &[u8]) -> Result<(u64, usize), AcpiError> {
    let legacy = bytes.get(..20).ok_or(AcpiError::Truncated)?;
    if legacy[..8] != *b"RSD PTR " {
        return Err(AcpiError::InvalidSignature);
    }
    checksum(legacy)?;
    if bytes[15] < 2 {
        let address = u32_at(bytes, 16)? as u64;
        check_physical(address, HEADER_SIZE)?;
        return Ok((address, 4));
    }
    let length = u32_at(bytes, 20)? as usize;
    if !(36..=TABLE_LIMIT).contains(&length) {
        return Err(AcpiError::InvalidLength);
    }
    checksum(bytes.get(..length).ok_or(AcpiError::Truncated)?)?;
    let xsdt = u64_at(bytes, 24)?;
    if xsdt != 0 {
        check_physical(xsdt, HEADER_SIZE)?;
        Ok((xsdt, 8))
    } else {
        let rsdt = u32_at(bytes, 16)? as u64;
        check_physical(rsdt, HEADER_SIZE)?;
        Ok((rsdt, 4))
    }
}

fn read_table(
    address: u64,
    storage: &mut [u8; TABLE_LIMIT],
    read: &mut impl FnMut(u64, &mut [u8]) -> Result<(), AcpiError>,
) -> Result<usize, AcpiError> {
    check_physical(address, HEADER_SIZE)?;
    read(address, &mut storage[..HEADER_SIZE])?;
    let length = u32_at(storage, 4)? as usize;
    if length < HEADER_SIZE {
        return Err(AcpiError::InvalidLength);
    }
    if length > TABLE_LIMIT {
        return Err(AcpiError::Capacity);
    }
    check_physical(address, length)?;
    read(address, &mut storage[..length])?;
    // Recheck the length after the full read; a changed header cannot extend a
    // borrow beyond the bytes whose checksum was verified.
    if u32_at(storage, 4)? as usize != length {
        return Err(AcpiError::InvalidLength);
    }
    checksum(&storage[..length])?;
    Ok(length)
}

fn hpet_address(table: &[u8]) -> Result<u64, AcpiError> {
    if table.len() < 56 {
        return Err(AcpiError::Truncated);
    }
    if table[..4] != *b"HPET" {
        return Err(AcpiError::InvalidSignature);
    }
    // The GAS names a register block; QEMU firmware leaves its width unspecified.
    if table[40] != 0
        || !matches!(table[41], 0 | 64)
        || table[42] != 0
        || !matches!(table[43], 0 | 4)
    {
        return Err(AcpiError::UnsupportedAddress);
    }
    let address = u64_at(table, 44)?;
    check_physical(address, 4096)?;
    if !address.is_multiple_of(4096) {
        return Err(AcpiError::InvalidAddress);
    }
    Ok(address)
}

fn check_physical(address: u64, length: usize) -> Result<(), AcpiError> {
    let end = address
        .checked_add(length as u64)
        .ok_or(AcpiError::InvalidAddress)?;
    if address == 0 || end > 1 << 52 {
        return Err(AcpiError::InvalidAddress);
    }
    Ok(())
}

fn checksum(bytes: &[u8]) -> Result<(), AcpiError> {
    if bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) != 0 {
        return Err(AcpiError::InvalidChecksum);
    }
    Ok(())
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, AcpiError> {
    let bytes = bytes.get(offset..offset + 4).ok_or(AcpiError::Truncated)?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, AcpiError> {
    let bytes = bytes.get(offset..offset + 8).ok_or(AcpiError::Truncated)?;
    Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix_checksum(bytes: &mut [u8], offset: usize) {
        bytes[offset] = 0;
        let sum = bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        bytes[offset] = 0u8.wrapping_sub(sum);
    }

    fn rsdp(xsdt: bool) -> [u8; 36] {
        let mut bytes = [0; 36];
        bytes[..8].copy_from_slice(b"RSD PTR ");
        bytes[16..20].copy_from_slice(&0x1000u32.to_le_bytes());
        if xsdt {
            bytes[15] = 2;
            bytes[20..24].copy_from_slice(&36u32.to_le_bytes());
            bytes[24..32].copy_from_slice(&0x1000u64.to_le_bytes());
        }
        fix_checksum(&mut bytes[..20], 8);
        fix_checksum(&mut bytes, 32);
        bytes
    }

    fn root(xsdt: bool) -> [u8; 44] {
        let mut table = [0; 44];
        table[..4].copy_from_slice(if xsdt { b"XSDT" } else { b"RSDT" });
        let length = if xsdt { 44u32 } else { 40u32 };
        table[4..8].copy_from_slice(&length.to_le_bytes());
        table[36..40].copy_from_slice(&0x2000u32.to_le_bytes());
        fix_checksum(&mut table[..length as usize], 9);
        table
    }

    fn hpet() -> [u8; 56] {
        let mut table = [0; 56];
        table[..4].copy_from_slice(b"HPET");
        table[4..8].copy_from_slice(&56u32.to_le_bytes());
        table[41] = 64;
        table[44..52].copy_from_slice(&0xfed0_0000u64.to_le_bytes());
        fix_checksum(&mut table, 9);
        table
    }

    fn discover(rsdp: &[u8], root: &[u8], hpet: &[u8]) -> Result<u64, AcpiError> {
        find_hpet(rsdp, |address, output| {
            let source = match address {
                0x1000 => root,
                0x2000 => hpet,
                _ => return Err(AcpiError::Read),
            };
            let source = source.get(..output.len()).ok_or(AcpiError::Read)?;
            output.copy_from_slice(source);
            Ok(())
        })
    }

    #[test]
    fn legacy_and_extended_roots_find_checked_hpet() {
        for xsdt in [false, true] {
            let rsdp = rsdp(xsdt);
            let rsdp = if xsdt { &rsdp[..] } else { &rsdp[..20] };
            assert_eq!(discover(rsdp, &root(xsdt), &hpet()), Ok(0xfed0_0000));
        }
    }

    #[test]
    fn truncated_and_corrupt_firmware_is_rejected() {
        let valid = rsdp(true);
        for length in 0..36 {
            assert!(discover(&valid[..length], &root(true), &hpet()).is_err());
        }
        let mut invalid = valid;
        invalid[10] ^= 1;
        assert_eq!(
            discover(&invalid, &root(true), &hpet()),
            Err(AcpiError::InvalidChecksum)
        );
        let mut invalid_root = root(true);
        invalid_root[10] ^= 1;
        assert_eq!(
            discover(&valid, &invalid_root, &hpet()),
            Err(AcpiError::InvalidChecksum)
        );
        let mut invalid_hpet = hpet();
        invalid_hpet[10] ^= 1;
        assert_eq!(
            discover(&valid, &root(true), &invalid_hpet),
            Err(AcpiError::InvalidChecksum)
        );
    }

    #[test]
    fn gas_layout_and_address_are_constrained() {
        let mut unspecified = hpet();
        unspecified[41] = 0;
        fix_checksum(&mut unspecified, 9);
        assert_eq!(
            discover(&rsdp(true), &root(true), &unspecified),
            Ok(0xfed0_0000)
        );
        for index in [40, 41, 42, 43] {
            let mut invalid = hpet();
            invalid[index] = 3;
            fix_checksum(&mut invalid, 9);
            assert_eq!(
                discover(&rsdp(true), &root(true), &invalid),
                Err(AcpiError::UnsupportedAddress)
            );
        }
        let mut invalid = hpet();
        invalid[44..52].copy_from_slice(&0xfed0_0001u64.to_le_bytes());
        fix_checksum(&mut invalid, 9);
        assert_eq!(
            discover(&rsdp(true), &root(true), &invalid),
            Err(AcpiError::InvalidAddress)
        );
        invalid[44..52].copy_from_slice(&(1u64 << 52).to_le_bytes());
        fix_checksum(&mut invalid, 9);
        assert_eq!(
            discover(&rsdp(true), &root(true), &invalid),
            Err(AcpiError::InvalidAddress)
        );
    }

    #[test]
    fn root_bounds_missing_tables_and_reader_errors_propagate() {
        let mut invalid = root(true);
        invalid[4..8].copy_from_slice(&4097u32.to_le_bytes());
        assert_eq!(
            discover(&rsdp(true), &invalid, &hpet()),
            Err(AcpiError::Capacity)
        );
        let mut missing = hpet();
        missing[..4].copy_from_slice(b"APIC");
        assert_eq!(
            discover(&rsdp(true), &root(true), &missing),
            Err(AcpiError::MissingHpet)
        );
        assert_eq!(
            find_hpet(&rsdp(true), |_, _| Err(AcpiError::Read)),
            Err(AcpiError::Read)
        );
    }

    #[test]
    fn duplicate_hpet_and_changed_length_fail_closed() {
        let mut duplicate = [0; 52];
        duplicate[..44].copy_from_slice(&root(true));
        duplicate[4..8].copy_from_slice(&52u32.to_le_bytes());
        duplicate[44..52].copy_from_slice(&0x2000u64.to_le_bytes());
        fix_checksum(&mut duplicate, 9);
        assert_eq!(
            discover(&rsdp(true), &duplicate, &hpet()),
            Err(AcpiError::DuplicateHpet)
        );
        let mut reads = 0;
        assert_eq!(
            find_hpet(&rsdp(true), |_, output| {
                let mut root = root(true);
                reads += 1;
                if reads == 2 {
                    root[4..8].copy_from_slice(&43u32.to_le_bytes());
                }
                output.copy_from_slice(&root[..output.len()]);
                Ok(())
            }),
            Err(AcpiError::InvalidLength)
        );
    }

    #[test]
    fn extended_checksum_and_root_shape_cannot_be_ignored() {
        let mut invalid_rsdp = rsdp(true);
        invalid_rsdp[34] = 1;
        assert_eq!(
            discover(&invalid_rsdp, &root(true), &hpet()),
            Err(AcpiError::InvalidChecksum)
        );
        let mut invalid_root = root(true);
        invalid_root[4..8].copy_from_slice(&43u32.to_le_bytes());
        fix_checksum(&mut invalid_root[..43], 9);
        assert_eq!(
            discover(&rsdp(true), &invalid_root, &hpet()),
            Err(AcpiError::InvalidLength)
        );
        let mut oversized_root = [0; 36 + 257 * 8];
        oversized_root[..4].copy_from_slice(b"XSDT");
        let length = oversized_root.len() as u32;
        oversized_root[4..8].copy_from_slice(&length.to_le_bytes());
        fix_checksum(&mut oversized_root, 9);
        assert_eq!(
            discover(&rsdp(true), &oversized_root, &hpet()),
            Err(AcpiError::Capacity)
        );
    }
}
