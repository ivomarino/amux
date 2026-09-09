//! AR-135's single-flight guard on `legacy_sessions_array` only ever covered the
//! WARM path (TTL expired, cache non-empty). A COLD cache — true on every
//! restart — hit `try_lock`'s `Err` arm, found `c.json` empty, and fell through
//! to an INDEPENDENT build: one per concurrent caller, each holding a pooled
//! read connection across ~100 tmux/git subprocesses at once. That is the exact
//! N-builders-one-pool failure AR-135 exists to prevent, just gated on "cache
//! empty" instead of "TTL expired" — and worse, because a restart is exactly
//! when every dashboard/fleet client reconnects and hits this endpoint at once.
//!
//! FIRST FIX (2026-09-09 morning): a loser on a cold cache waits (bounded 3s)
//! for the in-flight build instead of racing it, falling back to an
//! independent build past the deadline. Confirmed live the SAME day,
//! afternoon: that bound alone does not bound the builder COUNT. Under
//! SUSTAINED reconnect pressure (not one instantaneous burst — the real shape
//! of a restart), every new wave of waiters can independently miss the same
//! 3s deadline and each spin up its own build, stacking faster than any of
//! them finish. read_pool_exhausted recurred in bursts for minutes; box load
//! average hit 62 on 4 cores; amux-server-rs itself sat at 400%+ CPU.
//!
//! SECOND FIX: cap the number of concurrent independent builds at a CONSTANT
//! — 2 (the primary FLIGHT plus exactly one FALLBACK_FLIGHT) — no matter how
//! many requests arrive or how long they keep arriving. A waiter that cannot
//! get either lock keeps waiting rather than building a third copy. The wait
//! is bounded overall (15s) purely as a fail-SAFE: past that bound the
//! function returns an honest error (-> 500, which the dashboard's existing
//! degraded-UI banner already renders as "Worker updates are unavailable ...
//! Retry") instead of piling a third builder onto a pool that is, by
//! definition, already struggling if the first two haven't finished in 15s.
//!
//! Asserted on the SOURCE, matching this file's sibling
//! `sessions_list_off_runtime.rs`: a timing test for "did the stampede
//! collapse under sustained load" needs a controllable hang across ~100
//! subprocesses sustained over many seconds and would be the flakiest thing
//! in the suite. The property that regressed both times was textual — the
//! branch built an unbounded number of independent copies — and that is
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
        "the ORIGINAL cold-start failure is back: a loser on an empty cache must not build \
         independently the instant try_lock fails"
    );
}

#[test]
fn cold_sessions_cache_caps_independent_builders_at_two_not_unbounded() {
    assert!(
        SRC.contains("pub fn legacy_sessions_array"),
        "premise gone: the sync builder is not in this file any more"
    );
    assert!(
        SRC.contains("static FALLBACK_FLIGHT: std::sync::Mutex<()>"),
        "the second-generation fix is gone: without a SEPARATE fallback lock, a bounded wait \
         alone does not bound the builder COUNT under sustained (not just instantaneous) load \
         — every new wave of waiters can independently miss the same deadline and each start \
         its own build, which is exactly what recurred live on 2026-09-09 (read_pool_exhausted \
         for minutes, box load average 62 on 4 cores, amux-server-rs at 400%+ CPU)"
    );
    assert!(
        SRC.contains("FALLBACK_FLIGHT.try_lock()"),
        "a waiter that cannot get the PRIMARY flight lock must try the ONE fallback lock, not \
         build a third independent copy"
    );
    assert_eq!(
        SRC.matches("let conn = store.read()?;").count(),
        1,
        "there must be exactly ONE build call site left (the normal post-flight-lock build). \
         A second copy inside the wait loop is the original unbounded independent-build \
         fallback creeping back in outside the two-lock cap"
    );
    assert!(
        SRC.contains("anyhow::bail!"),
        "when BOTH the primary and fallback builders are still busy past the overall bound, \
         the function must FAIL SAFE (an honest error the dashboard already renders as a \
         retryable banner) rather than start a third builder on an already-struggling pool"
    );
    assert!(
        SRC.contains("sessions_cache_stuck"),
        "the fail-safe bail-out must log a verdict a sweep can grep for — silent failure here \
         is how the first version of this guard regressed unnoticed"
    );
}
