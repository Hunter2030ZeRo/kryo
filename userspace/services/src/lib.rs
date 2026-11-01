#![no_std]

//! Startup policy and service discovery for a configured user-space inventory.

use kryo_interfaces::startup::{MAX_GRANTS, ObjectKind};
use kryo_libsys::{Handle, HandleInfo, SEND, Startup, StartupGrant, TRANSFER};

pub mod role {
    pub const REGISTRATION: u64 = 1;
    pub const FACTORY: u64 = 2;
    pub const DEBUG: u64 = 3;
    pub const DIRECTORY: u64 = 4;
    pub const PEER_IMAGE: u64 = 5;
    pub const CLIENT_IMAGE: u64 = 6;
    pub const SERVER_FACTORY: u64 = 7;
    pub const ALLOWED_FACTORY: u64 = 8;
    pub const DENIED_FACTORY: u64 = 9;
    pub const CHECKPOINT: u64 = 10;
}

pub mod protocol {
    use super::PolicyError;

    pub const VERSION: u64 = 1;
    pub const REGISTER: u64 = 1;
    pub const LOOKUP: u64 = 2;
    pub const FOUND: u64 = 3;
    pub const DENIED: u64 = 4;
    pub const ECHO: u64 = 5;
    pub const ECHO_REPLY: u64 = 6;
    pub const RESTART: u64 = 7;
    pub const RESTARTED: u64 = 8;
    pub const QUOTA_HELD: u64 = 9;
    pub const QUOTA_RELEASE: u64 = 10;
    pub const ECHO_LATE_REPLY: u64 = 11;
    pub const LATE_ACK: u64 = 12;
    pub const LATE_RESUME: u64 = 13;

    pub const fn header(opcode: u64) -> u64 {
        (VERSION << 32) | opcode
    }

    pub fn opcode(message: [u64; 4]) -> Result<u64, PolicyError> {
        let opcode = message[0] as u32 as u64;
        if message[0] >> 32 != VERSION || !(REGISTER..=LATE_RESUME).contains(&opcode) {
            return Err(PolicyError::InvalidProtocol);
        }
        Ok(opcode)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceId(u64);

impl ServiceId {
    pub const fn new(value: u64) -> Result<Self, PolicyError> {
        if value == 0 {
            Err(PolicyError::InvalidId)
        } else {
            Ok(Self(value))
        }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instance {
    pub service: ServiceId,
    pub generation: u64,
}

impl Instance {
    pub const fn new(service: ServiceId, generation: u64) -> Result<Self, PolicyError> {
        if generation == 0 {
            Err(PolicyError::InvalidGeneration)
        } else {
            Ok(Self {
                service,
                generation,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyError {
    InvalidId,
    InvalidGeneration,
    GenerationExhausted,
    UnknownService,
    DuplicateService,
    InvalidPolicy,
    UnknownRole,
    DuplicateRole,
    MissingRole,
    WrongKind,
    DeniedRights,
    InvalidState,
    StaleInstance,
    InvalidEndpoint,
    AccessDenied,
    NotReady,
    InvalidProtocol,
}

#[derive(Clone, Copy, Debug)]
pub struct RoleRule {
    pub role: u64,
    pub kind: ObjectKind,
    /// Maximum accepted rights; a grant may carry a smaller nonempty subset.
    pub rights: u64,
    pub required: bool,
}

pub struct Manifest<'policy> {
    pub service: ServiceId,
    pub roles: &'policy [RoleRule],
}

impl Manifest<'_> {
    pub fn validate(&self, startup: &Startup) -> Result<Instance, PolicyError> {
        if startup.service != self.service.get() {
            return Err(PolicyError::UnknownService);
        }
        self.validate_entries(startup.grants())?;
        Instance::new(self.service, startup.instance)
    }

    pub fn validate_grants(&self, grants: &[StartupGrant]) -> Result<(), PolicyError> {
        self.validate_entries(grants.iter().copied())
    }

    fn validate_entries(
        &self,
        grants: impl IntoIterator<Item = StartupGrant>,
    ) -> Result<(), PolicyError> {
        if self.roles.len() > MAX_GRANTS {
            return Err(PolicyError::InvalidPolicy);
        }
        for (index, rule) in self.roles.iter().enumerate() {
            if rule.role == 0
                || rule.rights == 0
                || rule.rights & !rule.kind.allowed_rights() != 0
                || self.roles[..index].iter().any(|old| old.role == rule.role)
            {
                return Err(PolicyError::InvalidPolicy);
            }
        }
        let mut present = [false; MAX_GRANTS];
        for grant in grants {
            let index = self
                .roles
                .iter()
                .position(|rule| rule.role == grant.role)
                .ok_or(PolicyError::UnknownRole)?;
            let rule = self.roles[index];
            if present[index] {
                return Err(PolicyError::DuplicateRole);
            }
            if grant.kind != rule.kind {
                return Err(PolicyError::WrongKind);
            }
            if grant.rights == 0 || grant.rights & !rule.rights != 0 {
                return Err(PolicyError::DeniedRights);
            }
            present[index] = true;
        }
        if self
            .roles
            .iter()
            .enumerate()
            .any(|(index, rule)| rule.required && !present[index])
        {
            return Err(PolicyError::MissingRole);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Stopped,
    Starting,
    Ready,
    Stopping,
    Failed,
}

pub struct Record {
    service: ServiceId,
    generation: u64,
    state: State,
    endpoint: Option<Handle>,
}

impl Record {
    pub const fn new(service: ServiceId) -> Self {
        Self {
            service,
            generation: 0,
            state: State::Stopped,
            endpoint: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Access {
    pub client: ServiceId,
    pub target: ServiceId,
    pub rights: u64,
}

/// Bound by the supervisor to an endpoint it owns, never from message fields.
pub struct Gateway {
    client: ServiceId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub instance: Instance,
    pub endpoint: Handle,
    pub rights: u64,
}

pub struct Registry<'records, 'policy> {
    records: &'records mut [Record],
    access: &'policy [Access],
}

#[cfg(test)]
mod tests;

impl<'records, 'policy> Registry<'records, 'policy> {
    pub fn new(
        records: &'records mut [Record],
        access: &'policy [Access],
    ) -> Result<Self, PolicyError> {
        for (index, record) in records.iter().enumerate() {
            if records[..index]
                .iter()
                .any(|old| old.service == record.service)
            {
                return Err(PolicyError::DuplicateService);
            }
        }
        for (index, rule) in access.iter().enumerate() {
            if rule.rights != SEND
                || !records.iter().any(|record| record.service == rule.client)
                || !records.iter().any(|record| record.service == rule.target)
                || access[..index]
                    .iter()
                    .any(|old| old.client == rule.client && old.target == rule.target)
            {
                return Err(PolicyError::InvalidPolicy);
            }
        }
        Ok(Self { records, access })
    }

    pub fn begin(&mut self, service: ServiceId) -> Result<Instance, PolicyError> {
        let record = self.record_mut(service)?;
        if !matches!(record.state, State::Stopped | State::Failed) {
            return Err(PolicyError::InvalidState);
        }
        let generation = record
            .generation
            .checked_add(1)
            .ok_or(PolicyError::GenerationExhausted)?;
        record.generation = generation;
        record.state = State::Starting;
        record.endpoint = None;
        Instance::new(service, generation)
    }

    pub fn register(
        &mut self,
        expected: Instance,
        claimed: Instance,
        endpoint: Handle,
        info: HandleInfo,
    ) -> Result<(), PolicyError> {
        if claimed != expected {
            return Err(PolicyError::StaleInstance);
        }
        let record = self.current_mut(expected)?;
        if record.state != State::Starting {
            return Err(PolicyError::InvalidState);
        }
        if info.kind != ObjectKind::Endpoint || info.rights != (SEND | TRANSFER) {
            return Err(PolicyError::InvalidEndpoint);
        }
        record.endpoint = Some(endpoint);
        record.state = State::Ready;
        Ok(())
    }

    pub fn bind_gateway(&self, client: ServiceId) -> Result<Gateway, PolicyError> {
        self.record(client)?;
        Ok(Gateway { client })
    }

    pub fn lookup(
        &self,
        gateway: &Gateway,
        target: ServiceId,
        requested: u64,
    ) -> Result<Binding, PolicyError> {
        if requested != SEND
            || !self.access.iter().any(|rule| {
                rule.client == gateway.client
                    && rule.target == target
                    && requested & !rule.rights == 0
            })
        {
            return Err(PolicyError::AccessDenied);
        }
        let record = self.record(target)?;
        if record.state != State::Ready {
            return Err(PolicyError::NotReady);
        }
        Ok(Binding {
            instance: Instance::new(target, record.generation)?,
            endpoint: record.endpoint.ok_or(PolicyError::NotReady)?,
            rights: requested,
        })
    }

    pub fn stopping(&mut self, instance: Instance) -> Result<Option<Handle>, PolicyError> {
        let record = self.current_mut(instance)?;
        if !matches!(record.state, State::Starting | State::Ready) {
            return Err(PolicyError::InvalidState);
        }
        record.state = State::Stopping;
        Ok(record.endpoint.take())
    }

    pub fn stopped(&mut self, instance: Instance) -> Result<(), PolicyError> {
        let record = self.current_mut(instance)?;
        if record.state != State::Stopping {
            return Err(PolicyError::InvalidState);
        }
        record.state = State::Stopped;
        Ok(())
    }

    pub fn failed(&mut self, instance: Instance) -> Result<Option<Handle>, PolicyError> {
        let record = self.current_mut(instance)?;
        if !matches!(
            record.state,
            State::Starting | State::Ready | State::Stopping
        ) {
            return Err(PolicyError::InvalidState);
        }
        record.state = State::Failed;
        Ok(record.endpoint.take())
    }

    pub fn state(&self, service: ServiceId) -> Result<State, PolicyError> {
        Ok(self.record(service)?.state)
    }

    fn record(&self, service: ServiceId) -> Result<&Record, PolicyError> {
        self.records
            .iter()
            .find(|record| record.service == service)
            .ok_or(PolicyError::UnknownService)
    }

    fn record_mut(&mut self, service: ServiceId) -> Result<&mut Record, PolicyError> {
        self.records
            .iter_mut()
            .find(|record| record.service == service)
            .ok_or(PolicyError::UnknownService)
    }

    fn current_mut(&mut self, instance: Instance) -> Result<&mut Record, PolicyError> {
        let record = self.record_mut(instance.service)?;
        if instance.generation == 0 || record.generation != instance.generation {
            return Err(PolicyError::StaleInstance);
        }
        Ok(record)
    }
}
