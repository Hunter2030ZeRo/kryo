use crate::runtime::{
    BootFramePool, Endpoint, IpcRuntime, KernelMemory, KernelObject, KernelRegion, Object, Objects,
    RefCell, Tasks,
};
use kryo_core::capability::Rights;
use kryo_core::ipc::ChannelBudget;
use kryo_core::object::ObjectKind;
use kryo_core::process::ProcessId;
use kryo_core::task::TaskId;
use kryo_hal::DirectMap;

pub(super) struct Service {
    pub task: TaskId,
    pub owner: ProcessId,
    pub channels: ChannelBudget,
}

/// The optional libc probe receives a separate memory-backed validation peer.
/// Neither this fixture nor its authority is installed in normal services.
pub(super) fn prepare<'kernel, 'objects>(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &'kernel RefCell<KernelMemory>,
    region: &mut KernelRegion,
    tasks: &mut Tasks<'kernel, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
    objects: &'objects Objects,
) -> (Service, Object<'objects>) {
    let image = crate::boot::module_image("kryo-service-peer").expect("Missing libc file peer");
    let slot = region.find_stack(pool, direct).unwrap();
    let resources = super::super::launch::prepare(pool, direct, memory, region, slot, image)
        .unwrap_or_else(|mut failure| {
            failure.pending.close(pool, region);
            panic!("Cannot prepare libc file peer: {:?}", failure.error);
        });
    let task = tasks
        .spawn_suspended(resources)
        .unwrap_or_else(|_| panic!("Cannot publish libc file peer"));
    let owner = tasks.process_for(task).unwrap();
    let channels = ChannelBudget::new(16);
    let endpoint = objects
        .create(
            ObjectKind::Endpoint,
            KernelObject::Endpoint(RefCell::new(Endpoint::leased(
                task,
                channels.try_reserve().unwrap(),
            ))),
        )
        .unwrap();
    ipc.try_register(endpoint.try_clone().unwrap()).unwrap();
    let mut resources = tasks.resources_mut(task).unwrap();
    let root = resources
        .process
        .handles
        .insert(
            endpoint.try_clone().unwrap(),
            Rights::RECEIVE | Rights::CONTROL,
        )
        .unwrap();
    let gate = ipc
        .create_gate(
            task,
            endpoint,
            kryo_interfaces::file::NAMESPACE_LABEL,
            &mut resources.process.handles,
        )
        .unwrap();
    let namespace = resources
        .process
        .handles
        .lookup(gate, ObjectKind::EndpointGate, Rights::SEND)
        .unwrap();
    resources.process.handles.close(gate).unwrap();
    resources.context.rdi = root.as_raw();
    resources.context.rsi = 0;
    resources.context.rcx = 4;
    (
        Service {
            task,
            owner,
            channels,
        },
        namespace,
    )
}
