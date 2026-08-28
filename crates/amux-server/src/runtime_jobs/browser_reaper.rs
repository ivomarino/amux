//! Release a browser that has had nothing open for a while (AMUX-3829).
//!
//! Ethan, 2026-08-28: "multiple browsers shouldnt be an issue, but it should
//! clean up automatically after idle use." The prompting incident was a browser
//! held 18.1 HOURS with ZERO tabs, blocking him, because nothing in amux reaped
//! one — `runtime_jobs::registry` had no browser job at all.
//!
//! WHY THIS IS SAFE TO DO AUTOMATICALLY, which is not obvious in a subsystem
//! whose comments record three separate incidents of destroying people's staged
//! logins (AMUX-3063, AMUX-3414, AMUX-3610). A COMPLETED login lives on disk in
//! `playwright-auth/profiles/<name>/` and survives a stop: `stop_profile_as`
//! sends SIGTERM precisely so storage flushes, and the only `remove_dir_all` on
//! a profile is the explicit delete verb. So the question is never "will this
//! lose an account" — it is "will this lose an OPEN PAGE", and this job only
//! ever runs when there are none.
//!
//! IDLE IS CONTINUOUS EMPTINESS, NOT AGE. Reaping on "started N ago" would kill
//! a browser someone used a minute ago, and reaping the instant the last tab
//! closes would kill one they are about to reuse. So a profile has to be seen
//! empty on every check across the whole window; one real page anywhere in it
//! resets the clock. That also makes the signal honest under the multi-slot
//! registry (AMUX-3828): emptiness is per profile, so one worker's idle browser
//! is reaped without touching the neighbour a second worker is driving.
//!
//! WHAT IT WILL NOT DO. A browser whose CDP does not answer is NOT reaped.
//! Silence is not zero — the same distinction the takeover refusal draws — and
//! killing a browser because it failed to answer a poll would turn a transient
//! wedge into a destroyed session.

use std::collections::HashMap;
use std::time::Duration;

/// How long a profile must be CONTINUOUSLY empty before it is released.
/// `0` disables the job entirely.
///
/// One hour by default. The window is generous against the thing actually at
/// risk, which is only a relaunch: with no pages open there is no in-memory
/// state to lose and the profile's logins are on disk either way. It is short
/// enough that the 18-hour zombie that prompted this cannot recur.
pub fn reap_after_s() -> u64 {
    std::env::var("AMUX_BROWSER_IDLE_REAP_S")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(3600)
}

fn tick_secs() -> u64 {
    std::env::var("AMUX_BROWSER_REAP_TICK_S")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(120)
}

/// When each profile was FIRST seen empty. Absent = not currently empty.
///
/// ON DISK, NOT IN MEMORY, AND THAT IS THE WHOLE POINT (AMUX-3829, second pass).
///
/// The first version held this in a process-global map. The builder installs a
/// new binary and the server self-adopts on EVERY commit, which resets it, so
/// the window restarted every time anyone committed. Measured on the day this
/// was found: 22 builds between 06:33 and 16:26, median gap 16.7 minutes, and
/// only TWO gaps of 60 minutes or more. Against a 3600s window that is a reaper
/// which on a working day can almost never fire, while reporting `spawned:
/// true, ticks: N, status: ok` throughout.
///
/// So the card's claim that "the 18-hour zombie cannot recur" was false as
/// shipped. Nothing would have shown it: the job's own health is about the LOOP
/// running, and the loop was running perfectly.
///
/// The file is the reaper's alone. It is rewritten each tick, so a stale entry
/// for a profile that is no longer running is pruned rather than accumulated.
fn idle_path(home: &std::path::Path) -> std::path::PathBuf {
    home.join("browser-idle.json")
}

fn read_idle(home: &std::path::Path) -> HashMap<String, f64> {
    std::fs::read_to_string(idle_path(home))
        .ok()
        .and_then(|raw| serde_json::from_str::<HashMap<String, f64>>(&raw).ok())
        .unwrap_or_default()
}

fn write_idle(home: &std::path::Path, m: &HashMap<String, f64>) {
    if let Ok(text) = serde_json::to_string(m) {
        let _ = std::fs::write(idle_path(home), text);
    }
}

/// Does this CDP listing contain a real page?
///
/// Split out and pure so the rule is testable without a Chrome (ethos rule 7):
/// every live browser test in this repo is `#[ignore]`d and never runs in CI, so
/// a live-only test here would be a check that cannot fail.
///
/// `about:blank` and `chrome://` internals do NOT count as real. They are what
/// Chrome spawns on its own — a new-tab page, a popup opener — and counting
/// them would mean a browser Chrome keeps re-blanking is never reapable, which
/// is exactly the 18-hour case that prompted this.
pub fn has_real_page(targets: &[serde_json::Value]) -> bool {
    targets.iter().any(|t| {
        if t.get("type").and_then(serde_json::Value::as_str) != Some("page") {
            return false;
        }
        let u = t.get("url").and_then(serde_json::Value::as_str).unwrap_or("");
        !(u.is_empty() || u == "about:blank" || u.starts_with("chrome://"))
    })
}

/// Should this profile be released now?
///
/// Pure, so the whole decision has cells rather than only its plumbing. `None`
/// for `first_empty` means "not empty at this check", which must reset rather
/// than accumulate — otherwise a browser used every ten minutes still reaps.
pub fn should_reap(first_empty: Option<f64>, now: f64, after_s: u64) -> bool {
    if after_s == 0 {
        return false;
    }
    first_empty.is_some_and(|t| now - t >= after_s as f64)
}

fn now_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// One pass. Returns the profiles it released, so the caller can log a fact
/// rather than an intention.
async fn tick(home: &std::path::Path) -> Vec<String> {
    let after_s = reap_after_s();
    if after_s == 0 {
        return vec![];
    }
    let mut reaped = vec![];
    let now = now_f64();
    // `None` when the process never recorded a boot (tests), which the log line
    // then reports as absent rather than as "none of the window predates this
    // process" — a 0 there would be a claim nobody measured.
    let boot = crate::runtime_jobs::heartbeat::boot_at();
    let prior = read_idle(home);
    let mut next: HashMap<String, f64> = HashMap::new();
    for (profile, owner, _started, _pid, port) in crate::integrations::browser::running_all() {
        // CDP SILENCE IS NOT EMPTINESS. A browser that will not answer is left
        // alone: killing it would turn a transient wedge into a destroyed
        // session, and this job's whole safety argument rests on knowing there
        // is nothing open. Dropping the entry (rather than carrying it) resets
        // the window, which is the conservative direction.
        let Ok(listed) = crate::integrations::browser::cdp_list(port).await else {
            continue;
        };
        let empty = listed.as_array().map(|a| !has_real_page(a)).unwrap_or(false);
        if !empty {
            // One real page anywhere resets the clock: the entry is simply not
            // carried into `next`, which is written back at the end.
            continue;
        }
        let first_empty = *prior.get(&profile).unwrap_or(&now);
        next.insert(profile.clone(), first_empty);
        if !should_reap(Some(first_empty), now, after_s) {
            continue;
        }
        let idle_s = now - first_empty;
        tracing::info!(
            profile = %profile, owner = %owner, idle_s = idle_s as i64, after_s,
            // How much of the window predates this process. A non-zero value here
            // IS the restart-survival working; the in-memory version could only
            // ever print 0 and nobody would have known to look (AMUX-3829).
            pre_boot_s = boot.map(|b| (b - first_empty).max(0.0) as i64).unwrap_or(-1),
            "browser: releasing a profile with no real page open for the whole idle window \
             (AMUX-3829). Logins are on disk and survive; only a relaunch is lost."
        );
        // `stop_profile_as` records WHO in LAST_EXIT, so the next start can say
        // what happened rather than leaving the AMUX-3414 silence — a browser
        // that vanished with nothing on record cost two sessions a morning.
        crate::integrations::browser::stop_profile_as(home, &profile, "idle-reaper").await;
        next.remove(&profile);
        reaped.push(profile);
    }
    // Rewritten whole, so a profile that stopped running drops out instead of
    // accumulating: at 100x the browsers this is still one small file.
    if next != prior {
        write_idle(home, &next);
    }
    reaped
}

/// How long each running profile has been continuously empty, for
/// `/api/browser/status` (AMUX-3829). The countdown was invisible: the only way
/// to know whether a browser was about to be reaped was to read the process's
/// memory, so a reaper that could never fire looked identical to one that was
/// about to.
pub fn idle_ages(home: &std::path::Path, now: f64) -> HashMap<String, f64> {
    read_idle(home).into_iter().map(|(k, t)| (k, (now - t).max(0.0))).collect()
}

/// Spawn the loop, registered so a dead reaper is visible on
/// `/api/system-jobs` rather than silently absent.
pub fn spawn() {
    let interval = Duration::from_secs(tick_secs());
    let h = tokio::spawn(async move {
        let mut t = tokio::time::interval(interval);
        loop {
            t.tick().await;
            crate::runtime_jobs::registry::tick(crate::runtime_jobs::registry::ids::BROWSER_REAPER);
            let home = crate::integrations::browser::amux_home();
            let _ = tick(&home).await;
        }
    });
    crate::runtime_jobs::registry::adopt(
        crate::runtime_jobs::registry::ids::BROWSER_REAPER,
        Some(interval),
        &h,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// What counts as a real page, and what Chrome spawns on its own.
    #[test]
    fn only_a_real_page_keeps_a_browser_alive() {
        let page = |u: &str| json!({"type": "page", "url": u});
        assert!(has_real_page(&[page("https://studio.mixpeek.com/x")]));
        // THE CASE THAT PROMPTED THIS: Chrome respawns blanks, so counting them
        // would make a browser permanently unreapable — which is the 18-hour
        // zombie Ethan hit.
        assert!(!has_real_page(&[page("about:blank")]));
        assert!(!has_real_page(&[page("chrome://newtab/")]));
        assert!(!has_real_page(&[page("")]));
        assert!(!has_real_page(&[]), "no targets at all is empty");
        // Non-page targets (iframes, service workers) are not pages.
        assert!(!has_real_page(&[json!({"type": "iframe", "url": "https://x.com/"})]));
        // CONTROL: one real page among blanks keeps it alive. A predicate that
        // ignored the real one would reap a browser someone is using.
        assert!(has_real_page(&[page("about:blank"), page("https://x.com/")]));
    }

    /// AMUX-3829, second pass. The window must SURVIVE a restart, because the
    /// builder installs a new binary and the server self-adopts on every commit.
    /// Measured the day this was found: 22 builds in 10 hours, median gap 16.7
    /// minutes, only TWO gaps past the 3600s window. An in-memory clock made
    /// this a reaper that could almost never fire while reporting itself
    /// healthy, since the loop was running perfectly the whole time.
    #[test]
    fn the_idle_clock_is_read_back_from_disk_rather_than_restarting() {
        let tmp = std::env::temp_dir().join(format!("amux-reap-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).expect("tmpdir");

        // Nothing recorded yet: absent, not zero.
        assert!(super::read_idle(&tmp).is_empty());
        assert!(super::idle_ages(&tmp, 1000.0).is_empty());

        // A window that began 50 minutes ago, written by a PREVIOUS process.
        let mut m = std::collections::HashMap::new();
        m.insert("atlas".to_string(), 1000.0);
        super::write_idle(&tmp, &m);

        // A fresh process reads it back and the clock keeps running. This is the
        // assertion the in-memory version could not pass.
        assert_eq!(super::read_idle(&tmp).get("atlas"), Some(&1000.0));
        assert_eq!(super::idle_ages(&tmp, 4000.0).get("atlas"), Some(&3000.0));
        // And the reap decision therefore fires on the CARRIED window: 3600s
        // after the original first-empty, not 3600s after this process started.
        assert!(should_reap(Some(1000.0), 4600.0, 3600));
        assert!(!should_reap(Some(1000.0), 4599.0, 3600));

        // A corrupt or truncated file reads as "nothing recorded" rather than
        // panicking or inventing a timestamp — the conservative direction, since
        // it restarts the window instead of reaping early.
        std::fs::write(super::idle_path(&tmp), "{not json").expect("write");
        assert!(super::read_idle(&tmp).is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Idle is CONTINUOUS emptiness, and the window is a floor not a ceiling.
    #[test]
    fn a_profile_is_released_only_after_the_whole_window_empty() {
        // Empty for the full hour -> release.
        assert!(should_reap(Some(0.0), 3600.0, 3600));
        assert!(should_reap(Some(0.0), 99_999.0, 3600));
        // Short of it -> keep.
        assert!(!should_reap(Some(0.0), 3599.0, 3600));
        // NOT EMPTY AT THIS CHECK -> never. This is the cell that stops a
        // browser used every ten minutes from accumulating its way to a reap:
        // the caller passes None whenever a real page is present, which resets.
        assert!(!should_reap(None, 99_999.0, 3600));
        // DISABLED means disabled, at any age — the off switch has to work or
        // it is not an off switch (ethos rule 6).
        assert!(!should_reap(Some(0.0), 99_999.0, 0));
    }
}
