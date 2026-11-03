use super::{
    BootFramePool, CallStats, Handles, IpcRuntime, KernelMemory, KernelObject, KernelRegion,
    Objects, RefCell, Tasks,
};
use kryo_core::capability::Rights;
use kryo_core::ipc::ChannelBudget;
use kryo_core::object::ObjectKind;
use kryo_core::process::{ExitReason, ExitStatus};
use kryo_hal::DirectMap;

pub(crate) fn run(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    #[cfg(feature = "boot-tests")]
    {
        super::channels::checks::run();
        super::ipc_runtime::checks::run();
    }
    let before = pool.stats();
    let retained_before = memory.borrow().retained_pages();
    {
        let objects = Objects::growing();
        let mut tasks = Tasks::growing();
        let mut ipc = IpcRuntime::growing(&objects);
        let budget = ChannelBudget::new(8);
        let probe =
            crate::boot::module_image("kryo-service-probe").expect("Missing service probe image");
        let peer =
            crate::boot::module_image("kryo-service-peer").expect("Missing service peer image");
        let mut initial = super::launch::prepare(pool, direct, memory, region, 2, probe)
            .unwrap_or_else(|mut failure| {
                failure.pending.close(pool, region);
                panic!("Cannot prepare service probe: {:?}", failure.error);
            });
        let mut handles = Handles::growing();
        let factory = objects
            .create(
                ObjectKind::ChannelFactory,
                KernelObject::ChannelFactory(budget.clone()),
            )
            .unwrap();
        let image = objects
            .create(ObjectKind::Image, KernelObject::Image(peer))
            .unwrap();
        let debug = objects
            .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
            .unwrap();
        initial.context.rdi = handles
            .insert(factory, Rights::CREATE | Rights::DUP | Rights::TRANSFER)
            .unwrap()
            .as_raw();
        initial.context.rsi = handles.insert(image, Rights::EXECUTE).unwrap().as_raw();
        initial.context.rcx = handles.insert(debug, Rights::WRITE).unwrap().as_raw();
        initial.process.handles = handles;
        let supervisor = tasks.spawn(initial).expect("Cannot publish service probe");
        let process = tasks.process_for(supervisor).unwrap();
        let mut calls = CallStats::default();
        let deadline = kryo_hal::monotonic_time()
            .unwrap()
            .saturating_add(5_000_000_000);
        let mut result = None;
        for _ in 0..8192 {
            assert!(
                kryo_hal::monotonic_time().unwrap() < deadline,
                "Service probe stalled"
            );
            ipc.settle(&mut tasks);
            if result.is_none() {
                result = tasks.terminal(process);
            }
            let mut cursor = None;
            while let Some(id) = tasks.next_dying(cursor) {
                cursor = Some(id);
                drop(
                    tasks
                        .reap(id, &mut ipc, |resources| resources.close(pool, region))
                        .expect("Cannot retire service probe resources"),
                );
            }
            // Handle-table destruction can expose registry-only channels.
            ipc.settle(&mut tasks);
            if tasks.live_count() == 0 {
                break;
            }
            if tasks.has_runnable() {
                super::smp::step(&mut tasks, &mut calls, &mut ipc, region);
            } else if ipc.waits.next_deadline().is_some() {
                super::idle_until_deadline(&mut tasks, &mut ipc);
            } else {
                panic!("Service probe has no wake source");
            }
        }
        assert_eq!(
            result,
            Some(ExitStatus {
                reason: ExitReason::Exited,
                code: 0
            })
        );
        assert_eq!(calls.last_debug, Some((supervisor, 4, 2)));
        assert_eq!(calls.spawned, 2);
        assert_eq!(tasks.live_count(), 0);
        assert_eq!(ipc.waits.next_deadline(), None);
        assert_eq!(ipc.waits.next_ready(), None);
        assert_eq!(budget.live_count(), 0);
        assert_eq!(objects.live_count(), 0);
    }
    let retained = memory.borrow().retained_pages() - retained_before;
    let after = pool.stats();
    assert_eq!(after.total, before.total);
    assert_eq!(
        after.free,
        before.free - retained,
        "Service probe retained physical frames"
    );
    kryo_hal::emergency_write_str("SERVICE: native IPC and restart OK instances=2 quota=8\n");
}
