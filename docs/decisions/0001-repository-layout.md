# ADR 0001: Layered repository layout

Date: 2026-10-30

Status: Accepted

## Context

The repository needs stable locations for kernel mechanisms, hardware support, user-space services, and compatibility adapters. Directory creation must not force speculative Rust interfaces or ABI definitions.

## Decision

Use `boot/`, `hal/`, `interfaces/`, `kernel/`, `userspace/`, `compat/`, `tests/`, `tools/`, and `docs/`.

Maintain four Cargo packages: HAL, interfaces, kernel core, and kernel image. Add user-space packages when their execution environment is defined.

General drivers, VFS, filesystems, and networking belong in Ring 3 services. Core must not depend on those implementations, ICE, or BetAFS. BetAFS remains an external storage engine with an adapter under user space.

## Consequences

Reserved directories use `.gitkeep`. Add Rust module declarations only when a module has an implementation.

Features should fit under these boundaries. A change to the top-level layout requires a new decision describing its rationale and dependency changes.
