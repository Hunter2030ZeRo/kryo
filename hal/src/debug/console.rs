use core::arch::asm;
use core::fmt::{Arguments, Result, Write};

struct DebugConsole;

impl Write for DebugConsole {
    fn write_str(&mut self, text: &str) -> Result {
        emergency_write_str(text);
        Ok(())
    }
}

pub fn print(args: Arguments<'_>) -> Result {
    let mut console = DebugConsole;
    console.write_fmt(args)
}

pub fn emergency_write_str(text: &str) {
    for byte in text.bytes() {
        write_byte(byte);
    }
}

pub fn emergency_write_hex(value: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    emergency_write_str("0x");

    for digit in (0..16).rev() {
        let shift = digit * 4;
        let index = ((value >> shift) & 0xf) as usize;
        write_byte(HEX[index]);
    }
}

fn write_byte(byte: u8) {
    unsafe {
        asm!(
            "out dx, al",
            in("dx") 0xe9u16,
            in("al") byte,
            options(nomem, nostack, preserves_flags)
        );
    }
}
