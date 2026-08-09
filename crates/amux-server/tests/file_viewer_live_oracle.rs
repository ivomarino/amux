//! LIVE-ORACLE comparison for the AMUX-2598 file-viewer cutover: the same
//! GETs against the live Python server (:8822) and the native Rust router
//! in-process, diffed. `#[ignore]` because it needs the live server; run on
//! the dev box with:
//!
//!   CARGO_TARGET_DIR=/tmp/amux-cutover-fv \
//!     cargo test -p amux-server --test file_viewer_live_oracle -- --ignored --nocapture
//!
//! SAFETY, per the shared-checkout rules:
//! - live traffic is GET-only: /api/file, /api/file/raw, /api/file/vtt,
//!   /api/library — read-only viewer endpoints;
//! - fixtures live in a tempdir BOTH origins read (same machine); the one
//!   real file touched is a media file found under ~/Dev, read via a Range
//!   GET only;
//! - /health `build` is read before and after — a moved build invalidates
//!   the run (the 2026-08-08 mid-measurement-restart lesson).
//!
//! Deliberately NOT compared: /api/file/prepare + /transcode (they spawn
//! ffmpeg work on the live server — not read-only), and renderable ebooks
//! (the native origin answers a documented honest 501 there; see
//! docs/rust-migration/server-boundary.md).

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

fn native_app() -> axum::Router {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("t.db")).unwrap();
    std::mem::forget(dir);
    router(AppState {
        store: std::sync::Arc::new(store),
        started: std::time::Instant::now(),
        build_hash: "oracle".into(),
        auth_token: None,
    })
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

struct Got {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Got {
    fn header(&self, k: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str())
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

async fn live_get(path: &str, range: Option<&str>) -> Got {
    let mut req = client().get(format!("{PY}{path}"));
    if let Some(r) = range {
        req = req.header("range", r);
    }
    let resp = req.send().await.expect("live python GET");
    let status = resp.status().as_u16();
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body = resp.bytes().await.expect("live body").to_vec();
    Got { status, headers, body }
}

async fn native_get(app: &axum::Router, path: &str, range: Option<&str>) -> Got {
    let mut b = Request::builder().uri(path);
    if let Some(r) = range {
        b = b.header("range", r);
    }
    let res = app.clone().oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status().as_u16();
    let headers = res
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap().to_vec();
    Got { status, headers, body }
}

/// Bounded read-only hunt for a media file under ~/Dev (depth ≤ 4, entry
/// budget) — the range-request oracle wants a real, large-ish binary.
fn find_media_under_dev() -> Option<std::path::PathBuf> {
    let root = std::path::PathBuf::from(std::env::var("HOME").ok()?).join("Dev");
    let mut queue = std::collections::VecDeque::from([(root, 0usize)]);
    let mut budget = 30_000usize;
    let exts = ["mp4", "mov", "m4v", "webm", "mp3", "m4a", "png", "jpg"];
    let mut best: Option<(u64, std::path::PathBuf)> = None;
    while let Some((dir, depth)) = queue.pop_front() {
        if budget == 0 {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            budget = budget.saturating_sub(1);
            if budget == 0 {
                break;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" || name == "target" {
                continue;
            }
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if depth < 4 {
                    queue.push_back((p, depth + 1));
                }
            } else if p
                .extension()
                .map(|x| exts.contains(&x.to_string_lossy().to_lowercase().as_str()))
                .unwrap_or(false)
            {
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                if size > 200_000 && best.as_ref().map(|(s, _)| size > *s).unwrap_or(true) {
                    best = Some((size, p));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

fn assert_headers_agree(path: &str, py: &Got, rs: &Got, keys: &[&str]) {
    for k in keys {
        assert_eq!(
            py.header(k),
            rs.header(k),
            "{path}: header {k} disagrees (py {:?} vs native {:?})",
            py.header(k),
            rs.header(k)
        );
    }
}

#[tokio::test]
#[ignore = "needs the live python server on :8822 — run manually, GET-only"]
async fn live_python_and_native_rust_agree_on_file_viewer() {
    let h0 = live_get("/health", None).await;
    assert_eq!(h0.status, 200, "live python server on :8822 unreachable — this oracle needs it");
    let build0 = h0.json()["build"].as_str().unwrap_or("").to_string();

    let app = native_app();
    let base = std::env::temp_dir().join(format!("amux-fv-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let mut agreed = 0usize;

    // ---- /api/file: text / markdown / csv / binary / image / video card
    std::fs::write(base.join("a.txt"), "hello world\nsecond line\ncafé ✓\n").unwrap();
    std::fs::write(base.join("doc.md"), "# title\n\nbody\n").unwrap();
    std::fs::write(base.join("t.csv"), "a,b\n1,2\n").unwrap();
    std::fs::write(base.join("page.html"), "<h1>hi</h1>\n").unwrap();
    std::fs::write(base.join("blob.dat"), b"head\x00tail-binary").unwrap();
    // Tiny valid-enough PNG payload (content is not validated, only typed).
    std::fs::write(base.join("pic.png"), b"\x89PNG\r\n\x1a\nfake-pixels").unwrap();
    // Over the 2MB inline cap: viewer must answer raw_url instead.
    {
        let f = std::fs::File::create(base.join("big.jpg")).unwrap();
        f.set_len(2_500_000).unwrap();
    }
    std::fs::write(base.join("clip.mp4"), b"not-really-mp4-but-typed-as-one").unwrap();
    std::fs::write(base.join("clip.srt"), "1\n00:00:01,000 --> 00:00:02,000\nhi\n").unwrap();
    std::fs::write(base.join("clip.json"), r#"{"profile":"default","task":"demo"}"#).unwrap();
    // Download-only ebook: both origins serve the download card.
    std::fs::write(base.join("book.azw3"), b"proprietary").unwrap();

    for name in
        ["a.txt", "doc.md", "t.csv", "page.html", "blob.dat", "pic.png", "big.jpg", "clip.mp4",
         "book.azw3"]
    {
        let p = base.join(name);
        let path = format!("/api/file?path={}", urlenc(p.to_str().unwrap()));
        let (py, rs) = (live_get(&path, None).await, native_get(&app, &path, None).await);
        assert_eq!(py.status, rs.status, "{path}: status (py body: {:?})", py.json());
        assert_eq!(py.json(), rs.json(), "{path}: payload disagrees");
        agreed += 1;
    }

    // Error shapes: missing file, directory, sensitive path, relative w/o cwd.
    for path in [
        format!("/api/file?path={}", urlenc(base.join("nope.txt").to_str().unwrap())),
        format!("/api/file?path={}", urlenc(base.to_str().unwrap())),
        format!(
            "/api/file?path={}",
            urlenc(&format!("{}/.ssh/id_rsa", std::env::var("HOME").unwrap()))
        ),
        "/api/file?path=relative.txt".to_string(),
        "/api/file?path=".to_string(),
    ] {
        let (py, rs) = (live_get(&path, None).await, native_get(&app, &path, None).await);
        assert_eq!(py.status, rs.status, "{path}: status");
        assert_eq!(py.json(), rs.json(), "{path}: error payload");
        agreed += 1;
    }

    // ---- /api/file/raw on a text fixture: full-body + headers
    let txt = base.join("a.txt");
    let rawq = format!("/api/file/raw?path={}", urlenc(txt.to_str().unwrap()));
    let (py, rs) = (live_get(&rawq, None).await, native_get(&app, &rawq, None).await);
    assert_eq!(py.status, 200);
    assert_eq!(rs.status, 200);
    assert_headers_agree(&rawq, &py, &rs, &[
        "content-type", "content-disposition", "content-length", "accept-ranges", "etag",
        "cache-control",
    ]);
    assert_eq!(py.body, rs.body, "{rawq}: body bytes");
    agreed += 1;

    // Range on the text fixture (exact 206 semantics).
    let (py, rs) =
        (live_get(&rawq, Some("bytes=3-9")).await, native_get(&app, &rawq, Some("bytes=3-9")).await);
    assert_eq!(py.status, 206);
    assert_eq!(rs.status, 206);
    assert_headers_agree(&rawq, &py, &rs, &[
        "content-type", "content-range", "content-length", "accept-ranges", "etag",
        "cache-control", "content-disposition",
    ]);
    assert_eq!(py.body, rs.body, "{rawq}: range bytes");
    agreed += 1;

    // ---- Range request on a REAL media file under ~/Dev (read-only).
    if let Some(media) = find_media_under_dev() {
        let mq = format!("/api/file/raw?path={}", urlenc(media.to_str().unwrap()));
        for range in ["bytes=0-65535", "bytes=100000-", "bytes=0-"] {
            let (py, rs) =
                (live_get(&mq, Some(range)).await, native_get(&app, &mq, Some(range)).await);
            assert_eq!(py.status, 206, "{}: py status for {range}", media.display());
            assert_eq!(rs.status, 206, "{}: native status for {range}", media.display());
            assert_headers_agree(&mq, &py, &rs, &[
                "content-type", "content-range", "content-length", "accept-ranges", "etag",
                "cache-control", "content-disposition",
            ]);
            // Compare a bounded prefix (open-ended ranges can be huge).
            let n = 65_536.min(py.body.len()).min(rs.body.len());
            assert_eq!(py.body[..n], rs.body[..n], "{mq} {range}: body prefix");
            assert_eq!(py.body.len(), rs.body.len(), "{mq} {range}: body length");
            agreed += 1;
        }
        println!("range oracle media file: {}", media.display());
    } else {
        panic!("no media file under ~/Dev for the range oracle — widen find_media_under_dev");
    }

    // ---- /api/file/vtt
    let vq = format!("/api/file/vtt?path={}", urlenc(base.join("clip.srt").to_str().unwrap()));
    let (py, rs) = (live_get(&vq, None).await, native_get(&app, &vq, None).await);
    assert_eq!(py.status, rs.status, "{vq}");
    assert_headers_agree(&vq, &py, &rs, &["content-type", "cache-control"]);
    assert_eq!(py.body, rs.body, "{vq}: vtt body");
    agreed += 1;

    // ---- /api/library: opf-scan fixture dir + empty dir + error shapes
    let lib = base.join("library/Author");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("Dune - Frank Herbert.epub"), b"e").unwrap();
    std::fs::write(lib.join("Dune - Frank Herbert.mobi"), b"mm").unwrap();
    std::fs::write(lib.join("cover.jpg"), b"jpg").unwrap();
    std::fs::write(
        lib.join("metadata.opf"),
        r#"<?xml version="1.0"?><package xmlns:dc="http://purl.org/dc/elements/1.1/">
           <metadata><dc:title>Dune</dc:title><dc:creator>Frank Herbert</dc:creator>
           <dc:subject>scifi</dc:subject>
           <meta name="calibre:series" content="Dune Chronicles"/></metadata></package>"#,
    )
    .unwrap();
    let empty = base.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    for path in [
        format!("/api/library?path={}", urlenc(base.join("library").to_str().unwrap())),
        format!("/api/library?path={}", urlenc(empty.to_str().unwrap())),
        format!("/api/library?path={}", urlenc("/nope/lib")),
        "/api/library?path=".to_string(),
    ] {
        let (py, rs) = (live_get(&path, None).await, native_get(&app, &path, None).await);
        assert_eq!(py.status, rs.status, "{path}: status");
        assert_eq!(py.json(), rs.json(), "{path}: payload disagrees");
        agreed += 1;
    }

    // ---- build bracket: a moved build invalidates every comparison above.
    let h1 = live_get("/health", None).await;
    let build1 = h1.json()["build"].as_str().unwrap_or("").to_string();
    assert_eq!(
        build0, build1,
        "INVALID RUN: python build moved mid-measurement — two different servers were compared"
    );
    println!("file-viewer oracle: {agreed} request pairs agreed; build bracket held ({build0})");
    let _ = std::fs::remove_dir_all(&base);
}
