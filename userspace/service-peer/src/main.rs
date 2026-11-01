#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;
use kryo_libsys::{self as sys, Error, Grant, Handle};

const HELLO: u64 = 1;
const REQUEST: u64 = 2;
const RESPONSE: u64 = 3;
const STOP: u64 = 4;
const WAIT_NS: u64 = 1_000_000_000;

global_asm!(
    r#"
    .section .text.start,"ax"
    .global _start
    .type _start,@function
_start:
    cld
    xor ebp, ebp
    call {main}
    ud2
    .size _start, .-_start
"#,
    main = sym peer_main,
);

extern "C" fn peer_main(bootstrap: u64, factory: u64) -> ! {
    let bootstrap = Handle::from_raw(bootstrap).unwrap_or_else(|| sys::exit(200));
    let factory = Handle::from_raw(factory).unwrap_or_else(|| sys::exit(201));
    match serve(bootstrap, factory) {
        Ok(()) => sys::exit(0),
        Err(code) => sys::exit(code),
    }
}

fn serve(bootstrap: Handle, factory: Handle) -> Result<(), u64> {
    let endpoint = sys::create_endpoint(factory).map_err(|_| 202u64)?;
    let failure = sys::send_caps_until(
        bootstrap,
        [HELLO, 0, 0, 0],
        &[Grant {
            handle: endpoint,
            rights: sys::RECEIVE,
        }],
        deadline()?,
    )
    .err()
    .ok_or(203u64)?;
    if failure.error != Error::Kernel(-3) || failure.accepted {
        return Err(204);
    }
    sys::send_caps_until(
        bootstrap,
        [HELLO, 0, 0, 0],
        &[Grant {
            handle: endpoint,
            rights: sys::SEND,
        }],
        deadline()?,
    )
    .map_err(|_| 205u64)?;
    let request = sys::receive_until(endpoint, 0, deadline()?).map_err(|_| 206u64)?;
    let [kind, instance, value, reserved] = request.message;
    if kind != REQUEST || !(1..=2).contains(&instance) || value != 41 || reserved != 0 {
        return Err(207);
    }
    let reply = request.reply.ok_or(208u64)?;
    sys::reply(endpoint, reply, [RESPONSE, instance, value + 1, 0]).map_err(|_| 209u64)?;
    let stop = sys::receive_until(endpoint, 0, deadline()?).map_err(|_| 210u64)?;
    if stop.message != [STOP, instance, 0, 0] || stop.reply.is_some() {
        return Err(211);
    }
    // Owner death, rather than explicit endpoint close, invalidates the client.
    sys::close(bootstrap).map_err(|_| 212u64)?;
    sys::close(factory).map_err(|_| 213u64)?;
    Ok(())
}

fn deadline() -> Result<u64, u64> {
    sys::clock()
        .map_err(|_| 214u64)?
        .checked_add(WAIT_NS)
        .ok_or(215u64)
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    sys::exit(255)
}
