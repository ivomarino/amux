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

// ---- Python-parity list payload: full desc + full log --------------------
//
// The earlier L1 slimming (first-line desc + log_n instead of log) diverged
// from the Python oracle, whose plain list serves both whole — and the SPA
// renders `item.desc` and reads `item.log` (folded badge) straight off the
// LIST payload, so both were silently blank on the Rust dashboard
// (AMUX-2586 fix #4, measured live 2026-08-09). slim=1 stays the diet.

#[tokio::test]
async fn plain_list_serves_full_desc_and_log_slim_stays_the_diet() {
    let (app, _dir) = app();
    let long_desc = format!("first line {}\nsecond line body", "x".repeat(300));
    let v = create(&app, json!({ "title": "Big desc", "desc": long_desc })).await;
    let id = v["id"].as_str().unwrap().to_string();
    // Give the card a log line via a desc_append-style PATCH (log is
    // system-appended); a direct edit note lands in the card's log.
    let (_, _, _) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "desc": format!("{long_desc} edited") })),
    )
    .await;

    let (_, _, list) = send(&app, "GET", "/api/board", None).await;
    let item = &list.as_array().unwrap()[0];
    // Python's plain list: the WHOLE desc, the WHOLE log (string or null),
    // no desc_truncated / log_n / desc_len keys.
    assert!(item["desc"].as_str().unwrap().contains("second line"));
    assert!(item.get("desc_truncated").is_none());
    assert!(item.get("log_n").is_none());
    assert!(item.get("desc_len").is_none());
    assert!(
        item.get("log").is_some(),
        "log must be present in the plain list (SPA folded badge reads it)"
    );

    // slim=1 drops desc AND log, declaring desc_len + log_n instead.
    let (_, _, slim) = send(&app, "GET", "/api/board?slim=1", None).await;
    let item = &slim.as_array().unwrap()[0];
    assert!(item.get("desc").is_none());
    assert!(item.get("log").is_none());
    assert!(item["desc_len"].as_u64().is_some());
    assert!(item["log_n"].as_u64().is_some());

    // Detail: the whole desc, as ever.
    let (_, _, detail) = send(&app, "GET", &format!("/api/board/{id}"), None).await;
    assert!(detail["desc"].as_str().unwrap().contains("second line"));
    assert!(detail.get("desc_truncated").is_none());
}

// ---- GET /api/board/statuses (AMUX-2596) ---------------------------------
//
// The SPA builds its kanban columns from this list and silently falls back
// to a hardcoded default set on any failure — a 404 here meant custom
// Python-configured columns never rendered on the Rust origin.

#[tokio::test]
async fn board_statuses_serves_columns_or_python_defaults() {
    let (app, _dir) = app();
    let (st, _, v) = send(&app, "GET", "/api/board/statuses", None).await;
    assert_eq!(st, StatusCode::OK);
    let cols = v.as_array().unwrap();
    assert_eq!(cols.len(), 7, "python's builtin column set");
    assert_eq!(cols[0]["id"], json!("backlog"));
    assert_eq!(cols[2]["label"], json!("In Progress"));
    assert_eq!(cols[6]["id"], json!("discarded"));
}

// ---- PATCH {archived} — the SPA/CLI archive path (AMUX-2492 parity) ------

#[tokio::test]
async fn patch_archived_round_trip_with_cross_lane_guard() {
    let (app, _dir) = app();
    let v = create(&app, json!({ "title": "mine", "session": "lane-a" })).await;
    let id = v["id"].as_str().unwrap().to_string();

    // Cross-lane archive without authorized_by -> Python's 400 guard.
    let (st, _, e) = send_with(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "archived": 1 })),
        &[("X-Amux-Session", "lane-b")],
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(e["error"].as_str().unwrap().contains("authorized_by"), "{e}");
    assert_eq!(e["card_owner"], json!("lane-a"));

    // Same-lane archive: applied; the card leaves the active view.
    let (st, _, v) = send_with(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "archived": 1 })),
        &[("X-Amux-Session", "lane-a")],
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["archived"], json!(1));
    let (_, _, active) = send(&app, "GET", "/api/board?archived=0", None).await;
    assert!(active.as_array().unwrap().is_empty());

    // authorized_by is control, not "ignored"; and it unlocks cross-lane.
    let v2 = create(&app, json!({ "title": "theirs", "session": "lane-a" })).await;
    let id2 = v2["id"].as_str().unwrap().to_string();
    let (st, _, v3) = send_with(
        &app,
        "PATCH",
        &format!("/api/board/{id2}"),
        Some(json!({ "archived": "true", "authorized_by": "ethan" })),
        &[("X-Amux-Session", "lane-b")],
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v3}");
    assert_eq!(v3["archived"], json!(1));
    assert!(v3
        .get("ignored_fields")
        .and_then(|f| f.as_array())
        .map(|a| !a.iter().any(|x| x == "authorized_by"))
        .unwrap_or(true));

    // UN-archive is never gated — the un-do must stay reachable, even
    // cross-lane (restoring visibility is not destruction).
    let (st, _, r) = send_with(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "archived": 0 })),
        &[("X-Amux-Session", "lane-b")],
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{r}");
    assert_eq!(r["archived"], json!(0));
}

// ---- Python `archived` grammar + the tab-counter fetches (AMUX-2586 #5) --
//
// The SPA's board tab counters are fed by two fetches: the main list
// (`?archived=0`) and the archived merge (`?archived=1&done_limit=0`), and
// the full-text corpus by a BARE `?done_limit=0`. Python's grammar: absent
// or "" = NO filter; "1"/"true"/"yes" (lowercased) = archived-only; any
// other value = non-archived only. This pins all three against a fixture
// mixing archived x owned states, counting exactly what the SPA counts.

#[tokio::test]
async fn archived_grammar_matches_python_and_tab_counts_pin() {
    let (app, _dir) = app();
    // Fixture: 2 live owned, 3 live unowned, 4 archived unowned, 1 archived
    // owned. "Unowned" (the SPA chip) = open cards with no session.
    for i in 0..2 {
        create(&app, json!({ "title": format!("live owned {i}"), "session": "lane-a" })).await;
    }
    for i in 0..3 {
        create(&app, json!({ "title": format!("live unowned {i}"), "session": "" })).await;
    }
    let mut archived_ids = Vec::new();
    for i in 0..4 {
        let v = create(&app, json!({ "title": format!("arch unowned {i}"), "session": "" })).await;
        archived_ids.push(v["id"].as_str().unwrap().to_string());
    }
    let v = create(&app, json!({ "title": "arch owned", "session": "lane-a" })).await;
    archived_ids.push(v["id"].as_str().unwrap().to_string());
    for id in &archived_ids {
        let (st, _, _) =
            send(&app, "POST", &format!("/api/board/{id}/archive"), Some(json!({}))).await;
        assert_eq!(st, StatusCode::OK);
    }

    let count = |v: &Value| v.as_array().unwrap().len();
    let unowned_open = |v: &Value| {
        v.as_array()
            .unwrap()
            .iter()
            .filter(|i| {
                i["session"].as_str().unwrap_or("").is_empty()
                    && i["archived"].as_i64().unwrap_or(0) == 0
            })
            .count()
    };

    // Main SPA fetch: non-archived only.
    let (_, _, active) = send(&app, "GET", "/api/board?archived=0", None).await;
    assert_eq!(count(&active), 5);
    assert_eq!(unowned_open(&active), 3, "the Unowned chip's number");

    // Archived-merge fetch: archived rows ONLY (returning everything here
    // is what inflated the merged set the counters scan).
    let (_, _, arch) = send(&app, "GET", "/api/board?archived=1&done_limit=0", None).await;
    assert_eq!(count(&arch), 5);
    assert!(arch.as_array().unwrap().iter().all(|i| i["archived"] == json!(1)));

    // Case-insensitive truthy, Python's `.lower()`.
    let (_, _, arch2) = send(&app, "GET", "/api/board?archived=TRUE", None).await;
    assert_eq!(count(&arch2), 5);

    // Bare list (param absent): NO filter — the text-search corpus.
    let (_, _, all) = send(&app, "GET", "/api/board?done_limit=0", None).await;
    assert_eq!(count(&all), 10);

    // Python has no "all" spelling: any other value means non-archived.
    let (_, _, not_truthy) = send(&app, "GET", "/api/board?archived=all", None).await;
    assert_eq!(count(&not_truthy), 5);
    let (_, _, zero) = send(&app, "GET", "/api/board?archived=false", None).await;
    assert_eq!(count(&zero), 5);

    // The two counter feeds are disjoint and together cover the board.
    assert_eq!(count(&active) + count(&arch), count(&all));
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

    // A headerless force is REFUSED (Python parity: "force requires
    // attribution", amux-server.py ~70111). The refusal fires on `force`
    // itself, not on `eff_gate && force` — the ts-gke incident specimen was
    // an UNGATED transition, which a gate-conditioned check waves through.
    let card2 = create(&app, json!({ "title": "h2", "status": "doing" })).await;
    let id2 = card2["id"].as_str().unwrap().to_string();
    let (st, _, v) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id2}"),
        Some(json!({ "status": "done", "force": true })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(v["error"], json!("force requires attribution"));
    // And the card did not move.
    let (_, _, detail) = send(&app, "GET", &format!("/api/board/{id2}"), None).await;
    assert_eq!(detail["status"], json!("doing"));
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

    // Python's grammar: the BARE list has NO archived filter (the card is
    // still in it, flagged); `?archived=0` is the SPA's active view, which
    // excludes it; `?archived=1` finds it alone.
    let (_, _, list) = send(&app, "GET", "/api/board", None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list.as_array().unwrap()[0]["archived"], json!(1));
    let (_, _, list) = send(&app, "GET", "/api/board?archived=0", None).await;
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
    // Attributed force: this cell tests archived-immutability, not the
    // force-attribution refusal (which would 400 first and mask it).
    let (st, _, v3) = send_with(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "status": "done", "force": true })),
        &[("X-Amux-Session", "orch")],
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

    // Unknown keys are NAMED, never silently dropped (AC-263). `archived`
    // is no longer among them — it is a writable field since the AMUX-2492
    // parity port; `archived: 0` on an active card is an honest no-op.
    let (st, _, v) = send(
        &app,
        "PATCH",
        &format!("/api/board/{id}"),
        Some(json!({ "archived": 0, "bogus_key": true })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["applied"], json!(false));
    let ignored = v["ignored_fields"].as_array().unwrap();
    assert!(ignored.contains(&json!("bogus_key")) && !ignored.contains(&json!("archived")));

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

// ---- one-doing-per-session (AMUX-1707 parity) ----------------------------

#[tokio::test]
async fn second_doing_for_same_session_is_refused_with_named_escape() {
    let (app, _dir) = app();
    let first = create(
        &app,
        json!({ "title": "in flight", "status": "doing", "session": "lane-a" }),
    )
    .await;
    let second = create(
        &app,
        json!({ "title": "queued", "status": "todo", "session": "lane-a" }),
    )
    .await;
    let id2 = second["id"].as_str().unwrap().to_string();

    let (st, _, v) = send_with(
        &app,
        "PATCH",
        &format!("/api/board/{id2}"),
        Some(json!({ "status": "doing" })),
        &[("X-Amux-Session", "lane-a")],
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"], json!("already holding doing"));
    assert_eq!(v["holding"][0], first["id"]);
    // The escape must name the attributed CLI command (AMUX-2325).
    assert!(v["cli"].as_str().unwrap().contains("--override-doing"));

    // The named escape works, and a different session is never capped.
    let (st, _, v) = send_with(
        &app,
        "PATCH",
        &format!("/api/board/{id2}"),
        Some(json!({ "status": "doing", "override_doing": true, "gate_ack": true })),
        &[("X-Amux-Session", "lane-a")],
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");

    // Dormant types hold no WIP: a watch card in doing must not block.
    let third = create(
        &app,
        json!({ "title": "third", "status": "todo", "session": "lane-b" }),
    )
    .await;
    let id3 = third["id"].as_str().unwrap().to_string();
    let (st, _, v) = send_with(
        &app,
        "PATCH",
        &format!("/api/board/{id3}"),
        Some(json!({ "status": "doing", "gate_ack": true })),
        &[("X-Amux-Session", "lane-b")],
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
}

// ---- board status (column) mutations (the live 405, 2026-08-09) ----------

#[tokio::test]
async fn status_column_crud_matches_python() {
    let (app, _dir) = app();

    // PATCH on a builtin (the exact 405 repro: rename the review column).
    let (st, _, v) = send(
        &app,
        "PATCH",
        "/api/board/statuses/review",
        Some(json!({ "label": "In Review!" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], json!(true));

    // POST create -> slugified id, 201.
    let (st, _, v) = send(
        &app,
        "POST",
        "/api/board/statuses",
        Some(json!({ "label": "Waiting On Vendor" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{v}");
    assert_eq!(v["id"], json!("waiting-on-vendor"));

    // Reorder accepts the id.
    let (st, _, v) = send(
        &app,
        "PUT",
        "/api/board/statuses/reorder",
        Some(json!({ "order": ["waiting-on-vendor", "review"] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");

    // Builtin delete refused; custom delete moves cards to todo WITH an
    // audit line on each card (AMUX-2491).
    let (st, _, _) = send(&app, "DELETE", "/api/board/statuses/done", None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // Hand-INSERT a row in the custom status (the python-interop idiom):
    // rust card CREATE refuses statuses outside the typed vocabulary — a
    // known divergence from Python's dynamic columns, carded separately —
    // but rows in custom statuses EXIST in the shared DB and the column
    // delete must still audit + move them.
    let cid = "PY-777".to_string();
    {
        let conn = rusqlite::Connection::open(_dir.path().join("amux-test.db")).unwrap();
        conn.execute(
            "INSERT INTO issues (id, title, status, created, updated) \
             VALUES ('PY-777', 'stranded', 'waiting-on-vendor', 1786300000, 1786300000)",
            [],
        )
        .unwrap();
    }
    let (st, _, v) = send(&app, "DELETE", "/api/board/statuses/waiting-on-vendor", None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["moved"], json!(1));
    let (_, _, detail) = send(&app, "GET", &format!("/api/board/{cid}"), None).await;
    assert_eq!(detail["status"], json!("todo"));
    assert!(detail["log"]
        .as_str()
        .unwrap()
        .contains("column 'waiting-on-vendor' deleted by"));
}
