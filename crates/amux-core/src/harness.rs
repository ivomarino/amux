//! Durable contracts shared by checkpoints, handoffs, sensors and guide rules.

use crate::criteria::Criterion;
use crate::policy::AgentRole;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCheckpoint {
    pub task_id: String,
    pub version: u32,
    pub completed_steps: Vec<String>,
    pub next_action: String,
    pub artifacts: Vec<String>,
    pub unresolved: Vec<String>,
    pub input_hash: String,
    pub context_hash: Option<String>,
    pub last_result: Option<String>,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

impl TaskCheckpoint {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.task_id.trim().is_empty() {
            return Err("task_id is required");
        }
        if self.next_action.trim().is_empty() {
            return Err("next_action is required");
        }
        if self.input_hash.trim().is_empty() {
            return Err("input_hash is required");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffPacket {
    pub id: String,
    pub task_id: String,
    pub assignment_key: Option<String>,
    pub objective: String,
    pub criteria_version: u32,
    pub checkpoint_version: Option<u32>,
    pub checkpoint_hash: Option<String>,
    pub artifacts: Vec<String>,
    pub evidence: Vec<String>,
    pub assumptions: Vec<String>,
    pub unresolved: Vec<String>,
    #[serde(default)]
    pub concerns: Vec<String>,
    #[serde(default)]
    pub deviations: Vec<String>,
    #[serde(default)]
    pub findings: Vec<String>,
    #[serde(default)]
    pub requires_replan: bool,
    #[serde(default)]
    pub planning_scope_id: Option<String>,
    pub next_action: String,
    pub deadline: Option<String>,
    pub sender: String,
    pub receiver: String,
    pub created_at: DateTime<Utc>,
}

/// The durable interpretation of a user's goal.  The current row is replaced
/// as intent becomes clearer; every prior version remains in immutable history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalContract {
    pub id: String,
    pub objective: String,
    #[serde(default)]
    pub non_goals: Vec<String>,
    #[serde(default)]
    pub success_metrics: Vec<String>,
    #[serde(default)]
    pub performance_requirements: Vec<String>,
    #[serde(default)]
    pub resource_constraints: Vec<String>,
    pub dependency_policy: String,
    pub release_policy: String,
    pub expected_scope_min: u32,
    pub expected_scope_max: u32,
    pub version: u32,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

impl GoalContract {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.trim().is_empty() || self.objective.trim().is_empty() {
            return Err("goal id and objective are required");
        }
        if self.dependency_policy.trim().is_empty() || self.release_policy.trim().is_empty() {
            return Err("dependency_policy and release_policy are required");
        }
        if self.expected_scope_min == 0 || self.expected_scope_min > self.expected_scope_max {
            return Err("expected scope must be a non-zero ordered range");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningNodeStatus {
    Active,
    Waiting,
    NeedsReplan,
    Complete,
}

/// One recursively-owned slice of a goal.  This is deliberately not named
/// `Scope`: core's Scope is the configuration-inheritance hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningNode {
    pub id: String,
    pub goal_id: String,
    pub parent_id: Option<String>,
    pub task_id: Option<String>,
    pub role: AgentRole,
    pub owner: String,
    pub title: String,
    pub objective: String,
    pub status: PlanningNodeStatus,
    pub current_plan_version: u32,
    pub wake_count: u32,
    pub updated_at: DateTime<Utc>,
}

impl PlanningNode {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.trim().is_empty()
            || self.goal_id.trim().is_empty()
            || self.owner.trim().is_empty()
            || self.title.trim().is_empty()
            || self.objective.trim().is_empty()
        {
            return Err("planning node id, goal, owner, title and objective are required");
        }
        if self.parent_id.is_none() && self.role != AgentRole::RootPlanner {
            return Err("only a root planner may own a root planning node");
        }
        if self.parent_id.is_some() && self.role == AgentRole::RootPlanner {
            return Err("a child planning node cannot be a root planner");
        }
        Ok(())
    }
}

/// Replaceable plan projection. History is stored separately by the server;
/// callers always receive the exact version that became current.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanProjection {
    pub planning_node_id: String,
    pub version: u32,
    pub body: String,
    pub reason: String,
    pub authored_by: String,
    pub created_at: DateTime<Utc>,
}

impl PlanProjection {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.planning_node_id.trim().is_empty()
            || self.body.trim().is_empty()
            || self.reason.trim().is_empty()
            || self.authored_by.trim().is_empty()
        {
            return Err("planning node, plan body, reason and author are required");
        }
        Ok(())
    }
}

impl HandoffPacket {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.task_id.trim().is_empty() || self.objective.trim().is_empty() {
            return Err("task_id and objective are required");
        }
        if self.next_action.trim().is_empty() {
            return Err("next_action is required");
        }
        if self.sender.trim().is_empty() || self.receiver.trim().is_empty() {
            return Err("sender and receiver are required");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorProfile {
    pub task_type: String,
    pub criteria: Vec<Criterion>,
    pub version: u32,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuideRuleStatus {
    Candidate,
    Active,
    Retired,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementLayer {
    Guide,
    Sensor,
    Policy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuideRule {
    pub id: String,
    pub scope: String,
    pub name: String,
    pub content: String,
    pub owner: String,
    pub source_failure: String,
    pub rationale: String,
    pub status: GuideRuleStatus,
    pub enforcement_layer: EnforcementLayer,
    pub sensor_ref: Option<String>,
    pub version: u32,
    pub added_at: DateTime<Utc>,
    pub last_validated_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub superseded_by: Option<String>,
}

impl GuideRule {
    pub fn validate(&self, now: DateTime<Utc>) -> Vec<String> {
        let mut errors = Vec::new();
        if self.name.trim().is_empty() {
            errors.push("name is required".into());
        }
        if self.owner.trim().is_empty() {
            errors.push("owner is required".into());
        }
        if self.source_failure.trim().is_empty() {
            errors.push("source_failure is required".into());
        }
        if self.content.trim().is_empty() {
            errors.push("content is required".into());
        }
        if self.expires_at.is_some_and(|expiry| expiry <= now)
            && matches!(self.status, GuideRuleStatus::Active)
        {
            errors.push("active rule is expired".into());
        }
        if self.enforcement_layer == EnforcementLayer::Sensor && self.sensor_ref.is_none() {
            errors.push("sensor-enforced rule requires sensor_ref".into());
        }
        if self.status == GuideRuleStatus::Superseded && self.superseded_by.is_none() {
            errors.push("superseded rule requires superseded_by".into());
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_requires_a_walkable_next_action() {
        let checkpoint = TaskCheckpoint {
            task_id: "AMUX-1".into(),
            version: 1,
            completed_steps: vec![],
            next_action: "".into(),
            artifacts: vec![],
            unresolved: vec![],
            input_hash: "abc".into(),
            context_hash: None,
            last_result: None,
            updated_by: "worker".into(),
            updated_at: Utc::now(),
        };
        assert_eq!(checkpoint.validate(), Err("next_action is required"));
    }
}
