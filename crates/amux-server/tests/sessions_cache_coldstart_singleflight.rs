//! AR-135's single-flight guard on `legacy_sessions_array` only ever covered the
//! WARM path (TTL expired, cache non-empty). A COLD cache — true on every
//! restart — hit `try_lock`'s `Err` arm, found `c.json` empty, and fell through
//! to an INDEPENDENT build: one per concurrent caller, each holding a pooled
//! read connection across ~100 tmux/git subprocesses at once. That is the exact
//! N-builders-one-pool failure AR-135 exists to prevent, just gated on "cache
//! empty" instead of "TTL expired" — and worse, because a restart is exactly
//! when every dashboard/fleet client reconnects and hits this endpoint at once.
//!
//! Confirmed live 2026-09-09: a post-restart reconnect burst held
//! `read_pool_exhausted` for minutes (152 failures counted in one 60s window),
//! surfacing to users as the dashboard's "Worker updates are unavailable"
//! banner. The fix: a loser on a cold cache now WAITS (bounded, so a genuinely
//! hung builder per AF-301 cannot hang every waiter forever) for the in-flight
//! build's result instead of racing it.
//!
//! Asserted on the SOURCE, matching this file's sibling
//! `sessions_list_off_runtime.rs`: a timing test for "did the stampede
//! collapse" needs a controllable hang in ~100 subprocesses and would be the
//! flakiest test in the suite. The property that regressed is textual —
//! the cold-start branch built independently with no wait — and that is
//! exactly what this catches.

const SRC: &str = include_str!("../src/api/sessions_legacy.rs");

#[test]
fn cold_sessions_cache_waits_for_the_inflight_builder_instead_of_racing_it() {
    // CONTROL FIRST: if this moves or gets renamed the assertions below would
    // pass vacuously against a file that no longer contains the thing at all.
    assert!(
        SRC.contains("pub fn legacy_sessions_array"),
        "premise gone: the sync builder is not in this file any more"
    );
    assert!(
        SRC.contains("static FLIGHT: std::sync::Mutex<()>"),
        "premise gone: the single-flight guard is not in this file any more"
    );

    assert!(
        !SRC.contains("Cold start with a builder already in flight: fall through and build"),
        "the old cold-start failure is back: a loser on an empty cache must not build \
         independently the instant try_lock fails — that is N concurrent builders each \
         holding a read connection across ~100 subprocesses, on a cold cache which is \
         true on every restart (the worst possible moment, since that is when every \
         client reconnects and hits this endpoint at once)"
    );
    assert!(
        SRC.contains("acquired = Some(g)"),
        "a loser on a cold cache must retry for the in-flight builder's own result \
         (bounded) rather than immediately racing it into a second build"
    );
    assert!(
        SRC.contains("sessions_cache_coldstart_stampede"),
        "the bounded-wait giveup path must log a verdict a sweep can grep for — silent \
         failure here is how this regressed unnoticed the first time"
    );
}
