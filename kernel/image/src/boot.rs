#[cfg(feature = "boot-tests")]
mod checks;
mod framebuffer;
mod memory;
#[cfg(feature = "boot-tests")]
mod task_checks;
#[cfg(feature = "boot-tests")]
mod user_checks;
#[cfg(feature = "boot-tests")]
mod vm_backend;

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
static MEMORY_MAP: MemmapRequest = MemmapRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static HHDM: HhdmRequest = HhdmRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static EXECUTABLE_ADDRESS: ExecutableAddressRequest = ExecutableAddressRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static PAGING_MODE: PagingModeRequest = PagingModeRequest::new_exact(PagingMode::X86_64_4LVL);

// Debug initialization moves bounded VM metadata on the bootstrap stack.
#[used]
#[unsafe(link_section = ".requests")]
static STACK_SIZE: StackSizeRequest = StackSizeRequest::new(256 * 1024);

#[used]
#[unsafe(link_section = ".requests")]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

#[used]
#[unsafe(link_section = ".requests_end_marker")]
static REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();

fn check_protocol() {
    assert!(
        STACK_SIZE.response().is_some(),
        "Missing bootstrap stack response"
    );
    assert!(
        BASE_REVISION.is_supported(),
        "Unsupported Limine base revision"
    );
}
