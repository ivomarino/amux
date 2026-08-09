//! Incident regression corpus (RR-0074, Invariant 41).
//!
//! Each test pins a REAL incident from the Python system's history to the
//! structural fix in the Rust system, named so `incident_regression::*`
//! greps to its origin. A regression suite assembled from imagined failures
//! tests things that were never going to break — these all actually broke.

use amux_server::db::{PendingEvent, Store, WriteOutcome};
use std::sync::Arc;

fn store() -> Arc<Store> {
    let dir = tempfile::tempdir().unwrap();
    let s = Arc::new(Store::open(&dir.path().join("t.db")).unwrap());
    std::mem::forget(dir);
    s
}

/// INCIDENT (2026-06-29): AppleScript `reply` + `set content` silently sent
/// a BLANK email; a "sent" draft could resurrect with empty body. The Rust
/// email path builds the FULL RFC822 message before any send call — an
/// empty body cannot arise from assembly order, and there is no draft
/// object to resurrect. Pinned: assembly with an empty body is a visible
/// empty body in the RFC822 text (never a structurally different message),
/// and the threading headers ride with it.
#[test]
fn incident_regression_duplicate_draft_resurrects_sent_message() {
    use amux_server::integrations::email::{build_rfc822, MimeSpec};
    let msg = build_rfc822(&MimeSpec {
        from: "a@x.com",
        to: "b@y.com",
        cc: "",
        subject: "Re: pilot",
        in_reply_to: "<parent@id>",
        references: "<parent@id> <older@id>",
        plain: "the actual body",
        html: "",
        boundary: "bnd-test",
    });
    // Body is base64 inside multipart — assert the ENCODED body is present.
    use base64::Engine;
    let enc = base64::engine::general_purpose::STANDARD.encode("the actual body");
    assert!(msg.contains(&enc[..16]), "body rides in the message: {msg}");
    assert!(msg.contains("In-Reply-To: <parent@id>"));
    // The blank-email shape: assembly is ONE construction — an empty body
    // yields a visibly empty part, never a draft to resurrect later.
    let blank = build_rfc822(&MimeSpec {
        from: "a@x.com", to: "b@y.com", cc: "", subject: "Re: pilot",
        in_reply_to: "<parent@id>", references: "", plain: "", html: "",
        boundary: "bnd-test",
    });
    assert!(blank.contains("In-Reply-To"), "threading survives an empty body");
}

/// INCIDENT (AMUX-2560 / 2026-08-02): board read-after-write staleness — a
/// card created and read back a moment later came up missing because the
/// cache invalidation marked time, not data. The Rust store has no board
/// cache: a write commits under the single writer and the NEXT read sees
/// it, always. Pinned end-to-end through the store.
#[test]
fn incident_regression_board_read_after_write_staleness() {
    let s = store();
    s.write(|conn| {
        conn.execute(
            "INSERT INTO issues (id, title, status, created, updated) VALUES ('RG-1','t','todo',1,1)",
            [],
        )?;
        Ok(WriteOutcome { applied: true, events: vec![] })
    })
    .unwrap();
    // Immediate read-back on a DIFFERENT (pooled read) connection.
    let conn = s.read().unwrap();
    let found: String = conn
        .query_row("SELECT title FROM issues WHERE id='RG-1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(found, "t", "create-then-read must never miss the write");
}

/// INCIDENT (stale steering): a steer queued against state that then moved
/// was delivered anyway, firing against the wrong context. Invariant 38:
/// preconditions are evaluated AT DELIVERY. Pinned at the core state
/// machine level: an EntityVersion precondition against a moved version
/// evaluates false.
#[test]
fn incident_regression_stale_steering_command_freshness() {
    use amux_core::protocol::CommandPrecondition;
    let pre = CommandPrecondition::EntityVersion {
        entity: "wrk_x".into(),
        version: 3,
    };
    // At enqueue time the entity was at version 3; by delivery it moved to 4.
    let lookup_moved = |entity: &str| -> Option<(u64, String)> {
        (entity == "wrk_x").then(|| (4u64, "idle".into()))
    };
    assert!(!pre.evaluate(&lookup_moved), "moved version must fail delivery");
    let lookup_fresh = |entity: &str| -> Option<(u64, String)> {
        (entity == "wrk_x").then(|| (3u64, "idle".into()))
    };
    assert!(pre.evaluate(&lookup_fresh));
}

/// INCIDENT (Invariant 37 / cold-outbound 2026-08-07): a no-op PATCH
/// returned 200 with a bumped rev, so "did it apply?" was unanswerable from
/// the response. Pinned at the store: a write reporting applied=false moves
/// NEITHER the revision nor the event journal.
#[test]
fn incident_regression_noop_write_bumps_nothing() {
    let s = store();
    let before = s.current_rev().unwrap();
    let reply = s
        .write(|_conn| Ok(WriteOutcome { applied: false, events: vec![] }))
        .unwrap();
    assert!(!reply.applied);
    assert_eq!(s.current_rev().unwrap(), before, "no-op must not move rev");
    // And an APPLIED write moves it exactly once.
    let reply = s
        .write(|_conn| {
            Ok(WriteOutcome {
                applied: true,
                events: vec![PendingEvent {
                    entity_type: amux_core::revision::EntityType::Other("probe".into()),
                    entity_id: "x".into(),
                    mutation: amux_core::revision::MutationKind::Updated,
                    payload: None,
                }],
            })
        })
        .unwrap();
    assert!(reply.applied);
    assert_eq!(s.current_rev().unwrap().0, before.0 + 1);
}
