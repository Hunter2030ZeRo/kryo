use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use kryo_core::capability::{HandleTable, Rights};
use kryo_core::mm::frame::{
    FRAME_SIZE, FramePool, MemoryRegion, PhysRange, RegionKind, normalize_memory,
};
use kryo_core::mm::shared::{SharedBackend, SharedError, SharedMemory, SharedPage};
use kryo_core::mm::vm::{AddressSpace, Permissions, PrivateError, PrivateVm, VmBackend, VmError};
use kryo_core::object::{ObjectKind, ObjectRef, ObjectTable};
use kryo_core::resource::{Budget, Limits, Resource};

type Pool = FramePool<1, 4, 1>;
type Space<'pool> = AddressSpace<'pool, Pool, ModelBackend, 8, 16>;

fn resource_budget(pages: usize) -> Budget {
    Budget::new(Limits {
        shared_pages: pages,
        private_pages: 0,
        reserved_pages: 0,
        handles: 8,
        waits: 8,
        threads: 8,
        mappings: 8,
        tables: 8,
    })
}

#[test]
fn configured_vm_limits_bound_aliases_and_include_the_root_table() {
    let pool = pool(32);
    let before = pool.stats();
    let model = Rc::new(Model::default());
    assert!(matches!(
        AddressSpace::<_, _, 0, 0>::new_limited(
            &pool,
            |root| { Ok(ModelBackend::new(root, model.clone())) },
            1,
            0
        ),
        Err(VmError::TableCapacity)
    ));
    assert_eq!(pool.stats(), before);
    let mut space = AddressSpace::<_, _, 0, 0>::new_limited(
        &pool,
        |root| Ok(ModelBackend::new(root, model.clone())),
        1,
        4,
    )
    .unwrap();
    assert_eq!(space.owned_tables(), 1);
    space
        .map_page(0x40_0000, Permissions::ReadWrite, &[])
        .unwrap();
    assert_eq!(space.owned_tables(), 4);
    let full = pool.stats();
    assert!(matches!(
        space.map_page(0x40_1000, Permissions::ReadWrite, &[]),
        Err(VmError::MappingCapacity)
    ));
    assert_eq!(pool.stats(), full);
    space.unmap_page(0x40_0000).unwrap();
    let retained = pool.stats();
    assert_eq!(space.owned_tables(), 4);
    assert!(matches!(
        space.map_page(0x60_0000, Permissions::ReadWrite, &[]),
        Err(VmError::TableCapacity)
    ));
    assert_eq!(pool.stats(), retained);
    space
        .map_page(0x40_0000, Permissions::ReadWrite, &[])
        .unwrap();
    space.close().unwrap();
    assert_eq!(space.owned_tables(), 0);
    assert_eq!(pool.stats(), before);
}

#[test]
fn shared_payload_charge_survives_all_handles_and_failed_mapping_retirement() {
    let pool = pool(32);
    let before = pool.stats();
    let model = Rc::new(Model::default());
    let budget = resource_budget(2);
    let mut initializer = Initializer {
        model: model.clone(),
        fail_after: None,
        calls: 0,
    };
    let memory = Memory::new_charged(
        &pool,
        2,
        &mut initializer,
        budget.try_charge(Resource::SharedPages, 2).unwrap(),
    )
    .unwrap();
    let objects = Memories::new();
    let object = objects.create(ObjectKind::Memory, memory).unwrap();
    let mut first = AddressSpace::<_, _, 0, 0, Page<'_, '_>>::new_limited(
        &pool,
        |root| Ok(ModelBackend::new(root, model.clone())),
        1,
        4,
    )
    .unwrap();
    let mut second = shared_space(&pool, &model);
    first
        .map_shared_page(
            0x40_0000,
            Permissions::ReadOnly,
            Page::new(object.try_clone().unwrap(), 0),
        )
        .unwrap();
    second
        .map_shared_page(
            0x40_0000,
            Permissions::ReadWrite,
            Page::new(object.try_clone().unwrap(), 1),
        )
        .unwrap();
    assert!(matches!(
        first.map_shared_page(
            0x40_1000,
            Permissions::ReadOnly,
            Page::new(object.try_clone().unwrap(), 0)
        ),
        Err(VmError::MappingCapacity)
    ));
    drop(object);
    assert_eq!(budget.count(Resource::SharedPages), 2);
    first.close().unwrap();
    assert_eq!(budget.count(Resource::SharedPages), 2);
    model.failure.set(Some(Failure::Unmap));
    assert!(second.close().is_err());
    assert_eq!(budget.count(Resource::SharedPages), 2);
    second.close().unwrap();
    assert_eq!(budget.count(Resource::SharedPages), 0);
    assert_eq!(objects.live_count(), 0);
    assert_eq!(pool.stats(), before);
}

#[test]
fn charged_shared_creation_failure_refunds_all_unpublished_frames_and_credits() {
    let pool = pool(4);
    let before = pool.stats();
    let model = Rc::new(Model::default());
    let budget = resource_budget(4);
    let mut initializer = Initializer {
        model,
        fail_after: Some(1),
        calls: 0,
    };
    assert!(matches!(
        Memory::new_charged(
            &pool,
            2,
            &mut initializer,
            budget.try_charge(Resource::SharedPages, 2).unwrap()
        ),
        Err(SharedError::Backend(ModelError::Injected))
    ));
    assert_eq!(pool.stats(), before);
    assert_eq!(budget.count(Resource::SharedPages), 0);
    assert!(matches!(
        Memory::new_charged(
            &pool,
            2,
            &mut initializer,
            budget.try_charge(Resource::SharedPages, 1).unwrap()
        ),
        Err(SharedError::InvalidSize)
    ));
    assert_eq!(pool.stats(), before);
    assert_eq!(budget.count(Resource::SharedPages), 0);
}

#[test]
fn discarded_arena_cannot_refund_forgotten_shared_backing() {
    let pool = pool(8);
    let before = pool.stats();
    let model = Rc::new(Model::default());
    let budget = resource_budget(2);
    let mut initializer = Initializer {
        model,
        fail_after: None,
        calls: 0,
    };
    let memory = Memory::new_charged(
        &pool,
        2,
        &mut initializer,
        budget.try_charge(Resource::SharedPages, 2).unwrap(),
    )
    .unwrap();
    let objects = Memories::new();
    let object = objects.create(ObjectKind::Memory, memory).unwrap();
    core::mem::forget(object);
    drop(objects);
    assert_eq!(pool.stats().free, before.free - 2);
    assert_eq!(budget.count(Resource::SharedPages), 2);
}

#[test]
fn growing_vm_metadata_crosses_mapping_and_page_table_fixture_limits() {
    let pool = pool(256);
    let before = pool.stats();
    let model = Rc::new(Model::default());
    let mut space = AddressSpace::<_, _, 0, 0>::new_growing(&pool, |root| {
        Ok(ModelBackend::new(root, model.clone()))
    })
    .unwrap();
    for index in 0..96 {
        let address = 0x40_0000 + index * 0x20_0000;
        space
            .map_page(address, Permissions::ReadWrite, &[index as u8])
            .unwrap();
    }
    assert_eq!(space.mapped_pages(), 96);
    assert!(space.retained_tables() > 64);
    for index in 0..96 {
        let mut byte = [0];
        space
            .read_page(0x40_0000 + index * 0x20_0000, 0, &mut byte)
            .unwrap();
        assert_eq!(byte[0], index as u8);
    }
    model.failure.set(Some(Failure::Unmap));
    assert!(space.close().is_err());
    assert_eq!(space.mapped_pages(), 96);
    assert_ne!(pool.stats(), before);
    space.close().unwrap();
    assert_eq!(pool.stats(), before);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelError {
    Active,
    Injected,
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    Initialize,
    Seal,
    Map,
    Unmap,
    Retire,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Initialize(u64),
    Seal(u64),
    Map(u64),
    Unmap(u64),
    Flush,
    Restore(u64),
    Retire,
}

#[derive(Default)]
struct Model {
    memory: RefCell<BTreeMap<u64, Vec<u8>>>,
    active_root: Cell<u64>,
    events: RefCell<Vec<Event>>,
    failure: Cell<Option<Failure>>,
    fail_restore: Cell<bool>,
    fail_map_address: Cell<Option<u64>>,
}

struct ModelBackend {
    root: u64,
    model: Rc<Model>,
    mappings: BTreeMap<u64, u64>,
    table_paths: BTreeSet<(u8, u64)>,
}

impl ModelBackend {
    fn new(root: u64, model: Rc<Model>) -> Self {
        Self {
            root,
            model,
            mappings: BTreeMap::new(),
            table_paths: BTreeSet::new(),
        }
    }

    fn fail(&self, operation: Failure) -> Result<(), ModelError> {
        if self.model.failure.get() == Some(operation) {
            self.model.failure.set(None);
            Err(ModelError::Injected)
        } else {
            Ok(())
        }
    }

    fn paths(address: u64) -> [(u8, u64); 3] {
        [(3, address >> 39), (2, address >> 30), (1, address >> 21)]
    }
}

// The model mirrors backend publication and TLB ordering without touching physical RAM.
unsafe impl VmBackend for ModelBackend {
    type Error = ModelError;

    fn root_frame(&self) -> u64 {
        self.root
    }

    fn check_inactive(&self) -> Result<(), Self::Error> {
        if self.model.active_root.get() != 0 {
            Err(ModelError::Active)
        } else {
            Ok(())
        }
    }

    fn tables_needed(&self, address: u64) -> Result<usize, Self::Error> {
        Ok(Self::paths(address)
            .iter()
            .filter(|path| !self.table_paths.contains(path))
            .count())
    }

    unsafe fn initialize_frame(
        &mut self,
        physical: u64,
        contents: &[u8],
    ) -> Result<(), Self::Error> {
        self.fail(Failure::Initialize)?;
        let mut page = vec![0; FRAME_SIZE as usize];
        page[..contents.len()].copy_from_slice(contents);
        self.model.memory.borrow_mut().insert(physical, page);
        self.model
            .events
            .borrow_mut()
            .push(Event::Initialize(physical));
        Ok(())
    }

    unsafe fn seal_frame(&mut self, physical: u64, _: Permissions) -> Result<(), Self::Error> {
        self.fail(Failure::Seal)?;
        self.model.events.borrow_mut().push(Event::Seal(physical));
        Ok(())
    }

    unsafe fn restore_frame(&mut self, physical: u64) -> Result<(), Self::Error> {
        if self.model.fail_restore.get() {
            return Err(ModelError::Injected);
        }
        self.model
            .events
            .borrow_mut()
            .push(Event::Restore(physical));
        Ok(())
    }

    unsafe fn map(
        &mut self,
        address: u64,
        physical: u64,
        _: Permissions,
        tables: &[u64],
    ) -> Result<usize, Self::Error> {
        if self.model.fail_map_address.get() == Some(address) {
            self.model.fail_map_address.set(None);
            return Err(ModelError::Injected);
        }
        self.fail(Failure::Map)?;
        assert!(!self.mappings.contains_key(&address));
        let needed = self.tables_needed(address)?;
        assert!(tables.len() >= needed);
        self.table_paths.extend(Self::paths(address));
        self.mappings.insert(address, physical);
        self.model.events.borrow_mut().push(Event::Map(address));
        Ok(needed)
    }

    unsafe fn unmap(&mut self, address: u64) -> Result<u64, Self::Error> {
        self.fail(Failure::Unmap)?;
        let physical = self.mappings.remove(&address).ok_or(ModelError::Missing)?;
        self.model
            .events
            .borrow_mut()
            .extend([Event::Unmap(address), Event::Flush]);
        Ok(physical)
    }

    unsafe fn read_frame(
        &self,
        physical: u64,
        offset: usize,
        output: &mut [u8],
    ) -> Result<(), Self::Error> {
        output
            .copy_from_slice(&self.model.memory.borrow()[&physical][offset..offset + output.len()]);
        Ok(())
    }

    unsafe fn retire(&mut self) -> Result<(), Self::Error> {
        self.fail(Failure::Retire)?;
        assert!(self.mappings.is_empty());
        self.table_paths.clear();
        self.model
            .events
            .borrow_mut()
            .extend([Event::Retire, Event::Flush]);
        Ok(())
    }
}

fn pool(frames: u64) -> Pool {
    let pool = Pool::new();
    let mut regions = [MemoryRegion {
        range: PhysRange::from_base_len(0x1000, frames * FRAME_SIZE).unwrap(),
        kind: RegionKind::Usable,
    }];
    let mut output = [PhysRange::EMPTY; 1];
    let plan = normalize_memory(&mut regions, &mut [], &mut output).unwrap();
    // Logical model frames are never dereferenced as RAM.
    unsafe { pool.initialize(plan) }.unwrap();
    pool
}

fn space<'pool>(pool: &'pool Pool, model: &Rc<Model>) -> Space<'pool> {
    Space::new(pool, |root| Ok(ModelBackend::new(root, model.clone()))).unwrap()
}

#[test]
fn failed_root_construction_returns_its_frame() {
    let pool = pool(32);
    let before = pool.stats();
    let result = Space::new(&pool, |_| Err(ModelError::Injected));
    assert!(matches!(
        result,
        Err(VmError::Backend(ModelError::Injected))
    ));
    assert_eq!(pool.stats(), before);
}

#[test]
fn permissions_reject_write_execute() {
    assert_eq!(Permissions::new(true, true), None);
    assert_eq!(Permissions::new(false, false), Some(Permissions::ReadOnly));
    assert_eq!(Permissions::new(true, false), Some(Permissions::ReadWrite));
    assert_eq!(
        Permissions::new(false, true),
        Some(Permissions::ReadExecute)
    );
}

#[test]
fn invalid_ranges_and_contents_do_not_allocate() {
    let pool = pool(64);
    let model = Rc::new(Model::default());
    let mut space = space(&pool, &model);
    let before = pool.stats();
    for address in [
        0,
        1,
        0x1001,
        0x8000_0000_0000,
        0xffff_8000_0000_0000,
        u64::MAX,
    ] {
        assert_eq!(
            space.map_page(address, Permissions::ReadWrite, &[]),
            Err(VmError::InvalidAddress)
        );
    }
    assert_eq!(
        space.map_page(0x400000, Permissions::ReadWrite, &vec![0; 4097]),
        Err(VmError::InvalidContents)
    );
    assert_eq!(pool.stats(), before);
    space.close().unwrap();
    assert_eq!(pool.stats().free, 64);
}

#[test]
fn distinct_spaces_own_independent_data_at_the_same_virtual_address() {
    let pool = pool(64);
    let model = Rc::new(Model::default());
    let mut first = space(&pool, &model);
    let mut second = space(&pool, &model);
    assert_ne!(first.root_frame().unwrap(), second.root_frame().unwrap());
    first
        .map_page(0x400000, Permissions::ReadExecute, b"first")
        .unwrap();
    second
        .map_page(0x400000, Permissions::ReadOnly, b"other")
        .unwrap();
    let mut bytes = [0; 8];
    first.read_page(0x400000, 0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"first\0\0\0");
    second.read_page(0x400000, 0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"other\0\0\0");
    assert_eq!(first.permissions(0x400000), Some(Permissions::ReadExecute));
    assert_eq!(second.permissions(0x400000), Some(Permissions::ReadOnly));
    first.close().unwrap();
    second.read_page(0x400000, 0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"other\0\0\0");
    second.close().unwrap();
    assert_eq!(pool.stats().free, 64);
}

#[test]
fn duplicate_mapping_and_metadata_limits_preserve_owners() {
    let pool = pool(64);
    let model = Rc::new(Model::default());
    let mut limited: AddressSpace<'_, Pool, ModelBackend, 1, 3> =
        AddressSpace::new(&pool, |root| Ok(ModelBackend::new(root, model.clone()))).unwrap();
    limited
        .map_page(0x400000, Permissions::ReadWrite, &[])
        .unwrap();
    let before = pool.stats();
    assert_eq!(
        limited.map_page(0x400000, Permissions::ReadOnly, &[]),
        Err(VmError::AlreadyMapped)
    );
    assert_eq!(
        limited.map_page(0x401000, Permissions::ReadOnly, &[]),
        Err(VmError::MappingCapacity)
    );
    assert_eq!(pool.stats(), before);
    limited.close().unwrap();

    let mut no_tables: AddressSpace<'_, Pool, ModelBackend, 1, 2> =
        AddressSpace::new(&pool, |root| Ok(ModelBackend::new(root, model.clone()))).unwrap();
    let before = pool.stats();
    assert_eq!(
        no_tables.map_page(0x400000, Permissions::ReadWrite, &[]),
        Err(VmError::TableCapacity)
    );
    assert_eq!(pool.stats(), before);
    no_tables.close().unwrap();
}

#[test]
fn partial_table_allocation_failure_rolls_back() {
    let pool = pool(3);
    let model = Rc::new(Model::default());
    let mut space = space(&pool, &model);
    let before = pool.stats();
    assert!(matches!(
        space.map_page(0x400000, Permissions::ReadWrite, &[]),
        Err(VmError::Allocation(_))
    ));
    assert_eq!(pool.stats(), before);
    assert_eq!(space.mapped_pages(), 0);
    assert_eq!(space.retained_tables(), 0);
    assert!(model.events.borrow().is_empty());
    space.close().unwrap();
    assert_eq!(pool.stats().free, 3);
}

#[test]
fn backend_failures_before_publication_release_all_prepared_frames() {
    for operation in [Failure::Initialize, Failure::Seal, Failure::Map] {
        let pool = pool(32);
        let model = Rc::new(Model::default());
        let mut space = space(&pool, &model);
        let before = pool.stats();
        model.failure.set(Some(operation));
        assert_eq!(
            space.map_page(0x400000, Permissions::ReadExecute, b"code"),
            Err(VmError::Backend(ModelError::Injected))
        );
        assert_eq!(pool.stats(), before);
        assert_eq!(space.mapped_pages(), 0);
        assert_eq!(space.retained_tables(), 0);
        space.close().unwrap();
        assert_eq!(pool.stats().free, 32);
    }
}

#[test]
fn failed_alias_restore_retains_the_unpublished_data_owner() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let mut space = space(&pool, &model);
    model.failure.set(Some(Failure::Map));
    model.fail_restore.set(true);
    assert_eq!(
        space.map_page(0x400000, Permissions::ReadExecute, &[]),
        Err(VmError::Backend(ModelError::Injected))
    );
    assert_eq!(space.mapped_pages(), 0);
    assert_eq!(pool.stats().free, 30);
    model.fail_restore.set(false);
    space.close().unwrap();
    assert_eq!(pool.stats().free, 32);
}

#[test]
fn unmap_flushes_before_restoring_alias_and_reusing_the_data_frame() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let mut space = space(&pool, &model);
    space
        .map_page(0x400000, Permissions::ReadExecute, &[])
        .unwrap();
    assert_eq!(pool.stats().free, 27);
    model.events.borrow_mut().clear();
    space.unmap_page(0x400000).unwrap();
    let events = model.events.borrow();
    assert_eq!(events[0], Event::Unmap(0x400000));
    assert_eq!(events[1], Event::Flush);
    assert!(matches!(events[2], Event::Restore(_)));
    drop(events);
    assert_eq!(pool.stats().free, 28);
    assert_eq!(space.retained_tables(), 3);
    space
        .map_page(0x400000, Permissions::ReadOnly, &[])
        .unwrap();
    assert_eq!(space.retained_tables(), 3);
    space.close().unwrap();
    assert_eq!(pool.stats().free, 32);
}

#[test]
fn failed_unmap_cleanup_preserves_frames_for_close_retry() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let mut space = space(&pool, &model);
    space
        .map_page(0x400000, Permissions::ReadExecute, &[])
        .unwrap();
    model.fail_restore.set(true);
    assert_eq!(
        space.unmap_page(0x400000),
        Err(VmError::Backend(ModelError::Injected))
    );
    assert_eq!(space.mapped_pages(), 0);
    assert_eq!(pool.stats().free, 27);
    model.fail_restore.set(false);
    space.close().unwrap();
    assert_eq!(pool.stats().free, 32);
}

#[test]
fn root_retirement_failure_preserves_tables_and_root() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let mut space = space(&pool, &model);
    space
        .map_page(0x400000, Permissions::ReadWrite, &[])
        .unwrap();
    model.failure.set(Some(Failure::Retire));
    assert_eq!(space.close(), Err(VmError::Backend(ModelError::Injected)));
    assert_eq!(pool.stats().free, 28);
    space.close().unwrap();
    assert_eq!(pool.stats().free, 32);
}

#[test]
fn active_root_rejects_mutation_reads_and_close() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let mut space = space(&pool, &model);
    space
        .map_page(0x400000, Permissions::ReadWrite, &[])
        .unwrap();
    let before = pool.stats();
    let root = space.root_frame().unwrap();
    assert_eq!(
        space.with_root(|root| model.active_root.set(root)),
        Err(VmError::Backend(ModelError::Active))
    );
    assert_eq!(model.active_root.get(), root);
    assert_eq!(space.close(), Err(VmError::Backend(ModelError::Active)));
    assert_eq!(
        space.unmap_page(0x400000),
        Err(VmError::Backend(ModelError::Active))
    );
    assert_eq!(
        space.read_page(0x400000, 0, &mut []),
        Err(VmError::Backend(ModelError::Active))
    );
    assert_eq!(pool.stats(), before);
    model.active_root.set(0);
    space.close().unwrap();
}

#[test]
fn read_bounds_and_closed_spaces_are_checked() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let mut space = space(&pool, &model);
    space
        .map_page(0x400000, Permissions::ReadWrite, &[1, 2])
        .unwrap();
    assert_eq!(
        space.read_page(0x400000, 4096, &mut [0]),
        Err(VmError::InvalidContents)
    );
    assert_eq!(
        space.read_page(0x400000, usize::MAX, &mut [0]),
        Err(VmError::InvalidContents)
    );
    assert_eq!(
        space.read_page(0x401000, 0, &mut []),
        Err(VmError::NotMapped)
    );
    space.close().unwrap();
    assert_eq!(space.root_frame(), Err(VmError::Closed));
    assert_eq!(space.close(), Err(VmError::Closed));
    assert_eq!(
        space.map_page(0x400000, Permissions::ReadWrite, &[]),
        Err(VmError::Closed)
    );
}

#[test]
fn dropping_a_space_does_not_reclaim_unretired_frames() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let mut space = space(&pool, &model);
    space
        .map_page(0x400000, Permissions::ReadWrite, &[])
        .unwrap();
    let before = pool.stats();
    drop(space);
    assert_eq!(pool.stats(), before);
}

type Memory<'pool> = SharedMemory<'pool, Pool, 4>;
type Memories<'pool> = ObjectTable<Memory<'pool>, 2>;
type MemoryRef<'objects, 'pool> = ObjectRef<'objects, Memory<'pool>, 2>;
type SharedSpace<'pool, 'objects> =
    AddressSpace<'pool, Pool, ModelBackend, 8, 16, Page<'objects, 'pool>>;

struct Initializer {
    model: Rc<Model>,
    fail_after: Option<usize>,
    calls: usize,
}

unsafe impl SharedBackend for Initializer {
    type Error = ModelError;

    unsafe fn initialize_shared(&mut self, physical: u64) -> Result<(), Self::Error> {
        if self.fail_after == Some(self.calls) {
            return Err(ModelError::Injected);
        }
        self.calls += 1;
        self.model
            .memory
            .borrow_mut()
            .insert(physical, vec![0; FRAME_SIZE as usize]);
        self.model
            .events
            .borrow_mut()
            .push(Event::Initialize(physical));
        Ok(())
    }
}

struct Page<'objects, 'pool> {
    _object: MemoryRef<'objects, 'pool>,
    physical: u64,
}

impl<'objects, 'pool> Page<'objects, 'pool> {
    fn new(object: MemoryRef<'objects, 'pool>, index: usize) -> Self {
        let physical = object.with(|memory| memory.page_address(index)).unwrap();
        Self {
            _object: object,
            physical,
        }
    }
}

// The immutable object payload owns each page, and this reference retains it.
unsafe impl SharedPage for Page<'_, '_> {
    fn physical(&self) -> u64 {
        self.physical
    }
}

fn memory<'pool>(pool: &'pool Pool, model: &Rc<Model>, pages: usize) -> Memory<'pool> {
    let mut backend = Initializer {
        model: model.clone(),
        fail_after: None,
        calls: 0,
    };
    Memory::new(pool, pages, &mut backend).unwrap()
}

fn shared_space<'pool, 'objects>(
    pool: &'pool Pool,
    model: &Rc<Model>,
) -> SharedSpace<'pool, 'objects> {
    AddressSpace::new(pool, |root| Ok(ModelBackend::new(root, model.clone()))).unwrap()
}

#[test]
fn shared_memory_is_bounded_zeroed_once_and_released_by_its_owner() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let mut initializer = Initializer {
        model: model.clone(),
        fail_after: None,
        calls: 0,
    };
    for pages in [0, 5, usize::MAX] {
        assert!(matches!(
            Memory::new(&pool, pages, &mut initializer),
            Err(SharedError::InvalidSize)
        ));
    }
    assert_eq!(initializer.calls, 0);
    assert_eq!(pool.stats().free, 32);
    let memory = Memory::new(&pool, 3, &mut initializer).unwrap();
    assert_eq!(memory.page_count(), 3);
    assert_eq!(memory.byte_len(), 3 * FRAME_SIZE);
    assert_eq!(initializer.calls, 3);
    assert_eq!(pool.stats().free, 29);
    for index in 0..3 {
        let physical = memory.page_address(index).unwrap();
        assert!(
            model.memory.borrow()[&physical]
                .iter()
                .all(|byte| *byte == 0)
        );
    }
    assert_eq!(memory.page_address(3), None);
    assert_eq!(memory.page_address(usize::MAX), None);
    drop(memory);
    assert_eq!(pool.stats().free, 32);
}

#[test]
fn shared_memory_partial_allocation_failure_returns_unpublished_frames() {
    let pool = pool(2);
    let model = Rc::new(Model::default());
    let mut initializer = Initializer {
        model,
        fail_after: None,
        calls: 0,
    };
    assert!(matches!(
        Memory::new(&pool, 3, &mut initializer),
        Err(SharedError::Allocation(_))
    ));
    assert_eq!(initializer.calls, 2);
    assert_eq!(pool.stats().free, 2);
}

#[test]
fn shared_memory_partial_initialization_failure_returns_all_frame_owners() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let mut initializer = Initializer {
        model,
        fail_after: Some(1),
        calls: 0,
    };
    assert!(matches!(
        Memory::new(&pool, 3, &mut initializer),
        Err(SharedError::Backend(ModelError::Injected))
    ));
    assert_eq!(initializer.calls, 1);
    assert_eq!(pool.stats().free, 32);
}

#[test]
fn mappings_keep_shared_storage_after_the_last_handle_is_closed() {
    let pool = pool(64);
    let model = Rc::new(Model::default());
    let objects = Memories::new();
    let object = objects
        .create(ObjectKind::Memory, memory(&pool, &model, 2))
        .unwrap();
    let physical = object.with(|memory| memory.page_address(0)).unwrap();
    let mut handles = HandleTable::<_, 2, 2>::new();
    let handle = handles.insert(object, Rights::INSPECT).unwrap();
    let mut first = shared_space(&pool, &model);
    let mut second = shared_space(&pool, &model);
    for (space, permissions) in [
        (&mut first, Permissions::ReadWrite),
        (&mut second, Permissions::ReadOnly),
    ] {
        let object = handles
            .lookup(handle, ObjectKind::Memory, Rights::INSPECT)
            .unwrap();
        space
            .map_shared_page(0x400000, permissions, Page::new(object, 0))
            .unwrap();
    }
    assert_eq!(first.permissions(0x400000), Some(Permissions::ReadWrite));
    assert_eq!(second.permissions(0x400000), Some(Permissions::ReadOnly));
    handles.close(handle).unwrap();
    assert_eq!(objects.live_count(), 1);
    model.memory.borrow_mut().get_mut(&physical).unwrap()[0] = 42;
    let mut byte = [0];
    first.read_page(0x400000, 0, &mut byte).unwrap();
    assert_eq!(byte, [42]);
    second.read_page(0x400000, 0, &mut byte).unwrap();
    assert_eq!(byte, [42]);
    first.close().unwrap();
    assert_eq!(objects.live_count(), 1);
    second.read_page(0x400000, 0, &mut byte).unwrap();
    assert_eq!(byte, [42]);
    second.close().unwrap();
    assert_eq!(objects.live_count(), 0);
    assert_eq!(pool.stats().free, 64);
    assert_eq!(
        model
            .events
            .borrow()
            .iter()
            .filter(|event| matches!(event, Event::Initialize(_)))
            .count(),
        2
    );
    assert!(
        !model
            .events
            .borrow()
            .iter()
            .any(|event| matches!(event, Event::Seal(_) | Event::Restore(_)))
    );
}

#[test]
fn shared_mapping_failure_releases_tables_and_its_temporary_lease() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let objects = Memories::new();
    let object = objects
        .create(ObjectKind::Memory, memory(&pool, &model, 1))
        .unwrap();
    let mut space = shared_space(&pool, &model);
    let before = pool.stats();
    model.failure.set(Some(Failure::Map));
    assert_eq!(
        space.map_shared_page(
            0x400000,
            Permissions::ReadWrite,
            Page::new(object.try_clone().unwrap(), 0),
        ),
        Err(VmError::Backend(ModelError::Injected))
    );
    assert_eq!(object.reference_count(), 1);
    assert_eq!(pool.stats(), before);
    assert_eq!(space.mapped_pages(), 0);
    space.close().unwrap();
    drop(object);
    assert_eq!(pool.stats().free, 32);
}

#[test]
fn shared_map_partial_table_allocation_failure_preserves_the_object() {
    let pool = pool(4);
    let model = Rc::new(Model::default());
    let objects = Memories::new();
    let object = objects
        .create(ObjectKind::Memory, memory(&pool, &model, 1))
        .unwrap();
    let mut space = shared_space(&pool, &model);
    let before = pool.stats();
    assert!(matches!(
        space.map_shared_page(
            0x400000,
            Permissions::ReadWrite,
            Page::new(object.try_clone().unwrap(), 0),
        ),
        Err(VmError::Allocation(_))
    ));
    assert_eq!(object.reference_count(), 1);
    assert_eq!(pool.stats(), before);
    assert_eq!(space.retained_tables(), 0);
    space.close().unwrap();
    drop(object);
    assert_eq!(pool.stats().free, 4);
}

#[test]
fn shared_duplicate_and_metadata_limits_do_not_retain_temporary_leases() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let objects = Memories::new();
    let object = objects
        .create(ObjectKind::Memory, memory(&pool, &model, 1))
        .unwrap();
    let mut space: AddressSpace<'_, Pool, ModelBackend, 1, 3, Page<'_, '_>> =
        AddressSpace::new(&pool, |root| Ok(ModelBackend::new(root, model.clone()))).unwrap();
    space
        .map_shared_page(
            0x400000,
            Permissions::ReadWrite,
            Page::new(object.try_clone().unwrap(), 0),
        )
        .unwrap();
    let before = pool.stats();
    for (address, error) in [
        (0x400000, VmError::AlreadyMapped),
        (0x401000, VmError::MappingCapacity),
    ] {
        assert_eq!(
            space.map_shared_page(
                address,
                Permissions::ReadOnly,
                Page::new(object.try_clone().unwrap(), 0),
            ),
            Err(error)
        );
        assert_eq!(object.reference_count(), 2);
        assert_eq!(pool.stats(), before);
    }
    space.close().unwrap();
    let mut no_tables: AddressSpace<'_, Pool, ModelBackend, 1, 2, Page<'_, '_>> =
        AddressSpace::new(&pool, |root| Ok(ModelBackend::new(root, model.clone()))).unwrap();
    let before = pool.stats();
    assert_eq!(
        no_tables.map_shared_page(
            0x400000,
            Permissions::ReadOnly,
            Page::new(object.try_clone().unwrap(), 0),
        ),
        Err(VmError::TableCapacity)
    );
    assert_eq!(object.reference_count(), 1);
    assert_eq!(pool.stats(), before);
    no_tables.close().unwrap();
    drop(object);
    assert_eq!(pool.stats().free, 32);
}

#[test]
fn mixed_backing_close_retry_does_not_restore_or_release_shared_pages_early() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let objects = Memories::new();
    let object = objects
        .create(ObjectKind::Memory, memory(&pool, &model, 1))
        .unwrap();
    let mut space = shared_space(&pool, &model);
    space
        .map_page(0x400000, Permissions::ReadExecute, &[0xcc])
        .unwrap();
    space
        .map_shared_page(0x401000, Permissions::ReadWrite, Page::new(object, 0))
        .unwrap();
    let before = pool.stats();
    model.fail_restore.set(true);
    assert_eq!(space.close(), Err(VmError::Backend(ModelError::Injected)));
    assert_eq!(space.mapped_pages(), 1);
    assert_eq!(objects.live_count(), 1);
    assert_eq!(pool.stats(), before);
    model.fail_restore.set(false);
    space.close().unwrap();
    assert_eq!(objects.live_count(), 0);
    assert_eq!(pool.stats().free, 32);
}

#[test]
fn shared_unmap_failure_keeps_the_mapping_lease_until_close_retry() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let objects = Memories::new();
    let object = objects
        .create(ObjectKind::Memory, memory(&pool, &model, 1))
        .unwrap();
    let mut space = shared_space(&pool, &model);
    space
        .map_shared_page(0x400000, Permissions::ReadOnly, Page::new(object, 0))
        .unwrap();
    let before = pool.stats();
    model.failure.set(Some(Failure::Unmap));
    assert_eq!(space.close(), Err(VmError::Backend(ModelError::Injected)));
    assert_eq!(space.mapped_pages(), 1);
    assert_eq!(objects.live_count(), 1);
    assert_eq!(pool.stats(), before);
    space.close().unwrap();
    assert_eq!(objects.live_count(), 0);
    assert_eq!(pool.stats().free, 32);
}

#[test]
fn shared_maps_reject_executable_aliases_and_bad_virtual_addresses() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let objects = Memories::new();
    let object = objects
        .create(ObjectKind::Memory, memory(&pool, &model, 1))
        .unwrap();
    let mut space = shared_space(&pool, &model);
    let before = pool.stats();
    for address in [0, 0x1001, 0x8000_0000_0000, u64::MAX] {
        assert_eq!(
            space.map_shared_page(
                address,
                Permissions::ReadWrite,
                Page::new(object.try_clone().unwrap(), 0),
            ),
            Err(VmError::InvalidAddress)
        );
    }
    assert_eq!(
        space.map_shared_page(
            0x400000,
            Permissions::ReadExecute,
            Page::new(object.try_clone().unwrap(), 0),
        ),
        Err(VmError::InvalidPermissions)
    );
    assert_eq!(pool.stats(), before);
    assert_eq!(object.reference_count(), 1);
    space.close().unwrap();
    drop(object);
    assert_eq!(pool.stats().free, 32);
}

#[test]
fn shared_reads_reject_a_writer_running_in_another_root() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let objects = Memories::new();
    let object = objects
        .create(ObjectKind::Memory, memory(&pool, &model, 1))
        .unwrap();
    let mut first = shared_space(&pool, &model);
    let mut second = shared_space(&pool, &model);
    first
        .map_shared_page(
            0x400000,
            Permissions::ReadWrite,
            Page::new(object.try_clone().unwrap(), 0),
        )
        .unwrap();
    second
        .map_shared_page(0x400000, Permissions::ReadOnly, Page::new(object, 0))
        .unwrap();
    model.active_root.set(first.root_frame().unwrap());
    assert_eq!(
        second.read_page(0x400000, 0, &mut [0]),
        Err(VmError::Backend(ModelError::Active))
    );
    model.active_root.set(0);
    first.close().unwrap();
    second.close().unwrap();
    assert_eq!(pool.stats().free, 32);
}

#[test]
fn dropping_an_unclosed_space_forgets_its_shared_backing_lease() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let before_drop;
    {
        let objects = Memories::new();
        let object = objects
            .create(ObjectKind::Memory, memory(&pool, &model, 1))
            .unwrap();
        let mut space = shared_space(&pool, &model);
        space
            .map_shared_page(0x400000, Permissions::ReadWrite, Page::new(object, 0))
            .unwrap();
        before_drop = pool.stats();
        drop(space);
        assert_eq!(objects.live_count(), 1);
        assert_eq!(pool.stats(), before_drop);
    }
    // The arena also preserves forgotten references instead of dropping live RAM.
    assert_eq!(pool.stats(), before_drop);
}

fn private_budget(private_pages: usize, reserved_pages: usize) -> Budget {
    Budget::new(Limits {
        shared_pages: 0,
        private_pages,
        reserved_pages,
        handles: 0,
        waits: 0,
        threads: 0,
        mappings: 4096,
        tables: 128,
    })
}

#[test]
fn private_large_reservation_allocates_only_committed_zeroed_pages() {
    let pool = FramePool::<1, 16, 1>::new();
    let mut regions = [MemoryRegion {
        range: PhysRange::from_base_len(0x1000, 768 * FRAME_SIZE).unwrap(),
        kind: RegionKind::Usable,
    }];
    let mut output = [PhysRange::EMPTY; 1];
    let plan = normalize_memory(&mut regions, &mut [], &mut output).unwrap();
    unsafe { pool.initialize(plan) }.unwrap();
    let before = pool.stats();
    let model = Rc::new(Model::default());
    let budget = private_budget(1024, 65536);
    let mut space = AddressSpace::<_, _, 0, 0>::new_limited(
        &pool,
        |root| Ok(ModelBackend::new(root, model.clone())),
        4096,
        128,
    )
    .unwrap();
    let root_only = pool.stats();
    let mut memory = PrivateVm::new();
    let base = memory
        .reserve(&space, &budget, 256 * 1024 * 1024, FRAME_SIZE)
        .unwrap();
    assert_eq!(base, 0x1_0000_0000);
    assert_eq!(pool.stats(), root_only);
    assert_eq!(budget.count(Resource::ReservedPages), 65536);
    memory.commit(&mut space, base, 512 * FRAME_SIZE).unwrap();
    assert_eq!(budget.count(Resource::PrivatePages), 512);
    assert_eq!(space.mapped_pages(), 512);
    let mut page = [0xff; FRAME_SIZE as usize];
    for index in 0..512 {
        let address = base + index * FRAME_SIZE;
        assert_eq!(space.permissions(address), Some(Permissions::ReadWrite));
        space.read_page(address, 0, &mut page).unwrap();
        assert!(page.iter().all(|byte| *byte == 0));
    }
    assert!(!memory.retirement_pending());
    memory.release(&mut space, base).unwrap();
    assert_eq!(budget.count(Resource::PrivatePages), 0);
    assert_eq!(budget.count(Resource::ReservedPages), 0);
    space.close().unwrap();
    memory.clear_after_close(&space).unwrap();
    assert_eq!(pool.stats(), before);
}

#[test]
fn private_ranges_validate_alignment_quota_and_legacy_collisions() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let mut space = space(&pool, &model);
    space
        .map_page(0x1_0000_1000, Permissions::ReadOnly, &[])
        .unwrap();
    let budget = private_budget(2, 8);
    let mut memory = PrivateVm::new();
    for (bytes, alignment) in [
        (0, FRAME_SIZE),
        (1, FRAME_SIZE),
        (FRAME_SIZE, 0),
        (FRAME_SIZE, 8193),
    ] {
        assert_eq!(
            memory.reserve(&space, &budget, bytes, alignment),
            Err(PrivateError::InvalidRange)
        );
    }
    assert_eq!(
        memory.reserve(&space, &budget, 16 * FRAME_SIZE, FRAME_SIZE),
        Err(PrivateError::OutOfResources)
    );
    let base = memory
        .reserve(&space, &budget, 4 * FRAME_SIZE, FRAME_SIZE)
        .unwrap();
    assert_eq!(base, 0x1_0000_2000);
    assert!(memory.overlaps(base + FRAME_SIZE, FRAME_SIZE));
    assert!(!memory.overlaps(base + 4 * FRAME_SIZE, FRAME_SIZE));
    assert!(memory.overlaps(u64::MAX, 2));
    assert_eq!(
        memory.commit(&mut space, base + 1, FRAME_SIZE),
        Err(PrivateError::InvalidRange)
    );
    assert_eq!(
        memory.commit(&mut space, base, 3 * FRAME_SIZE),
        Err(PrivateError::OutOfResources)
    );
    assert_eq!(budget.count(Resource::PrivatePages), 0);
    memory.commit(&mut space, base, FRAME_SIZE).unwrap();
    assert_eq!(
        memory.commit(&mut space, base, FRAME_SIZE),
        Err(PrivateError::Conflict)
    );
    assert_eq!(
        memory.decommit(&mut space, base + 4 * FRAME_SIZE, FRAME_SIZE),
        Err(PrivateError::InvalidRange)
    );
    assert_eq!(
        memory.release(&mut space, base + FRAME_SIZE),
        Err(PrivateError::InvalidRange)
    );
    memory.release(&mut space, base).unwrap();
    space.close().unwrap();
}

#[test]
fn private_data_oom_and_mapping_metadata_failure_publish_nothing_and_refund() {
    for frames in [3, 32] {
        let pool = pool(frames);
        let model = Rc::new(Model::default());
        let mut space = AddressSpace::<_, _, 0, 0>::new_limited(
            &pool,
            |root| Ok(ModelBackend::new(root, model.clone())),
            if frames == 3 { 8 } else { 1 },
            16,
        )
        .unwrap();
        let budget = private_budget(8, 16);
        let mut memory = PrivateVm::new();
        let base = memory
            .reserve(&space, &budget, 8 * FRAME_SIZE, FRAME_SIZE)
            .unwrap();
        let before = pool.stats();
        assert!(matches!(
            memory.commit(&mut space, base, 4 * FRAME_SIZE),
            Err(PrivateError::Vm(_))
        ));
        assert_eq!(pool.stats(), before);
        assert_eq!(space.mapped_pages(), 0);
        assert!(model.events.borrow().is_empty());
        assert_eq!(budget.count(Resource::PrivatePages), 0);
        assert!(!memory.retirement_pending());
        memory.release(&mut space, base).unwrap();
        space.close().unwrap();
    }
}

#[test]
fn private_table_oom_after_publication_rolls_back_data_without_poisoning_process() {
    let pool = pool(6);
    let before = pool.stats();
    let model = Rc::new(Model::default());
    let budget = private_budget(2, 1024);
    let mut space = AddressSpace::<_, _, 0, 0>::new_limited(
        &pool,
        |root| Ok(ModelBackend::new(root, model.clone())),
        8,
        16,
    )
    .unwrap();
    let mut memory = PrivateVm::new();
    let base = memory
        .reserve(&space, &budget, 513 * FRAME_SIZE, FRAME_SIZE)
        .unwrap();
    let address = base + 511 * FRAME_SIZE;
    assert!(matches!(
        memory.commit(&mut space, address, 2 * FRAME_SIZE),
        Err(PrivateError::Vm(VmError::Allocation(_)))
    ));
    assert!(model.events.borrow().contains(&Event::Map(address)));
    assert_eq!(space.mapped_pages(), 0);
    assert_eq!(budget.count(Resource::PrivatePages), 0);
    assert!(!memory.retirement_pending());
    assert_eq!(space.retained_tables(), 3);
    memory.release(&mut space, base).unwrap();
    space.close().unwrap();
    assert_eq!(pool.stats(), before);
}

#[test]
fn private_failed_rollback_keeps_unpublished_owners_and_quota_until_retry() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let budget = private_budget(2, 8);
    let mut space = space(&pool, &model);
    let mut memory = PrivateVm::new();
    let base = memory
        .reserve(&space, &budget, 8 * FRAME_SIZE, FRAME_SIZE)
        .unwrap();
    model.fail_map_address.set(Some(base + FRAME_SIZE));
    model.fail_restore.set(true);
    assert_eq!(
        memory.commit(&mut space, base, 2 * FRAME_SIZE),
        Err(PrivateError::RetirementPending)
    );
    assert_eq!(space.mapped_pages(), 0);
    assert_eq!(budget.count(Resource::PrivatePages), 2);
    assert_eq!(memory.identity(base), None);
    assert!(memory.retirement_pending());
    assert_eq!(
        memory.commit(&mut space, base + 4 * FRAME_SIZE, FRAME_SIZE),
        Err(PrivateError::RetirementPending)
    );
    assert_eq!(
        memory.retry_retirement(&mut space),
        Err(PrivateError::RetirementPending)
    );
    assert_eq!(budget.count(Resource::PrivatePages), 2);
    model.fail_restore.set(false);
    memory.retry_retirement(&mut space).unwrap();
    assert_eq!(budget.count(Resource::PrivatePages), 0);
    assert!(!memory.retirement_pending());
    memory.commit(&mut space, base, FRAME_SIZE).unwrap();
    memory.release(&mut space, base).unwrap();
    space.close().unwrap();
}

#[test]
fn private_partial_decommit_preserves_live_identity_and_changes_recommit_identity() {
    let pool = pool(64);
    let model = Rc::new(Model::default());
    let budget = private_budget(8, 8);
    let mut space = space(&pool, &model);
    let mut memory = PrivateVm::new();
    let base = memory
        .reserve(&space, &budget, 8 * FRAME_SIZE, FRAME_SIZE)
        .unwrap();
    memory.commit(&mut space, base, 5 * FRAME_SIZE).unwrap();
    let old = memory.identity(base).unwrap();
    memory
        .decommit(&mut space, base + FRAME_SIZE, 2 * FRAME_SIZE)
        .unwrap();
    assert_eq!(budget.count(Resource::PrivatePages), 3);
    assert_eq!(budget.count(Resource::ReservedPages), 8);
    assert_eq!(memory.identity(base), Some(old));
    assert_eq!(memory.identity(base + 3 * FRAME_SIZE), Some(old));
    assert_eq!(memory.identity(base + FRAME_SIZE), None);
    memory
        .decommit(&mut space, base + FRAME_SIZE, 2 * FRAME_SIZE)
        .unwrap();
    memory
        .commit(&mut space, base + FRAME_SIZE, 2 * FRAME_SIZE)
        .unwrap();
    assert_ne!(memory.identity(base + FRAME_SIZE), Some(old));
    memory.release(&mut space, base).unwrap();
    let reused = memory
        .reserve(&space, &budget, 8 * FRAME_SIZE, FRAME_SIZE)
        .unwrap();
    assert_eq!(reused, base);
    memory.commit(&mut space, reused, FRAME_SIZE).unwrap();
    assert_ne!(memory.identity(reused).unwrap().0, old.0);
    memory.release(&mut space, reused).unwrap();
    space.close().unwrap();
}

#[test]
fn private_partial_retirement_refunds_only_completed_pages_and_release_keeps_reservation() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let budget = private_budget(3, 8);
    let mut space = space(&pool, &model);
    let mut memory = PrivateVm::new();
    let base = memory
        .reserve(&space, &budget, 8 * FRAME_SIZE, FRAME_SIZE)
        .unwrap();
    memory.commit(&mut space, base, 3 * FRAME_SIZE).unwrap();
    model.failure.set(Some(Failure::Unmap));
    assert_eq!(
        memory.decommit(&mut space, base, 3 * FRAME_SIZE),
        Err(PrivateError::RetirementPending)
    );
    assert_eq!(space.mapped_pages(), 1);
    assert_eq!(budget.count(Resource::PrivatePages), 1);
    assert_eq!(budget.count(Resource::ReservedPages), 8);
    model.failure.set(Some(Failure::Unmap));
    assert_eq!(
        memory.release(&mut space, base),
        Err(PrivateError::RetirementPending)
    );
    assert!(memory.overlaps(base, FRAME_SIZE));
    assert_eq!(budget.count(Resource::ReservedPages), 8);
    memory.retry_retirement(&mut space).unwrap();
    assert_eq!(budget.count(Resource::PrivatePages), 0);
    assert_eq!(budget.count(Resource::ReservedPages), 0);
    assert!(!memory.overlaps(base, FRAME_SIZE));
    space.close().unwrap();
}

#[test]
fn private_close_requires_its_own_closed_address_space() {
    let pool = pool(64);
    let model = Rc::new(Model::default());
    let budget = private_budget(2, 8);
    let mut owned = space(&pool, &model);
    let mut foreign = space(&pool, &model);
    let mut memory = PrivateVm::new();
    let base = memory
        .reserve(&owned, &budget, 8 * FRAME_SIZE, FRAME_SIZE)
        .unwrap();
    memory.commit(&mut owned, base, 2 * FRAME_SIZE).unwrap();
    assert_eq!(
        memory.clear_after_close(&owned),
        Err(PrivateError::Conflict)
    );
    foreign.close().unwrap();
    assert_eq!(
        memory.clear_after_close(&foreign),
        Err(PrivateError::Conflict)
    );
    assert_eq!(budget.count(Resource::PrivatePages), 2);
    assert_eq!(budget.count(Resource::ReservedPages), 8);
    owned.close().unwrap();
    memory.clear_after_close(&owned).unwrap();
    assert_eq!(budget.count(Resource::PrivatePages), 0);
    assert_eq!(budget.count(Resource::ReservedPages), 0);
}

#[test]
fn private_drop_without_close_retains_all_resource_charges() {
    let pool = pool(32);
    let model = Rc::new(Model::default());
    let budget = private_budget(1, 8);
    let mut space = space(&pool, &model);
    let mut memory = PrivateVm::new();
    let base = memory
        .reserve(&space, &budget, 8 * FRAME_SIZE, FRAME_SIZE)
        .unwrap();
    memory.commit(&mut space, base, FRAME_SIZE).unwrap();
    drop(memory);
    drop(space);
    assert_eq!(budget.count(Resource::PrivatePages), 1);
    assert_eq!(budget.count(Resource::ReservedPages), 8);
}
