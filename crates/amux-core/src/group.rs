//! Groups as first-class structural boundaries (Invariant 12, RR-0016).
//!
//! Groups replace the Python system's tag-based isolation. A tag is a label;
//! a label cannot own configuration, so tag-scoped env, gates, and columns
//! each grew their own ad-hoc lookup and drifted. A [`Group`] is an entity:
//! it OWNS its config, its board columns, its gates, and its members.
//! Membership is a foreign key relationship (`Vec<WorkerId>` here, a real FK
//! in the store), not a string a typo can silently fork.
//!
//! Scope isolation and config inheritance both flow through the ONE resolver
//! in [`crate::scope`] (Invariant 2): [`GroupConfig`] implements
//! [`Mergeable`] so `effective_config` layers Org -> Global -> Group ->
//! Worker uniformly, and [`Group::target_for`] turns membership into the
//! [`ResolutionTarget`] the resolver reads — visibility is decided in
//! exactly one place.

use crate::ids::{GateId, GroupId, WorkerId};
use crate::scope::{Mergeable, ResolutionTarget};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Group-scoped configuration, layered via Invariant 2. Every field's merge
/// rule is field-presence: a layer that says nothing about a field leaves
/// the lower layer's answer standing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupConfig {
    /// Environment variables. Merged PER KEY (like
    /// [`crate::scope::LayeredMap`]): a group overriding `API_URL` does not
    /// wipe the global `PATH`.
    pub environment: BTreeMap<String, String>,
    /// Board column names — groups may define their own board shape
    /// (Invariant 12). Merged as a WHOLE field: a column set is one coherent
    /// board layout, so a group either inherits the full lower-layer layout
    /// (`None`) or replaces it entirely (`Some`); per-item merging would
    /// produce a board nobody designed.
    pub columns: Option<Vec<String>>,
    /// Board column gates for this group (Invariant 18: gates are first-class
    /// entities; config references them by ID). A non-empty list REPLACES the
    /// lower layer's list — a gate set is a policy unit. Known limitation of
    /// presence-on-Vec: an empty list reads as "not set", so a group cannot
    /// express "no gates" over a global gate set; that needs `Option` at the
    /// API layer if it is ever required.
    pub gates: Vec<GateId>,
    /// Automation behavior knobs (auto-pickup, nudges, ...). Merged PER KEY,
    /// same reasoning as `environment`.
    pub automation: BTreeMap<String, String>,
}

impl Mergeable for GroupConfig {
    /// Apply `other` (the more specific layer) on top of `self`. Field
    /// presence wins; see each field's doc for what "presence" means there.
    fn merge(&mut self, other: &Self) {
        for (k, v) in &other.environment {
            self.environment.insert(k.clone(), v.clone());
        }
        if other.columns.is_some() {
            self.columns = other.columns.clone();
        }
        if !other.gates.is_empty() {
            self.gates = other.gates.clone();
        }
        for (k, v) in &other.automation {
            self.automation.insert(k.clone(), v.clone());
        }
    }
}

/// A first-class group (Invariant 12). Every worker belongs to exactly one
/// group (or the implicit global group); `members` is the authoritative
/// membership list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    /// Overrides global config via the Invariant 2 chain.
    pub config: GroupConfig,
    pub members: Vec<WorkerId>,
    /// Entity version (Invariant 35). Bumped ONLY on actual state change —
    /// see [`Group::add_member`] / [`Group::remove_member`].
    pub version: u64,
}

impl Group {
    /// Add a worker. Idempotent: adding an existing member is a no-op that
    /// returns `false` and does NOT bump `version` (Invariant 37: revision
    /// increments iff authoritative state changed; a no-op that bumps makes
    /// "did it change?" unreliable for every consumer).
    pub fn add_member(&mut self, worker: WorkerId) -> bool {
        if self.members.contains(&worker) {
            return false;
        }
        self.members.push(worker);
        self.version += 1;
        true
    }

    /// Remove a worker. Idempotent: removing a non-member returns `false`
    /// without bumping `version` (Invariant 37, as above).
    pub fn remove_member(&mut self, worker: &WorkerId) -> bool {
        let before = self.members.len();
        self.members.retain(|w| w != worker);
        if self.members.len() == before {
            return false;
        }
        self.version += 1;
        true
    }

    pub fn is_member(&self, worker: &WorkerId) -> bool {
        self.members.contains(worker)
    }

    /// The [`ResolutionTarget`] for `worker` relative to this group: the
    /// group is present iff the worker is a member. This is where membership
    /// becomes scope isolation — group-scoped values (config, memories,
    /// gates) apply to members and ONLY members, decided by the one resolver
    /// (Invariant 2) rather than a per-subsystem membership check that can
    /// drift.
    pub fn target_for(&self, worker: &WorkerId) -> ResolutionTarget {
        ResolutionTarget {
            worker: Some(worker.clone()),
            group: self.is_member(worker).then(|| self.id.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::{effective_config, Scope};

    fn group_id() -> GroupId {
        GroupId::from_ulid("01JGXV0000000000000000TEST".parse().unwrap())
    }

    fn worker(ulid: &str) -> WorkerId {
        WorkerId::from_ulid(ulid.parse().unwrap())
    }

    fn gate(ulid: &str) -> GateId {
        GateId::from_ulid(ulid.parse().unwrap())
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn empty_group() -> Group {
        Group {
            id: group_id(),
            name: "backend".into(),
            config: GroupConfig::default(),
            members: vec![],
            version: 1,
        }
    }

    #[test]
    fn membership_is_idempotent_and_noop_does_not_bump_version() {
        let mut g = empty_group();
        let a = worker("01JGXV0000000000000000AAAA");

        assert!(g.add_member(a.clone()));
        assert_eq!(g.version, 2);

        // Re-adding: no-op, no version bump, no duplicate row (Invariant 37).
        assert!(!g.add_member(a.clone()));
        assert_eq!(g.version, 2);
        assert_eq!(g.members.len(), 1);

        assert!(g.remove_member(&a));
        assert_eq!(g.version, 3);

        // Removing a non-member: no-op, no bump.
        assert!(!g.remove_member(&a));
        assert_eq!(g.version, 3);
    }

    #[test]
    fn scope_isolation_via_membership() {
        let mut g = empty_group();
        let member = worker("01JGXV0000000000000000AAAA");
        let outsider = worker("01JGXV0000000000000000BBBB");
        g.add_member(member.clone());

        let group_scope = Scope::Group { id: g.id.clone() };
        // Group-scoped values reach members through the one resolver...
        assert!(group_scope.applies_to(&g.target_for(&member)));
        // ...and never reach non-members.
        assert!(!group_scope.applies_to(&g.target_for(&outsider)));
    }

    #[test]
    fn config_inheritance_merge() {
        let g1 = gate("01JGXV0000000000000000AAAA");
        let global = GroupConfig {
            environment: map(&[("API_URL", "https://global"), ("PATH", "/usr/bin")]),
            columns: Some(vec!["todo".into(), "doing".into(), "done".into()]),
            gates: vec![g1.clone()],
            automation: map(&[("auto_pickup", "on")]),
        };
        let group = GroupConfig {
            environment: map(&[("API_URL", "https://group")]),
            columns: None,
            gates: vec![],
            automation: BTreeMap::new(),
        };

        let eff = effective_config(None, Some(&global), Some(&group), None);
        // Env merges per key: the override wins, the untouched key survives.
        assert_eq!(eff.environment["API_URL"], "https://group");
        assert_eq!(eff.environment["PATH"], "/usr/bin");
        // Absent fields inherit whole from the lower layer.
        assert_eq!(eff.columns.as_ref().unwrap().len(), 3);
        assert_eq!(eff.gates, vec![g1]);
        assert_eq!(eff.automation["auto_pickup"], "on");
    }

    #[test]
    fn config_columns_replace_wholesale() {
        let global = GroupConfig {
            columns: Some(vec!["todo".into(), "doing".into(), "done".into()]),
            ..GroupConfig::default()
        };
        let group = GroupConfig {
            columns: Some(vec!["inbox".into(), "shipped".into()]),
            ..GroupConfig::default()
        };
        let eff = effective_config(None, Some(&global), Some(&group), None);
        // A present column set replaces the whole layout, never merges items.
        assert_eq!(
            eff.columns.unwrap(),
            vec!["inbox".to_string(), "shipped".to_string()]
        );
    }

    #[test]
    fn serde_round_trip() {
        let mut g = empty_group();
        g.add_member(worker("01JGXV0000000000000000AAAA"));
        g.config.environment.insert("K".into(), "v".into());
        let json = serde_json::to_string(&g).unwrap();
        let back: Group = serde_json::from_str(&json).unwrap();
        assert_eq!(g, back);
    }
}
