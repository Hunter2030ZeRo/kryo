use super::super::vm_backend::{BackendError, UserSpace};
use super::{
    BootFramePool, CallStats, DirectMap, ExtendedState, IpcRuntime, KernelMemory, KernelRegion,
    Objects, Permissions, RefCell, Resources, SharedLease, TaskState, Tasks, UserContext,
    UserEntryError, UserReason, new_shared_space, step,
};
use kryo_core::exec::{ElfImage, LoadLimits};
use kryo_core::mm::vm::VmError;

const USER_STACK: u64 = 0x80_0000;
const STACK_PAGES: u64 = 4;
const TLS: u64 = 0x90_0000;
const RESULT: u64 = 0x40_3010;

#[derive(Debug)]
enum LoadError {
    ReservedAddress,
    Vm(VmError<BackendError>),
    Entry(UserEntryError),
}

pub(super) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let before = pool.stats();
    let bytes = super::super::native_image().expect("Missing native user module");
    let image =
        ElfImage::<8>::parse(bytes, LoadLimits { max_pages: 8 }).expect("Invalid native user ELF");
    let _ = kryo_hal::print(format_args!(
        "ELF: module validated bytes={} pages={} entry={:#x}\n",
        bytes.len(),
        image.page_count(),
        image.entry(),
    ));
    let objects = Objects::new();
    let mut tasks = Tasks::new();
    let mut ipc = IpcRuntime::with_objects(&objects);
    let first = tasks
        .spawn(new_task(pool, direct, kernel, region, 2, &image, 41))
        .unwrap();
    let second = tasks
        .spawn(new_task(pool, direct, kernel, region, 7, &image, 42))
        .unwrap();
    assert_ne!(
        tasks
            .resources_mut(first)
            .unwrap()
            .process
            .space
            .root_frame()
            .unwrap(),
        tasks
            .resources_mut(second)
            .unwrap()
            .process
            .space
            .root_frame()
            .unwrap()
    );
    let xsave = tasks
        .resources_mut(first)
        .unwrap()
        .extended
        .as_ref()
        .unwrap()
        .uses_xsave();
    let avx = tasks
        .resources_mut(first)
        .unwrap()
        .extended
        .as_ref()
        .unwrap()
        .avx_enabled();
    let mut calls = CallStats::default();
    let mut reaped = [false; 2];
    let mut switches = 0;
    for _ in 0..2048 {
        if tasks.live_count() == 0 {
            break;
        }
        assert!(
            tasks.has_runnable(),
            "Native ELF tasks blocked unexpectedly"
        );
        let (_, outcome) = step(&mut tasks, &mut calls, &mut ipc);
        let outcome = outcome.expect("Native context rejected");
        assert_ne!(outcome.reason, UserReason::Fault);
        switches += usize::from(outcome.reason == UserReason::Preempted);
        for (index, id) in [first, second].into_iter().enumerate() {
            if tasks.state(id) != Ok(TaskState::Dying) {
                continue;
            }
            let resources = tasks.resources_mut(id).unwrap();
            let identity = 41 + index as u64;
            assert_eq!(resources.exit_code, identity);
            assert!(resources.ticks > 0, "Native sample was never preempted");
            assert_eq!(resources.context.fs_base, TLS);
            assert_eq!(resources.context.gs_base, 0);
            let mut output = [0; 8];
            resources
                .process
                .space
                .read_page(RESULT & !4095, (RESULT & 4095) as usize, &mut output)
                .unwrap();
            assert_eq!(
                u64::from_le_bytes(output),
                [0x746c_7300_0001_d281, 0x746c_7300_0001_d2b2][index]
            );
            assert!(resources.wait.is_none());
            drop(
                tasks
                    .reap(id, |resources| resources.close(pool, region))
                    .unwrap(),
            );
            reaped[index] = true;
        }
    }
    assert_eq!(reaped, [true, true]);
    assert_eq!(calls.yields, 4);
    assert_eq!(calls.tls_sets, 2);
    assert_eq!(tasks.live_count(), 0);
    assert_eq!(ipc.transfer_count(), 0);
    drop(ipc);
    assert_eq!(objects.live_count(), 0);
    assert_eq!(pool.stats(), before);
    check_scalar_mode(pool, direct, kernel, region);
    let _ = kryo_hal::print(format_args!(
        "ELF: native execution OK tasks=2 preemptions={} xsave={} avx={}\n",
        switches, xsave, avx,
    ));
    kryo_hal::emergency_write_str("STATE: x87 SIMD TLS isolation OK\n");
}

fn new_task<'kernel, 'objects>(
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &'kernel RefCell<KernelMemory>,
    region: &mut KernelRegion,
    first_slot: usize,
    image: &ElfImage<'_, 8>,
    identity: u64,
) -> Resources<'kernel, 'objects> {
    let extended = ExtendedState::new().expect("Unsupported native CPU state");
    let mut space = new_shared_space(pool, kernel).expect("Cannot create native address space");
    let context = match load(&mut space, image, identity, extended.avx_enabled()) {
        Ok(context) => context,
        Err(error) => {
            space.close().expect("Cannot retire rejected native image");
            match error {
                LoadError::ReservedAddress => panic!("Native image overlaps startup memory"),
                LoadError::Vm(error) => panic!("Native mapping failed: {:?}", error),
                LoadError::Entry(error) => panic!("Native context failed: {:?}", error),
            }
        }
    };
    Resources {
        process: crate::runtime::ProcessResources {
            space,
            handles: super::Handles::new(),
            control: None,
            pending: None,
            pending_thread: None,
            space_closed: false,
        },
        context,
        stack: super::KernelStack::new(pool, direct, region, first_slot),
        ticks: 0,
        exit_code: u64::MAX,
        exit_reason: kryo_core::process::ExitReason::Exited,
        wait: None,
        receive_slots: None,
        received_wait: None,
        received_handles: [None; 2],
        pool,
        memory: kernel,
        extended: Some(extended),
        direct,
        user_slot: 0,
        user_stack_owned: false,
        thread_control: None,
    }
}

fn load<'objects>(
    space: &mut UserSpace<'_, SharedLease<'objects>>,
    image: &ElfImage<'_, 8>,
    identity: u64,
    avx: bool,
) -> Result<UserContext, LoadError> {
    for page in image.pages() {
        if (USER_STACK - 4096..USER_STACK + STACK_PAGES * 4096).contains(&page.address())
            || page.address() == TLS
        {
            return Err(LoadError::ReservedAddress);
        }
    }
    let mut context = UserContext::new(image.entry(), USER_STACK + STACK_PAGES * 4096)
        .map_err(LoadError::Entry)?;
    context.rdi = identity;
    context.rsi = TLS;
    context.rdx = u64::from(avx);
    let mut buffer = [0; 4096];
    for page in image.pages() {
        page.initialize(&mut buffer);
        space
            .map_page(page.address(), page.permissions(), &buffer)
            .map_err(LoadError::Vm)?;
    }
    for index in 0..STACK_PAGES {
        space
            .map_page(USER_STACK + index * 4096, Permissions::ReadWrite, &[])
            .map_err(LoadError::Vm)?;
    }
    space
        .map_page(TLS, Permissions::ReadWrite, &[])
        .map_err(LoadError::Vm)?;
    Ok(context)
}

fn check_scalar_mode(
    pool: &'static BootFramePool,
    direct: DirectMap,
    kernel: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let before = pool.stats();
    let mut program = super::Program::new();
    program.emit(&[0xd9, 0xee]); // fldz must still trap in scalar mode.
    let mut tasks = Tasks::new();
    let id = tasks
        .spawn(Resources::new(pool, direct, kernel, region, 2, &program))
        .unwrap();
    let mut calls = CallStats::default();
    let mut ipc = IpcRuntime::new();
    let mut faulted = false;
    let mut last_preemption = None;
    for _ in 0..32 {
        let (task, outcome) = step(&mut tasks, &mut calls, &mut ipc);
        assert_eq!(task, id, "Scalar FP check dispatched another task");
        let outcome = outcome.expect("Scalar FP context rejected");
        match outcome.reason {
            UserReason::Preempted => last_preemption = Some(outcome),
            UserReason::Fault if outcome.vector == 7 => {
                faulted = true;
                break;
            }
            _ => panic!("Expected scalar #NM, observed {:?}", outcome),
        }
    }
    if !faulted {
        let _ = kryo_hal::print(format_args!(
            "STATE: scalar #NM absent after 32 preemptions last={:?}\n",
            last_preemption,
        ));
        panic!("Scalar #NM check exhausted its execution budget");
    }
    drop(
        tasks
            .reap(id, |resources| resources.close(pool, region))
            .unwrap(),
    );
    assert_eq!(pool.stats(), before);
}
