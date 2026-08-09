//! Board API integration tests (RR-0049, RR-0055; Invariants 3, 18, 37, 40
//! and the L1 payload lesson), run with `tower::ServiceExt::oneshot` against
//! the real router + a temp-file store — never against ~/.amux/amux.db.
//!
//! The Python-interop test hand-INSERTs a row shaped exactly like a live
//! Python row (int timestamps, the `needsyou` spelling, `` `HH:MM` `` log
//! lines, JSON-array depends_on) and asserts the Rust API round-trips it
//! without corrupting a single column the Python server reads — the
//! strangler-fig requirement (Phase 11: both servers, same rows).

use amux_server::api::{router, AppState};
use amux_server::db::Store;
use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

fn app() -> (axum::Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("amux-test.db")).unwrap();
    let state = AppState {
        store: std::sync::Arc::new(store),
        started: std::time::Instant::now(),
        build_hash: "test".into(),
        auth_token: None,
    };
    (router(state), dir)
}

async fn send_with(
    app: &axum::Router,
    method: &str,
    path: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, Value) {
    let mut b = Request::builder().method(method).uri(path);
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let req = match body {
        Some(v) => b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    (status, headers, v)
}

async fn send(
    app: &axum::Router,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, HeaderMap, Value) {
    send_with(app, method, path, body, &[]).await
}

async fn create(app: &axum::Router, body: Value) -> Value {
    let (st, _, v) = send(app, "POST", "/api/board", Some(body)).await;
    assert_eq!(st, StatusCode::CREATED, "create failed: {v}");
    v
}

fn hdr<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .map(|v| v.to_str().unwrap())
        .unwrap_or_else(|| panic!("missing header {name}"))
}

// ---- create -> list -> detail lifecycle ----------------------------------

#[tokio::test]
async fn create_list_detail_lifecycle() {
    let (app, _dir) = app();

    // Session-derived prefix + shared-counter minting: my-project -> MP-1,
    // MP-2; no session -> AMUX-1.
    let a = create(
        &app,
        json!({ "title": "First card", "session": "my-project", "desc": "line one\nline two" }),
    )
    .await;
    assert_eq!(a["id"], json!("MP-1"));
    assert_eq!(a["status"], json!("todo"));
    assert_eq!(a["type"], json!("code"));
    assert_eq!(a["session"], json!("my-project"));
    assert_eq!(a["owner_type"], json!("agent"));
    assert!(a["created"].is_i64(), "created must be unix INTEGER seconds");
    assert!(a["updated"].is_i64());

    let b = create(&app, json!({ "title": "Second", "session": "my-project" })).await;
    assert_eq!(b["id"], json!("MP-2"));
    let c = create(&app, json!({ "title": "No lane" })).await;
    assert_eq!(c["id"], json!("AMUX-1"));

    // Missing title is a 400.
    let (st, _, v) = send(&app, "POST", "/api/board", Some(json!({ "desc": "x" }))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert_eq!(v["error"], json!("missing title"));

    // List: a BARE JSON ARRAY (the Python dashboard parses exactly that).
    let (st, _, list) = send(&app, "GET", "/api/board", None).await;
    assert_eq!(st, StatusCode::OK);
    let arr = list.as_array().expect("list must be a bare array");
    assert_eq!(arr.len(), 3);
    assert!(arr.iter().any(|i| i["id"] == json!("MP-1")));

    // Detail: full desc, log field present, ETag carries the rev.
    let (st, headers, detail) = send(&app, "GET", "/api/board/MP-1", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(detail["desc"], json!("line one\nline two"));
    assert!(detail.get("log").is_some());
    assert_eq!(hdr(&headers, "etag"), "W/\"MP-1-0\"");

    // Unknown id -> 404.
    let (st, _, _) = send(&app, "GET", "/api/board/NOPE-1", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // MO-3038: session OMITTED + X-Amux-Session header -> the sender's lane.
    let (st, _, v) = send_with(
        &app,
        "POST",
        "/api/board",
        Some(json!({ "title": "header lane" })),
        &[("X-Amux-Session", "orch")],
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(v["session"], json!("orch"));
    assert_eq!(v["id"], json!("ORCH-1"));
    assert_eq!(v["creator"], json!("orch"));
}

// ---- L1: desc truncation in lists, full in detail ------------------------

#[tokio::test]
async fn list_truncates_desc_detail_serves_it_whole() {
    let (app, _dir) = app();
    let long_desc = format!("first line {}\nsecond line body", "x".repeat(300));
    create(&app, json!({ "title": "Big desc", "desc": long_desc })).await;

    let (_, _, list) = send(&app, "GET", "/api/board", None).await;
    let item = &list.as_array().unwrap()[0];
    let shown = item["desc"].as_str().unwrap();
    assert!(shown.chars().count() <= 200, "list desc capped at 200 chars");
    assert!(!shown.contains("second line"), "list desc is first line only");
    assert_eq!(item["desc_truncated"], json!(true));
    assert!(item.get("log").is_none(), "log never ships in a list (L1)");
    assert!(item["log_n"].is_i64() || item["log_n"].is_u64());

    // slim=1 drops desc entirely and declares its length instead.
    let (_, _, slim) = send(&app, "GET", "/api/board?slim=1", None).await;
    let item = &slim.as_array().unwrap()[0];
    assert!(item.get("desc").is_none());
    assert_eq!(item["desc_len"].as_u64().unwrap() as usize, long_desc.chars().count());

    // Detail: the whole desc, untruncated.
    let id = list.as_array().unwrap()[0]["id"].as_str().unwrap().to_string();
    let (_, _, detail) = send(&app, "GET", &format!("/api/board/{id}"), None).await;
    assert_eq!(detail["desc"].as_str().unwrap(), long_desc);
    assert!(detail.get("desc_truncated").is_none());
}

// ---- done_limit + the truncation header quartet (Invariant 40) -----------

#[tokio::test]
async fn done_limit_caps_terminal_and_headers_announce_it() {
    let (app, _dir) = app();
    for i in 0..3 {
        create(&app, json!({ "title": format!("done {i}"), "status": "done" })).await;
    }
    create(&app, json!({ "title": "live", "status": "todo" })).await;

    // Cap bites: 2 of 3 terminal kept, active card never capped.
    let (st, headers, list) = send(&app, "GET", "/api/board?done_limit=2", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 3); // 1 active + 2 terminal
    assert_eq!(hdr(&headers, "x-amux-done-limit"), "2");
    assert_eq!(hdr(&headers, "x-amux-truncated"), "1");
    assert_eq!(hdr(&headers, "x-amux-terminal-total"), "3");
    assert_eq!(hdr(&headers, "x-amux-terminal-returned"), "2");

    // Default limit 100: nothing withheld, and the headers SAY so.
    let (_, headers, list) = send(&app, "GET", "/api/board", None).await;
    assert_eq!(list.as_array().unwrap().len(), 4);
    assert_eq!(hdr(&headers, "x-amux-done-limit"), "100");
    assert_eq!(hdr(&headers, "x-amux-truncated"), "0");
    assert_eq!(hdr(&headers, "x-amux-terminal-total"), "3");
    assert_eq!(hdr(&headers, "x-amux-terminal-returned"), "3");

    // done_limit=0 = unlimited (Python contract: totals report 0/0).
    let (_, headers, list) = send(&app, "GET", "/api/board?done_limit=0", None).await;
    assert_eq!(list.as_array().unwrap().len(), 4);
    assert_eq!(hdr(&headers, "x-amux-truncated"), "0");

    // Status/session filters run BEFORE the cap (AC-291's lesson).
    let (_, headers, list) = send(&app, "GET", "/api/board?status=done&done_limit=2", None).await;
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(hdr(&headers, "x-amux-terminal-total"), "3");
}

// ---- gate 409: Python-compatible body, then honest satisfaction ----------

#[tokio::test]
async fn gate_blocks_with_python_body_then_gate_checked_satisfies() {
    let (app, _dir) = app();
    let card = create(&app, json!({ "title": "gated", "status": "doing" })).await;
    let id = card["id"].as_str().unwrap().to_string();

    // Unacked doing->done on a code card: the exact 409 the CLI parses.
    let (st, _, v) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "status": "done" })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(v["error"], json!("gate not acknowledged"));
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["blocked"], json!(true));
    assert_eq!(
        v["gate"],
        json!(["Implemented and merged", "Tests / lint pass"])
    );
    assert_eq!(v["attempted_status"], json!("done"));
    assert_eq!(v["item"], json!(id));
    assert_eq!(v["item_type"], json!("code"));
    assert!(v["valid_types"].as_array().unwrap().contains(&json!("escalation")));
    // NB: no "status" key — a client reading the body instead of the HTTP
    // code must not misread the rejection as success (orch MO-2952).
    assert!(v.get("status").is_none());
    // Core's why-blocked answer rides along (Invariant 18): criterion,
    // missing evidence kind, serialized refusal kind.
    assert_eq!(v["kind"], json!("gate_blocked"));
    let wb = v["why_blocked"].as_array().unwrap();
    assert_eq!(wb.len(), 2);
    assert_eq!(wb[0]["criterion"], json!("Implemented and merged"));
    assert_eq!(wb[0]["missing"], json!("model_transcript"));

    // gate_checked that does NOT match every criterion is refused (AMUX-1719).
    let (st, _, v) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "status": "done", "gate_checked": ["Implemented and merged"] })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(v["error"], json!("gate_checked does not match the gate"));
    assert_eq!(v["missing"], json!(["Tests / lint pass"]));

    // The full ack passes, and the evidence lands in the card's log.
    let (st, _, v) = send_with(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({
            "status": "done",
            "gate_checked": ["Implemented and merged", "Tests / lint pass"]
        })),
        &[("X-Amux-Session", "worker-1")],
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["status"], json!("done"));
    assert_eq!(v["applied"], json!(true));
    let (_, _, detail) = send(&app, "GET", &format!("/api/board/{id}"), None).await;
    let log = detail["log"].as_str().unwrap();
    assert!(log.contains("worker-1: gate satisfied via gate_checked (2/2)"), "log: {log}");
    assert!(log.contains("worker-1: doing -> done"), "log: {log}");
}

#[tokio::test]
async fn gates_derive_from_type_and_retyping_is_the_honest_exit() {
    let (app, _dir) = app();
    let card = create(
        &app,
        json!({ "title": "self-resolved page", "status": "doing", "type": "escalation" }),
    )
    .await;
    let id = card["id"].as_str().unwrap().to_string();

    // An escalation is NOT gated on a merge — its gate is the honest one.
    let (st, _, v) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "status": "done" })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(
        v["gate"],
        json!(["Outcome recorded in the item (what happened, and why it is closed)"])
    );

    // gate_ack: true satisfies it wholesale.
    let (st, _, v) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "status": "done", "gate_ack": true })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["status"], json!("done"));

    // Unknown types are rejected at the door with the valid set — never
    // silently mis-gated (the seven 'decision' cards incident).
    let (st, _, v) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "type": "decision" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("unknown type"));
    assert!(v["valid_types"].as_array().unwrap().contains(&json!("watch")));
}

// ---- force: bypass WITH audit (ethos rule 6) -----------------------------

#[tokio::test]
async fn force_bypasses_the_gate_and_leaves_the_audit_line() {
    let (app, _dir) = app();
    let card = create(&app, json!({ "title": "hotfix", "status": "doing" })).await;
    let id = card["id"].as_str().unwrap().to_string();

    let (st, _, v) = send_with(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "status": "done", "force": true, "reason": "hotfix, evidence in PR" })),
        &[("X-Amux-Session", "tester")],
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["status"], json!("done"));

    // Read the log BACK — the force must be traceable from the card itself.
    let (_, _, detail) = send(&app, "GET", &format!("/api/board/{id}"), None).await;
    let log = detail["log"].as_str().unwrap();
    assert!(
        log.contains("force by tester: doing->done reason=hotfix, evidence in PR"),
        "force audit line missing from log: {log}"
    );

    // A headerless force is still audited, attributed to `anonymous`.
    let card2 = create(&app, json!({ "title": "h2", "status": "doing" })).await;
    let id2 = card2["id"].as_str().unwrap().to_string();
    let (st, _, _) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id2}"),
        Some(json!({ "status": "done", "force": true })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (_, _, detail) = send(&app, "GET", &format!("/api/board/{id2}"), None).await;
    assert!(detail["log"].as_str().unwrap().contains("force by anonymous: doing->done"));
}

// ---- archive / restore (RR-0055) -----------------------------------------

#[tokio::test]
async fn archive_restore_round_trip_preserves_every_field() {
    let (app, _dir) = app();
    let card = create(
        &app,
        json!({
            "title": "parked work", "status": "doing", "session": "my-project",
            "desc": "half-done", "type": "research", "tags": ["q3"]
        }),
    )
    .await;
    let id = card["id"].as_str().unwrap().to_string();

    let (st, _, v) = send_with(
        &app,
        "POST",
        &format!("/api/board/{id}/archive"),
        Some(json!({ "reason": "parking for Q4" })),
        &[("X-Amux-Session", "orch")],
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["applied"], json!(true));
    assert_eq!(v["archived"], json!(1));
    assert_eq!(v["status"], json!("doing"), "archive is a FLAG, not a status");

    // Archived cards are excluded from the default list but discoverable.
    let (_, _, list) = send(&app, "GET", "/api/board", None).await;
    assert!(list.as_array().unwrap().is_empty());
    let (_, _, list) = send(&app, "GET", "/api/board?archived=1", None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Double-archive: honest no-op, rev unmoved (Invariant 37).
    let rev_after_archive = v["rev"].as_i64().unwrap();
    let (st, _, v2) = send(&app, "POST", &format!("/api/board/{id}/archive"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v2["applied"], json!(false));
    assert_eq!(v2["rev"].as_i64().unwrap(), rev_after_archive);

    // A status PATCH on an archived card is refused (restore it first).
    let (st, _, v3) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "status": "done", "force": true })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert!(v3["error"].as_str().unwrap().contains("archived"));

    // Restore: back exactly where it was, every field intact.
    let (st, _, r) = send(&app, "POST", &format!("/api/board/{id}/restore"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(r["applied"], json!(true));
    assert_eq!(r["archived"], json!(0));
    assert_eq!(r["status"], json!("doing"));
    assert_eq!(r["title"], json!("parked work"));
    assert_eq!(r["desc"], json!("half-done"));
    assert_eq!(r["session"], json!("my-project"));
    assert_eq!(r["type"], json!("research"));
    assert_eq!(r["tags"], json!(["q3"]));
    let log = r["log"].as_str().unwrap();
    assert!(log.contains("orch: archived — parking for Q4"), "log: {log}");
    assert!(log.contains("restored"), "log: {log}");
}

// ---- circular depends_on -------------------------------------------------

#[tokio::test]
async fn circular_depends_on_is_rejected_with_the_cycle_path() {
    let (app, _dir) = app();
    let a = create(&app, json!({ "title": "A", "session": "g" })).await;
    let a_id = a["id"].as_str().unwrap().to_string();
    let b = create(&app, json!({ "title": "B", "session": "g", "depends_on": [a_id.clone()] })).await;
    let b_id = b["id"].as_str().unwrap().to_string();
    assert_eq!(b["depends_on"], json!([a_id.clone()]));

    // Closing the loop is a 400 naming the cycle.
    let (st, _, v) = send(
        &app,
        "PATCH",
        &format!("/api/board/{a_id}"),
        Some(json!({ "depends_on": [b_id.clone()] })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("circular depends_on"));
    let cycle = v["cycle"].as_array().unwrap();
    assert!(cycle.contains(&json!(a_id.clone())) && cycle.contains(&json!(b_id)));

    // A self-dependency at create is the same refusal.
    let (st, _, v) = send(
        &app,
        "PATCH",
        &format!("/api/board/{a_id}"),
        Some(json!({ "depends_on": [a_id.clone()] })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");

    // And nothing was written by the refusals.
    let (_, _, detail) = send(&app, "GET", &format!("/api/board/{a_id}"), None).await;
    assert_eq!(detail["depends_on"], json!([]));
}

// ---- no-op PATCH: applied:false, rev unmoved (Invariant 37) --------------

#[tokio::test]
async fn noop_patch_reports_applied_false_and_moves_nothing() {
    let (app, _dir) = app();
    let card = create(&app, json!({ "title": "steady", "desc": "d" })).await;
    let id = card["id"].as_str().unwrap().to_string();
    let rev0 = card["rev"].as_i64().unwrap();

    // Same values -> nothing changed.
    let (st, _, v) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "title": "steady", "desc": "d" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["applied"], json!(false));
    assert_eq!(v["rev"].as_i64().unwrap(), rev0);

    // Unknown keys are NAMED, never silently dropped (AC-263/AMUX-2492).
    let (st, _, v) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "archived": 1, "bogus_key": true })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["applied"], json!(false));
    let ignored = v["ignored_fields"].as_array().unwrap();
    assert!(ignored.contains(&json!("archived")) && ignored.contains(&json!("bogus_key")));

    // Same-status PATCH is also a no-op, not a phantom transition.
    let (st, _, v) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "status": "todo" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["applied"], json!(false));

    // Read back: rev truly unmoved, no log lines invented.
    let (_, _, detail) = send(&app, "GET", &format!("/api/board/{id}"), None).await;
    assert_eq!(detail["rev"].as_i64().unwrap(), rev0);
    assert_eq!(detail["log"], Value::Null);
}

// ---- optimistic concurrency ----------------------------------------------

#[tokio::test]
async fn stale_expect_rev_is_409_with_current_rev() {
    let (app, _dir) = app();
    let card = create(&app, json!({ "title": "contested" })).await;
    let id = card["id"].as_str().unwrap().to_string();

    // Move rev forward once.
    let (st, _, _) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "desc": "writer A", "expect_rev": 0 })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    // A stale writer against rev 0 gets the conflict WITH the current state.
    let (st, _, v) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "desc": "writer B", "expect_rev": 0 })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(v["error"], json!("rev conflict"));
    assert_eq!(v["current_rev"], json!(1));
    assert_eq!(v["item"]["desc"], json!("writer A"));

    // Nothing was clobbered.
    let (_, _, detail) = send(&app, "GET", &format!("/api/board/{id}"), None).await;
    assert_eq!(detail["desc"], json!("writer A"));
}

// ---- full lifecycle through the named transitions ------------------------

#[tokio::test]
async fn lifecycle_todo_doing_review_done_verified_via_state_machine() {
    let (app, _dir) = app();
    let card = create(&app, json!({ "title": "full run", "type": "chore" })).await;
    let id = card["id"].as_str().unwrap().to_string();

    // Chore gates are the honest non-code bar; ack each hop.
    for (target, expect) in [
        ("doing", "doing"),
        ("review", "review"),
        ("done", "done"),
        ("verified", "verified"),
    ] {
        let (st, _, v) = send_with(
            &app,
            "PATCH",
            &format!("/api/board/{id}"),
            Some(json!({ "status": target, "gate_ack": true })),
            &[("X-Amux-Session", "runner")],
        )
        .await;
        assert_eq!(st, StatusCode::OK, "-> {target}: {v}");
        assert_eq!(v["status"], json!(expect));
    }
    let (_, _, detail) = send(&app, "GET", &format!("/api/board/{id}"), None).await;
    let log = detail["log"].as_str().unwrap();
    for line in [
        "runner: todo -> doing",
        "runner: doing -> review",
        "runner: review -> done",
        "runner: done -> verified",
    ] {
        assert!(log.contains(line), "missing {line:?} in log: {log}");
    }
    // Verified work cannot be discarded — the state machine speaks.
    let (st, _, v) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "status": "discarded" })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert!(v["error"].as_str().unwrap().contains("archive it instead"), "{v}");
}

// ---- PYTHON INTEROP: a live-shaped row survives the Rust API -------------

#[tokio::test]
async fn python_shaped_row_round_trips_without_corruption() {
    let (app, dir) = app();
    let db_path = dir.path().join("amux-test.db");

    // Hand-INSERT a row exactly as the live Python server writes them:
    // int unix timestamps, the `needsyou` spelling, `` `HH:MM` `` log lines,
    // JSON-array depends_on TEXT, no `version` named (0002's default).
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO issues (id, title, \"desc\", status, session, creator, created, \
                 updated, owner_type, pos, notified, type, archived, depends_on, log, rev, pinned) \
             VALUES ('ORCH-42', 'Live python card', 'body from python\nsecond line', 'needsyou', \
                 'orch', 'orch', 1754000000, 1754000600, 'agent', -2048.0, 1, 'escalation', 0, \
                 '[\"ORCH-1\"]', '`09:14` created by orch\n`09:20` STATUS (orch): waiting on Ethan', \
                 7, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO issue_tags (issue_id, tag, added_at) \
             VALUES ('ORCH-42', 'mixpeek', 1754000000)",
            [],
        )
        .unwrap();
    }

    // GET: raw Python vocabulary preserved on the wire.
    let (st, _, detail) = send(&app, "GET", "/api/board/ORCH-42", None).await;
    assert_eq!(st, StatusCode::OK, "{detail}");
    assert_eq!(detail["status"], json!("needsyou"), "spelling preserved, not rewritten");
    assert_eq!(detail["depends_on"], json!(["ORCH-1"]), "JSON TEXT decoded to a list");
    assert_eq!(detail["created"], json!(1754000000));
    assert_eq!(detail["rev"], json!(7));
    assert_eq!(detail["tags"], json!(["mixpeek"]));
    assert!(detail["log"].as_str().unwrap().contains("`09:20` STATUS (orch)"));

    // It appears in the list, and a needs_you filter (core spelling) finds
    // the needsyou row — both vocabularies resolve.
    let (_, _, list) = send(&app, "GET", "/api/board?status=needs_you", None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    // A field-only PATCH must not rewrite the status spelling.
    let (st, _, v) = send(
        &app,
        "PATCH",
        "/api/board/ORCH-42",
        Some(json!({ "title": "Live python card (triaged)" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["status"], json!("needsyou"));

    // Status transition needsyou -> doing (core: Resume) with the type-
    // derived escalation gate acked, attributed via header.
    let (st, _, v) = send_with(
        &app,
        "PATCH",
        "/api/board/ORCH-42",
        Some(json!({ "status": "doing", "gate_ack": true })),
        &[("X-Amux-Session", "orch")],
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["status"], json!("doing"));

    // Now read the raw columns back the way the PYTHON server will: every
    // column it depends on must still be exactly the shape it writes.
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let (status, created, updated, desc, session, creator, dep, log, rev, owner_type): (
        String,
        i64,
        i64,
        String,
        String,
        String,
        String,
        String,
        i64,
        String,
    ) = conn
        .query_row(
            "SELECT status, created, updated, \"desc\", session, creator, depends_on, log, \
                 COALESCE(rev,0), owner_type FROM issues WHERE id = 'ORCH-42'",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(status, "doing");
    assert_eq!(created, 1754000000, "created is Python's, untouched");
    assert!(updated > 1754000600, "updated bumped, still unix seconds");
    assert_eq!(desc, "body from python\nsecond line", "desc untouched");
    assert_eq!(session, "orch");
    assert_eq!(creator, "orch", "creator column never written by PATCH");
    assert_eq!(dep, "[\"ORCH-1\"]", "depends_on TEXT still exact JSON");
    assert_eq!(owner_type, "agent");
    assert_eq!(rev, 9, "two applied PATCHes bumped Python's counter twice");
    // The Python log lines are intact and ours were APPENDED after them in
    // the same `HH:MM` format.
    assert!(log.starts_with("`09:14` created by orch\n`09:20` STATUS (orch): waiting on Ethan\n"));
    assert!(log.contains("orch: needsyou -> doing"), "log: {log}");

    // Timestamp column types stayed INTEGER (a Python `int(time.time())`
    // consumer would silently break on an RFC3339 string).
    let t: String = conn
        .query_row(
            "SELECT typeof(updated) FROM issues WHERE id = 'ORCH-42'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(t, "integer");
}

// ---- auth: the board sits inside the protected router --------------------

#[tokio::test]
async fn board_routes_sit_behind_auth_when_token_configured() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("amux-test.db")).unwrap();
    let state = AppState {
        store: std::sync::Arc::new(store),
        started: std::time::Instant::now(),
        build_hash: "test".into(),
        auth_token: Some("sekrit".into()),
    };
    let app = router(state);
    let (st, _, _) = send(&app, "GET", "/api/board", None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _, _) = send_with(
        &app,
        "GET",
        "/api/board",
        None,
        &[("authorization", "Bearer sekrit")],
    )
    .await;
    assert_eq!(st, StatusCode::OK);
}
