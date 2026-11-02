use super::{BootFramePool, IpcRuntime, KernelMemory, KernelObject, Object, Tasks};
use alloc::rc::Rc;
use core::cell::RefCell;
use kryo_core::capability::{Handle, Rights};
use kryo_core::device::dma::{DmaBackend, DmaBuffer, DmaLimits, DmaState};
use kryo_core::device::queue::{self, Plan, Queue, Request};
use kryo_core::mm::frame::{FRAME_SIZE, PhysRange};
use kryo_core::mm::shared::SharedBackend;
use kryo_core::object::ObjectKind;
use kryo_core::process::ProcessId;
use kryo_core::task::TaskId;
use kryo_hal::{DirectMap, VirtioError, VirtioPci, VirtioQueue};
use kryo_interfaces::syscall::{Call, Error};

mod probe;
pub(crate) use probe::run;

const ALLOWED_FEATURES: u64 = (1 << 32) | (1 << 5) | (1 << 9);
type Storage = DmaBuffer<'static, Backend, { crate::boot::memory::OUTPUT_CAPACITY }, 0, 0>;

pub(crate) struct Transport<'objects> {
    owner: ProcessId,
    object: Object<'objects>,
    hardware: Rc<RefCell<VirtioPci>>,
    storage: Option<Storage>,
    queue: Queue,
    cpu: u64,
    device: u64,
    direct: DirectMap,
    pending: Option<Pending<'objects>>,
    started: bool,
    #[cfg(feature = "boot-tests")]
    retirement_failures: u8,
}

struct Pending<'objects> {
    memory: Object<'objects>,
    plan: Plan,
    ticket: u64,
}

impl<'objects> Transport<'objects> {
    pub(crate) fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
    pub(crate) fn create(
        owner: ProcessId,
        object: Object<'objects>,
        hardware: VirtioPci,
        pool: &'static BootFramePool,
        direct: DirectMap,
        kernel: &RefCell<KernelMemory>,
    ) -> Result<Self, Error> {
        let hardware = Rc::new(RefCell::new(hardware));
        let limits = DmaLimits {
            max_pages: 3,
            physical: PhysRange::from_base_len(0, 1 << 32).unwrap(),
            address_limit: u32::MAX.into(),
        };
        // Initialization borrows VM policy only during this synchronous call.
        // The retained device backend owns no borrow of process/VM metadata.
        let storage = unsafe {
            DmaBuffer::new_with_initializer(
                pool,
                3,
                limits,
                Backend {
                    hardware: hardware.clone(),
                },
                |physical, bytes| {
                    for offset in (0..bytes).step_by(FRAME_SIZE as usize) {
                        kernel
                            .borrow_mut()
                            .initialize_shared(physical + offset)
                            .map_err(|_| VirtioError::InvalidState)?;
                    }
                    Ok(())
                },
            )
        }
        .map_err(|failure| {
            if let Some(storage) = failure.retained {
                // No device has seen these addresses. Retain every owner if
                // even constructor retirement cannot finish.
                core::mem::forget(storage);
            }
            Error::OutOfResources
        })?;
        let cpu = direct
            .frame_address(storage.physical_address())
            .map_err(|_| Error::InvalidArgument)?;
        let device = storage.range().address;
        // The entire run is private, coherent RW+NX RAM; the device remains reset.
        let queue = unsafe { Queue::new(cpu, device) }.map_err(|_| Error::InvalidArgument)?;
        Ok(Self {
            owner,
            object,
            hardware,
            storage: Some(storage),
            queue,
            cpu,
            device,
            direct,
            pending: None,
            started: false,
            #[cfg(feature = "boot-tests")]
            retirement_failures: 0,
        })
    }

    fn start(&mut self, features: u64) -> Result<u64, Error> {
        if self.started || self.pending.is_some() {
            return Err(Error::InvalidArgument);
        }
        let storage = self.storage.as_mut().ok_or(Error::PeerClosed)?;
        if storage.state() == DmaState::Mapped {
            storage.publish().map_err(|_| Error::InvalidArgument)?;
        }
        // Publication precedes queue_enable and DRIVER_OK; only kernel-created
        // descriptors can name bytes in this retained private DMA allocation.
        unsafe {
            self.hardware.borrow_mut().start(
                features,
                ALLOWED_FEATURES,
                VirtioQueue {
                    size: queue::QUEUE_SIZE,
                    descriptors: self.device + queue::DESCRIPTOR_OFFSET,
                    available: self.device + queue::AVAILABLE_OFFSET,
                    used: self.device + queue::USED_OFFSET,
                },
            )
        }
        .map_err(map_error)?;
        self.started = true;
        Ok(0)
    }

    fn submit(&mut self, memory: Object<'objects>, offset: u64, count: u64) -> Result<u64, Error> {
        if !self.started || self.pending.is_some() {
            return Err(Error::InvalidArgument);
        }
        let count = usize::try_from(count).map_err(|_| Error::InvalidArgument)?;
        if !(1..=queue::MAX_BUFFERS).contains(&count) {
            return Err(Error::InvalidArgument);
        }
        let mut requests = [Request {
            offset: 0,
            length: 0,
            flags: 0,
        }; queue::MAX_BUFFERS];
        let mut records = [0u8; 48];
        read_memory(&memory, self.direct, offset, &mut records[..count * 16])?;
        for (index, request) in requests.iter_mut().enumerate().take(count) {
            let record = &records[index * 16..index * 16 + 16];
            request.offset = u64::from_le_bytes(record[..8].try_into().unwrap());
            request.length = u32::from_le_bytes(record[8..12].try_into().unwrap());
            request.flags = u32::from_le_bytes(record[12..16].try_into().unwrap());
        }
        let bytes = memory.with(|object| match object {
            KernelObject::Memory(memory) => memory.byte_len(),
            _ => 0,
        });
        let plan = Plan::new(&requests[..count], bytes).map_err(|_| Error::InvalidArgument)?;
        // No worker executes user code during policy work. Snapshot all inputs
        // before availability publication; the hardware never sees user RAM.
        for buffer in plan.buffers() {
            let target = self.cpu + queue::BOUNCE_OFFSET + u64::from(buffer.bounce_offset);
            if buffer.device_writable {
                unsafe { (target as *mut u8).write_bytes(0, buffer.bytes as usize) };
            } else {
                let output = unsafe {
                    core::slice::from_raw_parts_mut(target as *mut u8, buffer.bytes as usize)
                };
                read_memory(&memory, self.direct, buffer.memory_offset, output)?;
            }
        }
        let ticket = self
            .queue
            .submit(&plan)
            .map_err(|_| Error::InvalidArgument)?;
        self.pending = Some(Pending {
            memory,
            plan,
            ticket,
        });
        // A notification error does not retract an already exposed buffer.
        self.hardware.borrow_mut().notify().map_err(map_error)?;
        Ok(ticket)
    }

    fn poll(&mut self, ticket: u64) -> Result<u64, Error> {
        let pending = self.pending.as_ref().ok_or(Error::InvalidArgument)?;
        if pending.ticket != ticket {
            return Err(Error::InvalidArgument);
        }
        let Some(length) = self
            .queue
            .poll(ticket)
            .map_err(|_| Error::InvalidArgument)?
        else {
            return Ok(0);
        };
        let pending = self.pending.take().unwrap();
        for buffer in pending
            .plan
            .buffers()
            .iter()
            .filter(|buffer| buffer.device_writable)
        {
            let source = self.cpu + queue::BOUNCE_OFFSET + u64::from(buffer.bounce_offset);
            // Used-ring acquire precedes these reads. The device has returned
            // the chain; user code remains stopped until copying finishes.
            let bytes =
                unsafe { core::slice::from_raw_parts(source as *const u8, buffer.bytes as usize) };
            write_memory(&pending.memory, self.direct, buffer.memory_offset, bytes)?;
        }
        self.hardware
            .borrow_mut()
            .acknowledge()
            .map_err(map_error)?;
        Ok(kryo_interfaces::virtio::COMPLETE | u64::from(length))
    }

    fn reset(&mut self) -> Result<u64, Error> {
        self.started = false;
        if !self.hardware.borrow_mut().reset().map_err(map_error)? {
            return Ok(0);
        }
        // VirtIO reset status zero forbids any further accesses to old queues.
        unsafe { self.queue.reset() };
        self.pending = None;
        self.hardware.borrow_mut().prepare().map_err(map_error)?;
        Ok(1)
    }

    fn close(&mut self) -> Result<(), VirtioError> {
        self.started = false;
        #[cfg(feature = "boot-tests")]
        if self.retirement_failures > 0 {
            self.retirement_failures -= 1;
            return Err(VirtioError::ResetPending);
        }
        if let Some(storage) = self.storage.take() {
            match storage.retire() {
                Ok(_) => {}
                Err(failure) => {
                    self.storage = failure.retained;
                    return Err(VirtioError::ResetPending);
                }
            }
        }
        self.pending = None;
        if !self.hardware.borrow_mut().close()? {
            return Err(VirtioError::ResetPending);
        }
        Ok(())
    }
}

pub(crate) fn dispatch<'objects>(
    caller: TaskId,
    call: Call,
    context: kryo_hal::UserContext,
    tasks: &mut Tasks<'_, 'objects>,
    ipc: &mut IpcRuntime<'objects>,
) -> Result<u64, Error> {
    let rights = match call {
        Call::VirtioStart | Call::VirtioReset => Rights::CONTROL,
        Call::VirtioSubmit => Rights::WRITE,
        _ => Rights::READ,
    };
    let owner = tasks.process_for(caller).ok_or(Error::PeerClosed)?;
    let device = tasks
        .resources_mut(caller)
        .unwrap()
        .process
        .handles
        .lookup(Handle::from_raw(context.rdi), ObjectKind::Device, rights)
        .map_err(super::native::map_error)?;
    device.with(|object| match object {
        KernelObject::Transport(expected) if *expected == owner => Ok(()),
        KernelObject::Transport(_) => Err(Error::PermissionDenied),
        _ => Err(Error::WrongType),
    })?;
    let transport = ipc.transport.as_mut().ok_or(Error::PeerClosed)?;
    if transport.object.id() != device.id() || transport.owner != owner {
        return Err(Error::PermissionDenied);
    }
    match call {
        Call::VirtioInfo => match context.rsi {
            0 => transport
                .hardware
                .borrow_mut()
                .features()
                .map_err(map_error),
            1 if transport.started => transport
                .hardware
                .borrow_mut()
                .read_config64(0)
                .map_err(map_error),
            2 => Ok(u64::from(queue::QUEUE_SIZE)),
            _ => Err(Error::InvalidArgument),
        },
        Call::VirtioStart => transport.start(context.rsi),
        Call::VirtioSubmit => {
            let memory = tasks
                .resources_mut(caller)
                .unwrap()
                .process
                .handles
                .lookup(
                    Handle::from_raw(context.rsi),
                    ObjectKind::Memory,
                    Rights::READ | Rights::WRITE,
                )
                .map_err(super::native::map_error)?;
            transport.submit(memory, context.rdx, context.r8)
        }
        Call::VirtioPoll => transport.poll(context.rsi),
        Call::VirtioReset => transport.reset(),
        _ => Err(Error::InvalidCall),
    }
}

pub(crate) fn retire_bindings(
    tasks: &Tasks<'_, '_>,
    ipc: &mut IpcRuntime<'_>,
) -> Result<(), VirtioError> {
    if let Some(transport) = &mut ipc.transport
        && tasks.terminal(transport.owner).is_some()
    {
        transport.close()?;
        ipc.transport = None;
    }
    Ok(())
}

struct Backend {
    hardware: Rc<RefCell<VirtioPci>>,
}

// Identity DMA is confined to private buffers and kernel-created descriptors.
// Only the temporary VM initializer may establish the CPU alias. Retirement
// retains this Rc (and therefore the device claim) until reset reports zero.
unsafe impl DmaBackend for Backend {
    type Mapping = u64;
    type Error = VirtioError;
    unsafe fn initialize(&mut self, _: u64, _: u64) -> Result<(), Self::Error> {
        Err(VirtioError::InvalidState)
    }
    unsafe fn map(&mut self, physical: u64, _: u64) -> Result<u64, Self::Error> {
        Ok(physical)
    }
    fn address(&self, mapping: &u64) -> u64 {
        *mapping
    }
    fn quiesce(&mut self, _: &mut u64) -> Result<(), Self::Error> {
        if self.hardware.borrow_mut().reset()? {
            Ok(())
        } else {
            Err(VirtioError::ResetPending)
        }
    }
    fn unmap(&mut self, _: &mut u64) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn map_error(error: VirtioError) -> Error {
    match error {
        VirtioError::Closed => Error::PeerClosed,
        VirtioError::Busy | VirtioError::ResetPending | VirtioError::ConfigurationChanged => {
            Error::OutOfResources
        }
        _ => Error::InvalidArgument,
    }
}

fn read_memory(
    object: &Object<'_>,
    direct: DirectMap,
    offset: u64,
    output: &mut [u8],
) -> Result<(), Error> {
    copy_memory(
        object,
        direct,
        offset,
        output.len(),
        |physical, within, done, count| {
            unsafe { direct.read_frame(physical, within, &mut output[done..done + count]) }
                .map_err(|_| Error::InvalidArgument)
        },
    )
}

fn write_memory(
    object: &Object<'_>,
    direct: DirectMap,
    offset: u64,
    input: &[u8],
) -> Result<(), Error> {
    copy_memory(
        object,
        direct,
        offset,
        input.len(),
        |physical, within, done, count| {
            unsafe { direct.write_frame(physical, within, &input[done..done + count]) }
                .map_err(|_| Error::InvalidArgument)
        },
    )
}

fn copy_memory(
    object: &Object<'_>,
    _: DirectMap,
    offset: u64,
    bytes: usize,
    mut copy: impl FnMut(u64, usize, usize, usize) -> Result<(), Error>,
) -> Result<(), Error> {
    object.with(|object| {
        let KernelObject::Memory(memory) = object else {
            return Err(Error::WrongType);
        };
        if offset
            .checked_add(bytes as u64)
            .is_none_or(|end| end > memory.byte_len())
        {
            return Err(Error::InvalidArgument);
        }
        let mut done = 0;
        while done < bytes {
            let position = offset + done as u64;
            let within = (position % FRAME_SIZE) as usize;
            let count = (FRAME_SIZE as usize - within).min(bytes - done);
            let physical = memory
                .page_address((position / FRAME_SIZE) as usize)
                .ok_or(Error::InvalidArgument)?;
            copy(physical, within, done, count)?;
            done += count;
        }
        Ok(())
    })
}
