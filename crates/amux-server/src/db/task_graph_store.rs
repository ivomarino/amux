//! A rebuildable projection of existing primitives, never a second task store.
//! The snapshot owns a read transaction to bind revision, nodes, edges and digest
//! to one SQLite snapshot. Nothing here infers relationships from task prose.

use amux_core::task_graph::{self, Adjacency, DagAnalysis};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Task,
    Worker,
    Artifact,
    Message,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    DependsOn,
    PartOf,
    AssignedTo,
    Produces,
    RecordedMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub key: String,
    pub attributes: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub table: String,
    pub record_id: String,
    pub field: String,
    pub revision: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub relation: Relation,
    pub to: String,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub code: String,
    pub task_ids: Vec<String>,
    pub field: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verification {
    pub valid: bool,
    pub findings: Vec<Finding>,
    pub dependencies: DagAnalysis,
    pub lineage: DagAnalysis,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub revision: u64,
    pub sha256: String,
    pub measured: bool,
    /// All non-deleted tasks, including archived tasks and terminal history.
    pub n_considered: usize,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub verification: Verification,
}

fn finding(findings: &mut Vec<Finding>, code: &str, ids: Vec<String>, field: &str) {
    findings.push(Finding {
        code: code.into(),
        task_ids: ids,
        field: field.into(),
    });
}
fn edge(
    from: &str,
    relation: Relation,
    to: String,
    table: &str,
    id: &str,
    field: &str,
    revision: Option<i64>,
) -> Edge {
    Edge {
        from: from.into(),
        relation,
        to,
        provenance: Provenance {
            table: table.into(),
            record_id: id.into(),
            field: field.into(),
            revision,
        },
    }
}

pub fn snapshot(conn: &Connection) -> anyhow::Result<Snapshot> {
    let tx = conn.unchecked_transaction()?;
    let revision = tx.query_row("SELECT rev FROM _amux_rev WHERE id=1", [], |r| r.get(0))?;
    let tasks =
        super::board_store::list_issues(&tx, &[], &[], super::board_store::ArchivedFilter::All)?;
    let ids: BTreeSet<String> = tasks.iter().map(|t| t.id.clone()).collect();
    let mut nodes = BTreeMap::<String, Node>::new();
    let mut edges = Vec::new();
    let (_, verification) = verify_graph(&tx)?;
    for task in &tasks {
        let node_id = format!("task:{}", task.id);
        nodes.insert(node_id.clone(), Node { id: node_id.clone(), kind: NodeKind::Task, key: task.id.clone(),
            attributes: json!({"title":task.title,"description":task.desc,"status":task.status,
                "type":task.item_type,"archived":task.archived != 0,"revision":task.rev,"updated":task.updated,
                "next_action":task.next_action,"acceptance_criteria":task.acceptance_criteria,
                "evidence":task.evidence,"source_ref":task.source_ref,
                "detail_url":format!("/api/board/{}",task.id)}) });
        for dep in task.depends_on.iter().collect::<BTreeSet<_>>() {
            edges.push(edge(
                &node_id,
                Relation::DependsOn,
                format!("task:{dep}"),
                "issues",
                &task.id,
                "depends_on",
                Some(task.rev),
            ));
        }
        if let Some(parent) = task.epic.as_deref().filter(|v| !v.is_empty()) {
            edges.push(edge(
                &node_id,
                Relation::PartOf,
                format!("task:{parent}"),
                "issues",
                &task.id,
                "epic",
                Some(task.rev),
            ));
        }
        if let Some(worker) = task.session.as_deref().filter(|v| !v.is_empty()) {
            let worker_id = format!("worker:{worker}");
            nodes.entry(worker_id.clone()).or_insert_with(|| Node {
                id: worker_id.clone(),
                kind: NodeKind::Worker,
                key: worker.into(),
                attributes: json!({"liveness_measured":false,"source":"recorded task assignment"}),
            });
            edges.push(edge(
                &node_id,
                Relation::AssignedTo,
                worker_id,
                "issues",
                &task.id,
                "session",
                Some(task.rev),
            ));
        }
    }
    {
        let mut stmt = tx.prepare("SELECT a.id,a.task_id,a.kind,a.ref_value,a.state,a.description,a.updated_at \
            FROM _amux_task_artifacts a JOIN issues i ON i.id=a.task_id WHERE i.deleted IS NULL ORDER BY a.id")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let task: String = row.get(1)?;
            let nid = format!("artifact:{id}");
            nodes.insert(nid.clone(), Node { id: nid.clone(), kind: NodeKind::Artifact, key: id.clone(), attributes: json!({
                "kind":row.get::<_,String>(2)?,"ref":row.get::<_,String>(3)?,"state":row.get::<_,String>(4)?,
                "description":row.get::<_,Option<String>>(5)?,"updated":row.get::<_,i64>(6)?,"existence_measured":false}) });
            edges.push(edge(
                &format!("task:{task}"),
                Relation::Produces,
                nid,
                "_amux_task_artifacts",
                &id,
                "task_id",
                None,
            ));
        }
    }
    {
        let mut stmt = tx.prepare("SELECT h.id,h.card_id,h.type,h.session,h.ts,h.origin,h.delivery \
            FROM cmd_history h JOIN issues i ON i.id=h.card_id WHERE i.deleted IS NULL ORDER BY h.id")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let id = row.get::<_, i64>(0)?.to_string();
            let task: String = row.get(1)?;
            let nid = format!("message:{id}");
            nodes.insert(
                nid.clone(),
                Node {
                    id: nid.clone(),
                    kind: NodeKind::Message,
                    key: id.clone(),
                    attributes: json!({
                "type":row.get::<_,Option<String>>(2)?,"session":row.get::<_,Option<String>>(3)?,
                "timestamp":row.get::<_,i64>(4)?,"origin":row.get::<_,Option<String>>(5)?,
                "delivery":row.get::<_,Option<String>>(6)?}),
                },
            );
            // card_id records association, not causation. A callback is not the
            // human message that created the task, so don't call this derived_from.
            edges.push(edge(
                &format!("task:{task}"),
                Relation::RecordedMessage,
                nid,
                "cmd_history",
                &id,
                "card_id",
                None,
            ));
        }
    }
    edges.sort_by(|a, b| (&a.from, a.relation, &a.to).cmp(&(&b.from, b.relation, &b.to)));
    let nodes: Vec<Node> = nodes.into_values().collect();
    let sha256 = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            &nodes,
            &edges,
            &verification.findings
        ))?)
    );
    tx.commit()?;
    Ok(Snapshot {
        schema_version: 1,
        revision,
        sha256,
        measured: true,
        n_considered: ids.len(),
        nodes,
        edges,
        verification,
    })
}

/// Cheap structural probe for the periodic invariant monitor. No task prose,
/// message bodies or artifact payloads are loaded during a sweep.
pub fn verify_graph(conn: &Connection) -> anyhow::Result<(usize, Verification)> {
    let mut stmt =
        conn.prepare("SELECT id, depends_on, epic FROM issues WHERE deleted IS NULL ORDER BY id")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let ids: BTreeSet<String> = rows.iter().map(|r| r.0.clone()).collect();
    let mut dependencies = Adjacency::new();
    let mut lineage = Adjacency::new();
    let mut findings = Vec::new();
    let mut invalid_sources = BTreeSet::new();
    for (id, raw, parent) in rows {
        let raw = raw.as_deref().filter(|v| !v.is_empty()).unwrap_or("[]");
        match serde_json::from_str::<Vec<String>>(raw) {
            Err(_) => {
                finding(
                    &mut findings,
                    "malformed_dependencies",
                    vec![id.clone()],
                    "depends_on",
                );
                invalid_sources.insert(id.clone());
            }
            Ok(deps) => {
                let mut seen = BTreeSet::new();
                for dep in deps {
                    if !seen.insert(dep.clone()) {
                        finding(
                            &mut findings,
                            "duplicate_dependency",
                            vec![id.clone(), dep],
                            "depends_on",
                        );
                        continue;
                    }
                    if !ids.contains(&dep) {
                        finding(
                            &mut findings,
                            "missing_task",
                            vec![id.clone(), dep.clone()],
                            "depends_on",
                        );
                    }
                    dependencies.entry(id.clone()).or_default().insert(dep);
                }
            }
        }
        if let Some(parent) = parent.filter(|v| !v.is_empty()) {
            if !ids.contains(&parent) {
                finding(
                    &mut findings,
                    "missing_task",
                    vec![id.clone(), parent.clone()],
                    "epic",
                );
            }
            lineage.entry(id).or_default().insert(parent);
        }
    }
    let build_nodes = ids.difference(&invalid_sources).cloned().collect();
    let mut dep_analysis = task_graph::analyze(&build_nodes, &dependencies);
    dep_analysis.unbuildable.extend(invalid_sources);
    dep_analysis.unbuildable.sort();
    let parent_analysis = task_graph::analyze(&ids, &lineage);
    for (field, analysis) in [("depends_on", &dep_analysis), ("epic", &parent_analysis)] {
        for cycle in &analysis.cycles {
            finding(&mut findings, "cycle", cycle.clone(), field);
        }
    }
    findings
        .sort_by(|a, b| (&a.code, &a.field, &a.task_ids).cmp(&(&b.code, &b.field, &b.task_ids)));
    Ok((
        ids.len(),
        Verification {
            valid: findings.is_empty(),
            findings,
            dependencies: dep_analysis,
            lineage: parent_analysis,
        },
    ))
}
