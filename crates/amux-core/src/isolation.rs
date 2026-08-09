//! Worker isolation policy and what it does to EVIDENCE (Invariants 33 +
//! 10 + 49, RR-0028k).
//!
//! Isolation is scoped configuration like everything else: the effective
//! policy for a worker resolves through the one four-tier resolver in
//! [`crate::scope`] (Invariant 2, Org -> Global -> Group -> Worker), never a
//! per-subsystem lookup. Changing it is a config change (Invariant 43), not
//! a new worker.
//!
//! **The Shared-isolation evidence rule** (the reason this module exists as
//! more than an enum): under [`IsolationPolicy::Shared`], workers share one
//! checkout, so a `cargo test` exit code is evidence about the TREE, not
//! about any individual worker's change ([`EvidenceScope::TreeLevel`]).
//! Three consequences, each owned by a sibling module:
//!
//! - An [`AttemptRecord`](crate::limits::AttemptRecord) under `Shared` must
//!   populate `tree_status` (the `git status` snapshot at failure time) —
//!   otherwise failure-feeds-forward (Invariant 49) is actively misleading,
//!   because attempt N+1 cannot distinguish "my code broke" from "the tree
//!   was dirty when I started". This repo has lived that exact incident:
//!   a peer's `git pull --rebase` replayed another session's unpushed
//!   commit (2026-08-03).
//! - Acceptance criteria needing per-worker attribution ("THIS worker's
//!   change did not break the build") must declare `Worktree` — under
//!   `Shared` such a criterion is unverifiable, and an unverifiable
//!   criterion is a gate with no honest exit (ethos rule 3).
//! - A dirty tree or merge conflict under `Shared` is
//!   `WaitingFor::TreeConflict { holder, path }` — a structured wait with a
//!   named holder (Invariant 10), not a stall to "fix".

use serde::{Deserialize, Serialize};

/// How a worker's filesystem is isolated from its peers (Invariant 33).
/// Plain variants: the parameters (worktree base path, container image) are
/// deployment configuration resolved at the scope layer, not identity
/// carried on every value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationPolicy {
    /// Workers share a single checkout. The default (Invariant 33) — and
    /// the mode with the evidence caveat documented at module level.
    #[default]
    Shared,
    /// A git worktree per worker: tree-level evidence becomes worker-level.
    Worktree,
    /// A container per worker (future): strongest isolation, same evidence
    /// semantics as `Worktree`.
    Container,
}

/// Who a piece of tree-derived evidence (test exit code, lint result, build
/// status) is ABOUT. The discriminator that keeps Shared-mode verification
/// honest: recording tree-level evidence as worker-level is how a passing
/// build gets attributed to the change that broke it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceScope {
    /// Evidence about the shared tree — every worker in it, collectively.
    TreeLevel,
    /// Evidence attributable to one worker's own change.
    WorkerLevel,
}

/// What scope of evidence a tree-derived check yields under `policy`.
/// Verification (Invariant 28) consults this before treating an exit code
/// as proof of an individual worker's claim.
pub fn evidence_scope(policy: IsolationPolicy) -> EvidenceScope {
    match policy {
        IsolationPolicy::Shared => EvidenceScope::TreeLevel,
        IsolationPolicy::Worktree | IsolationPolicy::Container => EvidenceScope::WorkerLevel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_yields_tree_level_evidence() {
        assert_eq!(evidence_scope(IsolationPolicy::Shared), EvidenceScope::TreeLevel);
    }

    #[test]
    fn worktree_and_container_yield_worker_level_evidence() {
        assert_eq!(
            evidence_scope(IsolationPolicy::Worktree),
            EvidenceScope::WorkerLevel
        );
        assert_eq!(
            evidence_scope(IsolationPolicy::Container),
            EvidenceScope::WorkerLevel
        );
    }

    #[test]
    fn shared_is_the_default() {
        // Invariant 33: default Shared.
        assert_eq!(IsolationPolicy::default(), IsolationPolicy::Shared);
    }

    #[test]
    fn serde_round_trip_snake_case() {
        for policy in [
            IsolationPolicy::Shared,
            IsolationPolicy::Worktree,
            IsolationPolicy::Container,
        ] {
            let json = serde_json::to_string(&policy).unwrap();
            let back: IsolationPolicy = serde_json::from_str(&json).unwrap();
            assert_eq!(policy, back);
        }
        assert_eq!(
            serde_json::to_string(&IsolationPolicy::Worktree).unwrap(),
            r#""worktree""#
        );
        assert_eq!(
            serde_json::to_string(&EvidenceScope::TreeLevel).unwrap(),
            r#""tree_level""#
        );
        let back: EvidenceScope = serde_json::from_str(r#""worker_level""#).unwrap();
        assert_eq!(back, EvidenceScope::WorkerLevel);
    }
}
