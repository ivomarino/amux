//! "You went idle with N uncommitted change(s)" — ported from Python
//! (AMUX-2638), with the one change that stops a known incident recurring.
//!
//! # The requirement, which is the whole reason this is a card
//!
//! Python derived "yours" from DIRTY-TREE MEMBERSHIP. Measured 2026-08-09: it
//! listed 11 files as mine to commit; I had touched NONE of them — they were a
//! peer's in-flight rust migration — while the staged-guard, asked the same
//! question at the same moment about the same files, answered foreign=4,
//! unclaimed=18, MINE=0. Two components, one question, opposite answers, and
//! the WRONG one was the one giving instructions. Followed literally it sweeps
//! a peer's work into your commit, which happened three times in one day
//! (762e06e, 325314d, 4bf767c).
//!
//! So ownership here comes from [`Ownership`], which is the staged-guard's
//! answer (`POST /api/git/staged-guard`, api/git_guard.rs) and nothing else.
//! `git status` supplies the LIST of dirty paths and no opinion about whose
//! they are. That split is enforced by the signature: [`build`] cannot see a
//! repo, so it cannot re-derive ownership even by accident.
//!
//! # Why a foreign file does not merely get filtered out
//!
//! It is named, loudly, with its owner. The recipient is about to run
//! `git add -A`; the useful thing is not silence about the peer's file, it is
//! "do not commit this one, it is theirs". Python learned that the hard way —
//! the branch was missing entirely and two sweeps (~93 and ~85 lines) followed.
//!
//! # Why `shared` is warned about rather than suppressed
//!
//! On a repo where two lanes routinely touch one file, "both edited it" is
//! satisfied almost always, so suppressing on `shared` would silence the nudge
//! permanently. Name the file as contested and say who else is in it; the
//! recipient can then stage per-hunk instead of per-file.

use std::collections::BTreeSet;

/// The staged-guard's verdict, transcribed. Every field is a list of paths.
///
/// Deliberately NOT constructible from a working tree: it exists only to carry
/// an answer the guard already gave.
#[derive(Debug, Default, Clone)]
pub struct Ownership {
    /// A peer edited it and this session did NOT. Never commit these.
    pub foreign: Vec<(String, String)>, // (path, owner)
    /// Both edited it. Contested, not forbidden.
    pub shared: Vec<(String, String)>, // (path, other owner)
    /// Nobody's edit record claims it. NOT "yours" — see [`build`].
    pub unclaimed: Vec<String>,
    /// The guard could not decide.
    pub undecided: Vec<String>,
    /// The guard is partially blind (a cotenant has no transcript), so an
    /// empty `foreign` does NOT clear their files.
    pub partial: Option<String>,
}

/// The nudge, or None when there is nothing honest to say.
///
/// `dirty` is the list of paths `git status` reported under the session's
/// working directory — a LIST, carrying no ownership claim.
pub fn build(
    dir: &str,
    dirty: &[String],
    own: &Ownership,
    provenance: &str,
) -> Option<String> {
    // PROVENANCE IS A REQUIRED PARAMETER, NOT A FIELD THE CALLER MAY FORGET
    // (tubescience, 2026-08-11). It shipped first as the second half of a
    // `(Vec<String>, String)` tuple, which a future caller retires with
    // `let (dirty, _) = ...` — one underscore and the warning silently stops
    // saying what it compared against, which is the exact degradation this
    // whole fix exists to make visible.
    //
    // Their formulation, from mechanising it in their own harness: "wherever
    // claims get emitted in a structured way, put the scope in the constructor
    // and make it mandatory. Prose is where scope goes to be optional." A
    // nudge is a claim about someone's uncommitted work; this signature is what
    // makes it impossible to state one without saying what it was measured
    // against. It covers the caller written months from now by someone who read
    // none of this — the population a remembered habit cannot reach.
    if dirty.is_empty() {
        return None;
    }
    // MINE means POSITIVELY ATTRIBUTED TO ME, never "not proven to be someone
    // else's" (AMUX-2638, reopened by Ethan 2026-08-10).
    //
    // The first port filtered out `foreign` and treated everything else as
    // mine. That is the same bug in a subtler dress: it told Ethan to "commit
    // completed work now" about CLAUDE.md, which he had never touched — the
    // guard classified it `unclaimed`, meaning NO session has an edit record
    // for it, and specifically he had no claim. Near-certainly a peer's
    // in-flight edit (last commit to that file was amux-homepage doing exactly
    // that work).
    //
    // "Not attributable to a peer" is not evidence that it is yours. Only a
    // positive claim is.
    let foreign_paths: BTreeSet<&str> = own.foreign.iter().map(|(p, _)| p.as_str()).collect();
    let unknown_paths: BTreeSet<&str> = own
        .unclaimed
        .iter()
        .map(String::as_str)
        .chain(own.undecided.iter().map(String::as_str))
        .collect();

    let mine: Vec<&String> = dirty
        .iter()
        .filter(|p| !foreign_paths.contains(p.as_str()) && !unknown_paths.contains(p.as_str()))
        .collect();
    let unknown: Vec<&String> =
        dirty.iter().filter(|p| unknown_paths.contains(p.as_str())).collect();

    if mine.is_empty() {
        // Nothing is positively yours. Saying "commit completed work now" here
        // is the instruction that cost this checkout three sweeps in two days.
        // But silence is also wrong when the tree is dirty and nobody can say
        // whose it is — so report the uncertainty AS uncertainty.
        if unknown.is_empty() {
            return None;
        }
        let n = unknown.len();
        let list: String = unknown.iter().take(10).map(|f| format!("  {f}\n")).collect();
        let mut m = format!(
            "You went idle with {n} uncommitted change(s) under {dir} whose OWNERSHIP IS \
             UNKNOWN — no session has an edit record for {}:\n{list}\n\
             Do NOT assume {} yours. `git add -A` here would commit whatever a peer is \
             mid-edit on. Check `git diff` and stage only what you recognise as your work.",
            if n == 1 { "it" } else { "them" },
            if n == 1 { "it is" } else { "they are" },
        );
        if let Some(why) = &own.partial {
            m.push_str(&format!("\n\nATTRIBUTION IS PARTIAL — {why}"));
        }
        m.push_str(&format!("\n\n({provenance})"));
        return Some(m);
    }

    let n = mine.len();
    let sample: String = mine
        .iter()
        .take(10)
        .map(|f| format!("  {f}\n"))
        .collect::<String>()
        + if n > 10 { "  …\n" } else { "" };

    let mut msg = format!(
        "You went idle with {n} uncommitted change(s) under your working directory ({dir}):\n\
         {sample}\n\
         Commit completed work now with a clear, descriptive message (group related changes). \
         If something is intentionally incomplete, commit a WIP checkpoint and say so. \
         Don't leave the working tree dirty."
    );

    if !unknown.is_empty() {
        let list: Vec<&str> = unknown.iter().take(4).map(|s| s.as_str()).collect();
        msg.push_str(&format!(
            "\n\nOWNERSHIP UNKNOWN — {} also dirty, with no edit record from any session. \
             Not counted above and not necessarily yours; check before staging.",
            list.join(", ")
        ));
    }
    if let Some(why) = &own.partial {
        msg.push_str(&format!("\n\nATTRIBUTION IS PARTIAL — {why}"));
    }
    if !own.shared.is_empty() {
        let who: BTreeSet<&str> = own.shared.iter().map(|(_, w)| w.as_str()).collect();
        let paths: Vec<&str> = own.shared.iter().take(4).map(|(p, _)| p.as_str()).collect();
        msg.push_str(&format!(
            "\n\nCONTESTED — {} also edited by {}. Stage per-HUNK (`git add -p`), not per-file: \
             `git add <file>` takes their in-flight hunks too.",
            paths.join(", "),
            who.into_iter().collect::<Vec<_>>().join("/")
        ));
    }

    if !own.foreign.is_empty() {
        let who: BTreeSet<&str> = own.foreign.iter().map(|(_, w)| w.as_str()).collect();
        let paths: Vec<&str> = own.foreign.iter().take(4).map(|(p, _)| p.as_str()).collect();
        let (was, it) = if own.foreign.len() == 1 { ("was", "it") } else { ("were", "them") };
        msg.push_str(&format!(
            "\n\nNOT YOURS — {} {} edited by {} and NOT by you. Do not commit {}: \
             `git add -A` or `git commit -a` would sweep a peer's in-flight work into your \
             commit under your name. Stage only the files you touched.",
            paths.join(", "),
            was,
            who.into_iter().collect::<Vec<_>>().join("/"),
            it
        ));
    }
    msg.push_str(&format!("\n\n({provenance})"));
    Some(msg)
}


// ---------------------------------------------------------------------------
// The firing path
// ---------------------------------------------------------------------------

use crate::api::AppState;
use serde_json::{json, Value};

/// Once per session per UTC day. Python's own audit found 87 nudges/day against
/// 75 human sends — each one a full-context turn into a cold-cache idle session,
/// the single largest automated token stream. A reminder that arrives twelve
/// times is not a reminder.
fn cap_key(session: &str, now: f64) -> String {
    let day = (now / 86_400.0).floor() as i64;
    format!("commit_nudge:{session}:{day}")
}


/// Drop tracked paths whose CONTENT already matches `origin/main`.
///
/// Reported by tubescience 2026-08-10, measured rather than argued: 6 of 7
/// paths in one ownership warning were BYTE-IDENTICAL to origin. They were not
/// edits at all.
///
/// The cause is a workflow, not a bug in git: several lanes land work with
/// scripts/graft-push.sh, which pushes a tree built from origin WITHOUT moving
/// the local branch. Local HEAD therefore sits permanently behind origin, and
/// every file anyone has successfully landed reads as modified forever after.
/// `git status` is answering "how does the worktree differ from local HEAD",
/// while the warning asks "what might a peer be mid-edit on" — an instrument
/// answering a question adjacent to the one asked.
///
/// It cost a real decision: a session read this signal on tenant_canary.py,
/// concluded a peer was mid-edit, and declined a three-line fix. The file was
/// identical to origin. And a warning wrong 6 times in 7 gets skimmed, so the
/// ONE real entry — an untracked draft that `git add -A` would genuinely have
/// swept up — arrived already discounted.
///
/// UNTRACKED IS NOT A SYNONYM FOR ABSENT-FROM-ORIGIN, and an earlier version of
/// this comment said it was. Reported by creative-dna and reproduced: a file
/// ADDED on origin after local HEAD reads `??` from `git status`, because the
/// local index predates the commit that added it. Untracked is the same
/// artifact for adds that dirty-status is for edits. So the test is PRESENCE ON
/// ORIGIN, never the porcelain letter — a `??` path that matches origin is a
/// phantom and is dropped, while a `??` path genuinely absent from origin is
/// the real unprotected-WIP case and is always kept.
async fn drop_paths_identical_to_origin(dir: &str, paths: Vec<String>) -> (Vec<String>, String) {
    // REFRESH origin FIRST, and say what we compared against (tubescience).
    //
    // Nothing in this job fetched, so the comparison read whatever the LOCAL
    // origin/main ref happened to be — current only if some session had been
    // fetching for its own reasons. Against a frozen ref every path landed
    // since fails equality, gets kept, and the warning degrades straight back
    // to the noise this filter removes.
    //
    // The danger is not the noise, it is that THE DEGRADED STATE IS
    // INDISTINGUISHABLE FROM THE PRE-FIX STATE: same warning, same phantoms,
    // and nobody files a second report because it looks like the bug already
    // fixed. So the fix goes quiet without anyone learning it went quiet.
    // A filter that can silently stop filtering must say what it filtered
    // against — hence the returned provenance string, which the warning prints.
    let fetched = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new("git")
            .args(["-C", dir, "fetch", "--quiet", "origin", "main"])
            .output(),
    )
    .await
    .ok()
    .and_then(|r| r.ok())
    .map(|o| o.status.success())
    .unwrap_or(false);

    let ref_age = tokio::process::Command::new("git")
        .args(["-C", dir, "log", "-1", "--format=%cr", "origin/main"])
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    let provenance = match (fetched, ref_age.as_deref()) {
        (true, Some(age)) => format!("compared against origin/main (just fetched; tip {age})"),
        (false, Some(age)) => format!(
            "compared against a STALE origin/main (fetch failed; tip {age}) —              paths landed since then will be listed even if they match origin"
        ),
        (_, None) => "NOT compared against origin (no origin/main ref) — every path listed".into(),
    };
    if ref_age.is_none() {
        // Cannot compare at all: keep everything. Noisy beats silent.
        return (paths, provenance);
    }

    let mut kept = Vec::new();
    for p in paths {
        // Does the path exist on origin at all? THE TRAP tubescience hit and
        // documented: `git rev-parse origin/main:<path>` on a path that does not
        // exist there prints the literal argument to STDOUT rather than failing
        // cleanly, so a naive is-empty check does not fire and an untracked file
        // scores as "differs from origin". Test the exit code via cat-file -e.
        let exists = tokio::process::Command::new("git")
            .args(["-C", dir, "cat-file", "-e", &format!("origin/main:{p}")])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !exists {
            kept.push(p); // genuinely absent from origin: the real WIP case
            continue;
        }
        let local = tokio::process::Command::new("git")
            .args(["-C", dir, "hash-object", "--", &p])
            .output()
            .await;
        let remote = tokio::process::Command::new("git")
            .args(["-C", dir, "rev-parse", &format!("origin/main:{p}")])
            .output()
            .await;
        let same = match (local, remote) {
            (Ok(l), Ok(r)) if l.status.success() && r.status.success() => {
                String::from_utf8_lossy(&l.stdout).trim()
                    == String::from_utf8_lossy(&r.stdout).trim()
            }
            // Cannot compare -> keep it. A path we failed to check is not a path
            // we have cleared.
            _ => false,
        };
        if !same {
            kept.push(p);
        }
    }
    (kept, provenance)
}

/// `git status --porcelain` under `dir`, as repo-relative paths.
///
/// Supplies the LIST only. Whose they are is the guard's answer, never this
/// function's — see the module docs for what re-deriving it cost.
async fn dirty_paths(dir: &str) -> Vec<String> {
    let out = tokio::process::Command::new("git")
        .args(["-C", dir, "status", "--porcelain", "--untracked-files=normal"])
        .output()
        .await;
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            // "XY path" or "XY old -> new"; take the destination.
            let rest = l.get(3..)?.trim();
            Some(rest.rsplit(" -> ").next().unwrap_or(rest).trim_matches('"').to_string())
        })
        .filter(|p| !p.is_empty())
        .collect()
}

/// Ownership for `paths`, from the staged-guard ITSELF — called as a function,
/// not reimplemented.
///
/// This is the card's requirement made literal: one implementation of "whose is
/// this", with two consumers. Two implementations diverge and each looks correct
/// alone, which is exactly how the guard said MINE=0 while the nudge said 11
/// files were mine, at the same moment about the same files.
async fn ownership_from_guard(session: &str, dir: &str, paths: &[String]) -> Option<Ownership> {
    let body = json!({ "dir": dir, "session": session, "paths": paths });
    let mut headers = axum::http::HeaderMap::new();
    if let Ok(v) = axum::http::HeaderValue::from_str(session) {
        headers.insert("x-amux-session", v);
    }
    // `None` state: this is an ownership PROBE, not a commit attempt, so it
    // must not fire the owner notifications the HTTP path sends (AMUX-2923).
    let (status, axum::Json(v)) = crate::api::git_guard::staged_guard_inner(
        None,
        headers,
        axum::body::Bytes::from(body.to_string()),
    )
    .await;
    if !status.is_success() || v.get("ok").and_then(Value::as_bool) != Some(true) {
        // Attribution unavailable. Return None and stay SILENT rather than
        // nudging without it — python kept the old over-nudging behaviour here
        // and that is the failure mode, not the safe default.
        return None;
    }
    let pairs = |k: &str| -> Vec<(String, String)> {
        v.get(k)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|f| {
                        Some((
                            f.get("path")?.as_str()?.to_string(),
                            f.get("owner").and_then(Value::as_str).unwrap_or("?").to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    // "I cannot tell" is a first-class answer here, and the reason this fix is
    // possible now: the guard reports when a cotenant has no transcript, so an
    // empty `foreign` does NOT clear their files.
    // `degraded` is an ARRAY of sentences on the live server, not a string —
    // an `as_str()` read silently dropped it, which would have shipped this fix
    // with its own disclosure permanently off. Handle both shapes.
    let partial = v
        .get("degraded")
        .and_then(|d| match d {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Array(a) if !a.is_empty() => {
                let joined: Vec<String> =
                    a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
                (!joined.is_empty()).then(|| joined.join("; "))
            }
            _ => None,
        })
        .or_else(|| {
            v.get("reason")
                .and_then(Value::as_str)
                .filter(|s| s.to_lowercase().contains("partial") || s.to_lowercase().contains("invisible"))
                .map(str::to_string)
        });
    let plain = |k: &str| -> Vec<String> {
        v.get(k)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|f| {
                        f.get("path")
                            .and_then(Value::as_str)
                            .or_else(|| f.as_str())
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(Ownership {
        foreign: pairs("foreign"),
        shared: pairs("shared"),
        undecided: plain("undecided"),
        partial,
        unclaimed: plain("unclaimed"),
    })
}

/// One sweep: nudge idle lanes that have uncommitted work OF THEIR OWN.
///
/// Delivery is `steer_enqueue`, never a direct send — the existing loop applies
/// the turn-boundary gate, so a nudge cannot land mid-turn (the AMUX-2642 rule
/// this repo already paid for once).
pub async fn nudge_tick(state: &AppState, lanes: &[(String, String)], now: f64) -> usize {
    let mut sent = 0usize;
    for (session, dir) in lanes {
        if session.is_empty() || dir.is_empty() {
            continue;
        }
        // Filtered against ORIGIN, not local HEAD — see
        // drop_paths_identical_to_origin for why `git status` alone answers the
        // wrong question on a graft-push checkout.
        let (dirty, provenance) =
            drop_paths_identical_to_origin(dir, dirty_paths(dir).await).await;
        if dirty.is_empty() {
            continue;
        }
        let Some(own) = ownership_from_guard(session, dir, &dirty).await else {
            continue;
        };
        // Provenance rides on EVERY nudge, not only the degraded ones. It is the
        // healthy line that makes the stale line legible: with a stamp present
        // in both states, "compared against a STALE origin/main" reads as a
        // difference. Printed only when degraded, it reads as ordinary prose and
        // gets skimmed exactly like the phantom paths it is warning about.
        let Some(msg) = build(dir, &dirty, &own, &provenance) else { continue };

        // Cap AFTER deciding there is something to say, so a suppressed-by-cap
        // day does not also consume the "nothing to say" path.
        let key = cap_key(session, now);
        let already = state
            .store
            .read()
            .ok()
            .and_then(|c| {
                c.query_row("SELECT 1 FROM prefs WHERE key=?1", rusqlite::params![key], |_| Ok(()))
                    .ok()
            })
            .is_some();
        if already {
            continue;
        }
        let k2 = key.clone();
        let _ = state
            .store
            .write_async(move |conn| {
                conn.execute(
                    "INSERT INTO prefs (key, value) VALUES (?1, '1') \
                     ON CONFLICT(key) DO NOTHING",
                    rusqlite::params![k2],
                )?;
                Ok(crate::db::WriteOutcome { applied: true, events: vec![] })
            })
            .await;
        crate::api::session_verbs::steer_enqueue(state, session, &msg, "commit-nudge", "").await;
        sent += 1;
    }
    sent
}

/// Spawn the sweep. LEVEL-triggered like board_drive's pickup, for the reason
/// stated there: this process re-execs on every deploy, so an edge-triggered
/// "went idle" is lost and an already-idle lane waits for a transition that
/// never comes.
pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let every = std::env::var("AMUX_COMMIT_NUDGE_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(600);
        // 0 disables. The knob is here because a nudge loop is exactly the kind
        // of automation that should be switchable off by config rather than by
        // a code change (D4's lesson about policy living in constants).
        if every == 0 {
            tracing::info!("commit-nudge: disabled (AMUX_COMMIT_NUDGE_SECS=0)");
            return;
        }
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(every)).await;
            // One call so the tick and the cadence it was paced at cannot be
            // recorded separately — `every` is resolved in here, so this is
            // the only place that knows it. Surfaces on /api/system-jobs.
            super::registry::tick_every(
                super::registry::ids::COMMIT_NUDGE,
                std::time::Duration::from_secs(every),
            );
            let lanes = idle_lanes_with_dirs(&state);
            if lanes.is_empty() {
                continue;
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            let n = nudge_tick(&state, &lanes, now).await;
            if n > 0 {
                tracing::info!(nudged = n, lanes = lanes.len(), "commit-nudge swept");
            }
        }
    })
}

/// Lanes that are IDLE and have a working directory.
///
/// Idle comes from the session's own report (`prefs.session_reports`, the D1
/// exit) rather than from a pane scrape — nudging a lane mid-turn is the thing
/// the steering boundary exists to prevent, and asking the harness is cheaper
/// and more truthful than inferring.
fn idle_lanes_with_dirs(state: &AppState) -> Vec<(String, String)> {
    let reports: Value = state
        .store
        .read()
        .ok()
        .and_then(|c| {
            c.query_row("SELECT value FROM prefs WHERE key='session_reports'", [], |r| {
                r.get::<_, String>(0)
            })
            .ok()
        })
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    let home = crate::api::groups::amux_home();
    let Ok(rd) = std::fs::read_dir(home.join("sessions")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("env") {
            continue;
        }
        let Some(name) = p.file_stem().and_then(|s| s.to_str()) else { continue };
        if reports.get(name).and_then(|r| r["state"].as_str()) != Some("idle") {
            continue;
        }
        let env = crate::config::parse_env_file(&p);
        if env.get("CC_ARCHIVED").map(|v| v == "1").unwrap_or(false) {
            continue;
        }
        if let Some(dir) = env.get("CC_DIR").filter(|d| !d.is_empty()) {
            out.push((name.to_string(), dir.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    /// A nudge may not exist without stating what it was measured against —
    /// asserted on EVERY branch, because the signature alone only proves the
    /// caller supplied a scope, not that the reader ever sees it. Both exits
    /// from `build` are exercised: the positively-mine path and the
    /// ownership-unknown path.
    #[test]
    fn every_nudge_states_what_it_compared_against() {
        let dirty = vec!["a.rs".to_string()];

        // `mine` is DERIVED — not foreign, not unknown — so a default
        // Ownership over a dirty path is exactly the positively-mine branch.
        let m = build("/repo", &dirty, &Ownership::default(), "SCOPE-MARKER")
            .expect("mine branch");
        assert!(m.contains("SCOPE-MARKER"), "mine branch dropped its scope: {m}");

        let unknown = Ownership { unclaimed: vec!["a.rs".to_string()], ..Default::default() };
        let u = build("/repo", &dirty, &unknown, "SCOPE-MARKER").expect("unknown branch");
        assert!(u.contains("SCOPE-MARKER"), "unknown branch dropped its scope: {u}");
    }

    /// The reported case, run against a REAL repo because the whole defect is a
    /// disagreement between what `git status` says and what origin holds — a
    /// mocked porcelain string cannot express it (creative-dna, 2026-08-10).
    ///
    /// A file added on origin AFTER local HEAD reads `??` from `git status`,
    /// because the local index predates the commit that added it. The earlier
    /// comment here asserted untracked paths "cannot match origin by
    /// definition" and this test is what refutes it: the assertion below fails
    /// the moment anyone "corrects" the code to trust the porcelain letter.
    #[tokio::test]
    async fn untracked_but_present_on_origin_is_a_phantom() {
        let root = std::env::temp_dir().join(format!("amux-nudge-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (bare, work) = (root.join("origin.git"), root.join("work"));
        std::fs::create_dir_all(&work).unwrap();
        let git = |dir: &std::path::Path, args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        std::process::Command::new("git")
            .args(["init", "--bare", "-b", "main"])
            .arg(&bare)
            .output()
            .unwrap();
        git(&work, &["init", "-b", "main"]);
        git(&work, &["config", "user.email", "t@t"]);
        git(&work, &["config", "user.name", "t"]);
        git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
        std::fs::write(work.join("base.txt"), "base\n").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-m", "base"]);
        git(&work, &["push", "-q", "origin", "main"]);

        // A peer lands a file on origin; we fetch the ref but never merge, so
        // local HEAD does not contain it.
        let peer = root.join("peer");
        std::process::Command::new("git")
            .args(["clone", "-q", bare.to_str().unwrap()])
            .arg(&peer)
            .output()
            .unwrap();
        git(&peer, &["config", "user.email", "t@t"]);
        git(&peer, &["config", "user.name", "t"]);
        std::fs::write(peer.join("landed.txt"), "from peer\n").unwrap();
        git(&peer, &["add", "-A"]);
        git(&peer, &["commit", "-m", "peer"]);
        git(&peer, &["push", "-q", "origin", "main"]);

        // Same bytes appear locally. `git status` calls this UNTRACKED.
        std::fs::write(work.join("landed.txt"), "from peer\n").unwrap();
        // ...and genuine unprotected WIP, which must survive the filter.
        std::fs::write(work.join("mywip.txt"), "mine\n").unwrap();

        let dir = work.to_str().unwrap();
        let porcelain = String::from_utf8(
            std::process::Command::new("git")
                .args(["-C", dir, "status", "--porcelain"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        assert!(porcelain.contains("?? landed.txt"), "premise: {porcelain}");

        let (kept, provenance) = drop_paths_identical_to_origin(
            dir,
            vec!["landed.txt".into(), "mywip.txt".into()],
        )
        .await;

        assert!(!kept.contains(&"landed.txt".to_string()), "untracked-but-on-origin is a phantom");
        assert!(kept.contains(&"mywip.txt".to_string()), "real WIP must never be dropped");
        assert!(provenance.contains("origin/main"), "must state what it compared against");
        let _ = std::fs::remove_dir_all(&root);
    }

    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    /// THE INCIDENT, rebuilt from its own numbers. 11 dirty files, none mine.
    /// Python nudged anyway and named them as work to commit; three sweeps
    /// followed. The honest output is silence.
    #[test]
    fn when_every_dirty_file_is_a_peers_there_is_no_nudge() {
        let dirty = s(&[
            "crates/a.rs", "crates/b.rs", "crates/c.rs", "crates/d.rs",
            "crates/e.rs", "crates/f.rs", "crates/g.rs", "crates/h.rs",
            "crates/i.rs", "crates/j.rs", "crates/k.rs",
        ]);
        let own = Ownership {
            foreign: dirty.iter().map(|p| (p.clone(), "amux-rust".into())).collect(),
            ..Default::default()
        };
        assert!(
            build("/repo", &dirty, &own, "test-provenance").is_none(),
            "a session with no work of its own must not be told to commit"
        );
    }

    /// The mirror: one of mine among a peer's must still nudge, and must NOT
    /// count theirs. Python's suppression branch got this wrong by comparing a
    /// pre-filter count to a post-filter one, and ate legitimate nudges.
    #[test]
    fn one_own_file_among_foreign_ones_nudges_and_counts_only_mine() {
        let dirty = s(&["mine.rs", "theirs_a.rs", "theirs_b.rs"]);
        let own = Ownership {
            foreign: vec![
                ("theirs_a.rs".into(), "peer".into()),
                ("theirs_b.rs".into(), "peer".into()),
            ],
            ..Default::default()
        };
        let msg = build("/repo", &dirty, &own, "test-provenance").expect("should nudge");
        assert!(msg.contains("1 uncommitted change(s)"), "must count MINE only: {msg}");
        assert!(msg.contains("mine.rs"));
        assert!(!msg.contains("  theirs_a.rs\n"), "a peer's file is not work to commit: {msg}");
    }

    /// A foreign file is NAMED, not silently filtered. The recipient is about
    /// to run `git add -A`; silence about the peer's file is not the useful
    /// output.
    #[test]
    fn foreign_files_are_named_with_their_owner_and_a_do_not_commit_warning() {
        let dirty = s(&["mine.rs", "theirs.rs"]);
        let own = Ownership {
            foreign: vec![("theirs.rs".into(), "amux-cloud".into())],
            ..Default::default()
        };
        let msg = build("/repo", &dirty, &own, "test-provenance").unwrap();
        assert!(msg.contains("NOT YOURS"), "{msg}");
        assert!(msg.contains("theirs.rs") && msg.contains("amux-cloud"), "{msg}");
        assert!(msg.contains("Do not commit it"), "{msg}");
        assert!(msg.contains("git add -A"), "name the command that causes the sweep: {msg}");
    }

    /// `shared` must NOT suppress. On a repo where two lanes touch one file
    /// routinely, suppressing would silence the nudge permanently — the
    /// opposite over-correction, and Python's comment says so explicitly.
    #[test]
    fn a_contested_file_warns_but_never_suppresses() {
        let dirty = s(&["hot.rs"]);
        let own = Ownership {
            shared: vec![("hot.rs".into(), "peer".into())],
            ..Default::default()
        };
        let msg = build("/repo", &dirty, &own, "test-provenance").expect("shared must not suppress");
        assert!(msg.contains("CONTESTED") && msg.contains("peer"), "{msg}");
        assert!(msg.contains("git add -p"), "per-hunk is the actionable advice: {msg}");
    }

    /// THE REOPEN (Ethan, 2026-08-10). This test previously asserted the
    /// OPPOSITE — that unclaimed counts as mine — and that wrong belief is
    /// exactly what shipped: the nudge told him to "commit completed work now"
    /// about CLAUDE.md, a file he had never touched, which the guard classified
    /// `unclaimed`.
    ///
    /// "No session has an edit record for it" is not "it is yours". Only a
    /// POSITIVE claim is. The honest output is the uncertainty itself.
    #[test]
    fn an_unclaimed_file_is_reported_as_unknown_never_as_yours() {
        let dirty = s(&["CLAUDE.md"]);
        let own = Ownership { unclaimed: s(&["CLAUDE.md"]), ..Default::default() };
        let msg = build("/repo", &dirty, &own, "test-provenance").expect("a dirty tree is still worth reporting");
        assert!(msg.contains("OWNERSHIP IS UNKNOWN"), "{msg}");
        assert!(
            !msg.contains("Commit completed work"),
            "must NOT instruct a commit of work that is not provably yours: {msg}"
        );
        assert!(msg.contains("git add -A"), "name the command that would sweep it: {msg}");
    }

    /// Unknowns alongside real work: the count must cover MINE only, and the
    /// unknown must be disclosed rather than folded in.
    #[test]
    fn unknown_files_are_disclosed_but_not_counted_as_mine() {
        let dirty = s(&["mine.rs", "CLAUDE.md"]);
        let own = Ownership { unclaimed: s(&["CLAUDE.md"]), ..Default::default() };
        let msg = build("/repo", &dirty, &own, "test-provenance").unwrap();
        assert!(msg.contains("1 uncommitted change(s)"), "count MINE only: {msg}");
        assert!(msg.contains("OWNERSHIP UNKNOWN") && msg.contains("CLAUDE.md"), "{msg}");
    }

    /// A blind guard must say so. An empty `foreign` from a partially-blind
    /// scan does NOT clear a peer's files, and the nudge has to pass that on.
    #[test]
    fn partial_attribution_is_disclosed() {
        let dirty = s(&["mine.rs"]);
        let own = Ownership {
            partial: Some("no transcript for cotenant amux-helper".into()),
            ..Default::default()
        };
        let msg = build("/repo", &dirty, &own, "test-provenance").unwrap();
        assert!(msg.contains("ATTRIBUTION IS PARTIAL"), "{msg}");
        assert!(msg.contains("amux-helper"), "name who is invisible: {msg}");
    }

    #[test]
    fn a_clean_tree_says_nothing() {
        assert!(build("/repo", &[], &Ownership::default(), "test-provenance").is_none());
    }

    /// Ten shown, the rest elided — a nudge that pastes 82 paths is a nudge
    /// nobody reads.
    #[test]
    fn a_long_list_is_capped_but_the_count_is_honest() {
        let dirty: Vec<String> = (0..25).map(|i| format!("f{i}.rs")).collect();
        let msg = build("/repo", &dirty, &Ownership::default(), "test-provenance").unwrap();
        assert!(msg.contains("25 uncommitted change(s)"), "count must be the TRUE total: {msg}");
        assert!(msg.contains('…'), "the list must show it was truncated: {msg}");
        assert!(!msg.contains("f24.rs"), "only the first ten are listed: {msg}");
    }
}
