#[cfg(feature = "boot-tests")]
mod checks;
mod framebuffer;
pub(crate) mod memory;
pub(crate) mod platform;
pub(crate) mod storage;
#[cfg(feature = "boot-tests")]
mod task_checks;
#[cfg(feature = "boot-tests")]
mod user_checks;
pub(crate) mod vm_backend;

use limine::request::ModulesRequest;
use limine::request::MpRequest;
use limine::request::RsdpRequest;
use limine::request::{
    ExecutableAddressRequest, FramebufferRequest, HhdmRequest, MemmapRequest, PagingModeRequest,
    StackSizeRequest,
};
use limine::{BaseRevision, RequestsEndMarker, RequestsStartMarker, paging::PagingMode};

pub(super) use framebuffer::init_framebuffer;
pub(super) use memory::init_physical_memory;

#[used]
#[unsafe(link_section = ".requests_start_marker")]
static REQUESTS_START: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(link_section = ".requests")]
static BASE_REVISION: BaseRevision = BaseRevision::new();

#[used]
#[unsafe(link_section = ".requests")]
pub(crate) static MEMORY_MAP: MemmapRequest = MemmapRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static HHDM: HhdmRequest = HhdmRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static EXECUTABLE_ADDRESS: ExecutableAddressRequest = ExecutableAddressRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static PAGING_MODE: PagingModeRequest = PagingModeRequest::new_exact(PagingMode::X86_64_4LVL);

// Unoptimized registry construction retains larger move temporaries. This is
// the Limine entry stack on each CPU; thread entry stacks remain guarded 16 KiB.
#[used]
#[unsafe(link_section = ".requests")]
static STACK_SIZE: StackSizeRequest = StackSizeRequest::new(if cfg!(debug_assertions) {
    512 * 1024
} else {
    256 * 1024
});

#[used]
#[unsafe(link_section = ".requests")]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
pub(crate) static MODULES: ModulesRequest = ModulesRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
pub(crate) static RSDP: RsdpRequest = RsdpRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
pub(crate) static MP: MpRequest = MpRequest::new(0);

#[derive(Debug)]
pub(crate) enum ModuleError {
    Missing,
    Duplicate,
    InvalidSize,
}

#[cfg(feature = "boot-tests")]
fn native_image() -> Result<&'static [u8], ModuleError> {
    module_image("kryo-native-smoke")
}

pub(crate) fn module_image(tag: &str) -> Result<&'static [u8], ModuleError> {
    module_image_limited(tag, 1024 * 1024)
}

pub(crate) fn module_image_limited(
    tag: &str,
    maximum_bytes: usize,
) -> Result<&'static [u8], ModuleError> {
    let modules = MODULES.response().ok_or(ModuleError::Missing)?.modules();
    let mut selected = None;
    for module in modules {
        if module.cmdline() != tag {
            continue;
        }
        if selected.is_some() {
            return Err(ModuleError::Duplicate);
        }
        let bytes = module.data();
        if bytes.is_empty() || bytes.len() > maximum_bytes {
            return Err(ModuleError::InvalidSize);
        }
        selected = Some(bytes);
    }
    // Boot module pages remain reserved; no borrowed image bytes are reclaimed.
    selected.ok_or(ModuleError::Missing)
}

#[used]
#[unsafe(link_section = ".requests_end_marker")]
static REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();

pub(crate) fn check_protocol() {
    assert!(
        STACK_SIZE.response().is_some(),
        "Missing bootstrap stack response"
    );
    assert!(
        BASE_REVISION.is_supported(),
        "Unsupported Limine base revision"
    );
}

#[cfg(not(feature = "boot-tests"))]
pub(crate) fn start_services() -> ! {
    let direct = unsafe { kryo_hal::DirectMap::new(HHDM.response().expect("Missing HHDM").offset) }
        .expect("Invalid direct map");
    let physical_end = MEMORY_MAP
        .response()
        .expect("Missing memory map")
        .entries()
        .iter()
        .filter(|entry| entry.type_ == limine::memmap::MEMMAP_USABLE)
        .map(|entry| {
            entry
                .base
                .checked_add(entry.length)
                .expect("Invalid usable range")
        })
        .max()
        .expect("No usable RAM");
    crate::runtime::service::run(memory::frames(), direct, physical_end)
}
