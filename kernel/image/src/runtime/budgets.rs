use kryo_core::resource::{Budget, Resource};
use kryo_interfaces::syscall::Error;

pub(crate) fn query(
    account: &Budget,
    index: u64,
    query: u64,
    space: Option<(usize, usize)>,
) -> Result<u64, Error> {
    if query > 1 {
        return Err(Error::InvalidArgument);
    }
    let resource = match index {
        0 => Some(Resource::SharedPages),
        1 => Some(Resource::Handles),
        2 => Some(Resource::Waits),
        3 => Some(Resource::Threads),
        6 => Some(Resource::PrivatePages),
        7 => Some(Resource::ReservedPages),
        _ => None,
    };
    if let Some(resource) = resource {
        return Ok(if query == 0 {
            account.limit(resource)
        } else {
            account.count(resource)
        } as u64);
    }
    match (index, query) {
        (4, 0) => Ok(account.limits().mappings as u64),
        (5, 0) => Ok(account.limits().tables as u64),
        (4, 1) => space
            .map(|counts| counts.0 as u64)
            .ok_or(Error::InvalidArgument),
        (5, 1) => space
            .map(|counts| counts.1 as u64)
            .ok_or(Error::InvalidArgument),
        _ => Err(Error::InvalidArgument),
    }
}

#[cfg(feature = "boot-tests")]
pub(crate) mod checks;
