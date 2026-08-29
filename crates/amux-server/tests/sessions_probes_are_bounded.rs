//! Every subprocess the fleet list spawns must be bounded (AF-301).
//!
//! `GET /api/sessions` shells out to enumerate the fleet. Five of those calls
//! were bare `.output()`, which blocks until the child exits with no timeout —
//! and one of them, a `pgrep`, ran once per shell pane, so a 50-lane fleet did
//! ~50 unbounded spawns per cache miss (TTL 2s).
//!
//! MEASURED: /api/sessions max latency was 697,890ms in the 24h window of
//! 2026-08-28, across eight concurrent requests that all ended within one
//! second of each other — the shape of several callers blocked on the same
//! wedged external process. That starved the runtime and 500'd the dashboard
//! (AF-300).
//!
//! The file has carried `run_bounded` the whole time, whose own WARN says
//! "capture blocked GET /api/sessions for as long as tmux took". The fix
//! existed; these call sites bypassed it. This test is why they cannot again.

const SRC: &str = include_str!("../src/api/sessions_legacy.rs");

/// Lines that actually RUN a command, ignoring comments — a `.output()` named
/// in a comment explaining why it is gone must not fail this.
fn code_lines_with_bare_output() -> Vec<(usize, String)> {
    SRC.lines()
        .enumerate()
        .filter(|(_, l)| {
            let t = l.trim_start();
            // Prose, not code: this file's comments and WARN strings QUOTE
            // `.output()` when explaining why a call is gone, and a string
            // continuation is not a comment so a prefix test alone misses it.
            // Backticks mark prose in this codebase; a real call site has none.
            !t.starts_with("//")
                && !t.starts_with("///")
                && !t.contains('`')
                && t.contains(".output()")
        })
        .map(|(i, l)| (i + 1, l.trim().to_string()))
        .collect()
}

#[test]
fn the_fleet_list_spawns_nothing_unbounded() {
    // CONTROL: the helper the assertion depends on must still be here, or this
    // would pass against a file that had lost its bounding entirely.
    assert!(
        SRC.contains("fn run_bounded("),
        "premise gone: run_bounded is not in this file, so `.output()` being absent proves nothing"
    );
    assert!(
        SRC.contains("fn run_bounded_output("),
        "premise gone: run_bounded_output is not in this file"
    );
    assert!(
        SRC.contains("fn probe_budget()"),
        "premise gone: the probe budget is not in this file"
    );

    let offenders = code_lines_with_bare_output();
    assert!(
        offenders.is_empty(),
        "unbounded subprocess call(s) in the fleet-list path — each blocks until the child \
         exits, which is how one wedged tmux held GET /api/sessions for 697s and took the \
         dashboard down (AF-300/AF-301). Use run_bounded / run_bounded_output / \
         capture_pane_bounded: {offenders:?}"
    );
}

/// The per-pane `pgrep` needs a TOTAL deadline, not only a per-call one: fifty
/// calls each just under budget is still minutes.
#[test]
fn the_per_pane_child_probe_has_a_total_budget() {
    assert!(
        SRC.contains("let probe_start = std::time::Instant::now();"),
        "the pgrep loop lost its total-budget clock"
    );
    assert!(
        SRC.contains("pgrep_skipped"),
        "the pgrep loop must COUNT what it skipped: a lane dropped there reads as shell-only, \
         so silent truncation makes the fleet look idler than it is (ethos rule 4)"
    );
}
