//! amux-core: the system's vocabulary. Shared types, scope resolution, board
//! state machine, command/event protocol. Pure logic — no I/O, no async, no
//! database. Everything here is testable with plain `cargo test`.
//!
//! Module ↔ RR-item map (docs/rust-rebuild-plan.md §Execution Checklist):
//! - `scope`         RR-0002 (Invariant 2)
//! - `session`       RR-0004 (Invariants 1, 33, 8)
//! - `protocol`      RR-0006 (Invariants 5, 34)
//! - `provider`      RR-0007 (Invariants 8, 20)
//! - `revision`      RR-0008 (Invariants 35, 37)

pub mod ids;
pub mod protocol;
pub mod provider;
pub mod revision;
pub mod scope;
pub mod session;
