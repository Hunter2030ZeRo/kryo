#[cfg(feature = "boot-tests")]
mod checks;
mod framebuffer;
mod memory;

use limine::request::{
    ExecutableAddressRequest, FramebufferRequest, HhdmRequest, MemmapRequest, PagingModeRequest,
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

#[used]
#[unsafe(link_section = ".requests")]
static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();

#[used]
#[unsafe(link_section = ".requests_end_marker")]
static REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();

fn check_protocol() {
    assert!(
        BASE_REVISION.is_supported(),
        "Unsupported Limine base revision"
    );
}
