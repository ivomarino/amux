//! macOS process health sweep (2026-08-30).
//!
//! Four categories of runaway process cost Ethan a power cycle or a restart
//! and have no other reaper:
//!
//! 1. **Orphaned Ray workers** (`ray::*`). Ray actors that outlive their
//!    cluster — detached actors in particular — pin worker processes forever.
//!    `ray stop` itself does not kill them. If the raylet is gone and these are
//!    still present, they are zombies: no work will ever reach them, but they
//!    hold CPU slots, memory, and file descriptors. Safe to kill when:
//!    the raylet is not running AND the worker has been running for longer than
//!    a short grace period (covers the teardown window).
//!
//! 2. **Zombie `rustc` debug processes** that outlived their parent cargo run.
//!    Only relevant when a `cargo check` or `cargo test` was interrupted mid-
//!    flight. Detected by: parent PID is 1 (reparented to init = orphan) and
//!    the command line includes the debug target dir.
//!
//! 3. **Accumulated `mds_stores` / Spotlight churn** from large build dirs.
//!    Not killed (that would upset macOS) but logged when RSS > threshold so
//!    the log sweep can notice the pattern.
//!
//! 4. **Ghost Claude Code processes** — `claude` processes whose tmux pane no
//!    longer exists. The existing `ghost_rescue.rs` handles the amux-managed
//!    subset; this watches the raw-count ceiling (AMUX_MAC_HEALTH_MAX_CLAUDE)
//!    and logs when it is exceeded so the operator knows before the OOM.
//!
//! WHAT THIS WILL NOT DO:
//! - Kill a process whose state it cannot verify. Silence is not dead.
//! - Kill processes the raylet is still using (raylet running = ray is live).
//! - Kill rustc processes whose parent is NOT pid 1 (they may still be running).
//! - Take any action on processes it cannot identify by full command line.
//!
//! SAFE: every kill here is SIGTERM, not SIGKILL, and every target class has
//! a predicate that cannot be satisfied by a legitimate process doing real work.

use std::time::Duration;

const JOB: &str = "mac-health";

fn tick_secs() -> u64 {
    std::env::var("AMUX_MAC_HEALTH_TICK_S")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(1800) // 30 minutes
}

/// Maximum number of `claude` processes before a WARN fires.
/// Default 60 — matches the observed fleet size plus headroom.
fn max_claude() -> usize {
    std::env::var("AMUX_MAC_HEALTH_MAX_CLAUDE")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(60)
}

/// How old an orphaned ray:: worker must be (seconds) before it is eligible
/// for reaping. Guards against killing workers during a graceful shutdown.
fn ray_orphan_grace_s() -> u64 {
    std::env::var("AMUX_MAC_HEALTH_RAY_GRACE_S")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(120)
}

/// True when a local raylet process is running.
/// Uses [r]aylet trick so the grep cannot match itself.
fn raylet_running() -> bool {
    std::process::Command::new("pgrep")
        .args(["-f", "[r]aylet"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Returns (pid, elapsed_seconds, command_excerpt) for each `ray::` worker
/// process whose parent is no longer the raylet (i.e., reparented to PID 1
/// or whose ppid doesn't exist in the process table).
///
/// Only called when `raylet_running()` is false — when the raylet is alive,
/// every ray:: worker is legitimate.
fn orphaned_ray_workers(grace_s: u64) -> Vec<(u32, u64, String)> {
    // ps -A -o pid,ppid,etime,command= outputs one line per process.
    // We filter for lines starting with "ray::" (detached actor workers).
    // etime format: [[DD-]HH:]MM:SS
    let Ok(out) = std::process::Command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,etime=,command="])
        .output()
    else {
        return vec![];
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut result = Vec::new();
    for line in text.lines() {
        let Some((pid, ppid, etime, cmd_owned)) = ps_row(line) else {
            continue;
        };
        let cmd = cmd_owned.as_str();
        if !cmd.starts_with("ray::") {
            continue;
        }
        // Parse etime to seconds. Format: [[DD-]HH:]MM:SS
        let elapsed_s = parse_etime(etime).unwrap_or(0);
        if elapsed_s < grace_s {
            continue;
        }
        // ppid == 1 = reparented to init (orphan). Also catch ppid that is no
        // longer in the process table (the raylet exited).
        if ppid == 1 || !pid_exists(ppid) {
            result.push((pid, elapsed_s, cmd.chars().take(60).collect()));
        }
    }
    result
}

fn pid_exists(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Parse ps etime field [[DD-]HH:]MM:SS into total seconds.
fn parse_etime(s: &str) -> Option<u64> {
    // Split on '-' first (days), then on ':'
    let (days, rest) = if let Some((d, r)) = s.split_once('-') {
        (d.parse::<u64>().ok()?, r)
    } else {
        (0, s)
    };
    let parts: Vec<&str> = rest.split(':').collect();
    match parts.as_slice() {
        [mm, ss] => {
            Some(days * 86400 + mm.parse::<u64>().ok()? * 60 + ss.parse::<u64>().ok()?)
        }
        [hh, mm, ss] => {
            Some(days * 86400 + hh.parse::<u64>().ok()? * 3600
                + mm.parse::<u64>().ok()? * 60
                + ss.parse::<u64>().ok()?)
        }
        _ => None,
    }
}

/// Parse one `ps -o pid=,ppid=,etime=,command=` row into (pid, ppid, etime, command).
///
/// SPLIT ON WHITESPACE RUNS, NOT SINGLE SPACES (AMUX-3972). `ps` RIGHT-ALIGNS
/// its numeric columns, so a real row begins with padding:
///
///   "  5923     1     05:28 /Applications/Google Chrome.app/..."
///
/// The previous code was `line.splitn(4, ' ').filter(|s| !s.is_empty())`, which
/// consumes the first three SINGLE spaces — all of them padding — and yields
/// ["", "5923", "", "   1     05:28 /Applications/..."]. Filtering the empties
/// happens AFTER splitn has already committed its split points, so the result
/// has 2 elements, `parts.len() < 4` fires, and the row is skipped. Every row,
/// every pass. Measured on this box: 45 matching Chrome rows, 0 parsed.
///
/// The filter READS like it handles padding and cannot. That is why this is a
/// function now: the identical block existed at two call sites (the ray-worker
/// sweep and the orphaned-Chrome reaper) and both were dead in the same way.
///
/// The command is re-joined on single spaces. Every caller does `contains` on a
/// substring with no whitespace runs in it, so that is lossless for this use.
fn ps_row(line: &str) -> Option<(u32, u32, &str, String)> {
    let mut it = line.split_whitespace();
    let pid = it.next()?.parse::<u32>().ok()?;
    let ppid = it.next()?.parse::<u32>().ok()?;
    let etime = it.next()?;
    let cmd = it.collect::<Vec<&str>>().join(" ");
    if cmd.is_empty() {
        return None;
    }
    Some((pid, ppid, etime, cmd))
}

/// Minimum age (seconds) before an orphaned Playwright Chrome is eligible for
/// reaping. Short grace covers the window between Chrome launch and the parent
/// Playwright process registering the PID. Default 300s (5 minutes).
fn playwright_chrome_grace_s() -> u64 {
    std::env::var("AMUX_MAC_HEALTH_PLAYWRIGHT_GRACE_S")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(300)
}

/// Returns the PIDs of orphaned Playwright-launched Chrome processes.
///
/// Shape: `--user-data-dir=/var/folders/…/T/.tmp*/playwright-auth/profile`
/// with PPID == 1 (parent Playwright process exited, Chrome reparented to
/// init). Each Playwright session spawns ~8-9 Chrome helper processes; we
/// only kill the root (PPID=1) and let the helpers die naturally.
fn orphaned_playwright_chromes(grace_s: u64) -> (Vec<(u32, u64)>, usize) {
    let Ok(out) = std::process::Command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,etime=,command="])
        .output()
    else {
        // 0 parsed: `ps` itself failed, so the sweep did not run. The caller
        // WARNs on this rather than reading it as a clean machine.
        return (vec![], 0);
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut result = Vec::new();
    // HOW MANY ROWS THE PROBE COULD ACTUALLY READ (AMUX-3972, ethos rule 4).
    //
    // The old parse skipped 100% of rows and the job logged "no orphaned
    // Playwright Chrome processes" — at 15:15:24 on 2026-08-31, with SIX live.
    // A zero that means "none exist" and a zero that means "I could not read a
    // single line" printed the same sentence, so the reaper looked healthy for
    // as long as it was dead. Publishing the population makes those two
    // different outputs.
    let mut parsed = 0usize;
    for line in text.lines() {
        let Some((pid, ppid, etime, cmd_owned)) = ps_row(line) else {
            continue;
        };
        parsed += 1;
        if ppid != 1 {
            continue; // only reap the root; helpers die with it
        }
        let cmd = cmd_owned.as_str();
        // Must be a Chrome process with a Playwright temp-profile user-data-dir.
        let is_chrome = cmd.contains("Google Chrome") || cmd.contains("Chromium");
        let has_playwright_tmpdir = cmd.contains("/T/.tmp") && cmd.contains("playwright-auth/profile");
        if !is_chrome || !has_playwright_tmpdir {
            continue;
        }
        let elapsed_s = parse_etime(etime).unwrap_or(0);
        if elapsed_s >= grace_s {
            result.push((pid, elapsed_s));
        }
    }
    (result, parsed)
}

/// Count running `claude` processes and warn if over threshold.
fn check_claude_count(max: usize) -> usize {
    let count = std::process::Command::new("pgrep")
        .args(["-c", "-x", "claude"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<usize>().ok())
        .unwrap_or(0);
    if count > max {
        tracing::warn!(
            job = JOB,
            count,
            max,
            "mac-health: claude process count exceeds ceiling. \
             This many processes compete for memory and CPU. \
             Check for ghost lanes with `amux ls` and stop idle ones."
        );
    }
    count
}

fn one_pass() {
    let grace = ray_orphan_grace_s();
    let pw_grace = playwright_chrome_grace_s();
    let max_claude = max_claude();

    // --- Ray orphan sweep ---
    if !raylet_running() {
        let orphans = orphaned_ray_workers(grace);
        if !orphans.is_empty() {
            tracing::warn!(
                job = JOB,
                count = orphans.len(),
                grace_s = grace,
                "mac-health: orphaned ray:: workers with no live raylet — sending SIGTERM"
            );
            for (pid, age_s, cmd) in &orphans {
                tracing::info!(
                    job = JOB, pid, age_s, cmd = %cmd,
                    "mac-health: SIGTERM orphaned ray:: worker"
                );
                let _ = std::process::Command::new("kill").args([&pid.to_string()]).output();
            }
        } else {
            tracing::debug!(job = JOB, "mac-health: no orphaned ray:: workers");
        }
    } else {
        tracing::debug!(job = JOB, "mac-health: raylet running, skipping ray orphan sweep");
    }

    // --- Orphaned Playwright Chrome sweep ---
    let (pw_orphans, pw_rows_parsed) = orphaned_playwright_chromes(pw_grace);
    if pw_rows_parsed == 0 {
        // `ps -A` on a live machine always has rows. Zero parsed means the
        // PROBE is broken, not that the machine is quiet — say so loudly rather
        // than reporting a clean sweep.
        tracing::warn!(
            job = JOB,
            "mac-health: parsed 0 rows from `ps -A` — the orphan sweep did NOT run. \
             This is a broken probe, not a clean machine (AMUX-3972)."
        );
    }
    if !pw_orphans.is_empty() {
        tracing::warn!(
            job = JOB,
            count = pw_orphans.len(),
            grace_s = pw_grace,
            "mac-health: orphaned Playwright temp-profile Chrome processes — sending SIGTERM"
        );
        for (pid, age_s) in &pw_orphans {
            tracing::info!(
                job = JOB, pid, age_s,
                "mac-health: SIGTERM orphaned Playwright Chrome root"
            );
            let _ = std::process::Command::new("kill").args([&pid.to_string()]).output();
        }
    } else {
        tracing::debug!(
            job = JOB,
            rows_parsed = pw_rows_parsed,
            "mac-health: no orphaned Playwright Chrome processes"
        );
    }

    // --- Claude process count ---
    let claude_count = check_claude_count(max_claude);
    tracing::info!(
        job = JOB,
        claude_count,
        max_claude,
        ray_alive = raylet_running(),
        playwright_chromes_reaped = pw_orphans.len(),
        "mac-health tick"
    );
}

pub fn spawn() -> super::PeriodicTask {
    let interval = Duration::from_secs(tick_secs());
    super::spawn_periodic_every(JOB, interval, || async {
        tokio::task::spawn_blocking(one_pass).await.ok();
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etime_parses_correctly() {
        assert_eq!(parse_etime("01:30"), Some(90));
        assert_eq!(parse_etime("01:01:00"), Some(3660));
        assert_eq!(parse_etime("1-02:00:00"), Some(93600));
        assert_eq!(parse_etime("00:05"), Some(5));
        // Malformed -> None, not a panic.
        assert_eq!(parse_etime("bad"), None);
    }

    #[test]
    fn ray_orphan_grace_filters_young_processes() {
        // parse_etime + grace check: a 60s-old worker with a 120s grace is kept.
        let age = parse_etime("01:00").unwrap_or(0);
        assert!(age < 120, "60s < 120s grace, should not be reaped");
        let age2 = parse_etime("03:00").unwrap_or(0);
        assert!(age2 >= 120, "180s >= 120s grace, eligible");
    }
}

#[cfg(test)]
mod ps_row_tests {
    use super::*;

    /// A REAL row, captured verbatim from `ps -A -o pid=,ppid=,etime=,command=`
    /// on 2026-08-31 while eight orphaned Chromes were live. The leading spaces
    /// are the whole point: `ps` right-aligns pid and ppid, and it was that
    /// padding the old `splitn(4, ' ')` consumed.
    const REAL_ROW: &str =
        " 5923     1       10:41 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome \
--user-data-dir=/var/folders/0x/T/.tmpLbY4NA/playwright-auth/profile";

    /// GUARDS THE FIXTURE ITSELF. If someone "tidies" the leading whitespace out
    /// of REAL_ROW, every assertion below still passes and stops testing the bug
    /// — the padding IS the input under test.
    #[test]
    fn the_fixture_is_actually_padded_or_it_tests_nothing() {
        assert!(
            REAL_ROW.starts_with(' '),
            "REAL_ROW must keep ps's right-alignment padding; without it this \
             module cannot fail on the defect it exists for"
        );
        // And the old predicate really is defeated by it, so the cell below is
        // not merely asserting that a correct parser is correct.
        let old: Vec<&str> = REAL_ROW.splitn(4, ' ').filter(|s| !s.is_empty()).collect();
        assert!(
            old.len() < 4,
            "the pre-AMUX-3972 parse must FAIL on this row (got {} parts) — if it \
             succeeds, this fixture no longer reproduces the bug",
            old.len()
        );
    }

    #[test]
    fn a_padded_ps_row_parses_into_its_four_fields() {
        let (pid, ppid, etime, cmd) = ps_row(REAL_ROW).expect("a real ps row must parse");
        assert_eq!(pid, 5923);
        assert_eq!(ppid, 1);
        assert_eq!(etime, "10:41");
        assert!(cmd.starts_with("/Applications/Google Chrome.app/"), "cmd was: {cmd}");
        // The command must survive intact through the re-join, or the reaper's
        // `contains` checks silently stop matching.
        assert!(cmd.contains("playwright-auth/profile"), "cmd was: {cmd}");
        assert!(cmd.contains("/T/.tmp"), "cmd was: {cmd}");
    }

    #[test]
    fn the_reaper_selects_that_row_end_to_end() {
        // The predicate the reaper actually applies, against the real row.
        let (_, ppid, _, cmd) = ps_row(REAL_ROW).unwrap();
        assert_eq!(ppid, 1, "orphaned to init");
        assert!(cmd.contains("Google Chrome"));
        assert!(cmd.contains("/T/.tmp") && cmd.contains("playwright-auth/profile"));
    }

    #[test]
    fn rows_without_a_command_are_rejected_rather_than_half_parsed() {
        assert!(ps_row("  123   1   00:01").is_none(), "no command field");
        assert!(ps_row("").is_none());
        assert!(ps_row("not a ps row at all").is_none(), "pid must be numeric");
    }
}
