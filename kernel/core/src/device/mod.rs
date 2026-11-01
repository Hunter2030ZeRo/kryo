use crate::process::ProcessId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisterWindow {
    pub offset: u64,
    pub length: u64,
    pub readable: bool,
    pub writable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceState {
    Active,
    Quiescing,
    Retired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceError {
    InvalidWindow,
    OverlappingWindows,
    InvalidAccess,
    PermissionDenied,
    Revoked,
    NotQuiescing,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RevokeError<E> {
    State(DeviceError),
    Cleanup(E),
}

/// Policy for one process's mediated, 32-bit device-register accesses.
///
/// Windows describe offsets, not physical authority. The image must retain its
/// trusted device mapping and check handle rights before calling validate.
/// Keep this policy serialized with hardware access and revocation. Object
/// references supply identity and generation; copying an address grants nothing.
pub struct DeviceAuthority<const WINDOWS: usize> {
    owner: ProcessId,
    windows: [RegisterWindow; WINDOWS],
    state: DeviceState,
}

impl<const WINDOWS: usize> DeviceAuthority<WINDOWS> {
    pub fn new(owner: ProcessId, windows: [RegisterWindow; WINDOWS]) -> Result<Self, DeviceError> {
        if WINDOWS == 0 {
            return Err(DeviceError::InvalidWindow);
        }
        for (index, window) in windows.iter().enumerate() {
            let end = window
                .offset
                .checked_add(window.length)
                .ok_or(DeviceError::InvalidWindow)?;
            if window.length == 0
                || window.offset % 4 != 0
                || window.length % 4 != 0
                || (!window.readable && !window.writable)
            {
                return Err(DeviceError::InvalidWindow);
            }
            if windows[..index]
                .iter()
                .any(|other| window.offset < other.offset + other.length && other.offset < end)
            {
                return Err(DeviceError::OverlappingWindows);
            }
        }
        Ok(Self {
            owner,
            windows,
            state: DeviceState::Active,
        })
    }

    pub const fn owner(&self) -> ProcessId {
        self.owner
    }

    pub const fn state(&self) -> DeviceState {
        self.state
    }

    pub fn validate(
        &self,
        caller: ProcessId,
        offset: u64,
        bytes: u64,
        write: bool,
    ) -> Result<(), DeviceError> {
        if caller != self.owner {
            return Err(DeviceError::PermissionDenied);
        }
        if self.state != DeviceState::Active {
            return Err(DeviceError::Revoked);
        }
        if bytes != 4 || offset % 4 != 0 {
            return Err(DeviceError::InvalidAccess);
        }
        let end = offset
            .checked_add(bytes)
            .ok_or(DeviceError::InvalidAccess)?;
        let window = self
            .windows
            .iter()
            .find(|window| offset >= window.offset && end <= window.offset + window.length)
            .ok_or(DeviceError::InvalidAccess)?;
        if if write {
            window.writable
        } else {
            window.readable
        } {
            Ok(())
        } else {
            Err(DeviceError::PermissionDenied)
        }
    }

    /// Stops new mediated accesses before the image starts hardware cleanup.
    pub fn begin_revoke(&mut self) -> bool {
        if self.state == DeviceState::Active {
            self.state = DeviceState::Quiescing;
            true
        } else {
            false
        }
    }

    /// Records cleanup success; failure leaves access revoked for retry.
    ///
    /// The supplied operation must disable the device path and retire pending
    /// IRQ delivery before reporting success. This policy cannot prove hardware
    /// quiescence and never releases mappings, DMA pins, or other resource owners.
    /// Keep those owners retained until their explicit cleanup succeeds.
    pub fn finish_revoke<E>(
        &mut self,
        cleanup: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), RevokeError<E>> {
        match self.state {
            DeviceState::Active => Err(RevokeError::State(DeviceError::NotQuiescing)),
            DeviceState::Retired => Ok(()),
            DeviceState::Quiescing => {
                cleanup().map_err(RevokeError::Cleanup)?;
                self.state = DeviceState::Retired;
                Ok(())
            }
        }
    }
}
