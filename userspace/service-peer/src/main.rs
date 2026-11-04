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
const MEMORY_VIEW: u64 = 0x200_0000;
const MEMORY_PATTERN: u64 = 0xfedc_ba98_7654_3210;

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

extern "C" fn peer_main(bootstrap: u64, factory: u64, _features: u64, mode: u64) -> ! {
    let bootstrap = Handle::from_raw(bootstrap).unwrap_or_else(|| sys::exit(200));
    let factory = Handle::from_raw(factory).unwrap_or_else(|| sys::exit(201));
    if mode > 1 {
        sys::exit(216);
    }
    match serve(bootstrap, factory, mode) {
        Ok(()) => sys::exit(0),
        Err(code) => sys::exit(code),
    }
}

fn serve(bootstrap: Handle, factory: Handle, mode: u64) -> Result<(), u64> {
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
    let capacity = usize::from(mode == 1);
    let request = sys::receive_until(endpoint, capacity, deadline()?).map_err(|_| 206u64)?;
    let [kind, instance, value, reserved] = request.message;
    if kind != REQUEST || !(1..=2).contains(&instance) || value != 41 || reserved != 0 {
        return Err(207);
    }
    let reply = request.reply.ok_or(208u64)?;
    if mode == 1 {
        let [Some(memory), None] = request.handles else {
            return Err(217);
        };
        check_memory(memory)?;
    }
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

fn check_memory(memory: Handle) -> Result<(), u64> {
    let info = sys::handle_info(memory).map_err(|_| 218u64)?;
    if info.kind != sys::ObjectKind::Memory || info.rights != (sys::READ | sys::MAP) {
        return Err(219);
    }
    // This fixture owns a vacant view. The C requester keeps the shared page
    // unchanged until reply; reduced rights do not make its other aliases immutable.
    if unsafe { sys::memory_map(memory, MEMORY_VIEW, 0, true) } != Err(Error::Kernel(-3)) {
        return Err(220);
    }
    unsafe { sys::memory_map(memory, MEMORY_VIEW, 0, false) }.map_err(|_| 221u64)?;
    for index in 0..512 {
        // The mapped object retains its backing. No reference escapes this
        // volatile read, and the request protocol excludes concurrent writes.
        let value = unsafe { (MEMORY_VIEW as *const u64).add(index).read_volatile() };
        if value != (MEMORY_PATTERN ^ index as u64) {
            return Err(222);
        }
    }
    // All raw reads have ended before the mapping and received handle retire.
    unsafe { sys::memory_unmap(MEMORY_VIEW) }.map_err(|_| 223u64)?;
    sys::close(memory).map_err(|_| 224u64)?;
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
