use super::{Batch, Handles, KernelObject, Object, Objects, RefCell};
use alloc::vec::Vec;
use kryo_core::capability::{Handle, Rights, TransferSpec};
use kryo_core::object::ObjectKind;
use kryo_core::process::ProcessId;
use kryo_core::resource::Budget;
use kryo_interfaces::startup::{
    ENTRY_WORDS, Entry, HEADER_WORDS, MAGIC, MAX_GRANTS, Metadata, ObjectKind as WireKind, VERSION,
};
use kryo_interfaces::syscall::Error;

pub(crate) struct LaunchManifest {
    owner: ProcessId,
    metadata: Metadata,
    entries: Vec<Entry>,
    budget: Option<Handle>,
}

pub(crate) struct Prepared<'objects> {
    metadata: Metadata,
    grants: Vec<PreparedGrant<'objects>>,
    account: Option<Budget>,
}

struct PreparedGrant<'objects> {
    entry: Entry,
    batch: Batch<'objects>,
}

pub(crate) fn create<'objects>(
    owner: ProcessId,
    version: u64,
    metadata: Metadata,
    handles: &mut Handles<'objects>,
    arena: &'objects Objects,
) -> Result<Handle, Error> {
    if version != VERSION {
        return Err(Error::InvalidArgument);
    }
    validate_metadata(metadata)?;
    publish(
        KernelObject::LaunchManifest(RefCell::new(LaunchManifest {
            owner,
            metadata,
            entries: Vec::new(),
            budget: None,
        })),
        ObjectKind::LaunchManifest,
        Rights::READ | Rights::WRITE,
        handles,
        arena,
    )
}

pub(crate) fn grant(
    owner: ProcessId,
    manifest: Handle,
    entry: Entry,
    handles: &Handles<'_>,
) -> Result<(), Error> {
    let object = handles
        .lookup(manifest, ObjectKind::LaunchManifest, Rights::WRITE)
        .map_err(super::native::map_error)?;
    object.with(|object| {
        let KernelObject::LaunchManifest(manifest) = object else {
            unreachable!()
        };
        let manifest = manifest.borrow();
        if manifest.owner != owner {
            return Err(Error::PermissionDenied);
        }
        validate_entry(entry)?;
        if manifest.entries.len() == MAX_GRANTS {
            return Err(Error::OutOfResources);
        }
        if manifest
            .entries
            .iter()
            .any(|prior| prior.role == entry.role)
        {
            return Err(Error::InvalidArgument);
        }
        Ok(())
    })?;
    // The source remains descriptive until spawn. Acquire no object reference
    // while the manifest's arena callback is active.
    drop(stage(entry, handles)?);
    object.with(|object| {
        let KernelObject::LaunchManifest(manifest) = object else {
            unreachable!()
        };
        let mut manifest = manifest.borrow_mut();
        manifest
            .entries
            .try_reserve(1)
            .map_err(|_| Error::OutOfResources)?;
        manifest.entries.push(entry);
        Ok(())
    })
}

pub(crate) fn prepare<'objects>(
    owner: ProcessId,
    manifest: Handle,
    handles: &Handles<'objects>,
) -> Result<Prepared<'objects>, Error> {
    let object = handles
        .lookup(manifest, ObjectKind::LaunchManifest, Rights::READ)
        .map_err(super::native::map_error)?;
    let (metadata, entries, budget) = object.with(|object| {
        let KernelObject::LaunchManifest(manifest) = object else {
            unreachable!()
        };
        let manifest = manifest.borrow();
        if manifest.owner != owner {
            return Err(Error::PermissionDenied);
        }
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(manifest.entries.len())
            .map_err(|_| Error::OutOfResources)?;
        entries.extend_from_slice(&manifest.entries);
        Ok((manifest.metadata, entries, manifest.budget))
    })?;
    validate_metadata(metadata)?;
    let account = budget
        .map(|handle| account_from(handles, handle))
        .transpose()?;
    let mut grants = Vec::new();
    grants
        .try_reserve_exact(entries.len())
        .map_err(|_| Error::OutOfResources)?;
    for entry in entries {
        grants.push(PreparedGrant {
            entry,
            batch: stage(entry, handles)?,
        });
    }
    Ok(Prepared {
        metadata,
        grants,
        account,
    })
}

impl<'objects> Prepared<'objects> {
    pub(crate) fn account(&self) -> Option<Budget> {
        self.account.clone()
    }
    /// Install only in an unpublished child. Failure discards its whole handle
    /// table; no partially installed capability has been exposed.
    pub(crate) fn install(
        self,
        handles: &mut Handles<'objects>,
        arena: &'objects Objects,
    ) -> Result<Handle, Error> {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(self.grants.len())
            .map_err(|_| Error::OutOfResources)?;
        for grant in self.grants {
            let reservation = handles.reserve(1).map_err(super::native::map_error)?;
            let installed = match handles.commit_transfer(reservation, grant.batch) {
                Ok(installed) => installed[0].expect("Private startup grant invariant"),
                Err(failure) => {
                    handles
                        .release_reservation(failure.reservation)
                        .expect("Private startup reservation invariant");
                    return Err(super::native::map_error(failure.error));
                }
            };
            entries.push(Entry {
                handle: installed.as_raw(),
                ..grant.entry
            });
        }
        install_record(self.metadata, &entries, handles, arena)
    }
}

/// Trusted boot assembles the private handles first. This record describes
/// exactly those handles and cannot mint or amplify their rights.
pub(crate) fn install_record<'objects>(
    metadata: Metadata,
    entries: &[Entry],
    handles: &mut Handles<'objects>,
    arena: &'objects Objects,
) -> Result<Handle, Error> {
    validate_metadata(metadata)?;
    if entries.len() > MAX_GRANTS {
        return Err(Error::InvalidArgument);
    }
    for (index, entry) in entries.iter().copied().enumerate() {
        validate_entry(entry)?;
        if entries[..index]
            .iter()
            .any(|prior| prior.role == entry.role)
        {
            return Err(Error::InvalidArgument);
        }
        let actual = handles
            .describe(Handle::from_raw(entry.handle))
            .map_err(super::native::map_error)?;
        if wire_kind(actual.0) != entry.kind {
            return Err(Error::WrongType);
        }
        if actual.1.bits() != entry.rights {
            return Err(Error::PermissionDenied);
        }
    }
    let length = HEADER_WORDS + entries.len() * ENTRY_WORDS;
    let mut words = Vec::new();
    words
        .try_reserve_exact(length)
        .map_err(|_| Error::OutOfResources)?;
    words.extend_from_slice(&[
        MAGIC,
        VERSION,
        length as u64,
        metadata.service,
        metadata.instance,
        metadata.argument,
        entries.len() as u64,
    ]);
    for entry in entries {
        words.extend_from_slice(&[entry.role, entry.kind as u64, entry.rights, entry.handle]);
    }
    publish(
        KernelObject::StartupRecord(words),
        ObjectKind::StartupRecord,
        Rights::READ,
        handles,
        arena,
    )
}

pub(crate) fn read(record: &Object<'_>, index: u64) -> Result<u64, Error> {
    let index = usize::try_from(index).map_err(|_| Error::InvalidArgument)?;
    record.with(|object| {
        let KernelObject::StartupRecord(words) = object else {
            return Err(Error::WrongType);
        };
        words.get(index).copied().ok_or(Error::InvalidArgument)
    })
}

fn stage<'objects>(entry: Entry, handles: &Handles<'objects>) -> Result<Batch<'objects>, Error> {
    validate_entry(entry)?;
    let source = Handle::from_raw(entry.handle);
    let (kind, _) = handles.describe(source).map_err(super::native::map_error)?;
    if wire_kind(kind) != entry.kind {
        return Err(Error::WrongType);
    }
    handles
        .prepare_transfer(&[TransferSpec {
            handle: source,
            rights: Rights::from_bits(entry.rights).ok_or(Error::InvalidArgument)?,
        }])
        .map_err(super::native::map_error)
}

fn validate_metadata(metadata: Metadata) -> Result<(), Error> {
    if metadata.service == 0 || metadata.instance == 0 {
        return Err(Error::InvalidArgument);
    }
    Ok(())
}

fn validate_entry(entry: Entry) -> Result<(), Error> {
    if entry.role == 0 || entry.rights == 0 || entry.rights & !entry.kind.allowed_rights() != 0 {
        return Err(Error::InvalidArgument);
    }
    Ok(())
}

fn publish<'objects>(
    value: KernelObject,
    kind: ObjectKind,
    rights: Rights,
    handles: &mut Handles<'objects>,
    arena: &'objects Objects,
) -> Result<Handle, Error> {
    let reservation = handles.reserve(1).map_err(super::native::map_error)?;
    let object = match arena.create(kind, value) {
        Ok(object) => object,
        Err(_) => {
            handles
                .release_reservation(reservation)
                .expect("Startup object reservation rollback invariant");
            return Err(Error::OutOfResources);
        }
    };
    match handles.commit_transfer(reservation, Batch::from_object(object, rights)) {
        Ok(installed) => Ok(installed[0].expect("Startup object result invariant")),
        Err(failure) => {
            handles
                .release_reservation(failure.reservation)
                .expect("Startup object commit rollback invariant");
            Err(super::native::map_error(failure.error))
        }
    }
}

pub(crate) const fn wire_kind(kind: ObjectKind) -> WireKind {
    match kind {
        ObjectKind::TaskControl => WireKind::TaskControl,
        ObjectKind::Memory => WireKind::Memory,
        ObjectKind::Endpoint => WireKind::Endpoint,
        ObjectKind::Notification => WireKind::Notification,
        ObjectKind::ProcessControl => WireKind::ProcessControl,
        ObjectKind::ThreadControl => WireKind::ThreadControl,
        ObjectKind::Image => WireKind::Image,
        ObjectKind::DebugConsole => WireKind::DebugConsole,
        ObjectKind::Device => WireKind::Device,
        ObjectKind::ChannelFactory => WireKind::ChannelFactory,
        ObjectKind::LaunchManifest => WireKind::LaunchManifest,
        ObjectKind::StartupRecord => WireKind::StartupRecord,
        ObjectKind::ResourceBudget => WireKind::ResourceBudget,
    }
}

#[cfg(feature = "boot-tests")]
pub(crate) mod checks;
#[cfg(feature = "boot-tests")]
pub(crate) mod receive_checks;

pub(crate) fn assign_budget(
    owner: ProcessId,
    manifest: Handle,
    account: Handle,
    handles: &Handles<'_>,
) -> Result<(), Error> {
    let object = handles
        .lookup(manifest, ObjectKind::LaunchManifest, Rights::WRITE)
        .map_err(super::native::map_error)?;
    object.with(|object| {
        let KernelObject::LaunchManifest(manifest) = object else {
            unreachable!()
        };
        if manifest.borrow().owner != owner {
            return Err(Error::PermissionDenied);
        }
        Ok(())
    })?;
    drop(account_from(handles, account)?);
    object.with(|object| {
        let KernelObject::LaunchManifest(manifest) = object else {
            unreachable!()
        };
        manifest.borrow_mut().budget = Some(account);
    });
    Ok(())
}

fn account_from(handles: &Handles<'_>, handle: Handle) -> Result<Budget, Error> {
    let account = handles
        .lookup(
            handle,
            ObjectKind::ResourceBudget,
            Rights::ASSIGN | Rights::TRANSFER,
        )
        .map_err(super::native::map_error)?;
    account.with(|object| match object {
        KernelObject::ResourceBudget(account) => Ok(account.clone()),
        _ => Err(Error::WrongType),
    })
}
