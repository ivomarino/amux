//! /api/env/apply integration tests (AMUX-2977) — the declarative environment
//! loader, exercised through the REAL router + oneshot against a temp store and
//! a temp `AMUX_HOME`, never the machine's fleet directory.
//!
//! The claim under test is the ethos one: one YAML CONVERGES onto the
//! primitives (a group is its name, a worker is its env file), so re-applying
//! the same spec must report `unchanged`, not a second create. A loader that
//! duplicated on every apply would be the accumulate-not-discriminate failure
//! (ethos rule 5); these tests are what make "idempotent" a checkable property
//! rather than a docstring.

use amux_server::api::{router, AppState};
use amux_server::db::Store;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

/// ONE temp home for the whole file, set exactly once — `AMUX_HOME` is
/// process-global and these tests run on parallel threads, so a per-test
/// `set_var` would race (the pattern proven in session_report_attribution.rs).
/// Each integration test file is its own binary, so this home is private to it.
static HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();

fn home() -> &'static std::path::Path {
    HOME.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("AMUX_HOME", dir.path());
        dir
    })
    .path()
}

fn app(db_tag: &str) -> (axum::Router, std::sync::Arc<Store>) {
    let home = home();
    // A worker write needs sessions/ to exist; the handler create_dir_all's it,
    // but a real home always has it, so mirror that.
    std::fs::create_dir_all(home.join("sessions")).unwrap();
    // A DB PER TEST — the whole file shares one temp home, so a shared db file
    // would have parallel tokio tests re-run migrations on the same handle and
    // collide (UNIQUE _amux_migrations.version / "database is locked").
    let store =
        std::sync::Arc::new(Store::open(&home.join(format!("env-config-{db_tag}.db"))).unwrap());
    let state = AppState {
        store: store.clone(),
        started: std::time::Instant::now(),
        build_hash: "test".into(),
        auth_token: None,
    };
    (router(state), store)
}

async fn apply(app: &axum::Router, yaml: &str, dry: bool) -> (StatusCode, Value) {
    let uri = if dry { "/api/env/apply?dry_run=1" } else { "/api/env/apply" };
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-yaml")
        .body(Body::from(yaml.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, v)
}

/// The action the report assigned to a given (kind, name), or None if absent.
fn action_for<'a>(body: &'a Value, kind: &str, name: &str) -> Option<&'a str> {
    body["report"]
        .as_array()?
        .iter()
        .find(|e| e["kind"] == kind && e["name"] == name)
        .and_then(|e| e["action"].as_str())
}

fn group_row(store: &Store, name: &str) -> Option<(String, String)> {
    let conn = store.read().ok()?;
    conn.query_row(
        "SELECT department, goal FROM group_config WHERE name=?1",
        [name],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )
    .ok()
}

/// A spec whose group + worker names are unique to the calling test — the whole
/// file shares one temp home, so worker env FILES and group names collide across
/// parallel tests unless namespaced. Returns (yaml, group_name, worker_name).
fn spec_for(tag: &str) -> (String, String, String) {
    let group = format!("env2977-{tag}-eng");
    let worker = format!("env2977-{tag}-backend");
    let yaml = format!(
        "groups:\n  - name: {group}\n    department: Engineering\n    goal: Ship the platform\n\
         \nworkers:\n  - name: {worker}\n    groups: [{group}]\n    desc: Backend API work\n    model: sonnet\n"
    );
    (yaml, group, worker)
}

/// GUARD: this suite must never be able to write into the real fleet dir.
#[test]
fn this_suite_cannot_reach_the_live_amux_home() {
    let h = home();
    let tmp = std::env::temp_dir();
    assert!(
        h.starts_with(&tmp),
        "AMUX_HOME must be under the system temp dir, got {h:?}"
    );
}

#[tokio::test]
async fn dry_run_reports_creates_and_writes_nothing() {
    let (app, store) = app("dry");
    let (yaml, group, worker) = spec_for("dry");
    let (st, body) = apply(&app, &yaml, true).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["dry_run"], true);
    assert_eq!(action_for(&body, "group", &group), Some("create"));
    assert_eq!(action_for(&body, "worker", &worker), Some("create"));
    // Nothing landed: no group row, no env file.
    assert!(group_row(&store, &group).is_none(), "dry run wrote a group");
    let env_file = home().join("sessions").join(format!("{worker}.env"));
    assert!(!env_file.exists(), "dry run wrote a worker env file");
}

#[tokio::test]
async fn apply_then_reapply_converges_to_unchanged() {
    let (app, store) = app("apply");
    let (yaml, group, worker) = spec_for("apply");

    // First apply: both created.
    let (st, body) = apply(&app, &yaml, false).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["applied"], true);
    assert_eq!(body["errors"].as_array().map(|a| a.len()), Some(0));
    assert_eq!(action_for(&body, "group", &group), Some("create"));
    assert_eq!(action_for(&body, "worker", &worker), Some("create"));

    // The group landed in group_config with exactly the spec's fields.
    assert_eq!(
        group_row(&store, &group),
        Some(("Engineering".into(), "Ship the platform".into()))
    );

    // The worker env file landed, 0600, with the derived keys.
    let env_file = home().join("sessions").join(format!("{worker}.env"));
    let contents = std::fs::read_to_string(&env_file).expect("worker env file written");
    assert!(contents.contains(&format!("CC_TAGS=\"{group}\"")), "groups -> CC_TAGS: {contents}");
    assert!(contents.contains("CC_DESC=\"Backend API work\""), "desc -> CC_DESC: {contents}");
    assert!(contents.contains("CC_FLAGS=\"--model sonnet\""), "model -> CC_FLAGS: {contents}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&env_file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "worker env file must be 0600, got {mode:o}");
    }

    // Re-apply the SAME spec: idempotent — both report unchanged, nothing new.
    let (st, body) = apply(&app, &yaml, false).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(action_for(&body, "group", &group), Some("unchanged"));
    assert_eq!(action_for(&body, "worker", &worker), Some("unchanged"));
}

#[tokio::test]
async fn changing_a_field_reports_update_not_create() {
    let (app, store) = app("change");
    let base = r#"
groups:
  - name: env2977-changing
    department: Ops
    goal: v1
"#;
    let changed = r#"
groups:
  - name: env2977-changing
    department: Ops
    goal: v2-revised
"#;
    let (_, _) = apply(&app, base, false).await;
    let (st, body) = apply(&app, changed, false).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(action_for(&body, "group", "env2977-changing"), Some("update"));
    assert_eq!(
        group_row(&store, "env2977-changing").map(|(_, g)| g),
        Some("v2-revised".into())
    );
}

#[tokio::test]
async fn phase_2_stanzas_are_reported_not_dropped() {
    let (app, _store) = app("phase2");
    let spec = r#"
schedules:
  - title: nightly
columns:
  - name: review
global:
  AMUX_HELPER_MODEL: haiku
"#;
    let (st, body) = apply(&app, spec, true).await;
    assert_eq!(st, StatusCode::OK);
    let report = body["report"].as_array().unwrap();
    let has = |kind: &str| report.iter().any(|e| e["kind"] == kind && e["action"] == "not-yet-applied");
    assert!(has("schedules"), "schedules stanza must be announced, not silently dropped");
    assert!(has("columns"), "columns stanza must be announced");
    assert!(has("global"), "global stanza must be announced");
}

#[tokio::test]
async fn files_are_seeded_idempotently_and_relative_paths_error() {
    let (app, _store) = app("files");
    // An absolute path under the temp home so the test never writes outside it.
    let doc = home().join("seed").join("welcome.md");
    let doc_s = doc.to_string_lossy().to_string();
    let yaml = format!(
        "files:\n  - path: {doc_s}\n    content: |\n      hello vertical\n  - path: relative/nope.md\n    content: x\n"
    );

    // Dry run: the absolute file reports create, the relative one errors, and
    // nothing is written.
    let (st, body) = apply(&app, &yaml, true).await;
    assert_eq!(st, StatusCode::OK);
    let report = body["report"].as_array().unwrap();
    let file_row = report.iter().find(|e| e["path"] == doc_s).unwrap();
    assert_eq!(file_row["action"], "create");
    let rel_row = report.iter().find(|e| e["kind"] == "file" && e["action"] == "error").unwrap();
    assert!(rel_row["detail"].as_str().unwrap().contains("absolute"));
    assert!(!doc.exists(), "dry run must not write the file");

    // Apply: the file lands with exactly the content.
    let (st, _) = apply(&app, &yaml, false).await;
    assert_eq!(st, StatusCode::OK);
    let written = std::fs::read_to_string(&doc).expect("seed file written");
    assert_eq!(written, "hello vertical\n");

    // Re-apply the same spec: the file reports unchanged.
    let (_, body) = apply(&app, &yaml, false).await;
    let file_row = body["report"].as_array().unwrap().iter().find(|e| e["path"] == doc_s).unwrap();
    assert_eq!(file_row["action"], "unchanged");
}

#[tokio::test]
async fn invalid_yaml_is_a_400_not_a_panic() {
    let (app, _store) = app("badyaml");
    let (st, body) = apply(&app, "groups: [unterminated", false).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap_or("").contains("invalid YAML"));
}
