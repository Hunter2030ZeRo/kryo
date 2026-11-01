use super::*;
use kryo_libsys::{CONTROL, CREATE, DUP, RECEIVE};

fn id(value: u64) -> ServiceId {
    ServiceId::new(value).unwrap()
}

fn handle(value: u64) -> Handle {
    Handle::from_raw(value).unwrap()
}

fn endpoint_info() -> HandleInfo {
    HandleInfo {
        kind: ObjectKind::Endpoint,
        rights: SEND | TRANSFER,
    }
}

fn factory_grant(rights: u64) -> StartupGrant {
    StartupGrant {
        role: role::FACTORY,
        kind: ObjectKind::ChannelFactory,
        rights,
        handle: handle(7),
    }
}

#[test]
fn manifest_requires_named_roles_and_rejects_unlisted_authority() {
    let roles = [RoleRule {
        role: role::FACTORY,
        kind: ObjectKind::ChannelFactory,
        rights: CREATE,
        required: true,
    }];
    let manifest = Manifest {
        service: id(10),
        roles: &roles,
    };
    assert_eq!(manifest.validate_grants(&[factory_grant(CREATE)]), Ok(()));
    assert_eq!(manifest.validate_grants(&[]), Err(PolicyError::MissingRole));
    assert_eq!(
        manifest.validate_grants(&[factory_grant(CREATE | TRANSFER)]),
        Err(PolicyError::DeniedRights)
    );
    assert_eq!(
        manifest.validate_grants(&[factory_grant(CREATE), factory_grant(CREATE)]),
        Err(PolicyError::DuplicateRole)
    );
    let mut grant = factory_grant(CREATE);
    grant.role = role::DEBUG;
    assert_eq!(
        manifest.validate_grants(&[grant]),
        Err(PolicyError::UnknownRole)
    );
    grant.role = role::FACTORY;
    grant.kind = ObjectKind::Endpoint;
    assert_eq!(
        manifest.validate_grants(&[grant]),
        Err(PolicyError::WrongKind)
    );
}

#[test]
fn manifests_allow_only_explicit_transitive_rights() {
    let mut rules = [RoleRule {
        role: role::REGISTRATION,
        kind: ObjectKind::Endpoint,
        rights: SEND,
        required: true,
    }];
    let grant = StartupGrant {
        role: role::REGISTRATION,
        kind: ObjectKind::Endpoint,
        rights: SEND | TRANSFER,
        handle: handle(1),
    };
    assert_eq!(
        Manifest {
            service: id(10),
            roles: &rules
        }
        .validate_grants(&[grant]),
        Err(PolicyError::DeniedRights)
    );
    rules[0].rights |= TRANSFER;
    assert_eq!(
        Manifest {
            service: id(10),
            roles: &rules
        }
        .validate_grants(&[grant]),
        Ok(())
    );
}

#[test]
fn malformed_policy_fails_closed() {
    let rule = RoleRule {
        role: role::FACTORY,
        kind: ObjectKind::ChannelFactory,
        rights: CREATE,
        required: false,
    };
    let duplicates = [rule, rule];
    assert_eq!(
        Manifest {
            service: id(10),
            roles: &duplicates
        }
        .validate_grants(&[]),
        Err(PolicyError::InvalidPolicy)
    );
    let invalid_kind_rights = [RoleRule {
        rights: RECEIVE,
        ..rule
    }];
    assert_eq!(
        Manifest {
            service: id(10),
            roles: &invalid_kind_rights
        }
        .validate_grants(&[]),
        Err(PolicyError::InvalidPolicy)
    );
    let excessive = [rule; MAX_GRANTS + 1];
    assert_eq!(
        Manifest {
            service: id(10),
            roles: &excessive
        }
        .validate_grants(&[]),
        Err(PolicyError::InvalidPolicy)
    );
}

#[test]
fn failed_start_consumes_generation_and_stale_registration_cannot_publish() {
    let mut records = [Record::new(id(10))];
    let mut registry = Registry::new(&mut records, &[]).unwrap();
    let first = registry.begin(id(10)).unwrap();
    assert_eq!(first.generation, 1);
    assert_eq!(registry.state(id(10)), Ok(State::Starting));
    registry.failed(first).unwrap();
    let second = registry.begin(id(10)).unwrap();
    assert_eq!(second.generation, 2);
    assert_eq!(
        registry.register(second, first, handle(1), endpoint_info()),
        Err(PolicyError::StaleInstance)
    );
    assert_eq!(registry.state(id(10)), Ok(State::Starting));
    registry
        .register(second, second, handle(2), endpoint_info())
        .unwrap();
    assert_eq!(registry.state(id(10)), Ok(State::Ready));
    assert_eq!(registry.begin(id(10)), Err(PolicyError::InvalidState));
}

#[test]
fn registration_is_bound_to_expected_service_and_endpoint_authority() {
    let mut records = [Record::new(id(10)), Record::new(id(11))];
    let mut registry = Registry::new(&mut records, &[]).unwrap();
    let instance = registry.begin(id(10)).unwrap();
    let forged = Instance::new(id(11), instance.generation).unwrap();
    assert_eq!(
        registry.register(instance, forged, handle(1), endpoint_info()),
        Err(PolicyError::StaleInstance)
    );
    for info in [
        HandleInfo {
            kind: ObjectKind::ChannelFactory,
            rights: CREATE,
        },
        HandleInfo {
            kind: ObjectKind::Endpoint,
            rights: SEND,
        },
        HandleInfo {
            kind: ObjectKind::Endpoint,
            rights: SEND | TRANSFER | CONTROL,
        },
    ] {
        assert_eq!(
            registry.register(instance, instance, handle(1), info),
            Err(PolicyError::InvalidEndpoint)
        );
        assert_eq!(registry.state(id(10)), Ok(State::Starting));
    }
}

#[test]
fn gateway_binding_controls_lookup_and_never_grants_transitive_authority() {
    let mut records = [
        Record::new(id(10)),
        Record::new(id(20)),
        Record::new(id(21)),
    ];
    let access = [Access {
        client: id(20),
        target: id(10),
        rights: SEND,
    }];
    let mut registry = Registry::new(&mut records, &access).unwrap();
    let allowed = registry.bind_gateway(id(20)).unwrap();
    let denied = registry.bind_gateway(id(21)).unwrap();
    assert_eq!(
        registry.lookup(&allowed, id(10), SEND),
        Err(PolicyError::NotReady)
    );
    let instance = registry.begin(id(10)).unwrap();
    registry
        .register(instance, instance, handle(1), endpoint_info())
        .unwrap();
    assert_eq!(
        registry.lookup(&allowed, id(10), SEND),
        Ok(Binding {
            instance,
            endpoint: handle(1),
            rights: SEND
        })
    );
    assert_eq!(
        registry.lookup(&denied, id(10), SEND),
        Err(PolicyError::AccessDenied)
    );
    for requested in [0, SEND | TRANSFER, SEND | CONTROL, SEND | DUP] {
        assert_eq!(
            registry.lookup(&allowed, id(10), requested),
            Err(PolicyError::AccessDenied)
        );
    }
    // The packet's claimed client is not an input to this authorization API.
    let claimed = id(20);
    assert_ne!(denied.client, claimed);
    assert_eq!(
        registry.lookup(&denied, id(10), SEND),
        Err(PolicyError::AccessDenied)
    );
}

#[test]
fn stopping_withdraws_lookup_before_replacement_and_keeps_old_snapshot() {
    let mut records = [Record::new(id(10)), Record::new(id(20))];
    let access = [Access {
        client: id(20),
        target: id(10),
        rights: SEND,
    }];
    let mut registry = Registry::new(&mut records, &access).unwrap();
    let gateway = registry.bind_gateway(id(20)).unwrap();
    let first = registry.begin(id(10)).unwrap();
    registry
        .register(first, first, handle(1), endpoint_info())
        .unwrap();
    let old = registry.lookup(&gateway, id(10), SEND).unwrap();
    assert_eq!(registry.stopping(first), Ok(Some(handle(1))));
    assert_eq!(
        registry.lookup(&gateway, id(10), SEND),
        Err(PolicyError::NotReady)
    );
    assert_eq!(registry.begin(id(10)), Err(PolicyError::InvalidState));
    registry.stopped(first).unwrap();
    let second = registry.begin(id(10)).unwrap();
    registry
        .register(second, second, handle(2), endpoint_info())
        .unwrap();
    assert_eq!(
        registry.lookup(&gateway, id(10), SEND).unwrap().instance,
        second
    );
    assert_eq!(old.instance, first);
    assert_eq!(old.endpoint, handle(1));
    assert_eq!(registry.failed(first), Err(PolicyError::StaleInstance));
}

#[test]
fn generation_never_wraps_and_registry_reconstruction_keeps_history() {
    let mut records = [Record::new(id(10))];
    records[0].generation = u64::MAX - 1;
    {
        let mut registry = Registry::new(&mut records, &[]).unwrap();
        let last = registry.begin(id(10)).unwrap();
        assert_eq!(last.generation, u64::MAX);
        registry.failed(last).unwrap();
    }
    let mut registry = Registry::new(&mut records, &[]).unwrap();
    assert_eq!(
        registry.begin(id(10)),
        Err(PolicyError::GenerationExhausted)
    );
    assert_eq!(registry.state(id(10)), Ok(State::Failed));
}

#[test]
fn identifiers_and_invalid_inventory_do_not_create_authority() {
    assert_eq!(ServiceId::new(0), Err(PolicyError::InvalidId));
    assert_eq!(
        Instance::new(id(10), 0),
        Err(PolicyError::InvalidGeneration)
    );
    let mut duplicate = [Record::new(id(10)), Record::new(id(10))];
    assert!(matches!(
        Registry::new(&mut duplicate, &[]),
        Err(PolicyError::DuplicateService)
    ));
    let mut records = [Record::new(id(10))];
    let invalid_access = [Access {
        client: id(20),
        target: id(10),
        rights: SEND,
    }];
    assert!(matches!(
        Registry::new(&mut records, &invalid_access),
        Err(PolicyError::InvalidPolicy)
    ));
    let mut registry = Registry::new(&mut records, &[]).unwrap();
    assert!(matches!(
        registry.bind_gateway(id(20)),
        Err(PolicyError::UnknownService)
    ));
    assert_eq!(registry.begin(id(20)), Err(PolicyError::UnknownService));
}

#[test]
fn protocol_rejects_unknown_versions_and_operations() {
    assert_eq!(
        protocol::opcode([protocol::header(protocol::LOOKUP), 0, 0, 0]),
        Ok(protocol::LOOKUP)
    );
    assert_eq!(
        protocol::opcode([protocol::header(protocol::ECHO_LATE_REPLY), 0, 0, 0]),
        Ok(protocol::ECHO_LATE_REPLY)
    );
    assert_eq!(
        protocol::opcode([protocol::header(protocol::LATE_ACK), 0, 0, 0]),
        Ok(protocol::LATE_ACK)
    );
    assert_eq!(
        protocol::opcode([protocol::header(protocol::LATE_RESUME), 0, 0, 0]),
        Ok(protocol::LATE_RESUME)
    );
    assert_eq!(
        protocol::opcode([protocol::LOOKUP, 0, 0, 0]),
        Err(PolicyError::InvalidProtocol)
    );
    assert_eq!(
        protocol::opcode([protocol::header(99), 0, 0, 0]),
        Err(PolicyError::InvalidProtocol)
    );
}
