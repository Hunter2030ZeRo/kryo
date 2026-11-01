use super::{Batch, Handles, IpcRuntime, KernelObject, RefCell};
use kryo_core::capability::{Handle, Rights};
use kryo_core::ipc::{Endpoint, Notification};
use kryo_core::object::ObjectKind;
use kryo_core::task::TaskId;
use kryo_interfaces::syscall::{Call, Error};

pub(crate) fn create<'objects>(
    owner: TaskId,
    factory: Handle,
    call: Call,
    handles: &mut Handles<'objects>,
    ipc: &mut IpcRuntime<'objects>,
) -> Result<Handle, Error> {
    let factory = handles
        .lookup(factory, ObjectKind::ChannelFactory, Rights::CREATE)
        .map_err(super::native::map_error)?;
    let arena = ipc.arena.ok_or(Error::InvalidCall)?;
    let reservation = handles.reserve(1).map_err(super::native::map_error)?;
    let prepared = (|| {
        let lease = factory.with(|object| match object {
            KernelObject::ChannelFactory(budget) => {
                budget.try_reserve().map_err(|_| Error::OutOfResources)
            }
            _ => Err(Error::WrongType),
        })?;
        let (kind, value, rights) = match call {
            Call::EndpointCreate => (
                ObjectKind::Endpoint,
                KernelObject::Endpoint(RefCell::new(Endpoint::leased(owner, lease))),
                Rights::SEND | Rights::RECEIVE | Rights::CONTROL | Rights::DUP | Rights::TRANSFER,
            ),
            Call::NotificationCreate => (
                ObjectKind::Notification,
                KernelObject::Notification(RefCell::new(Notification::leased(owner, lease))),
                Rights::WAIT | Rights::SIGNAL | Rights::DUP | Rights::TRANSFER,
            ),
            _ => return Err(Error::InvalidCall),
        };
        let object = arena
            .create(kind, value)
            .map_err(|_| Error::OutOfResources)?;
        let registered = object.try_clone().map_err(|_| Error::OutOfResources)?;
        ipc.try_register(registered)
            .map_err(|_| Error::OutOfResources)?;
        Ok((object, rights))
    })();
    let (object, rights) = match prepared {
        Ok(value) => value,
        Err(error) => {
            handles
                .release_reservation(reservation)
                .expect("Creation reservation invariant");
            return Err(error);
        }
    };
    let id = object.id();
    match handles.commit_transfer(reservation, Batch::from_object(object, rights)) {
        Ok(handles) => Ok(handles[0].expect("Creation result invariant")),
        Err(failure) => {
            ipc.unregister(id);
            handles
                .release_reservation(failure.reservation)
                .expect("Creation rollback invariant");
            Err(super::native::map_error(failure.error))
        }
    }
}

#[cfg(feature = "boot-tests")]
pub(crate) mod checks;
