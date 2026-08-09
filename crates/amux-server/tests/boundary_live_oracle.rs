//! LIVE-ORACLE comparison for the AMUX-2597 nativized families: the same
//! GET requests against the live Python server (:8822) and the native Rust
//! router in-process, diffed. `#[ignore]` because it needs the live server;
//! run on the dev box with:
//!
//!   CARGO_TARGET_DIR=/tmp/amux-boundary-target \
//!     cargo test -p amux-server --test boundary_live_oracle -- --ignored --nocapture
//!
//! SAFETY, per the shared-checkout rules:
//! - LIVE server traffic is GET-only (list/read/search/ls/groups/config);
//! - the live DB is opened READ-ONLY to copy group_config rows into a temp
//!   store — no Store::open (whose migrations WRITE) ever touches
//!   ~/.amux/amux.db;
//! - AMUX_HOME points at the real ~/.amux only for env-file READS
//!   (parse_env_file), and is restored after;
//! - the /health `build` hash is read before and after and the run is
//!   invalidated if it moved (the 2026-08-08 lesson: a mid-measurement
//!   restart corroborates the wrong conclusion).

use amux_server::api::{router, AppState};
use amux_server::db::Store;
use axum::body::Body;
use axum::http::Request;
use serde_json::Value;
use tower::ServiceExt;

const PY: &str = "https://localhost:8822";

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap()
}

async fn live_get(path: &str, session_hdr: Option<&str>) -> Option<(u16, Value)> {
    let mut req = client().get(format!("{PY}{path}"));
    if let Some(s) = session_hdr {
        req = req.header("X-Amux-Session", s);
    }
    let resp = req.send().await.ok()?;
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.ok()?;
    Some((status, body))
}

async fn native_get(app: &axum::Router, path: &str, session_hdr: Option<&str>) -> (u16, Value) {
    let mut b = Request::builder().uri(path);
    if let Some(s) = session_hdr {
        b = b.header("X-Amux-Session", s);
    }
    let res = app.clone().oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn normalize(v: &Value) -> Value {
    match v {
        Value::Array(items) => {
            let mut arr: Vec<Value> = items.iter().map(normalize).collect();
            if !arr.is_empty() && arr.iter().all(|i| i.get("line").is_some() && i.get("path").is_some()) {
                arr.sort_by_key(|i| {
                    (i["path"].as_str().unwrap_or("").to_string(), i["line"].as_i64().unwrap_or(0))
                });
            }
            Value::Array(arr)
        }
        Value::Object(map) => Value::Object(
            map.iter()
                // key_valid/key_error: python serves its background
                // validator's cache; rust deliberately answers the
                // pre-validation null/"" (see server-boundary.md).
                .filter(|(k, _)| !matches!(k.as_str(), "elapsed_ms" | "key_valid" | "key_error"))
                .map(|(k, val)| (k.clone(), normalize(val)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[tokio::test]
#[ignore = "needs the live python server on :8822 — run manually, GET-only"]
async fn live_python_and_native_rust_agree() {
    let Some((_, h0)) = live_get("/health", None).await else {
        panic!("live python server on :8822 unreachable — this oracle needs it (loud skip)");
    };
    let build0 = h0["build"].as_str().unwrap_or("").to_string();

    // Scratch tree both origins read (same machine, absolute paths).
    let base = std::env::temp_dir().join(format!("amux-boundary-oracle-{}", std::process::id()));
    let root = base.join("fstree");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.txt"), "hello world\nsecond line with needle here\n").unwrap();
    std::fs::write(root.join("bin.dat"), b"h\xc3\xa9llo binary\x00tail").unwrap();
    std::fs::write(root.join("sub/nested.md"), "needle in sub\n").unwrap();
    std::fs::write(root.join("utf8.txt"), "café\n").unwrap();
    let root_s = root.canonicalize().unwrap().to_string_lossy().into_owned();

    // Native app: REAL ~/.amux for env-file reads; TEMP store seeded with a
    // READ-ONLY copy of the live group_config rows.
    let real_home = format!("{}/.amux", std::env::var("HOME").unwrap());
    std::env::set_var("AMUX_HOME", &real_home);
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("t.db")).unwrap();
    let live_rows: Vec<(String, String, String, String, i64)> = {
        let live = rusqlite::Connection::open_with_flags(
            format!("{real_home}/amux.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("open live db read-only");
        let mut stmt = live
            .prepare("SELECT name, department, goal, kpis, human_cost FROM group_config")
            .expect("group_config exists in live db");
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .flatten()
            .collect();
        rows
    };
    let seed = live_rows.clone();
    store
        .write(move |conn| {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS group_config (name TEXT PRIMARY KEY,
                 department TEXT NOT NULL DEFAULT '', goal TEXT NOT NULL DEFAULT '',
                 kpis TEXT NOT NULL DEFAULT '[]', human_cost INTEGER NOT NULL DEFAULT 0,
                 updated INTEGER NOT NULL DEFAULT 0)",
                [],
            )?;
            for (n, d, g, k, h) in &seed {
                conn.execute(
                    "INSERT OR REPLACE INTO group_config (name, department, goal, kpis, human_cost, updated) VALUES (?1,?2,?3,?4,?5,0)",
                    rusqlite::params![n, d, g, k, h],
                )?;
            }
            Ok(amux_server::db::WriteOutcome { applied: true, events: vec![] })
        })
        .unwrap();
    let app = router(AppState {
        store: std::sync::Arc::new(store),
        started: std::time::Instant::now(),
        build_hash: "oracle".into(),
        auth_token: None,
    });

    let enc_root = urlenc(&root_s);
    let mut compared = 0;
    let mut diffs: Vec<String> = vec![];
    let paths = vec![
        format!("/api/fs/list?path={enc_root}"),
        format!("/api/fs/read?path={enc_root}%2Fa.txt"),
        format!("/api/fs/read?path={enc_root}%2Fbin.dat"),
        format!("/api/fs/read?path={enc_root}%2Futf8.txt&max_bytes=4"),
        format!("/api/fs/search?path={enc_root}&q=needle"),
        format!("/api/fs/search?path={enc_root}&q=ne.dle&literal=0"),
        format!("/api/fs/search?path={enc_root}&q=needle&glob=*.md"),
        format!("/api/ls?path={enc_root}"),
        format!("/api/ls?path={enc_root}&hidden=1"),
        format!("/api/autocomplete/dir?q={enc_root}%2Fs"),
        "/api/groups".to_string(),
        "/api/tags".to_string(),
        "/api/identity".to_string(),
    ];
    let mut all = paths;
    // Per-group config for every live group (GET-only).
    if let Some((_, groups)) = live_get("/api/groups", None).await {
        for g in groups["groups"].as_array().unwrap_or(&vec![]) {
            if let Some(name) = g["name"].as_str() {
                all.push(format!("/api/groups/{}/config", urlenc(name)));
            }
        }
    }
    for path in &all {
        let Some((ls, lb)) = live_get(path, None).await else {
            diffs.push(format!("{path}: live GET failed"));
            continue;
        };
        let (ns, nb) = native_get(&app, path, None).await;
        if ls != ns || normalize(&lb) != normalize(&nb) {
            diffs.push(format!(
                "{path}\n  live   {ls}: {}\n  native {ns}: {}",
                serde_json::to_string(&normalize(&lb)).unwrap(),
                serde_json::to_string(&normalize(&nb)).unwrap()
            ));
        } else {
            compared += 1;
        }
    }

    // Scoped groups view: first tagged + first untagged live session.
    let sessions_dir = std::path::Path::new(&real_home).join("sessions");
    let mut tagged = None;
    let mut untagged = None;
    for e in std::fs::read_dir(&sessions_dir).unwrap().flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("env") {
            continue;
        }
        let has_tags = std::fs::read_to_string(&p)
            .unwrap_or_default()
            .lines()
            .any(|l| l.starts_with("CC_TAGS=") && l.trim_end() != "CC_TAGS=" && l.trim_end() != "CC_TAGS=\"\"");
        let name = p.file_stem().unwrap().to_string_lossy().into_owned();
        if has_tags && tagged.is_none() {
            tagged = Some(name);
        } else if !has_tags && untagged.is_none() {
            untagged = Some(name);
        }
        if tagged.is_some() && untagged.is_some() {
            break;
        }
    }
    for caller in [tagged, untagged, Some("no-such-session-oracle".into())].into_iter().flatten() {
        let Some((ls, lb)) = live_get("/api/groups", Some(&caller)).await else { continue };
        let (ns, nb) = native_get(&app, "/api/groups", Some(&caller)).await;
        if ls != ns || normalize(&lb) != normalize(&nb) {
            diffs.push(format!(
                "/api/groups as {caller}\n  live   {ls}: {}\n  native {ns}: {}",
                serde_json::to_string(&normalize(&lb)).unwrap(),
                serde_json::to_string(&normalize(&nb)).unwrap()
            ));
        } else {
            compared += 1;
        }
    }

    std::env::remove_var("AMUX_HOME");
    std::fs::remove_dir_all(&base).ok();

    // Measurement bracket: the live server must not have changed under us.
    let (_, h1) = live_get("/health", None).await.expect("health recheck");
    let build1 = h1["build"].as_str().unwrap_or("");
    assert_eq!(
        build0, build1,
        "INVALID RUN: python build moved during the oracle — remeasure"
    );

    println!("live-oracle: {compared} paths agreed; {} diffs", diffs.len());
    assert!(
        diffs.is_empty(),
        "live python and native rust disagree:\n{}",
        diffs.join("\n---\n")
    );
    assert!(compared >= 12, "too few comparisons ran ({compared})");
}
