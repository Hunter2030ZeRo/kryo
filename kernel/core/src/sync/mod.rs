pub mod wait;
mod word;

pub use wait::{ParkResult, WaitError, WaitId, WaitTable};
pub use word::{WordWaitError, WordWaiters};
