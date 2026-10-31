#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Runnable,
    Running,
    Blocked,
    Dying,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReturnReason {
    Preempted,
    Yielded,
    Call,
    Fault,
    Exit,
}
