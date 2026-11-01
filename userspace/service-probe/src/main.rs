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
const TIMEOUT_NS: u64 = 2_000_000;

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
    main = sym probe_main,
);

extern "C" fn probe_main(factory: u64, image: u64, _features: u64, debug: u64) -> ! {
    let factory = Handle::from_raw(factory).unwrap_or_else(|| sys::exit(200));
    let image = Handle::from_raw(image).unwrap_or_else(|| sys::exit(201));
    let debug = Handle::from_raw(debug).unwrap_or_else(|| sys::exit(202));
    match probe(factory, image, debug) {
        Ok(()) => {
            sys::debug_log(debug, 4, 2).unwrap_or_else(|_| sys::exit(203));
            sys::close(debug).unwrap_or_else(|_| sys::exit(204));
            sys::exit(0)
        }
        Err(code) => sys::exit(code),
    }
}

fn probe(factory: Handle, image: Handle, debug: Handle) -> Result<(), u64> {
    let stripped = sys::duplicate(factory, sys::TRANSFER).map_err(|_| 205u64)?;
    if sys::create_endpoint(stripped) != Err(Error::Kernel(-3))
        || sys::create_notification(stripped) != Err(Error::Kernel(-3))
    {
        return Err(206);
    }
    sys::close(stripped).map_err(|_| 207u64)?;
    if sys::create_endpoint(debug) != Err(Error::Kernel(-4)) {
        return Err(208);
    }

    let bootstrap = sys::create_endpoint(factory).map_err(|_| 209u64)?;
    let notification = sys::create_notification(factory).map_err(|_| 210u64)?;
    check_notification(notification)?;
    check_budget(factory, 6)?;

    let mut old_endpoint = None;
    for instance in 1..=2 {
        let process = sys::spawn(
            image,
            &[
                Grant {
                    handle: bootstrap,
                    rights: sys::SEND | sys::TRANSFER,
                },
                Grant {
                    handle: factory,
                    rights: sys::CREATE | sys::TRANSFER,
                },
            ],
        )
        .map_err(|_| 211u64)?;
        let advertisement = sys::receive_until(bootstrap, 1, deadline()?).map_err(|_| 212u64)?;
        if advertisement.message != [HELLO, 0, 0, 0]
            || advertisement.reply.is_some()
            || advertisement.handles[1].is_some()
        {
            return Err(213);
        }
        let endpoint = advertisement.handles[0].ok_or(214u64)?;
        if Some(endpoint) == old_endpoint {
            return Err(215);
        }
        let response = sys::call_until(endpoint, [REQUEST, instance, 41, 0], deadline()?)
            .map_err(|_| 216u64)?;
        if response != [RESPONSE, instance, 42, 0] {
            return Err(217);
        }
        if let Err(failure) = sys::receive_until(endpoint, 0, deadline()?) {
            if failure.error != Error::Kernel(-3) || failure.accepted {
                return Err(218);
            }
        } else {
            return Err(219);
        }
        if instance == 1 {
            check_budget(factory, 5)?;
        }
        sys::send_until(endpoint, [STOP, instance, 0, 0], deadline()?).map_err(|_| 220u64)?;
        let status = sys::process_wait_until(process, deadline()?).map_err(|_| 221u64)?;
        if status.code != 0 || status.reason != sys::ExitReason::Exited {
            return Err(222);
        }
        let failure = sys::call_until(endpoint, [REQUEST, instance, 41, 0], deadline()?)
            .err()
            .ok_or(223u64)?;
        if failure.error != Error::Kernel(-7) || failure.accepted {
            return Err(224);
        }
        if let Some(old) = old_endpoint {
            let failure = sys::call_until(old, [REQUEST, instance - 1, 41, 0], deadline()?)
                .err()
                .ok_or(225u64)?;
            if failure.error != Error::Kernel(-7) || failure.accepted {
                return Err(226);
            }
            sys::close(old).map_err(|_| 227u64)?;
        }
        old_endpoint = Some(endpoint);
        sys::close(process).map_err(|_| 228u64)?;
    }
    sys::close(old_endpoint.unwrap()).map_err(|_| 229u64)?;
    sys::close(notification).map_err(|_| 230u64)?;
    sys::close(bootstrap).map_err(|_| 231u64)?;
    sys::close(factory).map_err(|_| 232u64)?;
    sys::close(image).map_err(|_| 233u64)?;
    Ok(())
}

fn check_notification(notification: Handle) -> Result<(), u64> {
    let signal = sys::duplicate(notification, sys::SIGNAL).map_err(|_| 234u64)?;
    if sys::wait_notification_until(signal, deadline()?) != Err(Error::Kernel(-3)) {
        return Err(235);
    }
    sys::signal(signal).map_err(|_| 236u64)?;
    sys::signal(signal).map_err(|_| 237u64)?;
    sys::wait_notification_until(notification, deadline()?).map_err(|_| 238u64)?;
    let deadline = sys::clock()
        .map_err(|_| 239u64)?
        .checked_add(TIMEOUT_NS)
        .ok_or(240u64)?;
    if sys::wait_notification_until(notification, deadline) != Err(Error::Kernel(-10)) {
        return Err(241);
    }
    sys::close(signal).map_err(|_| 242u64)?;
    Ok(())
}

fn check_budget(factory: Handle, available: usize) -> Result<(), u64> {
    let mut channels = [None; 6];
    for (index, slot) in channels.iter_mut().enumerate().take(available) {
        *slot = Some(if index % 2 == 0 {
            sys::create_endpoint(factory).map_err(|_| 243u64)?
        } else {
            sys::create_notification(factory).map_err(|_| 244u64)?
        });
    }
    if sys::create_endpoint(factory) != Err(Error::Kernel(-5))
        || sys::create_notification(factory) != Err(Error::Kernel(-5))
    {
        return Err(245);
    }
    for channel in channels.into_iter().flatten() {
        sys::close(channel).map_err(|_| 246u64)?;
    }
    let reused = sys::create_endpoint(factory).map_err(|_| 247u64)?;
    sys::close(reused).map_err(|_| 248u64)?;
    Ok(())
}

fn deadline() -> Result<u64, u64> {
    sys::clock()
        .map_err(|_| 249u64)?
        .checked_add(WAIT_NS)
        .ok_or(250u64)
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    sys::exit(255)
}
