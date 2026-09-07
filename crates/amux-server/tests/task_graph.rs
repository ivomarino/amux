//! Real router + migrated SQLite; no fleet mutation. Covers persisted legacy
//! corruption alongside new writes, not just a pristine DAG in memory.
use amux_server::api::{router, AppState};
use amux_server::db::{Store, WriteOutcome};
use axum::{body::Body, http::Request};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

fn app() -> (axum::Router, Arc<Store>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("graph.db")).unwrap());
    let app = router(AppState {
        store: store.clone(),
        started: std::time::Instant::now(),
        build_hash: "test".into(),
        auth_token: None,
        reconciled: Arc::new(std::sync::atomic::AtomicBool::new(true)),
    });
    (app, store, dir)
}
fn seed(store: &Store, sql: &'static str) {
    store
        .write(move |conn| {
            conn.execute_batch(sql)?;
            Ok(WriteOutcome {
                applied: true,
                events: vec![],
            })
        })
        .unwrap();
}
async fn request(app: &axum::Router, method: &str, path: &str, body: Value) -> (u16, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("Content-Type", "application/json")
                .header("X-Amux-Session", "graph-test")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), 32 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn graph_projects_provenance_and_rebuilds_identically() {
    let (app, store, _dir) = app();
    seed(
        &store,
        r#"
      INSERT INTO issues(id,title,status,session,created,updated,type,epic,depends_on) VALUES
        ('G-1','parent','doing','worker',1,1,'epic',NULL,'["G-2"]'),
        ('G-2','build','todo','worker',1,1,'code','G-1','[]');
      INSERT INTO _amux_task_artifacts(id,task_id,kind,ref_value,state,created_at,updated_at)
        VALUES ('ART-1','G-2','verification','/tmp/graph-proof.txt','created',1,1);
      INSERT INTO cmd_history(text,type,session,ts,card_id,origin,delivery)
        VALUES('Build the graph','direct','worker',1000,'G-1','human','confirmed');
    "#,
    );
    let (status, first) = request(&app, "GET", "/api/graph/board", Value::Null).await;
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["measured"], true);
    assert_eq!(first["n_considered"], 2);
    assert_eq!(first["schema_version"], 1);
    assert_eq!(
        first["verification"]["valid"], true,
        "a parent waiting on its child is not a lineage cycle"
    );
    assert_eq!(
        first["verification"]["dependencies"]["layers"],
        json!([["G-2"], ["G-1"]])
    );
    let edges = first["edges"].as_array().unwrap();
    for relation in [
        "depends_on",
        "part_of",
        "assigned_to",
        "produces",
        "recorded_message",
    ] {
        let edge = edges
            .iter()
            .find(|e| e["relation"] == relation)
            .expect(relation);
        assert!(edge["provenance"]["field"].is_string());
        assert!(edge["provenance"]["record_id"].is_string());
    }
    let artifact = first["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["kind"] == "artifact")
        .unwrap();
    assert_eq!(artifact["attributes"]["ref"], "/tmp/graph-proof.txt");
    assert_eq!(artifact["attributes"]["existence_measured"], false);
    let (_, second) = request(&app, "GET", "/api/graph/board", Value::Null).await;
    assert_eq!(first, second);
    seed(
        &store,
        "UPDATE _amux_task_artifacts SET state='deployed' WHERE id='ART-1'",
    );
    let (_, changed) = request(&app, "GET", "/api/graph/board", Value::Null).await;
    assert_ne!(
        first["sha256"], changed["sha256"],
        "artifact state is part of the snapshot identity"
    );
}

#[tokio::test]
async fn persisted_corruption_is_visible_and_cannot_mask_a_new_cycle() {
    let (app, store, _dir) = app();
    seed(
        &store,
        r#"
      INSERT INTO issues(id,title,status,created,updated,depends_on,epic) VALUES
        ('G-1','old cycle','todo',1,1,'["G-2"]',NULL),
        ('G-2','old cycle','todo',1,1,'["G-1"]',NULL),
        ('G-3','new edit','todo',1,1,'[]',NULL),
        ('G-4','new dependency','todo',1,1,'["G-3"]',NULL),
        ('G-5','dangling','todo',1,1,'["missing"]',NULL),
        ('G-6','malformed','todo',1,1,'not json',NULL),
        ('G-7','lineage','todo',1,1,'[]','G-8'),
        ('G-8','lineage','todo',1,1,'[]','G-7');
    "#,
    );
    let (status, rejected) = request(
        &app,
        "PATCH",
        "/api/board/G-3",
        json!({"depends_on":["G-4"]}),
    )
    .await;
    assert_eq!(status, 400, "{rejected}");
    assert_eq!(rejected["cycle"], json!(["G-3", "G-4", "G-3"]));
    let (_, card) = request(&app, "GET", "/api/board/G-3", Value::Null).await;
    assert_eq!(
        card["depends_on"],
        json!([]),
        "refusal must leave the row unchanged"
    );
    let (status, graph) = request(&app, "GET", "/api/graph/board", Value::Null).await;
    assert_eq!(status, 200, "{graph}");
    assert_eq!(graph["verification"]["valid"], false);
    assert_eq!(
        graph["verification"]["dependencies"]["cycles"],
        json!([["G-1", "G-2"]])
    );
    assert_eq!(
        graph["verification"]["lineage"]["cycles"],
        json!([["G-7", "G-8"]])
    );
    let findings = graph["verification"]["findings"].as_array().unwrap();
    assert!(findings.iter().any(|f| f["code"] == "missing_task"));
    assert!(findings
        .iter()
        .any(|f| f["code"] == "malformed_dependencies"));
    assert!(graph["verification"]["dependencies"]["unbuildable"]
        .as_array()
        .unwrap()
        .contains(&json!("G-6")));
    // A stale cycle must not prevent an independent, legitimate edge edit.
    let (status, result) = request(
        &app,
        "PATCH",
        "/api/board/G-3",
        json!({"depends_on":["G-7"]}),
    )
    .await;
    assert_eq!(status, 200, "{result}");
}

#[tokio::test]
async fn parent_cycles_are_refused_inside_the_write_transaction() {
    let (app, store, _dir) = app();
    seed(
        &store,
        "INSERT INTO issues(id,title,status,created,updated,type,epic) VALUES \
        ('G-1','parent','todo',1,1,'epic',NULL),('G-2','child','todo',1,1,'epic','G-1')",
    );
    let (status, result) = request(&app, "PATCH", "/api/board/G-1", json!({"epic":"G-2"})).await;
    assert_eq!(status, 400, "{result}");
    assert_eq!(result["code"], "lineage_cycle");
    let (_, row) = request(&app, "GET", "/api/board/G-1", Value::Null).await;
    assert!(row["epic"].is_null());
}

#[tokio::test]
async fn a_failed_graph_probe_never_reports_an_empty_success() {
    let (app, store, _dir) = app();
    seed(&store, "ALTER TABLE issues RENAME TO broken_issues");
    let (status, result) = request(&app, "GET", "/api/graph/board", Value::Null).await;
    assert_eq!(status, 500);
    assert_eq!(result["measured"], false);
    assert_eq!(result["n_considered"], 0);
    assert!(result["why_unmeasured"].is_string());
}

#[tokio::test]
async fn map_mutations_cannot_create_a_second_board_graph() {
    let (app, store, _dir) = app();
    for (method, path, body) in [
        (
            "POST",
            "/api/graph/board/import-vault",
            json!({"path":"/tmp"}),
        ),
        (
            "PATCH",
            "/api/graph/board/nodes/task:G-1",
            json!({"label":"fake task"}),
        ),
    ] {
        let (status, result) = request(&app, method, path, body).await;
        assert_eq!(status, 409, "{result}");
        assert_eq!(result["code"], "task_graph_read_only");
    }
    let conn = store.read().unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM graph_nodes WHERE graph_id='board'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn graph_cli_check_distinguishes_valid_invalid_and_unmeasured() {
    let (app, store, _dir) = app();
    let (_, good) = request(&app, "GET", "/api/graph/board", Value::Null).await;
    seed(
        &store,
        "INSERT INTO issues(id,title,status,created,updated,depends_on) VALUES \
        ('G-1','cycle','todo',1,1,'[\"G-1\"]')",
    );
    let (_, bad) = request(&app, "GET", "/api/graph/board", Value::Null).await;
    let cli = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../amux");
    for (fixture, expected) in [
        (good, 0),
        (bad, 1),
        (json!({"measured":false,"n_considered":0}), 2),
    ] {
        // Execute the actual fleet CLI against a loopback response fixture.
        // Sourcing it would exit at the CLI's read-whole-file safety boundary,
        // never reaching a mocked command; an empty stdout must not pass.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let payload = fixture.clone();
        let fixture_app = axum::Router::new()
            .route("/health", axum::routing::get(|| async { "ok" }))
            .route(
                "/api/graph/board",
                axum::routing::get(move || {
                    let body = payload.clone();
                    async move { axum::Json(body) }
                }),
            );
        let server = tokio::spawn(async move {
            axum::serve(listener, fixture_app).await.unwrap();
        });
        let cli = cli.clone();
        let scratch = tempfile::tempdir().unwrap();
        let out = tokio::task::spawn_blocking(move || {
            std::process::Command::new("bash")
                .arg(cli)
                .args(["board", "graph", "--json", "--check"])
                .env("AMUX_API", &url)
                .env("AMUX_URL", &url)
                .env("CC_HOME", scratch.path())
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        server.abort();
        assert_eq!(
            out.status.code(),
            Some(expected),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        if expected != 2 {
            let exported: Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(
                exported, fixture,
                "--json must preserve the versioned snapshot"
            );
        }
    }
}
