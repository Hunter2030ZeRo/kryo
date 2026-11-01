use super::*;
use crate::runtime::{BootFramePool, CallStats, IpcRuntime, KernelMemory, KernelRegion, Tasks};
use core::sync::atomic::{AtomicBool, Ordering};
use kryo_core::capability::CapabilityError;
use kryo_core::process::ProcessTable;
use kryo_hal::{DirectMap, UserContext};

const METADATA: Metadata = Metadata {
    service: 11,
    instance: 7,
    argument: 42,
};
static FAIL_PUBLICATION: AtomicBool = AtomicBool::new(false);

pub(crate) fn run() {
    let mut owners = ProcessTable::<(), 2, 0>::new();
    let owner = owners.insert(()).unwrap();
    let other = owners.insert(()).unwrap();
    validation(owner, other);
    stale_source(owner);
    private_install(owner);
    capacity_rollback(owner);
    kryo_hal::emergency_write_str("STARTUP: authority and private grant rollback OK\n");
}

fn source<'objects>(objects: &'objects Objects, handles: &mut Handles<'objects>) -> Handle {
    let object = objects
        .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
        .unwrap();
    handles
        .insert(object, Rights::WRITE | Rights::TRANSFER)
        .unwrap()
}

fn entry(source: Handle, role: u64) -> Entry {
    Entry {
        role,
        kind: WireKind::DebugConsole,
        rights: Rights::WRITE.bits(),
        handle: source.as_raw(),
    }
}

fn count(object: &Object<'_>) -> usize {
    object.with(|object| {
        let KernelObject::LaunchManifest(manifest) = object else {
            unreachable!()
        };
        manifest.borrow().entries.len()
    })
}

fn validation(owner: ProcessId, other: ProcessId) {
    let objects = Objects::growing();
    let mut handles = Handles::growing();
    assert_eq!(
        create(owner, VERSION + 1, METADATA, &mut handles, &objects),
        Err(Error::InvalidArgument)
    );
    assert_eq!(
        create(
            owner,
            VERSION,
            Metadata {
                service: 0,
                ..METADATA
            },
            &mut handles,
            &objects
        ),
        Err(Error::InvalidArgument)
    );
    let source = source(&objects, &mut handles);
    let plan = create(owner, VERSION, METADATA, &mut handles, &objects).unwrap();
    let object = handles
        .lookup(plan, ObjectKind::LaunchManifest, Rights::READ)
        .unwrap();
    assert_eq!(
        handles.duplicate(plan, Rights::READ),
        Err(CapabilityError::PermissionDenied)
    );
    assert!(matches!(
        handles.prepare_transfer(&[TransferSpec {
            handle: plan,
            rights: Rights::READ
        }]),
        Err(CapabilityError::PermissionDenied)
    ));
    assert_eq!(
        grant(other, plan, entry(source, 1), &handles),
        Err(Error::PermissionDenied)
    );
    assert!(matches!(
        prepare(other, plan, &handles),
        Err(Error::PermissionDenied)
    ));
    for invalid in [
        Entry {
            role: 0,
            ..entry(source, 1)
        },
        Entry {
            rights: 0,
            ..entry(source, 1)
        },
        Entry {
            rights: Rights::EXECUTE.bits(),
            ..entry(source, 1)
        },
    ] {
        assert_eq!(
            grant(owner, plan, invalid, &handles),
            Err(Error::InvalidArgument)
        );
        assert_eq!(count(&object), 0);
    }
    assert_eq!(
        grant(
            owner,
            plan,
            Entry {
                kind: WireKind::Memory,
                rights: Rights::READ.bits(),
                ..entry(source, 1)
            },
            &handles
        ),
        Err(Error::WrongType)
    );
    let restricted = handles
        .insert(
            objects
                .create(ObjectKind::DebugConsole, KernelObject::DebugConsole)
                .unwrap(),
            Rights::WRITE,
        )
        .unwrap();
    assert_eq!(
        grant(owner, plan, entry(restricted, 1), &handles),
        Err(Error::PermissionDenied)
    );
    grant(owner, plan, entry(source, 1), &handles).unwrap();
    assert_eq!(
        grant(owner, plan, entry(source, 1), &handles),
        Err(Error::InvalidArgument)
    );
    for role in 2..=MAX_GRANTS as u64 {
        grant(owner, plan, entry(source, role), &handles).unwrap();
    }
    assert_eq!(
        grant(owner, plan, entry(source, MAX_GRANTS as u64 + 1), &handles),
        Err(Error::OutOfResources)
    );
    assert_eq!(count(&object), MAX_GRANTS);
    assert_eq!(handles.reserved_count(), 0);
    let empty = create(owner, VERSION, METADATA, &mut handles, &objects).unwrap();
    crate::heap::fail_next();
    assert_eq!(
        grant(owner, empty, entry(source, 1), &handles),
        Err(Error::OutOfResources)
    );
    let empty_object = handles
        .lookup(empty, ObjectKind::LaunchManifest, Rights::READ)
        .unwrap();
    assert_eq!(count(&empty_object), 0);
    grant(owner, empty, entry(source, 1), &handles).unwrap();
    crate::heap::fail_next();
    assert!(matches!(
        prepare(owner, empty, &handles),
        Err(Error::OutOfResources)
    ));
    assert_eq!(count(&empty_object), 1);
    assert_eq!(handles.reserved_count(), 0);
}

fn stale_source(owner: ProcessId) {
    let objects = Objects::growing();
    let mut handles = Handles::growing();
    let original = source(&objects, &mut handles);
    let plan = create(owner, VERSION, METADATA, &mut handles, &objects).unwrap();
    grant(owner, plan, entry(original, 1), &handles).unwrap();
    handles.close(original).unwrap();
    assert_eq!(
        objects.live_count(),
        1,
        "A manifest must not retain descriptive sources"
    );
    let replacement = source(&objects, &mut handles);
    assert_ne!(replacement, original);
    assert!(matches!(
        prepare(owner, plan, &handles),
        Err(Error::InvalidHandle)
    ));
    assert_eq!(handles.reserved_count(), 0);
}

fn prepared<'objects>(
    owner: ProcessId,
    source: Handle,
    handles: &mut Handles<'objects>,
    objects: &'objects Objects,
) -> Prepared<'objects> {
    let plan = create(owner, VERSION, METADATA, handles, objects).unwrap();
    for role in 1..=3 {
        grant(owner, plan, entry(source, role), handles).unwrap();
    }
    let prepared = prepare(owner, plan, handles).unwrap();
    handles.close(plan).unwrap();
    prepared
}

fn private_install(owner: ProcessId) {
    let objects = Objects::growing();
    let mut parent = Handles::growing();
    let source = source(&objects, &mut parent);
    let prepared = prepared(owner, source, &mut parent, &objects);
    parent.close(source).unwrap();
    assert_eq!(
        objects.live_count(),
        1,
        "Prepared grants must own their references"
    );
    let mut child = Handles::growing();
    let record = prepared.install(&mut child, &objects).unwrap();
    let object = child
        .lookup(record, ObjectKind::StartupRecord, Rights::READ)
        .unwrap();
    assert_eq!(child.live_count(), 4);
    assert_eq!(read(&object, 0), Ok(MAGIC));
    assert_eq!(
        read(&object, 2),
        Ok((HEADER_WORDS + 3 * ENTRY_WORDS) as u64)
    );
    assert_eq!(read(&object, 3), Ok(METADATA.service));
    assert_eq!(read(&object, 4), Ok(METADATA.instance));
    assert_eq!(read(&object, 5), Ok(METADATA.argument));
    assert_eq!(read(&object, 6), Ok(3));
    for index in 0..3 {
        let offset = (HEADER_WORDS + index * ENTRY_WORDS) as u64;
        assert_eq!(read(&object, offset), Ok(index as u64 + 1));
        let handle = Handle::from_raw(read(&object, offset + 3).unwrap());
        assert_eq!(
            child.describe(handle),
            Ok((ObjectKind::DebugConsole, Rights::WRITE))
        );
    }
    assert_eq!(read(&object, u64::MAX), Err(Error::InvalidArgument));
    assert_eq!(
        child.duplicate(record, Rights::READ),
        Err(CapabilityError::PermissionDenied)
    );
    assert!(matches!(
        child.lookup(record, ObjectKind::StartupRecord, Rights::WRITE),
        Err(CapabilityError::PermissionDenied)
    ));
    drop(object);
    drop(child);
    assert_eq!(objects.live_count(), 0);
}

fn capacity_rollback(owner: ProcessId) {
    for limit in [0, 2, 3] {
        let objects = Objects::growing();
        let mut parent = Handles::growing();
        let source = source(&objects, &mut parent);
        let held = parent
            .lookup(source, ObjectKind::DebugConsole, Rights::WRITE)
            .unwrap();
        let baseline = held.reference_count();
        let prepared = prepared(owner, source, &mut parent, &objects);
        let mut child = Handles::limited(limit);
        assert_eq!(
            prepared.install(&mut child, &objects),
            Err(Error::OutOfResources)
        );
        assert_eq!(child.reserved_count(), 0);
        drop(child);
        assert_eq!(held.reference_count(), baseline);
        assert_eq!(objects.live_count(), 1);
    }
    let objects = Objects::limited(1);
    let mut handles = Handles::growing();
    let source = source(&objects, &mut handles);
    let before = handles.live_count();
    assert_eq!(
        install_record(
            METADATA,
            &[Entry {
                rights: (Rights::WRITE | Rights::TRANSFER).bits(),
                ..entry(source, 1)
            }],
            &mut handles,
            &objects,
        ),
        Err(Error::OutOfResources)
    );
    assert_eq!(handles.live_count(), before);
    assert_eq!(handles.reserved_count(), 0);
    assert_eq!(objects.live_count(), 1);
}

pub(crate) fn before_publication() {
    if FAIL_PUBLICATION.swap(false, Ordering::Relaxed) {
        crate::heap::fail_next();
    }
}

pub(crate) fn spawn(
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
) {
    let image =
        crate::boot::module_image("kryo-worker").expect("Missing startup failure fixture image");
    for case in 0..3 {
        spawn_failure(case, pool, direct, memory, region, image);
    }
    kryo_hal::emergency_write_str("STARTUP: spawn publication rollback OK cases=3\n");
}

fn spawn_failure(
    case: usize,
    pool: &'static BootFramePool,
    direct: DirectMap,
    memory: &RefCell<KernelMemory>,
    region: &mut KernelRegion,
    image: &'static [u8],
) {
    let before = pool.stats();
    let retained_before = memory.borrow().retained_pages();
    {
        let objects = Objects::limited(if case == 1 { 3 } else { 8 });
        let mut tasks = Tasks::growing();
        let mut ipc = IpcRuntime::growing(&objects);
        let mut resources = super::super::launch::prepare(pool, direct, memory, region, 2, image)
            .unwrap_or_else(|_| panic!("Cannot prepare startup failure parent"));
        if case == 0 {
            resources.process.handles = Handles::limited(3);
        }
        let image = objects
            .create(ObjectKind::Image, KernelObject::Image(image))
            .unwrap();
        let image = resources
            .process
            .handles
            .insert(image, Rights::EXECUTE)
            .unwrap();
        let source = source(&objects, &mut resources.process.handles);
        let parent = tasks.spawn(resources).unwrap();
        let owner = tasks.process_for(parent).unwrap();
        let resources = tasks.resources_mut(parent).unwrap();
        let plan = create(
            owner,
            VERSION,
            METADATA,
            &mut resources.process.handles,
            &objects,
        )
        .unwrap();
        grant(owner, plan, entry(source, 1), &resources.process.handles).unwrap();
        let mut context: UserContext = resources.context;
        context.rdi = image.as_raw();
        context.rsi = plan.as_raw();
        let handle_count = resources.process.handles.live_count();
        tasks.reserve_process().unwrap();
        let allocation = pool.stats();
        let retained_allocation = memory.borrow().retained_pages();
        if case == 2 {
            FAIL_PUBLICATION.store(true, Ordering::Relaxed);
        }
        let mut stats = CallStats::default();
        assert_eq!(
            super::super::process::spawn_manifest(
                parent, context, &mut tasks, &mut ipc, &mut stats, region
            ),
            Err(Error::OutOfResources),
            "Startup failure case {}",
            case
        );
        assert!(!FAIL_PUBLICATION.load(Ordering::Relaxed));
        assert_eq!(stats.spawned, 0);
        assert_eq!(tasks.live_count(), 1);
        assert_eq!(objects.live_count(), 3);
        let resources = tasks.resources_mut(parent).unwrap();
        assert_eq!(resources.process.handles.live_count(), handle_count);
        assert_eq!(resources.process.handles.reserved_count(), 0);
        assert!(resources.process.pending.is_none());
        assert_eq!(
            pool.stats().free,
            allocation.free - (memory.borrow().retained_pages() - retained_allocation)
        );
        tasks.stop(parent).unwrap();
        ipc.settle(&mut tasks);
        drop(
            tasks
                .reap(parent, |resources| resources.close(pool, region))
                .unwrap(),
        );
        assert_eq!(objects.live_count(), 0);
    }
    assert_eq!(
        pool.stats().free,
        before.free - (memory.borrow().retained_pages() - retained_before)
    );
}
