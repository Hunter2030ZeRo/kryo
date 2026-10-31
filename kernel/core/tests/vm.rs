use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use kryo_core::mm::frame::{
    FRAME_SIZE, FramePool, MemoryRegion, PhysRange, RegionKind, normalize_memory,
};
use kryo_core::mm::vm::{AddressSpace, Permissions, VmBackend, VmError};

type Pool = FramePool<1, 4, 1>;
type Space<'pool> = AddressSpace<'pool, Pool, ModelBackend, 8, 16>;

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
        if self.model.active_root.get() == self.root {
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
