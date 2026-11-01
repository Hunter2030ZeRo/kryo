mod extended;
mod traps;

use super::cpu::{MAX_CPUS, current_cpu};
use core::arch::global_asm;
use core::cell::UnsafeCell;
use core::mem::offset_of;
use x86_64::instructions::interrupts;
use x86_64::registers::control::{Cr0, Cr0Flags, Cr3, Cr4, Cr4Flags};
use x86_64::registers::model_specific::{Efer, EferFlags, FsBase, GsBase, Msr};
use x86_64::registers::segmentation::{DS, ES, FS, GS, SS, Segment};
use x86_64::{PhysAddr, VirtAddr};

pub use extended::ExtendedState;
pub(super) use traps::install_handlers;

const TIMER_SLICE_NS: u64 = 2_000_000;
const USER_FLAGS: u64 = (1 << 0)
    | (1 << 2)
    | (1 << 4)
    | (1 << 6)
    | (1 << 7)
    | (1 << 8)
    | (1 << 10)
    | (1 << 11)
    | (1 << 16)
    | (1 << 18)
    | (1 << 21);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserEntryError {
    InvalidAddress,
    UnalignedStack,
    InvalidRoot,
    InvalidKernelStack,
    InterruptsEnabled,
    UnsupportedCpuState,
    NotInitialized,
    AlreadyRunning,
    TimerUnavailable,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserContext {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rsp: u64,
    pub rflags: u64,
    pub fs_base: u64,
    pub gs_base: u64,
}

impl UserContext {
    pub fn new(entry: u64, stack: u64) -> Result<Self, UserEntryError> {
        validate_addresses(entry, stack)?;
        if !stack.is_multiple_of(16) {
            return Err(UserEntryError::UnalignedStack);
        }
        Ok(Self {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rip: entry,
            rsp: stack,
            rflags: 2,
            fs_base: 0,
            gs_base: 0,
        })
    }

    pub fn set_fs_base(&mut self, base: u64) -> Result<(), UserEntryError> {
        validate_bases(base, 0)?;
        self.fs_base = base;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserReason {
    Exited,
    Fault,
    Call,
    Preempted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserOutcome {
    pub reason: UserReason,
    pub status: u64,
    pub vector: u64,
    pub error: u64,
    pub address: u64,
    pub instruction: u64,
}

impl UserOutcome {
    const EMPTY: Self = Self {
        reason: UserReason::Exited,
        status: 0,
        vector: 0,
        error: 0,
        address: 0,
        instruction: 0,
    };
}

#[repr(C)]
struct RunState {
    kernel_rsp: u64,
    kernel_cr3: u64,
    kernel_cr0: u64,
    kernel_cr4: u64,
    kernel_flags: u64,
    kernel_xcr0: u64,
    fp_area: *mut u8,
    fp_format: u64,
    fp_mask: u64,
    fs_base: u64,
    gs_base: u64,
    sysenter_cs: u64,
    kernel_entry_stack: u64,
    efer: u64,
    selectors: [u16; 5],
    context: *mut UserContext,
    has_sysenter: bool,
    active: bool,
    legacy_exit: bool,
    preemptible: bool,
    kernel_fp: *mut u8,
    outcome: UserOutcome,
}

#[repr(C, align(64))]
struct LocalRun(UnsafeCell<RunState>);

// Each registered CPU owns its slot, with IRQs off and no nested user run.
unsafe impl Sync for LocalRun {}

static RUN_STATES: [LocalRun; MAX_CPUS] = [const {
    LocalRun(UnsafeCell::new(RunState {
        kernel_rsp: 0,
        kernel_cr3: 0,
        kernel_cr0: 0,
        kernel_cr4: 0,
        kernel_flags: 0,
        kernel_xcr0: 0,
        fp_area: core::ptr::null_mut(),
        fp_format: 0,
        fp_mask: 0,
        fs_base: 0,
        gs_base: 0,
        sysenter_cs: 0,
        kernel_entry_stack: 0,
        efer: 0,
        selectors: [0; 5],
        context: core::ptr::null_mut(),
        has_sysenter: false,
        active: false,
        legacy_exit: false,
        preemptible: false,
        kernel_fp: core::ptr::null_mut(),
        outcome: UserOutcome::EMPTY,
    }))
}; MAX_CPUS];

fn local_state() -> Option<*mut RunState> {
    Some(RUN_STATES[current_cpu()?.index()].0.get())
}

pub(crate) fn user_active() -> bool {
    local_state().is_some_and(|state| unsafe { (*state).active })
}

// This helper runs before user FP state is saved. The entire HAL is compiled
// soft-float; it only identifies stable CPU-local storage after GPR capture.
extern "C" fn trap_state() -> *mut () {
    local_state().unwrap_or(core::ptr::null_mut()).cast::<()>()
}

global_asm!(
    include_str!("entry.S"),
    trap_state = sym trap_state,
    kernel_rsp = const offset_of!(RunState, kernel_rsp),
    kernel_cr3 = const offset_of!(RunState, kernel_cr3),
    kernel_cr0 = const offset_of!(RunState, kernel_cr0),
    kernel_cr4 = const offset_of!(RunState, kernel_cr4),
    kernel_flags = const offset_of!(RunState, kernel_flags),
    kernel_xcr0 = const offset_of!(RunState, kernel_xcr0),
    kernel_efer = const offset_of!(RunState, efer),
    fp_area = const offset_of!(RunState, fp_area),
    fp_format = const offset_of!(RunState, fp_format),
    fp_mask = const offset_of!(RunState, fp_mask),
    kernel_fp = const offset_of!(RunState, kernel_fp),
    trap_code = const traps::FRAME_CODE_OFFSET,
    context_rip = const offset_of!(UserContext, rip),
    context_rsp = const offset_of!(UserContext, rsp),
    context_flags = const offset_of!(UserContext, rflags),
    fault_dispatch = sym traps::fault_dispatch,
    exit_dispatch = sym traps::exit_dispatch,
    device_dispatch = sym traps::device_dispatch,
    timer_dispatch = sym traps::timer_dispatch,
    reschedule_dispatch = sym traps::reschedule_dispatch,
    shootdown_dispatch = sym traps::shootdown_dispatch,
);

unsafe extern "C" {
    fn kryo_enter_user(
        root: u64,
        context: *const UserContext,
        code: u64,
        data: u64,
        state: *mut (),
    );
    fn kryo_leave_user(state: *mut ()) -> !;
}

/// Runs a bounded scalar bootstrap program using the legacy int80 exit gate.
///
/// # Safety
/// The CPU, mapping, and CPU-state requirements of run_context apply. The default
/// boot entry stack is retained and IRQs stay disabled; there is no preemption.
pub unsafe fn run_user(
    root: u64,
    instruction: u64,
    stack: u64,
) -> Result<UserOutcome, UserEntryError> {
    let mut context = UserContext::new(instruction, stack)?;
    let kernel_stack = super::tss::kernel_stack().as_u64();
    unsafe { run(root, &mut context, kernel_stack, false, true, None, None) }
}

/// Resumes one scalar user context until a call, fault, or timer preemption.
///
/// # Safety
/// Call on a registered CPU with its own initialized GDT/TSS, a loaded IDT, and
/// IRQs disabled. Its scheduler must exclusively own this context and its space's
/// execution lease until return; another CPU must not run the same context.
/// The inactive root must retain all kernel supervisor mappings. Retain its user
/// frames and tables, and the supplied kernel entry stack, until this call returns.
/// The entry stack must be mapped writable supervisor memory with sufficient space.
/// Do not mutate mappings while the context runs. Preemptible runs require an
/// initialized clock and calibrated local timer. FP/SIMD state is unsupported
/// and traps instead. Preemptible runs use a two-millisecond one-shot slice.
pub unsafe fn run_context(
    root: u64,
    context: &mut UserContext,
    kernel_stack_top: u64,
    preemptible: bool,
) -> Result<UserOutcome, UserEntryError> {
    unsafe {
        run(
            root,
            context,
            kernel_stack_top,
            preemptible,
            false,
            None,
            None,
        )
    }
}

/// Resumes a context with eagerly preserved x87, SSE, and optional AVX state.
///
/// # Safety
/// All run_context requirements apply. Retain exclusive kernel-owned state storage
/// until return. Compile all kernel code with the bare-metal soft-float target;
/// hardware FP/SIMD instructions are restricted to the entry/save mechanisms.
pub unsafe fn run_context_extended(
    root: u64,
    context: &mut UserContext,
    extended: &mut ExtendedState,
    kernel_stack_top: u64,
    preemptible: bool,
) -> Result<UserOutcome, UserEntryError> {
    unsafe {
        run(
            root,
            context,
            kernel_stack_top,
            preemptible,
            false,
            Some(extended),
            None,
        )
    }
}

/// Resumes a user context with an absolute monotonic timer deadline.
///
/// # Safety
/// The run_context requirements apply, including retained initialized clock and
/// calibrated timer mappings. If supplied, retain exclusive extended-state
/// storage until return. The scheduler must recheck deadlines after every return;
/// an IRQ can arrive early from an older event or a bounded hardware chunk.
pub unsafe fn run_context_until(
    root: u64,
    context: &mut UserContext,
    extended: Option<&mut ExtendedState>,
    kernel_stack_top: u64,
    deadline_ns: u64,
) -> Result<UserOutcome, UserEntryError> {
    unsafe {
        run(
            root,
            context,
            kernel_stack_top,
            true,
            false,
            extended,
            Some(deadline_ns),
        )
    }
}

unsafe fn run(
    root: u64,
    context: &mut UserContext,
    kernel_stack_top: u64,
    preemptible: bool,
    legacy_exit: bool,
    extended: Option<&mut ExtendedState>,
    deadline_ns: Option<u64>,
) -> Result<UserOutcome, UserEntryError> {
    let current_root = Cr3::read().0.start_address().as_u64();
    validate_root(root, current_root)?;
    validate_addresses(context.rip, context.rsp)?;
    validate_bases(context.fs_base, context.gs_base)?;
    validate_kernel_stack(kernel_stack_top)?;
    check_cpu()?;
    let (code, data) = super::gdt::user_selectors().ok_or(UserEntryError::NotInitialized)?;
    let cpu = current_cpu().ok_or(UserEntryError::NotInitialized)?;
    let state = RUN_STATES[cpu.index()].0.get();
    if unsafe { (*state).active } {
        return Err(UserEntryError::AlreadyRunning);
    }
    let kernel_xcr0 = if let Some(extended) = extended.as_ref() {
        unsafe { extended::validate_cpu(extended)? }
    } else {
        0
    };
    if preemptible {
        if !crate::timer_initialized() {
            return Err(UserEntryError::TimerUnavailable);
        }
        let deadline = match deadline_ns {
            Some(deadline) => deadline,
            None => crate::monotonic_time()
                .map_err(|_| UserEntryError::TimerUnavailable)?
                .checked_add(TIMER_SLICE_NS)
                .ok_or(UserEntryError::TimerUnavailable)?,
        };
        unsafe { crate::arm_deadline(deadline) }.map_err(|_| UserEntryError::TimerUnavailable)?;
        if !crate::timer_ready() {
            return Err(UserEntryError::TimerUnavailable);
        }
    }
    context.rflags = sanitize_flags(context.rflags, preemptible);
    let has_sysenter = core::arch::x86_64::__cpuid(1).edx & (1 << 11) != 0;
    let sysenter_cs = if has_sysenter {
        unsafe { Msr::new(0x174).read() }
    } else {
        0
    };
    unsafe {
        (*state).kernel_cr0 = Cr0::read_raw();
        (*state).kernel_cr4 = Cr4::read_raw();
        (*state).kernel_xcr0 = kernel_xcr0;
        (*state).fp_area = core::ptr::null_mut();
        (*state).fp_format = 0;
        (*state).fp_mask = 0;
        (*state).fs_base = FsBase::read().as_u64();
        (*state).gs_base = GsBase::read().as_u64();
        (*state).kernel_entry_stack = super::tss::kernel_stack().as_u64();
        (*state).selectors = [
            DS::get_reg().0,
            ES::get_reg().0,
            FS::get_reg().0,
            GS::get_reg().0,
            SS::get_reg().0,
        ];
        // Raw restoration must retain architectural bits absent from the crate's flags.
        (*state).efer = Efer::read_raw();
        (*state).has_sysenter = has_sysenter;
        (*state).sysenter_cs = sysenter_cs;
        (*state).context = context;
        (*state).outcome = UserOutcome::EMPTY;
        (*state).legacy_exit = legacy_exit;
        (*state).preemptible = preemptible;
        (*state).kernel_fp = extended::kernel_area(cpu);
        (*state).active = true;

        super::tss::set_kernel_stack(VirtAddr::new(kernel_stack_top));
        FS::set_reg(x86_64::structures::gdt::SegmentSelector(0));
        GS::set_reg(x86_64::structures::gdt::SegmentSelector(0));
        FsBase::write(VirtAddr::new(context.fs_base));
        GsBase::write(VirtAddr::new(context.gs_base));
        Efer::update(|flags| {
            flags.remove(EferFlags::SYSTEM_CALL_EXTENSIONS);
            if extended.is_some() {
                flags.remove(EferFlags::FAST_FXSAVE_FXRSTOR);
            }
        });
        if has_sysenter {
            Msr::new(0x174).write(0);
        }
        // Native TLS bases are installed through validated saved context, not WRFSBASE.
        Cr4::update(|flags| {
            flags.remove(Cr4Flags::PERFORMANCE_MONITOR_COUNTER | Cr4Flags::FSGSBASE)
        });
        if let Some(extended) = extended {
            let layout = extended.layout();
            (*state).fp_area = extended.area_ptr();
            (*state).fp_format = layout.format as u64;
            (*state).fp_mask = layout.mask;
            extended::prepare(layout, cpu);
        } else {
            Cr0::update(|flags| {
                flags.insert(Cr0Flags::TASK_SWITCHED | Cr0Flags::MONITOR_COPROCESSOR)
            });
        }
        kryo_enter_user(
            root,
            context,
            u64::from(code),
            u64::from(data),
            state.cast::<()>(),
        );
        (*state).active = false;
        (*state).context = core::ptr::null_mut();
        (*state).fp_area = core::ptr::null_mut();
        Ok((*state).outcome)
    }
}

fn validate_root(root: u64, current: u64) -> Result<(), UserEntryError> {
    if root == 0
        || !root.is_multiple_of(4096)
        || root == current
        || PhysAddr::try_new(root).is_err()
    {
        return Err(UserEntryError::InvalidRoot);
    }
    let maximum = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    let bits = if maximum >= 0x8000_0008 {
        core::arch::x86_64::__cpuid(0x8000_0008).eax & 0xff
    } else {
        36
    };
    if bits > 52 || bits == 0 || (root + 4095) >> bits != 0 {
        return Err(UserEntryError::InvalidRoot);
    }
    Ok(())
}

fn validate_addresses(instruction: u64, stack: u64) -> Result<(), UserEntryError> {
    if !(4096..0x0000_8000_0000_0000).contains(&instruction)
        || !(4096..0x0000_8000_0000_0000).contains(&stack)
    {
        return Err(UserEntryError::InvalidAddress);
    }
    Ok(())
}

fn validate_bases(fs: u64, gs: u64) -> Result<(), UserEntryError> {
    if fs >= 0x0000_8000_0000_0000 || gs >= 0x0000_8000_0000_0000 {
        return Err(UserEntryError::InvalidAddress);
    }
    Ok(())
}

fn validate_kernel_stack(stack: u64) -> Result<(), UserEntryError> {
    if stack < 0xffff_8000_0000_1000
        || !stack.is_multiple_of(16)
        || VirtAddr::try_new(stack).is_err()
    {
        return Err(UserEntryError::InvalidKernelStack);
    }
    Ok(())
}

fn sanitize_flags(flags: u64, preemptible: bool) -> u64 {
    (flags & USER_FLAGS) | 2 | if preemptible { 1 << 9 } else { 0 }
}

fn check_cpu() -> Result<(), UserEntryError> {
    if interrupts::are_enabled() {
        return Err(UserEntryError::InterruptsEnabled);
    }
    let unsupported = Cr4Flags::L5_PAGING
        | Cr4Flags::PCID
        | Cr4Flags::PROTECTION_KEY_USER
        | Cr4Flags::PROTECTION_KEY_SUPERVISOR
        | Cr4Flags::CONTROL_FLOW_ENFORCEMENT;
    if Cr4::read().intersects(unsupported)
        || !Cr0::read().contains(Cr0Flags::WRITE_PROTECT)
        || !Efer::read().contains(EferFlags::NO_EXECUTE_ENABLE)
    {
        return Err(UserEntryError::UnsupportedCpuState);
    }
    Ok(())
}

unsafe fn finish(outcome: UserOutcome) -> ! {
    let state = local_state().expect("Unregistered context owner");
    unsafe {
        (*state).outcome = outcome;
        if (*state).preemptible {
            assert!(crate::stop_timer().is_ok(), "Cannot stop user timer");
        }
        super::tss::set_kernel_stack(VirtAddr::new((*state).kernel_entry_stack));
        let selectors = (*state).selectors;
        DS::set_reg(x86_64::structures::gdt::SegmentSelector(selectors[0]));
        ES::set_reg(x86_64::structures::gdt::SegmentSelector(selectors[1]));
        FS::set_reg(x86_64::structures::gdt::SegmentSelector(selectors[2]));
        GS::set_reg(x86_64::structures::gdt::SegmentSelector(selectors[3]));
        SS::set_reg(x86_64::structures::gdt::SegmentSelector(selectors[4]));
        FsBase::write(VirtAddr::new((*state).fs_base));
        GsBase::write(VirtAddr::new((*state).gs_base));
        if (*state).has_sysenter {
            Msr::new(0x174).write((*state).sysenter_cs);
        }
        kryo_leave_user(state.cast::<()>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_run_slots_are_distinct_and_cache_line_aligned() {
        for (index, state) in RUN_STATES.iter().enumerate() {
            let pointer = state.0.get() as usize;
            assert_eq!(pointer % 64, 0);
            for earlier in &RUN_STATES[..index] {
                assert_ne!(pointer, earlier.0.get() as usize);
            }
        }
    }

    #[test]
    fn user_addresses_and_initial_alignment_are_validated() {
        assert!(UserContext::new(0x400000, 0x800000).is_ok());
        for address in [0, 0x800, 0x0000_8000_0000_0000, 0xffff_8000_0000_0000] {
            assert_eq!(
                validate_addresses(address, 0x800000),
                Err(UserEntryError::InvalidAddress)
            );
            assert_eq!(
                validate_addresses(0x400000, address),
                Err(UserEntryError::InvalidAddress)
            );
        }
        assert_eq!(
            UserContext::new(0x400000, 0x800001),
            Err(UserEntryError::UnalignedStack)
        );
        assert!(validate_addresses(0x400000, 0x800008).is_ok());
    }

    #[test]
    fn flags_cannot_grant_privilege_or_override_preemption() {
        let blocked = (3 << 12) | (1 << 14) | (1 << 17) | (1 << 19) | (1 << 20);
        for preemptible in [false, true] {
            let flags = sanitize_flags(u64::MAX, preemptible);
            assert_eq!(flags & blocked, 0);
            assert_eq!(flags & (1 << 9) != 0, preemptible);
            assert_eq!(flags & USER_FLAGS, USER_FLAGS);
            assert_eq!(flags & !(USER_FLAGS | 2 | (1 << 9)), 0);
        }
    }

    #[test]
    fn bases_and_kernel_stacks_cannot_escape_their_address_ranges() {
        assert!(validate_bases(0, 0x1234).is_ok());
        assert_eq!(
            validate_bases(1 << 47, 0),
            Err(UserEntryError::InvalidAddress)
        );
        assert_eq!(
            validate_bases(0, 1 << 63),
            Err(UserEntryError::InvalidAddress)
        );
        assert!(validate_kernel_stack(0xffff_a000_0010_0000).is_ok());
        assert_eq!(
            validate_kernel_stack(0x800000),
            Err(UserEntryError::InvalidKernelStack)
        );
        assert_eq!(
            validate_kernel_stack(0xffff_a000_0010_0001),
            Err(UserEntryError::InvalidKernelStack)
        );
    }

    #[test]
    fn tls_update_checks_the_base_before_changing_saved_state() {
        let mut context = UserContext::new(0x400000, 0x800000).unwrap();
        assert_eq!(context.set_fs_base(0x900000), Ok(()));
        for base in [1 << 47, u64::MAX] {
            assert_eq!(
                context.set_fs_base(base),
                Err(UserEntryError::InvalidAddress)
            );
            assert_eq!(context.fs_base, 0x900000);
        }
        assert_eq!(context.set_fs_base(0), Ok(()));
        assert_eq!(context.fs_base, 0);
        assert_eq!(context.gs_base, 0);
    }

    #[test]
    fn assembly_register_layout_matches_the_saved_prefix() {
        assert_eq!(offset_of!(UserContext, rax), 0);
        assert_eq!(offset_of!(UserContext, r15), 14 * 8);
        assert_eq!(offset_of!(UserContext, rip), 15 * 8);
        assert_eq!(size_of::<UserContext>(), 20 * 8);
        assert_eq!(offset_of!(RunState, efer), 13 * 8);
        assert_eq!(
            offset_of!(RunState, selectors) - offset_of!(RunState, efer),
            8
        );
    }
}
