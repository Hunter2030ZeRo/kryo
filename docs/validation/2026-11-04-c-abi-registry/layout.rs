// Compile-only Rust host views, independent of the C layout fixtures.
#![allow(dead_code)]
use core::ffi::c_void;
use core::mem::{align_of, offset_of, size_of};

#[repr(C)]
struct Timespec {
    tv_sec: i64,
    tv_nsec: i64,
}

#[repr(C)]
struct Timeval {
    tv_sec: i64,
    tv_usec: i64,
}

#[repr(C)]
struct Itimerspec {
    it_interval: Timespec,
    it_value: Timespec,
}

#[repr(C)]
struct Itimerval {
    it_interval: Timeval,
    it_value: Timeval,
}

#[repr(C)]
struct Stat {
    st_dev: u64,
    st_ino: u64,
    st_nlink: u64,
    st_mode: u32,
    st_uid: u32,
    st_gid: u32,
    reserved0: u32,
    st_rdev: u64,
    st_size: i64,
    st_blksize: i64,
    st_blocks: i64,
    st_atim: Timespec,
    st_mtim: Timespec,
    st_ctim: Timespec,
    reserved1: u64,
}

#[repr(C)]
struct Flock {
    l_type: i16,
    l_whence: i16,
    reserved0: u32,
    l_start: i64,
    l_len: i64,
    l_pid: i32,
    reserved1: u32,
}

#[repr(C)]
struct Pollfd {
    fd: i32,
    events: i16,
    revents: i16,
}

#[repr(C)]
struct Sigset {
    bits: [u64; 1],
}

#[repr(C)]
union Sigval {
    sival_int: i32,
    sival_ptr: *mut c_void,
}

#[repr(C)]
struct Siginfo {
    si_signo: i32,
    si_code: i32,
    si_errno: i32,
    si_pid: i32,
    si_uid: u32,
    si_status: i32,
    si_addr: *mut c_void,
    si_band: i64,
    si_value: Sigval,
}

#[repr(C)]
union Handler {
    one: Option<unsafe extern "C" fn(i32)>,
    three: Option<unsafe extern "C" fn(i32, *mut Siginfo, *mut c_void)>,
}

#[repr(C)]
struct Sigaction {
    handler: Handler,
    sa_mask: Sigset,
    sa_flags: i32,
    reserved: u32,
}

#[repr(C)]
struct SignalStack {
    ss_sp: *mut c_void,
    ss_size: usize,
    ss_flags: i32,
    reserved: u32,
}

#[repr(C)]
struct Sigevent {
    sigev_notify: i32,
    sigev_signo: i32,
    sigev_value: Sigval,
    sigev_notify_function: Option<unsafe extern "C" fn(Sigval)>,
    sigev_notify_attributes: *mut c_void,
}

#[repr(C)]
struct Mcontext {
    version: u32,
    size: u32,
    gregs: [u64; 18],
    fs_base: u64,
    gs_base: u64,
    xstate: *const c_void,
    xstate_bytes: u32,
    xstate_format: u32,
    xstate_features: u64,
    fault_address: u64,
    fault_error: u64,
    trap: u32,
    reserved: u32,
}

#[repr(C)]
struct Ucontext {
    uc_flags: u64,
    uc_link: *mut Ucontext,
    uc_sigmask: Sigset,
    uc_stack: SignalStack,
    uc_mcontext: Mcontext,
}

macro_rules! layout {
    ($ty:ty, $size:expr, $align:expr, $($field:ident: $offset:expr),* $(,)?) => {
        const _: () = {
            assert!(size_of::<$ty>() == $size);
            assert!(align_of::<$ty>() == $align);
            $(assert!(offset_of!($ty, $field) == $offset);)*
        };
    };
}

layout!(Timespec, 16, 8, tv_sec: 0, tv_nsec: 8);
layout!(Timeval, 16, 8, tv_sec: 0, tv_usec: 8);
layout!(Itimerspec, 32, 8, it_interval: 0, it_value: 16);
layout!(Itimerval, 32, 8, it_interval: 0, it_value: 16);
layout!(Stat, 128, 8,
    st_dev: 0, st_ino: 8, st_nlink: 16, st_mode: 24, st_uid: 28,
    st_gid: 32, reserved0: 36, st_rdev: 40, st_size: 48, st_blksize: 56,
    st_blocks: 64, st_atim: 72, st_mtim: 88, st_ctim: 104, reserved1: 120);
layout!(Flock, 32, 8, l_type: 0, l_whence: 2, reserved0: 4,
    l_start: 8, l_len: 16, l_pid: 24, reserved1: 28);
layout!(Pollfd, 8, 4, fd: 0, events: 4, revents: 6);
layout!(Sigset, 8, 8, bits: 0);
layout!(Sigval, 8, 8, sival_int: 0, sival_ptr: 0);
layout!(Siginfo, 48, 8, si_signo: 0, si_code: 4, si_errno: 8, si_pid: 12,
    si_uid: 16, si_status: 20, si_addr: 24, si_band: 32, si_value: 40);
layout!(Sigaction, 24, 8, handler: 0, sa_mask: 8, sa_flags: 16, reserved: 20);
layout!(SignalStack, 24, 8, ss_sp: 0, ss_size: 8, ss_flags: 16, reserved: 20);
layout!(Sigevent, 32, 8, sigev_notify: 0, sigev_signo: 4, sigev_value: 8,
    sigev_notify_function: 16, sigev_notify_attributes: 24);
layout!(Mcontext, 216, 8, version: 0, size: 4, gregs: 8, fs_base: 152,
    gs_base: 160, xstate: 168, xstate_bytes: 176, xstate_format: 180,
    xstate_features: 184, fault_address: 192, fault_error: 200,
    trap: 208, reserved: 212);
layout!(Ucontext, 264, 8, uc_flags: 0, uc_link: 8, uc_sigmask: 16,
    uc_stack: 24, uc_mcontext: 48);
