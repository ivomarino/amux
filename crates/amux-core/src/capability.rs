//! Capability policy replaces approval gates (Invariant 52, RR-0028g).
//!
//! No action gates on human approval. Instead, a typed policy loaded at
//! startup defines what the agent may do, at what rate, and under what
//! conditions — irreversible actions get a mandatory dry-run + evidence
//! trail instead of a waiting human. This is also how personal / cloud /
//! concierge deployments differ: one construct, two jobs
//! ([`DeploymentProfile`] selects the default policy table).
//!
//! **Single enforcement chokepoint** (Invariant 36: one source of truth):
//! every [`ActionClass`] invocation routes through
//! [`CapabilityPolicy::check`] before execution. It is one function, not
//! per-call-site checks — per-call-site policy is how two spend controls
//! ended up disagreeing in the plan's own first draft. The `#[must_use]`
//! verdict plus a lint pass for unguarded action sites enforce routing
//! structurally; an unrouted action site fails CI.
//!
//! Note there is deliberately NO `SpendMoney` variant: token spend is a
//! continuous metered resource governed by
//! [`FleetCircuitBreaker::window_budget_tokens`](crate::circuit::FleetCircuitBreaker)
//! (Invariant 48). A discrete action class for it would be a second spend
//! control — an Invariant 36 violation.

use crate::ids::WorkerId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Which deployment this policy is for. The profile selects the default
/// policy table at load time (server side); core records it so a policy
/// value is self-describing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentProfile {
    /// Local dev machine, single user.
    Personal,
    /// Multi-tenant, per-user isolation.
    Cloud,
    /// Managed service, external-facing.
    Concierge,
}

/// The canonical set of governable discrete actions (Invariant 52). Closed
/// on purpose: an action class the policy cannot name is an action the
/// policy cannot govern, so new side-effecting capabilities must land here
/// (and get a [`dry_run_definition`]) before any call site may perform them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionClass {
    GitPush,
    SendEmail,
    ExternalApiWrite,
    DeleteData,
    DatabaseMigration,
    StartWorker,
    StopWorker,
    InstallSoftware,
    NetworkRequest,
    FileWrite,
}

impl ActionClass {
    /// Every action class, for exhaustive iteration (policy validation,
    /// the dry-run coverage test). Keep in sync with the enum — the
    /// exhaustive `match` in [`ActionClass::name`] is what the compiler
    /// checks; this array is what tests iterate.
    pub const ALL: [ActionClass; 10] = [
        ActionClass::GitPush,
        ActionClass::SendEmail,
        ActionClass::ExternalApiWrite,
        ActionClass::DeleteData,
        ActionClass::DatabaseMigration,
        ActionClass::StartWorker,
        ActionClass::StopWorker,
        ActionClass::InstallSoftware,
        ActionClass::NetworkRequest,
        ActionClass::FileWrite,
    ];

    /// Stable snake_case name — the key in [`CapabilityPolicy::rules`] and
    /// the config file. Matches the serde representation so a policy file
    /// and a serialized policy use one spelling.
    pub fn name(self) -> &'static str {
        match self {
            ActionClass::GitPush => "git_push",
            ActionClass::SendEmail => "send_email",
            ActionClass::ExternalApiWrite => "external_api_write",
            ActionClass::DeleteData => "delete_data",
            ActionClass::DatabaseMigration => "database_migration",
            ActionClass::StartWorker => "start_worker",
            ActionClass::StopWorker => "stop_worker",
            ActionClass::InstallSoftware => "install_software",
            ActionClass::NetworkRequest => "network_request",
            ActionClass::FileWrite => "file_write",
        }
    }

    /// Reverse of [`ActionClass::name`]. `None` for an unknown name — the
    /// caller decides whether that is an error ([`CapabilityPolicy::from_pairs`]
    /// says yes: unknown policy keys are errors, never silently ignored,
    /// Invariant 37).
    pub fn from_name(name: &str) -> Option<ActionClass> {
        Self::ALL.into_iter().find(|a| a.name() == name)
    }
}

/// What the policy says about one action class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CapabilityConstraint {
    Allowed,
    RateLimited { per_hour: u32 },
    /// Mandatory dry-run + evidence step, then execute. The per-action
    /// meaning of "dry run" is [`dry_run_definition`].
    DryRunFirst,
    /// Execute in a sandbox, never production.
    SandboxOnly,
    /// Proceed only with evidence attached (e.g. a recorded dry-run or an
    /// independent verification, Invariant 28).
    RequiresEvidence,
    Denied,
}

impl std::str::FromStr for CapabilityConstraint {
    type Err = PolicyError;

    /// Config-file spelling: `allowed` | `rate_limited:<per_hour>` |
    /// `dry_run_first` | `sandbox_only` | `requires_evidence` | `denied`.
    fn from_str(s: &str) -> Result<Self, PolicyError> {
        match s.trim() {
            "allowed" => Ok(CapabilityConstraint::Allowed),
            "dry_run_first" => Ok(CapabilityConstraint::DryRunFirst),
            "sandbox_only" => Ok(CapabilityConstraint::SandboxOnly),
            "requires_evidence" => Ok(CapabilityConstraint::RequiresEvidence),
            "denied" => Ok(CapabilityConstraint::Denied),
            other => {
                if let Some(rate) = other.strip_prefix("rate_limited:") {
                    let per_hour = rate
                        .trim()
                        .parse::<u32>()
                        .map_err(|_| PolicyError::BadRate(other.to_string()))?;
                    Ok(CapabilityConstraint::RateLimited { per_hour })
                } else {
                    Err(PolicyError::UnknownConstraint(other.to_string()))
                }
            }
        }
    }
}

/// The chokepoint's answer. `#[must_use]`: dropping a verdict on the floor
/// is exactly the unrouted-action-site defect the chokepoint exists to
/// prevent, so ignoring one is a compiler warning CI turns into a failure.
#[must_use]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CapabilityVerdict {
    Proceed,
    /// Execute the dry-run variant first ([`dry_run_definition`]), record
    /// the evidence as a `DurableEvent`, then check again with the evidence
    /// attached.
    DryRunFirst,
    RateLimited { retry_after_secs: u64 },
    Denied { reason: String },
}

/// Context for one [`CapabilityPolicy::check`] call. Core is pure: the
/// caller supplies the window state (`recent_invocations`) and any recorded
/// dry-run evidence; core never counts or reads clocks itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionContext {
    pub worker: Option<WorkerId>,
    /// Reference to recorded dry-run/verification evidence (e.g. a
    /// `DurableEvent` id). Presence is what satisfies `DryRunFirst` and
    /// `RequiresEvidence`.
    pub dry_run_evidence: Option<String>,
    /// Invocations of this action class in the current one-hour window,
    /// counted by the store.
    pub recent_invocations: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// Unknown policy keys are errors, never silently ignored (Invariant
    /// 37) — a typo'd action name that silently parses is an ungoverned
    /// action.
    #[error("unknown action class {0:?} in capability policy")]
    UnknownAction(String),
    #[error("unknown capability constraint {0:?} (expected allowed | rate_limited:<per_hour> | dry_run_first | sandbox_only | requires_evidence | denied)")]
    UnknownConstraint(String),
    #[error("bad rate in constraint {0:?}: per_hour must be a u32")]
    BadRate(String),
}

/// The loaded policy: profile + per-action rules, keyed by
/// [`ActionClass::name`].
// TODO(RR-0028g): toml loading (capability-policy.toml) moves to amux-server
// config, which parses the file and feeds `from_pairs` — core stays
// dependency-light and never does I/O.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPolicy {
    pub profile: DeploymentProfile,
    pub rules: BTreeMap<String, CapabilityConstraint>,
}

impl CapabilityPolicy {
    /// Build a policy from flat `action_name -> constraint_spec` pairs (the
    /// shape a config file decodes to). Unknown action names and malformed
    /// constraints are ERRORS: a policy that silently drops a rule is a
    /// policy nobody can audit (Invariant 37, ethos rule 6).
    pub fn from_pairs(
        profile: DeploymentProfile,
        pairs: &BTreeMap<String, String>,
    ) -> Result<Self, PolicyError> {
        let mut rules = BTreeMap::new();
        for (name, spec) in pairs {
            let action = ActionClass::from_name(name)
                .ok_or_else(|| PolicyError::UnknownAction(name.clone()))?;
            rules.insert(action.name().to_string(), spec.parse()?);
        }
        Ok(CapabilityPolicy { profile, rules })
    }

    /// THE enforcement chokepoint (Invariant 52): every action site calls
    /// this before executing, and nothing else decides. Fail-closed: an
    /// action with no rule is `Denied` — an action the policy never named
    /// was never granted, and a fail-open default would make a missing
    /// config line an invisible capability grant.
    pub fn check(&self, action: ActionClass, ctx: &ActionContext) -> CapabilityVerdict {
        let Some(rule) = self.rules.get(action.name()) else {
            return CapabilityVerdict::Denied {
                reason: format!(
                    "no capability rule for {}; policy is fail-closed",
                    action.name()
                ),
            };
        };
        match rule {
            CapabilityConstraint::Allowed => CapabilityVerdict::Proceed,
            CapabilityConstraint::RateLimited { per_hour } => {
                if ctx.recent_invocations >= *per_hour {
                    // Core has no window clock; one full window is the
                    // conservative bound. The server, which owns the window
                    // timestamps, refines retry_after to the actual expiry.
                    CapabilityVerdict::RateLimited {
                        retry_after_secs: 3600,
                    }
                } else {
                    CapabilityVerdict::Proceed
                }
            }
            CapabilityConstraint::DryRunFirst => {
                if ctx.dry_run_evidence.is_some() {
                    CapabilityVerdict::Proceed
                } else {
                    CapabilityVerdict::DryRunFirst
                }
            }
            CapabilityConstraint::SandboxOnly => {
                // The production chokepoint denies outright: the verdict
                // vocabulary has no "sandbox" arm because production
                // execution is simply not granted. The sandbox executor
                // consults the constraint directly; the reason tells the
                // agent where the action IS allowed (ethos rule 3: an
                // honest exit must exist).
                CapabilityVerdict::Denied {
                    reason: format!(
                        "{} is sandbox_only: execute it in a sandbox, never production",
                        action.name()
                    ),
                }
            }
            CapabilityConstraint::RequiresEvidence => {
                if ctx.dry_run_evidence.is_some() {
                    CapabilityVerdict::Proceed
                } else {
                    CapabilityVerdict::Denied {
                        reason: format!(
                            "{} requires recorded evidence and none was attached",
                            action.name()
                        ),
                    }
                }
            }
            CapabilityConstraint::Denied => CapabilityVerdict::Denied {
                reason: format!("{} is denied by policy", action.name()),
            },
        }
    }
}

/// What "dry run" MEANS per action class (Invariant 52's table). Exhaustive
/// on purpose: an action class without a dry-run definition makes
/// `DryRunFirst` an unsatisfiable constraint for it — a gate with no honest
/// exit (ethos rule 3). The compiler enforces coverage; the test asserts
/// every definition is non-empty.
pub fn dry_run_definition(action: ActionClass) -> &'static str {
    match action {
        ActionClass::GitPush => "--dry-run",
        ActionClass::SendEmail => "render to DurableEvent",
        ActionClass::ExternalApiWrite => "log without HTTP",
        ActionClass::DeleteData => "list affected rows",
        ActionClass::DatabaseMigration => "in-memory DB clone",
        ActionClass::StartWorker => "validate config and resolve backend without spawning",
        ActionClass::StopWorker => "report which session would be terminated, without signaling it",
        ActionClass::InstallSoftware => "resolve package and version without installing",
        ActionClass::NetworkRequest => "log method, URL and body without sending",
        ActionClass::FileWrite => "diff the would-be content against the current file",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ActionContext {
        ActionContext {
            worker: Some(WorkerId::from_ulid(
                "01JGXV0000000000000000TEST".parse().unwrap(),
            )),
            dry_run_evidence: None,
            recent_invocations: 0,
        }
    }

    fn policy_with(action: ActionClass, constraint: CapabilityConstraint) -> CapabilityPolicy {
        let mut rules = BTreeMap::new();
        rules.insert(action.name().to_string(), constraint);
        CapabilityPolicy {
            profile: DeploymentProfile::Personal,
            rules,
        }
    }

    #[test]
    fn allowed_proceeds_and_denied_denies() {
        let p = policy_with(ActionClass::FileWrite, CapabilityConstraint::Allowed);
        assert_eq!(p.check(ActionClass::FileWrite, &ctx()), CapabilityVerdict::Proceed);

        let p = policy_with(ActionClass::DeleteData, CapabilityConstraint::Denied);
        assert!(matches!(
            p.check(ActionClass::DeleteData, &ctx()),
            CapabilityVerdict::Denied { .. }
        ));
    }

    #[test]
    fn unlisted_action_is_fail_closed() {
        let p = policy_with(ActionClass::FileWrite, CapabilityConstraint::Allowed);
        assert!(matches!(
            p.check(ActionClass::GitPush, &ctx()),
            CapabilityVerdict::Denied { .. }
        ));
    }

    #[test]
    fn dry_run_first_demands_then_accepts_evidence() {
        let p = policy_with(ActionClass::GitPush, CapabilityConstraint::DryRunFirst);
        assert_eq!(
            p.check(ActionClass::GitPush, &ctx()),
            CapabilityVerdict::DryRunFirst
        );

        let mut with_evidence = ctx();
        with_evidence.dry_run_evidence = Some("evt_01JGXV0000000000000000TEST".into());
        assert_eq!(
            p.check(ActionClass::GitPush, &with_evidence),
            CapabilityVerdict::Proceed
        );
    }

    #[test]
    fn requires_evidence_denies_without_and_proceeds_with() {
        let p = policy_with(ActionClass::SendEmail, CapabilityConstraint::RequiresEvidence);
        assert!(matches!(
            p.check(ActionClass::SendEmail, &ctx()),
            CapabilityVerdict::Denied { .. }
        ));

        let mut with_evidence = ctx();
        with_evidence.dry_run_evidence = Some("evt_01JGXV0000000000000000TEST".into());
        assert_eq!(
            p.check(ActionClass::SendEmail, &with_evidence),
            CapabilityVerdict::Proceed
        );
    }

    #[test]
    fn rate_limit_trips_at_cap_not_below() {
        let p = policy_with(
            ActionClass::NetworkRequest,
            CapabilityConstraint::RateLimited { per_hour: 5 },
        );
        let mut c = ctx();
        c.recent_invocations = 4;
        assert_eq!(
            p.check(ActionClass::NetworkRequest, &c),
            CapabilityVerdict::Proceed
        );
        c.recent_invocations = 5;
        assert_eq!(
            p.check(ActionClass::NetworkRequest, &c),
            CapabilityVerdict::RateLimited {
                retry_after_secs: 3600
            }
        );
    }

    #[test]
    fn sandbox_only_denies_production_with_instructive_reason() {
        let p = policy_with(
            ActionClass::DatabaseMigration,
            CapabilityConstraint::SandboxOnly,
        );
        match p.check(ActionClass::DatabaseMigration, &ctx()) {
            CapabilityVerdict::Denied { reason } => assert!(reason.contains("sandbox")),
            other => panic!("expected Denied, got {other:?}"),
        }
    }

    #[test]
    fn every_action_class_has_a_dry_run_definition() {
        // The match in dry_run_definition is compiler-exhaustive; this
        // asserts no definition is an empty placeholder, and pins the five
        // definitions Invariant 52 specifies verbatim.
        for action in ActionClass::ALL {
            assert!(
                !dry_run_definition(action).is_empty(),
                "empty dry-run definition for {}",
                action.name()
            );
        }
        assert_eq!(dry_run_definition(ActionClass::GitPush), "--dry-run");
        assert_eq!(
            dry_run_definition(ActionClass::SendEmail),
            "render to DurableEvent"
        );
        assert_eq!(
            dry_run_definition(ActionClass::ExternalApiWrite),
            "log without HTTP"
        );
        assert_eq!(
            dry_run_definition(ActionClass::DeleteData),
            "list affected rows"
        );
        assert_eq!(
            dry_run_definition(ActionClass::DatabaseMigration),
            "in-memory DB clone"
        );
    }

    #[test]
    fn action_names_round_trip() {
        for action in ActionClass::ALL {
            assert_eq!(ActionClass::from_name(action.name()), Some(action));
        }
        assert_eq!(ActionClass::from_name("spend_money"), None);
    }

    #[test]
    fn from_pairs_parses_valid_policy() {
        let mut pairs = BTreeMap::new();
        pairs.insert("git_push".to_string(), "dry_run_first".to_string());
        pairs.insert("send_email".to_string(), "rate_limited:3".to_string());
        pairs.insert("delete_data".to_string(), "denied".to_string());

        let p = CapabilityPolicy::from_pairs(DeploymentProfile::Cloud, &pairs).unwrap();
        assert_eq!(p.profile, DeploymentProfile::Cloud);
        assert_eq!(
            p.rules["git_push"],
            CapabilityConstraint::DryRunFirst
        );
        assert_eq!(
            p.rules["send_email"],
            CapabilityConstraint::RateLimited { per_hour: 3 }
        );
        assert_eq!(p.rules["delete_data"], CapabilityConstraint::Denied);
    }

    #[test]
    fn from_pairs_rejects_unknown_action_and_bad_constraint() {
        let mut pairs = BTreeMap::new();
        pairs.insert("spend_money".to_string(), "allowed".to_string());
        let err = CapabilityPolicy::from_pairs(DeploymentProfile::Personal, &pairs).unwrap_err();
        assert!(matches!(err, PolicyError::UnknownAction(_)));

        let mut pairs = BTreeMap::new();
        pairs.insert("git_push".to_string(), "maybe".to_string());
        let err = CapabilityPolicy::from_pairs(DeploymentProfile::Personal, &pairs).unwrap_err();
        assert!(matches!(err, PolicyError::UnknownConstraint(_)));

        let mut pairs = BTreeMap::new();
        pairs.insert("git_push".to_string(), "rate_limited:lots".to_string());
        let err = CapabilityPolicy::from_pairs(DeploymentProfile::Personal, &pairs).unwrap_err();
        assert!(matches!(err, PolicyError::BadRate(_)));
    }

    #[test]
    fn verdict_serde_round_trip() {
        let v = CapabilityVerdict::RateLimited {
            retry_after_secs: 3600,
        };
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"kind":"rate_limited","retry_after_secs":3600}"#);
        let back: CapabilityVerdict = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }
}
