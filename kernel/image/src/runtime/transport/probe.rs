use super::super::{CallStats, Handles, KernelRegion, Objects};
use super::*;
use kryo_core::sched::TaskState;

pub(crate) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    if crate::boot::module_image("kryo-block-probe-enabled").ok()
        != Some(b"KRYO-BLOCK-PROBE-V1\n".as_slice())
    {
        return;
    }
    let function = crate::boot::storage::block_device()
        .expect("Enabled block probe has no modern VirtIO block device");
    let before = pool.stats();
    let retained = kernel.borrow().retained_pages();
    for mode in 0..2 {
        instance(pool, direct, kernel, region, function, mode);
    }
    assert_eq!(
        pool.stats().free + kernel.borrow().retained_pages() - retained,
        before.free
    );
    kryo_hal::emergency_write_str("BLOCK: Ring3 commands and private DMA retirement OK cases=2\n");
}

fn instance(
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
    function: kryo_hal::PciFunction,
    mode: u64,
) {
    // The explicit boot marker authorizes this validation driver. Its first
    // read additionally verifies the disposable disk signature before writes.
    let mut hardware = unsafe { VirtioPci::new(function) }.expect("Cannot claim VirtIO transport");
    let deadline = now() + 2_000_000_000;
    while !hardware.reset().expect("Cannot reset VirtIO transport") {
        pause(deadline);
    }
    hardware
        .prepare()
        .expect("Cannot acknowledge VirtIO transport");
    let read_only = hardware.features().expect("Cannot read VirtIO features") & (1 << 5) != 0;
    let objects = Objects::new();
    let mut tasks = Tasks::new();
    let mut ipc = IpcRuntime::with_objects(&objects);
    let image = crate::boot::module_image("kryo-virtio-block").expect("Missing Ring3 block driver");
    let slot = region
        .find_stack(pool, direct)
        .expect("No block driver entry-stack cell");
    let mut resources = super::super::launch::prepare(pool, direct, kernel, region, slot, image)
        .unwrap_or_else(|_| panic!("Cannot prepare Ring3 block driver"));
    resources.process.handles = Handles::growing();
    let task = tasks
        .spawn(resources)
        .expect("Cannot publish Ring3 block driver");
    let owner = tasks.process_for(task).unwrap();
    let device = objects
        .create(ObjectKind::Device, KernelObject::Transport(owner))
        .unwrap();
    let transport = Transport::create(
        owner,
        device.try_clone().unwrap(),
        hardware,
        pool,
        direct,
        kernel,
    )
    .unwrap_or_else(|error| panic!("Cannot prepare DMA transport: {:?}", error));
    #[cfg(feature = "boot-tests")]
    let transport = {
        let mut transport = transport;
        if mode == 1 {
            transport.retirement_failures = 1;
        }
        transport
    };
    let mut resources = tasks.resources_mut(task).unwrap();
    resources.context.rdi = resources
        .process
        .handles
        .insert(
            device.try_clone().unwrap(),
            Rights::READ | Rights::WRITE | Rights::CONTROL,
        )
        .unwrap()
        .as_raw();
    resources.context.rsi = mode;
    resources.context.rdx = resources
        .process
        .handles
        .insert(device.try_clone().unwrap(), Rights::READ)
        .unwrap()
        .as_raw();
    let slot = region
        .find_stack(pool, direct)
        .expect("No foreign caller entry-stack cell");
    let mut intruder = super::super::launch::prepare(pool, direct, kernel, region, slot, image)
        .unwrap_or_else(|_| panic!("Cannot prepare foreign device caller"));
    intruder.process.handles = Handles::growing();
    intruder.context.rdi = intruder
        .process
        .handles
        .insert(
            device.try_clone().unwrap(),
            Rights::READ | Rights::WRITE | Rights::CONTROL,
        )
        .unwrap()
        .as_raw();
    intruder.context.rsi = 2;
    let intruder = tasks
        .spawn(intruder)
        .expect("Cannot publish foreign device caller");
    let intruder_owner = tasks.process_for(intruder).unwrap();
    ipc.transport = Some(transport);
    let mut calls = CallStats::default();
    let deadline = now() + 5_000_000_000;
    let mut retained_failure = false;
    loop {
        assert!(now() < deadline, "Ring3 block driver stalled");
        super::super::expire_waits(&mut tasks, &mut ipc);
        if let Some(status) = tasks.terminal(owner) {
            assert_eq!(status.reason, kryo_core::process::ExitReason::Exited);
            assert_eq!(status.code, 0, "Ring3 block driver failed mode={}", mode);
            let before = pool.stats();
            match retire_bindings(&tasks, &mut ipc) {
                Ok(()) => {}
                Err(VirtioError::ResetPending) => {
                    assert_eq!(pool.stats(), before);
                    let transport = ipc.transport.as_ref().unwrap();
                    assert!(transport.storage.is_some());
                    if mode == 1 {
                        assert!(transport.pending.is_some());
                        retained_failure = true;
                    }
                    pause(deadline);
                }
                Err(error) => panic!("Block retirement failed with owners retained: {:?}", error),
            }
        }
        if tasks.terminal(owner).is_some()
            && tasks.terminal(intruder_owner).is_some()
            && ipc.transport.is_none()
        {
            assert_eq!(tasks.terminal(intruder_owner).unwrap().code, 0);
            break;
        }
        if tasks.has_runnable() {
            super::super::smp::step(&mut tasks, &mut calls, &mut ipc, region);
        } else {
            if ipc.waits.next_deadline().is_some() {
                super::super::idle_until_deadline(&mut tasks, &mut ipc);
            } else {
                pause(deadline);
            }
        }
    }
    #[cfg(feature = "boot-tests")]
    if mode == 1 {
        assert!(retained_failure);
        kryo_hal::emergency_write_str("BLOCK: cleanup retry retains queue and memory\n");
    }
    #[cfg(not(feature = "boot-tests"))]
    let _ = retained_failure;
    ipc.settle(&mut tasks);
    assert_eq!(tasks.state(task), Ok(TaskState::Dying));
    drop(
        tasks
            .reap(task, &mut ipc, |resources| resources.close(pool, region))
            .unwrap(),
    );
    drop(
        tasks
            .reap(intruder, &mut ipc, |resources| {
                resources.close(pool, region)
            })
            .unwrap(),
    );
    drop(ipc);
    drop(device);
    assert_eq!(objects.live_count(), 0);
    let _ = kryo_hal::print(format_args!(
        "BLOCK: driver complete mode={} readonly={}\n",
        mode, read_only
    ));
}

fn now() -> u64 {
    kryo_hal::monotonic_time().expect("Missing block clock")
}

fn pause(deadline: u64) {
    let now = now();
    assert!(now < deadline, "VirtIO retirement timed out; retain owners");
    unsafe { kryo_hal::arm_deadline(deadline.min(now + 100_000)) }
        .expect("Cannot arm block retry timer");
    kryo_hal::idle_until_interrupt().expect("Cannot wait for block retry");
    unsafe { kryo_hal::stop_timer() }.expect("Cannot stop block retry timer");
}
