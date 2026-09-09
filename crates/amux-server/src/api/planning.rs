//! Versioned goal intent and recursive planning ownership.
//!
//! This is the semantic planning plane above the deterministic dispatcher.
//! It produces and revises board-shaped work; it does not assign workers or
//! execute code.

use super::AppState;
use amux_core::harness::{GoalContract, PlanProjection, PlanningNode, PlanningNodeStatus};
use amux_core::policy::AgentRole;
use amux_core::revision::{EntityType, MutationKind};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/goals", axum::routing::post(create_goal))
        .route("/goals/{id}", axum::routing::get(get_goal).put(update_goal))
        .route(
            "/goals/{id}/nodes",
            axum::routing::post(create_node).get(list_nodes),
        )
        .route("/planning-nodes/{id}", axum::routing::get(get_node))
        .route(
            "/planning-nodes/{id}/plan",
            axum::routing::get(get_plan).put(put_plan),
        )
}

fn actor(headers: &HeaderMap) -> String {
    headers
        .get("x-amux-session")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or("api-anonymous")
        .to_string()
}

fn internal(error: impl std::fmt::Display) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": error.to_string()})),
    )
        .into_response()
}

fn forbidden(code: &str, error: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({"error": error, "code": code})),
    )
        .into_response()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalInput {
    id: Option<String>,
    objective: String,
    #[serde(default)]
    non_goals: Vec<String>,
    #[serde(default)]
    success_metrics: Vec<String>,
    #[serde(default)]
    performance_requirements: Vec<String>,
    #[serde(default)]
    resource_constraints: Vec<String>,
    dependency_policy: String,
    release_policy: String,
    expected_scope_min: u32,
    expected_scope_max: u32,
    root_owner: Option<String>,
    root_title: Option<String>,
}

fn contract(id: String, body: &GoalInput, updated_by: String) -> GoalContract {
    GoalContract {
        id,
        objective: body.objective.clone(),
        non_goals: body.non_goals.clone(),
        success_metrics: body.success_metrics.clone(),
        performance_requirements: body.performance_requirements.clone(),
        resource_constraints: body.resource_constraints.clone(),
        dependency_policy: body.dependency_policy.clone(),
        release_policy: body.release_policy.clone(),
        expected_scope_min: body.expected_scope_min,
        expected_scope_max: body.expected_scope_max,
        version: 0,
        updated_by,
        updated_at: Utc::now(),
    }
}

async fn create_goal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<GoalInput>,
) -> Response {
    let caller = actor(&headers);
    if caller == "api-anonymous" {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "goal creation requires an identified planning session",
                "code": "planning_identity_required"
            })),
        )
            .into_response();
    }
    if body
        .root_owner
        .as_deref()
        .is_some_and(|owner| owner != caller)
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "a root planner may assign only its own verified session identity",
                "code": "root_owner_mismatch",
                "caller": caller,
                "requested_owner": body.root_owner
            })),
        )
            .into_response();
    }
    let id = body
        .id
        .clone()
        .unwrap_or_else(|| format!("goal_{}", ulid::Ulid::new().to_string().to_lowercase()));
    let goal = contract(id.clone(), &body, caller.clone());
    if let Err(error) = goal.validate() {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": error}))).into_response();
    }
    let root = PlanningNode {
        id: format!("plan_{}", ulid::Ulid::new().to_string().to_lowercase()),
        goal_id: id.clone(),
        parent_id: None,
        task_id: None,
        role: AgentRole::RootPlanner,
        owner: caller,
        title: body
            .root_title
            .clone()
            .unwrap_or_else(|| "Root plan".to_string()),
        objective: body.objective.clone(),
        status: PlanningNodeStatus::Active,
        current_plan_version: 0,
        wake_count: 0,
        updated_at: Utc::now(),
    };
    if let Err(error) = root.validate() {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": error}))).into_response();
    }
    let result = std::sync::Arc::new(std::sync::Mutex::new(None));
    let result_w = result.clone();
    let write = state
        .store
        .write_async(move |conn| {
            let stored = crate::db::harness_store::put_goal_contract(conn, &goal)?;
            crate::db::harness_store::insert_planning_node(conn, &root)?;
            *result_w.lock().expect("goal result") = Some((stored.clone(), root.clone()));
            Ok(crate::db::WriteOutcome {
                applied: true,
                events: vec![
                    crate::db::PendingEvent {
                        entity_type: EntityType::Other("goal_contract".into()),
                        entity_id: stored.id.clone(),
                        mutation: MutationKind::Created,
                        payload: serde_json::to_value(&stored).ok(),
                    },
                    crate::db::PendingEvent {
                        entity_type: EntityType::Other("planning_node".into()),
                        entity_id: root.id.clone(),
                        mutation: MutationKind::Created,
                        payload: serde_json::to_value(&root).ok(),
                    },
                ],
            })
        })
        .await;
    match write {
        Ok(_) => match result.lock().expect("goal result").clone() {
            Some((goal, root)) => (
                StatusCode::CREATED,
                Json(json!({"goal": goal, "root": root})),
            )
                .into_response(),
            None => internal("goal write produced no result"),
        },
        Err(error) => internal(error),
    }
}

async fn update_goal(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<GoalInput>,
) -> Response {
    let caller = actor(&headers);
    let owner =
        match state.store.read().and_then(|conn| {
            crate::db::harness_store::goal_root_owner(&conn, &id).map_err(Into::into)
        }) {
            Ok(owner) => owner,
            Err(error) => return internal(error),
        };
    match owner.as_deref() {
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "no goal", "id": id})),
            )
                .into_response()
        }
        Some(owner) if owner != caller => {
            return forbidden(
                "goal_owner_required",
                "only the owning root planner may revise this goal",
            )
        }
        Some(_) => {}
    }
    let goal = contract(id.clone(), &body, caller.clone());
    if let Err(error) = goal.validate() {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": error}))).into_response();
    }
    let result = std::sync::Arc::new(std::sync::Mutex::new(None));
    let result_w = result.clone();
    match state
        .store
        .write_async(move |conn| {
            crate::db::harness_store::ensure_goal_root_owner(conn, &id, &caller)?;
            let stored = crate::db::harness_store::put_goal_contract(conn, &goal)?;
            *result_w.lock().expect("goal result") = Some(stored.clone());
            Ok(crate::db::WriteOutcome {
                applied: true,
                events: vec![crate::db::PendingEvent {
                    entity_type: EntityType::Other("goal_contract".into()),
                    entity_id: id,
                    mutation: MutationKind::Updated,
                    payload: serde_json::to_value(&stored).ok(),
                }],
            })
        })
        .await
    {
        Ok(_) => match result.lock().expect("goal result").clone() {
            Some(goal) => Json(goal).into_response(),
            None => internal("goal write produced no result"),
        },
        Err(error) => internal(error),
    }
}

async fn get_goal(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let conn = match state.store.read() {
        Ok(conn) => conn,
        Err(error) => return internal(error),
    };
    match crate::db::harness_store::get_goal_contract(&conn, &id) {
        Ok(Some(goal)) => {
            let history = match crate::db::harness_store::goal_contract_history(&conn, &id) {
                Ok(history) => history,
                Err(error) => return internal(error),
            };
            Json(json!({
                "measured": true,
                "n_considered": history.len(),
                "current": goal,
                "history": history
            }))
            .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no goal", "id": id})),
        )
            .into_response(),
        Err(error) => internal(error),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeInput {
    parent_id: String,
    task_id: Option<String>,
    role: AgentRole,
    owner: String,
    title: String,
    objective: String,
}

async fn create_node(
    State(state): State<AppState>,
    Path(goal_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<NodeInput>,
) -> Response {
    let caller = actor(&headers);
    let parent = match state.store.read().and_then(|conn| {
        crate::db::harness_store::get_planning_node(&conn, &body.parent_id).map_err(Into::into)
    }) {
        Ok(parent) => parent,
        Err(error) => return internal(error),
    };
    match parent {
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "planning node parent does not exist"})),
            )
                .into_response()
        }
        Some(parent) if parent.owner != caller || !parent.role.is_planner() => {
            return forbidden(
                "parent_planner_owner_required",
                "only the owning parent planner may delegate this node",
            )
        }
        Some(_) => {}
    }
    let existing_role = match state.store.read().and_then(|conn| {
        crate::db::harness_store::planning_role_for_actor(&conn, &body.owner).map_err(Into::into)
    }) {
        Ok(role) => role,
        Err(error) => return internal(error),
    };
    if existing_role.is_some_and(|role| role.is_planner() != body.role.is_planner()) {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "an active planning identity cannot also own execution or verification work",
                "code": "planning_identity_role_conflict",
                "owner": body.owner
            })),
        )
            .into_response();
    }
    let node = PlanningNode {
        id: format!("plan_{}", ulid::Ulid::new().to_string().to_lowercase()),
        goal_id,
        parent_id: Some(body.parent_id),
        task_id: body.task_id,
        role: body.role,
        owner: body.owner,
        title: body.title,
        objective: body.objective,
        status: PlanningNodeStatus::Active,
        current_plan_version: 0,
        wake_count: 0,
        updated_at: Utc::now(),
    };
    if let Err(error) = node.validate() {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": error}))).into_response();
    }
    let copy = node.clone();
    match state
        .store
        .write_async(move |conn| {
            let inserted =
                crate::db::harness_store::insert_delegated_planning_node(conn, &copy, &caller)?;
            Ok(crate::db::WriteOutcome {
                applied: inserted,
                events: inserted
                    .then(|| crate::db::PendingEvent {
                        entity_type: EntityType::Other("planning_node".into()),
                        entity_id: copy.id.clone(),
                        mutation: MutationKind::Created,
                        payload: serde_json::to_value(&copy).ok(),
                    })
                    .into_iter()
                    .collect(),
            })
        })
        .await
    {
        Ok(_) => (StatusCode::CREATED, Json(node)).into_response(),
        Err(error) => internal(error),
    }
}

async fn list_nodes(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let conn = match state.store.read() {
        Ok(conn) => conn,
        Err(error) => return internal(error),
    };
    match crate::db::harness_store::list_planning_nodes(&conn, &id) {
        Ok(nodes) => Json(json!({
            "measured": true,
            "n_considered": nodes.len(),
            "items": nodes
        }))
        .into_response(),
        Err(error) => internal(error),
    }
}

async fn get_node(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let conn = match state.store.read() {
        Ok(conn) => conn,
        Err(error) => return internal(error),
    };
    match crate::db::harness_store::get_planning_node(&conn, &id) {
        Ok(Some(node)) => Json(node).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no planning node"})),
        )
            .into_response(),
        Err(error) => internal(error),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanInput {
    body: String,
    reason: String,
}

async fn put_plan(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<PlanInput>,
) -> Response {
    let caller = actor(&headers);
    let node = match state.store.read().and_then(|conn| {
        crate::db::harness_store::get_planning_node(&conn, &id).map_err(Into::into)
    }) {
        Ok(node) => node,
        Err(error) => return internal(error),
    };
    match node {
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "no planning node"})),
            )
                .into_response()
        }
        Some(node) if node.owner != caller || !node.role.is_planner() => {
            return forbidden(
                "planning_node_owner_required",
                "only the owning planner may revise this plan",
            )
        }
        Some(_) => {}
    }
    let plan = PlanProjection {
        planning_node_id: id.clone(),
        version: 0,
        body: body.body,
        reason: body.reason,
        authored_by: caller,
        created_at: Utc::now(),
    };
    if let Err(error) = plan.validate() {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": error}))).into_response();
    }
    let result = std::sync::Arc::new(std::sync::Mutex::new(None));
    let result_w = result.clone();
    match state
        .store
        .write_async(move |conn| {
            let stored = crate::db::harness_store::put_plan(conn, &plan)?;
            *result_w.lock().expect("plan result") = Some(stored.clone());
            Ok(crate::db::WriteOutcome {
                applied: true,
                events: vec![crate::db::PendingEvent {
                    entity_type: EntityType::Other("plan".into()),
                    entity_id: id,
                    mutation: MutationKind::Updated,
                    payload: serde_json::to_value(&stored).ok(),
                }],
            })
        })
        .await
    {
        Ok(_) => match result.lock().expect("plan result").clone() {
            Some(plan) => Json(plan).into_response(),
            None => internal("plan write produced no result"),
        },
        Err(error) => internal(error),
    }
}

async fn get_plan(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let conn = match state.store.read() {
        Ok(conn) => conn,
        Err(error) => return internal(error),
    };
    let current = match crate::db::harness_store::current_plan(&conn, &id) {
        Ok(current) => current,
        Err(error) => return internal(error),
    };
    let history = match crate::db::harness_store::plan_history(&conn, &id) {
        Ok(history) => history,
        Err(error) => return internal(error),
    };
    Json(json!({
        "measured": true,
        "n_considered": history.len(),
        "current": current,
        "history": history
    }))
    .into_response()
}
