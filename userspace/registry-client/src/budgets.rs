use super::Failure;
use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use kryo_libsys::{self as sys, BudgetQuery, Handle, Resource};

const MEMORY_VIEW: u64 = 0x0200_0000;
const WAIT_NS: u64 = 1_000_000_000;
static FACTORY: AtomicU64 = AtomicU64::new(0);
static NOTIFICATIONS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static RELEASE: AtomicBool = AtomicBool::new(false);
static WAIT_REJECTED: AtomicBool = AtomicBool::new(false);
static ERROR: AtomicBool = AtomicBool::new(false);
static FINISHED: AtomicUsize = AtomicUsize::new(0);

global_asm!(
    r#"
    .section .text,"ax"
    .global budget_thread_start
    .type budget_thread_start,@function
budget_thread_start:
    cld
    xor ebp, ebp
    call {main}
    ud2
    .size budget_thread_start, .-budget_thread_start
"#,
    main = sym thread_main,
);

unsafe extern "C" {
    fn budget_thread_start() -> !;
}

pub(super) fn check(factory: Handle, debug: Handle, service: u64) -> Result<(), Failure> {
    for (resource, expected) in [
        (Resource::SharedPages, 8),
        (Resource::Handles, 32),
        (Resource::Waits, 2),
        (Resource::Threads, 4),
    ] {
        if sys::budget_self(resource, BudgetQuery::Limit).map_err(|_| Failure::BudgetPolicy)?
            != expected
        {
            return Err(Failure::BudgetPolicy);
        }
    }
    let plan = sys::manifest_create(service, 1, 0).map_err(|_| Failure::BudgetPolicy)?;
    if sys::manifest_budget(plan, factory) != Err(sys::Error::Kernel(-4)) {
        return Err(Failure::BudgetPolicy);
    }
    sys::close(plan).map_err(|_| Failure::BudgetPolicy)?;
    handles(factory)?;
    mark(debug, service, 1)?;
    shared_memory()?;
    mark(debug, service, 2)?;
    threads(factory)?;
    mark(debug, service, 3)?;
    Ok(())
}

fn handles(factory: Handle) -> Result<(), Failure> {
    let endpoint = sys::create_endpoint(factory).map_err(|_| Failure::BudgetHandles)?;
    let baseline = used(Resource::Handles)?;
    let mut copies = [None; 32];
    for slot in copies.iter_mut().take((32 - baseline) as usize) {
        *slot = Some(sys::duplicate(endpoint, sys::SEND).map_err(|_| Failure::BudgetHandles)?);
    }
    if used(Resource::Handles)? != 32
        || sys::duplicate(endpoint, sys::SEND) != Err(sys::Error::Kernel(-5))
        || sys::memory_create(1) != Err(sys::Error::Kernel(-5))
        || used(Resource::SharedPages)? != 0
    {
        return Err(Failure::BudgetHandles);
    }
    let rejected = sys::receive_for(endpoint, 1, 1_000_000)
        .err()
        .ok_or(Failure::BudgetHandles)?;
    if rejected.error != sys::Error::Kernel(-5) || rejected.accepted || used(Resource::Waits)? != 0
    {
        return Err(Failure::BudgetHandles);
    }
    // Shutdown does not need another handle or wait credit.
    sys::close_endpoint(endpoint).map_err(|_| Failure::BudgetHandles)?;
    for copy in copies.into_iter().flatten() {
        sys::close(copy).map_err(|_| Failure::BudgetHandles)?;
    }
    sys::close(endpoint).map_err(|_| Failure::BudgetHandles)?;
    if used(Resource::Handles)? != baseline - 1 {
        return Err(Failure::BudgetHandles);
    }
    Ok(())
}

fn shared_memory() -> Result<(), Failure> {
    let memory = sys::memory_create(8).map_err(|_| Failure::BudgetMemory)?;
    if used(Resource::SharedPages)? != 8 || sys::memory_create(1) != Err(sys::Error::Kernel(-5)) {
        return Err(Failure::BudgetMemory);
    }
    // This fixture owns the vacant test VA and never creates a Rust reference
    // into it. No other thread exists until this mapping has retired.
    unsafe { sys::memory_map(memory, MEMORY_VIEW, 0, false) }.map_err(|_| Failure::BudgetMemory)?;
    sys::close(memory).map_err(|_| Failure::BudgetMemory)?;
    if used(Resource::SharedPages)? != 8 || sys::memory_create(1) != Err(sys::Error::Kernel(-5)) {
        return Err(Failure::BudgetMemory);
    }
    unsafe { sys::memory_unmap(MEMORY_VIEW) }.map_err(|_| Failure::BudgetMemory)?;
    if used(Resource::SharedPages)? != 0 {
        return Err(Failure::BudgetMemory);
    }
    let reused = sys::memory_create(1).map_err(|_| Failure::BudgetMemory)?;
    sys::close(reused).map_err(|_| Failure::BudgetMemory)?;
    Ok(())
}

fn threads(factory: Handle) -> Result<(), Failure> {
    if used(Resource::Threads)? != 1 || used(Resource::Waits)? != 0 {
        return Err(Failure::BudgetThreads);
    }
    FACTORY.store(factory.raw(), Ordering::Release);
    let mut controls = [None; 3];
    for (index, slot) in controls.iter_mut().enumerate() {
        // Native entry CALLs Rust. Arguments are numbers; shared data is static
        // and atomic, and all workers retire before the fixture returns.
        *slot = Some(
            unsafe { sys::thread_create(budget_thread_start, index as u64) }
                .map_err(|_| Failure::BudgetThreads)?,
        );
        wait_until(|| {
            if index < 2 {
                Ok(used(Resource::Waits)? == index as u64 + 1)
            } else {
                Ok(WAIT_REJECTED.load(Ordering::Acquire))
            }
        })?;
    }
    if used(Resource::Threads)? != 4 || used(Resource::Waits)? != 2 {
        return Err(Failure::BudgetThreads);
    }
    if unsafe { sys::thread_create(budget_thread_start, 3) } != Err(sys::Error::Kernel(-5)) {
        return Err(Failure::BudgetThreads);
    }
    for control in controls.into_iter().flatten() {
        sys::close(control).map_err(|_| Failure::BudgetThreads)?;
    }
    if unsafe { sys::thread_create(budget_thread_start, 3) } != Err(sys::Error::Kernel(-5))
        || sys::sleep_until(u64::MAX) != Err(sys::Error::Kernel(-5))
    {
        return Err(Failure::BudgetThreads);
    }
    for notification in NOTIFICATIONS.iter().take(2) {
        let notification =
            Handle::from_raw(notification.load(Ordering::Acquire)).ok_or(Failure::BudgetThreads)?;
        sys::signal(notification).map_err(|_| Failure::BudgetThreads)?;
    }
    RELEASE.store(true, Ordering::Release);
    wait_until(|| Ok(FINISHED.load(Ordering::Acquire) == 3 && used(Resource::Threads)? == 1))?;
    if used(Resource::Waits)? != 0 {
        return Err(Failure::BudgetThreads);
    }
    Ok(())
}

extern "C" fn thread_main(index: u64) -> ! {
    let result = run_thread(index as usize);
    if result.is_err() {
        ERROR.store(true, Ordering::Release);
    }
    FINISHED.fetch_add(1, Ordering::Release);
    // Successful workers released their handles. An error is reported to the
    // main thread, whose process exit retires any failed worker's remainder.
    unsafe { sys::thread_exit(u64::from(result.is_err())) }
}

fn run_thread(index: usize) -> Result<(), ()> {
    let slot = NOTIFICATIONS.get(index).ok_or(())?;
    let factory = Handle::from_raw(FACTORY.load(Ordering::Acquire)).ok_or(())?;
    let notification = sys::create_notification(factory).map_err(|_| ())?;
    slot.store(notification.raw(), Ordering::Release);
    let result = sys::wait_notification(notification);
    if index < 2 {
        result.map_err(|_| ())?;
    } else {
        if result != Err(sys::Error::Kernel(-5)) {
            return Err(());
        }
        WAIT_REJECTED.store(true, Ordering::Release);
        while !RELEASE.load(Ordering::Acquire) {
            sys::yield_now().map_err(|_| ())?;
        }
    }
    sys::close(notification).map_err(|_| ())
}

fn wait_until(mut condition: impl FnMut() -> Result<bool, Failure>) -> Result<(), Failure> {
    let deadline = sys::clock()
        .map_err(|_| Failure::BudgetThreads)?
        .checked_add(WAIT_NS)
        .ok_or(Failure::BudgetThreads)?;
    loop {
        if ERROR.load(Ordering::Acquire) {
            return Err(Failure::BudgetThreads);
        }
        if condition()? {
            return Ok(());
        }
        if sys::clock().map_err(|_| Failure::BudgetThreads)? >= deadline {
            return Err(Failure::BudgetThreads);
        }
        sys::yield_now().map_err(|_| Failure::BudgetThreads)?;
    }
}

fn used(resource: Resource) -> Result<u64, Failure> {
    sys::budget_self(resource, BudgetQuery::Used).map_err(|_| Failure::BudgetPolicy)
}

fn mark(debug: Handle, service: u64, stage: u64) -> Result<(), Failure> {
    sys::debug_log(debug, 12, (service << 32) | stage).map_err(|_| Failure::BudgetPolicy)
}
