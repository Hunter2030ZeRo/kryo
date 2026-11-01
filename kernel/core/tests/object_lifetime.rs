use core::cell::Cell;
use kryo_core::object::{ObjectKind, ObjectTable};
use std::rc::Rc;

struct Backing(Rc<Cell<usize>>);

impl Drop for Backing {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn forgotten_references_keep_backing_alive_after_arena_teardown() {
    let drops = Rc::new(Cell::new(0));
    let objects = ObjectTable::<Backing, 1>::new();
    let reference = objects
        .create(ObjectKind::Memory, Backing(drops.clone()))
        .unwrap();
    core::mem::forget(reference);
    drop(objects);
    assert_eq!(drops.get(), 0);
}

#[test]
fn released_references_destroy_backing_before_arena_teardown() {
    let drops = Rc::new(Cell::new(0));
    let objects = ObjectTable::<Backing, 1>::new();
    let reference = objects
        .create(ObjectKind::Memory, Backing(drops.clone()))
        .unwrap();
    drop(reference);
    assert_eq!(drops.get(), 1);
    drop(objects);
    assert_eq!(drops.get(), 1);
}
