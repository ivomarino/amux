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

/// The freshness axis (MG-1467): for each dirty path, WHICH DIRECTION it
/// differs from origin/main. Parallel to [`Ownership`], and like it,
/// deliberately NOT constructible from a working tree. It carries an answer the
/// caller computed with [`freshness_from_repo`] so that [`build`] stays
/// repo-blind.
///
/// Only `stale` and `same` change what build does. `new` and `edited` are both
/// ordinary commit-worthy work; they are tracked for completeness and so a
/// caller can inspect the split, but build derives "commit-worthy" as
/// everything not-same and not-stale rather than reading them. That keeps a
/// default (empty) Freshness behaving exactly like it did before this axis.
#[derive(Debug, Default, Clone)]
pub struct Freshness {
    /// origin has commits on the path that local HEAD lacks: the worktree copy
    /// is OLDER. Committing it SILENTLY REVERTS origin. Restore, do not commit.
    pub stale: Vec<String>,
    /// BOTH directions have commits on the path — novel and stale at once.
    /// Neither single-arm remedy is safe: commit reverts origin's side,
    /// restore reverts the local side, and every restore-safety test passes
    /// while doing it (locally-committed content is "reachable from a commit"
    /// too). Live specimen 2026-08-20: mixpeek .githooks/pre-push, origin
    /// carrying 96ea161803 and local HEAD carrying the MG-1483 guard — the
    /// two-bucket classifier filed it STALE and the prescribed restore
    /// silently disarmed a data-loss push guard. Merge, or hand to the owner.
    pub diverged: Vec<String>,
    /// Absent from origin/main. Genuinely new work; LOST if never committed.
    pub new: Vec<String>,
    /// Differs from origin, which has not moved past HEAD on this path. A
    /// plausible in-flight edit.
    pub edited: Vec<String>,
    /// Byte-identical to origin: dirty only because local HEAD is behind.
    /// Suppressed entirely as noise (48 of 321 on the motivating checkout).
    pub same: Vec<String>,
}

/// The nudge, or None when there is nothing honest to say.
///
/// `dirty` is the list of paths `git status` reported under the session's
/// working directory — a LIST, carrying no ownership claim.
///
/// Two axes arrive pre-computed, for one reason: [`build`] is deliberately
/// unable to see a repo, so it cannot re-derive either by accident. `own` is
/// WHOSE a file is (the staged-guard's answer). `fresh` is WHICH DIRECTION it
/// differs from origin/main (MG-1467). They need opposite handling, so one
/// undifferentiated count cannot serve both. It mis-instructed five sessions in
/// three days, telling them to commit files a commit would SILENTLY REVERT.
pub fn build(
    dir: &str,
    dirty: &[String],
    own: &Ownership,
    fresh: &Freshness,
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

    // FRESHNESS is the second axis (MG-1467). Ownership answers WHOSE a file is;
    // freshness answers WHICH DIRECTION it differs from origin/main, and two of
    // its four classes flip what build says, in OPPOSITE directions. That is why
    // one count mis-instructed five sessions in three days:
    //
    //   SAME:  worktree is byte-identical to origin, dirty only because local
    //          HEAD is behind (a graft-push artifact). Nothing is wrong, so it
    //          is SUPPRESSED. 48 of 321 paths were this on the checkout that
    //          motivated the card, and they are why the raw count read alarming.
    //   STALE: origin has commits on the path that local HEAD lacks, so the
    //          worktree copy is OLDER. `git add -A` carries it forward and
    //          SILENTLY REVERTS origin (no conflict, the older file just wins,
    //          AMUX-3000). Rendered FIRST, with the RESTORE command, and never
    //          told to commit. This is the inverse of the normal nudge.
    //
    // NEW and EDITED are ordinary commit-worthy work and keep today's messaging.
    // build derives "commit-worthy" as everything NOT same and NOT stale, so a
    // default (empty) Freshness reproduces today's behaviour exactly. That is
    // also the honest state when local HEAD is not behind origin.
    let same: BTreeSet<&str> = fresh.same.iter().map(String::as_str).collect();
    let stale_set: BTreeSet<&str> = fresh.stale.iter().map(String::as_str).collect();
    let diverged_set: BTreeSet<&str> = fresh.diverged.iter().map(String::as_str).collect();

    // Drop SAME first. If that empties the set there is nothing honest to say:
    // identical-to-origin is the same non-event as a clean tree.
    let dirty: Vec<&str> =
        dirty.iter().map(String::as_str).filter(|p| !same.contains(*p)).collect();
    if dirty.is_empty() {
        return None;
    }

    let stale_paths: Vec<&str> =
        dirty.iter().copied().filter(|p| stale_set.contains(*p)).collect();
    let diverged_paths: Vec<&str> =
        dirty.iter().copied().filter(|p| diverged_set.contains(*p)).collect();
    let commit_worthy: Vec<String> = dirty
        .iter()
        .copied()
        .filter(|p| !stale_set.contains(*p) && !diverged_set.contains(*p))
        .map(str::to_string)
        .collect();

    let mut sections: Vec<String> = Vec::new();

    // DIVERGED before everything: both remedies the rest of this message
    // teaches are wrong for these paths, and the reader must hit that before
    // either recipe. The missing cell that disarmed the mixpeek MG-1483 guard
    // (2026-08-20): both directions carry commits, so commit reverts origin's
    // side, restore reverts the local side, and the restore-safety check
    // passes throughout because locally-committed content is
    // reachable-from-a-commit too.
    if !diverged_paths.is_empty() {
        let n = diverged_paths.len();
        let list: String = diverged_paths
            .iter()
            .take(10)
            .map(|p| format!("  {p}\n"))
            .collect::<String>()
            + if n > 10 { "  …\n" } else { "" };
        sections.push(format!(
            "DIVERGED: {n} of your dirty file(s) under {dir} have commits in BOTH directions — \
             origin/main carries commits on them that your HEAD lacks, AND your HEAD carries \
             commits origin lacks. They are novel and stale AT ONCE, so NEITHER standard remedy \
             is safe:\n{list}\
             Committing carries your side forward and SILENTLY REVERTS origin's commits; \
             `git checkout origin/main -- <path>` reverts YOUR landed commits — and the \
             find-object restore-safety check PASSES while it does, because locally-committed \
             content is reachable-from-a-commit too (that is how the mixpeek MG-1483 push guard \
             was silently disarmed, 2026-08-20). MERGE the two versions (for append-only files, \
             union-merge per .claude/rules/frustrations.md), or hand the path to its owner. Do \
             not clear these with any single-arm command."
        ));
    }

    // STALE FIRST and most prominently. The opposite instruction to the rest of
    // the nudge: do NOT commit these, restore them.
    //
    // RESTORE-SAFETY DISCRIMINATOR (AMUX-3264, cold-outbound live near-miss
    // 2026-08-17). Once a path is known behind origin, the remaining question is
    // pure-old-copy (safe to `git checkout origin/main -- <path>`) vs
    // carries-novel-mid-edit (a restore DELETES it irreversibly). The advice
    // below prescribes `git log --all --oneline --find-object=<blob> -- <path>`,
    // which answers by REACHABILITY FROM A COMMIT: non-empty means the exact
    // content is in a commit somewhere, empty means it is in none.
    //
    // Both `--all` and the `-- <path>` pathspec look droppable and are NOT:
    //   * `--all` (not HEAD-only): a blob committed on another branch or on
    //     origin reads EMPTY under a HEAD-only search and would be misjudged
    //     novel. `--all` widens what counts as committed so a genuine old copy
    //     living elsewhere is still recognised.
    //   * `-- <path>` (cold-outbound): a blob committed under a DIFFERENT path (a
    //     move, a copied fixture) reads not-found for THIS path.
    // Every residual edge case (other-path, an errored or timed-out command, an
    // empty result) yields a FALSE "novel", which the advice resolves to
    // DO-NOT-RESTORE. That bias is deliberate and MUST NOT be tightened back
    // toward the destructive side: a declined restore is recoverable, a deleted
    // keystroke is not. The unsound recipe this replaces,
    // `git cat-file -e $(git hash-object <path>)`, failed OPEN: `git add` alone
    // writes the blob into the object DB without committing, so it answered EXISTS
    // for a never-committed mid-edit and its `git checkout` remedy deleted it.
    if !stale_paths.is_empty() {
        let n = stale_paths.len();
        let list: String = stale_paths
            .iter()
            .take(10)
            .map(|p| format!("  {p}\n"))
            .collect::<String>()
            + if n > 10 { "  …\n" } else { "" };
        sections.push(format!(
            "STALE: {n} of your dirty file(s) under {dir} are OLDER than origin/main. Origin \
             has commits on these paths that your local HEAD does not (this checkout is behind). \
             DO NOT COMMIT them. `git add -A` or `git commit -a` would carry the older copy \
             forward and SILENTLY REVERT origin (no conflict, the older file just wins, \
             AMUX-3000):\n\
             {list}\
             But do NOT blind-restore them either. Behind-on-history does not prove the \
             worktree is a pure old copy: a path can be behind origin AND carry NOVEL \
             uncommitted content (mid-edit), and `git checkout origin/main -- <path>` DELETES \
             that novel work irreversibly (AMUX-3172/AMUX-3188; social-media caught 16 such \
             paths whose worktree matched NEITHER local HEAD nor origin). PROVE the copy is a \
             pure old revision per path BEFORE restoring: \
             `git log --all --oneline --find-object=$(git hash-object <path>) -- <path>`. RESTORE \
             (`git checkout origin/main -- <path>`) ONLY if it prints a commit: that means this \
             exact content is reachable from a commit on some ref, so it is a genuine old copy and \
             restoring loses nothing. ANY other outcome is DO-NOT-RESTORE, commit the path \
             instead. An empty result means the content is in NO commit anywhere, a novel mid-edit \
             a restore would DELETE; an error, a timeout, or a result you cannot read is also \
             DO-NOT-RESTORE, because a declined restore is recoverable and a deleted keystroke is \
             not. Do NOT substitute `git cat-file -e $(git hash-object <path>)`: `git add` (and \
             `git hash-object -w`) writes the blob into the object DB WITHOUT committing, so it \
             answers yes for a never-committed mid-edit and cannot separate a committed old copy \
             from novel work. It is strictly weaker than was-this-committed and its remedy here is \
             a delete (cold-outbound, 2026-08-17: server-fast-checks.yml was mid-keystroke and in \
             no commit on any ref, yet that recipe answered yes; restoring would have deleted it)."
        ));
    }

    // The commit-worthy body: NEW/EDITED paths, rendered under the ownership
    // axis exactly as before the freshness split.
    if let Some(body) = commit_worthy_body(dir, &commit_worthy, own) {
        sections.push(body);
    }

    if sections.is_empty() {
        // Everything was SAME (returned above) or the paths left were
        // foreign/unknown with nothing positively ours: no stale warning and
        // nothing to commit.
        return None;
    }
    let mut msg = sections.join("\n\n");
    msg.push_str(&format!("\n\n({provenance})"));
    Some(msg)
}

/// The commit-worthy body: paths that are neither STALE nor identical to origin,
/// rendered under the ownership axis (mine / unknown / foreign / shared). Returns
/// the message WITHOUT the trailing provenance so [`build`] can stamp it exactly
/// once across a STALE section and this body.
/// An append-only, multi-writer shared file — `frustrations.md` is the canonical
/// one (matched case-insensitively; macOS resolves FRUSTRATIONS.md to the same
/// file). Its whole failure mode is that the two direction remedies this nudge
/// prescribes BOTH lose data on it, so it needs its own directive.
fn is_append_only_shared(path: &str) -> bool {
    std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .eq_ignore_ascii_case("frustrations.md")
}

/// The union-merge directive for any append-only shared file in the dirty set,
/// or None if there is none. Appended to whichever nudge fires (AMUX-3367): an
/// append-only file can be NOVEL AND STALE at once — you appended locally while a
/// peer landed different entries on origin — so committing the path REVERTS the
/// peer's entries and restoring DELETES yours. The direction test cannot separate
/// these because both are true, so name the file and prescribe the only safe
/// operation, which neither generic remedy performs.
fn append_only_note(dirty: &[String]) -> Option<String> {
    let mut ao: Vec<&str> = dirty
        .iter()
        .map(String::as_str)
        .filter(|p| is_append_only_shared(p))
        .collect();
    ao.sort();
    ao.dedup();
    if ao.is_empty() {
        return None;
    }
    Some(format!(
        "\n\nAPPEND-ONLY SHARED FILE — {} can be NOVEL AND STALE at once (you and a peer \
         appended to different bases), so BOTH remedies above lose data on it: committing the \
         path REVERTS the peer's origin-only entries, `git checkout origin/main -- <path>` \
         DELETES yours. UNION-MERGE instead — `git checkout origin/main -- <path>` to take their \
         version, then RE-APPEND your entries and commit. Never a plain add/commit or a bare \
         restore (AMUX-3367).",
        ao.join(", ")
    ))
}

fn commit_worthy_body(dir: &str, dirty: &[String], own: &Ownership) -> Option<String> {
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
             mid-edit on — OR a STALE copy origin/main has moved past, which committing would \
             silently REVERT (no conflict; the older file just wins, AMUX-3000). For each, \
             prove the direction with the ANCESTRY test, NOT the diff line-count: \
             worktree-has-more/has-less misclassifies about 2 in 10 (studio-plg measured it on \
             this checkout: files with MORE lines than origin that are nonetheless stale). \
             `git log --oneline HEAD..origin/main -- <path>`: prints a commit = origin has work \
             you lack = STALE, so do not commit; restore with `git checkout origin/main -- <path>` \
             ONLY after `git log --all --find-object=$(git hash-object <path>) -- <path>` prints a \
             commit (empty or errored means the stale copy carries novel mid-edit a restore would \
             DELETE, so commit the path instead) AND the reverse test \
             `git log --oneline origin/main..HEAD -- <path>` prints NOTHING — if BOTH directions \
             print commits the path has DIVERGED (novel and stale at once) and either single-arm \
             remedy destroys one side's landed work: MERGE the versions or hand the path to its \
             owner; \
             prints NOTHING = origin has nothing you lack, so it is current content, and if it \
             is not yours it is a peer's mid-edit: hands off either way. Do NOT use \
             `git cat-file -e $(git hash-object <path>)` for this: blob existence cannot separate \
             an OLD revision from a CURRENT one that is merely unpushed, so on a checkout ahead \
             of origin it calls the whole tree STALE and its remedy reverts it. \
             Stage only what you recognise as your work AND whose ancestry test prints nothing.",
            if n == 1 { "it" } else { "them" },
            if n == 1 { "it is" } else { "they are" },
        );
        if let Some(why) = &own.partial {
            m.push_str(&format!("\n\nATTRIBUTION IS PARTIAL — {why}"));
        }
        if let Some(note) = append_only_note(dirty) {
            m.push_str(&note);
        }
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
         A DIFFERENCE FROM origin/main IS NOT A DIRECTION (AMUX-3000). Each of these differs \
         from origin/main, but the guard does not know which side is NEWER — and on this \
         graft-push checkout the worktree is frequently the OLDER side. A stale worktree copy \
         is a LOADED REVERT: `git add -A` carries the older version forward and silently undoes \
         the newer one on origin (no conflict, the older file just wins). So BEFORE staging, \
         PROVE the direction per path with the ANCESTRY test. Do NOT read the line-count of a \
         `git diff` as the verdict: worktree-has-more/has-less misclassifies about 2 in 10 \
         (studio-plg measured it on this checkout: files with MORE lines than origin that are \
         nonetheless stale, because origin has since moved to a shorter version). Roughly 1 in 4 \
         differing paths here are novel mid-edit a blind `checkout` would DESTROY irreversibly, \
         so the direction test is the headline, not a caveat:\n\
         \u{2022} `git log --oneline HEAD..origin/main -- <path>`: if it prints ANY commit, \
         origin has work on this path that your HEAD lacks, so the worktree copy is genuinely \
         older (STALE); do not commit. Restore with `git checkout origin/main -- <path>` ONLY \
         after BOTH: `git log --all --find-object=$(git hash-object <path>) -- <path>` prints a \
         commit (empty or errored means the stale copy carries novel mid-edit a restore would \
         DELETE, so commit the path instead), AND the REVERSE test \
         `git log --oneline origin/main..HEAD -- <path>` prints NOTHING.\n\
         \u{2022} if BOTH directions print commits, the path has DIVERGED — novel AND stale at \
         once — and commit and restore each destroy one side's landed work while every test \
         above passes as prescribed. MERGE the two versions (or hand the path to its owner); \
         neither single-arm remedy is safe. Live specimen 2026-08-20: mixpeek \
         .githooks/pre-push, origin carrying 96ea161803 and HEAD carrying the MG-1483 guard \
         chain — the one-direction protocol read it STALE and the prescribed restore silently \
         disarmed a data-loss push guard.\n\
         \u{2022} if it prints NOTHING, origin has nothing you lack: the content is yours to \
         keep, `checkout` would DESTROY it, and the safe action is COMMIT, not restore.\n\
         This is the same predicate the guard itself classifies with, so its verdict and your \
         check cannot disagree. Do NOT substitute \
         `git cat-file -e $(git hash-object <path>)`: blob existence cannot tell an OLD revision \
         from a CURRENT one that is merely unpushed, because both were committed at some point \
         and both answer yes. On a checkout that sits ahead of origin every committed file \
         answers yes, so that test reports STALE for the entire tree and its remedy reverts it \
         (AMUX-3000 follow-up, measured 2026-08-16: five committed-but-unpushed paths, five \
         false STALEs, one of them a whole feature shipped that day).\n\
         When the check is ambiguous or you cannot run it, commit — a redundant commit is \
         recoverable and a restore is not. (A `git diff` still shows WHAT changed; just never \
         read worktree-has-more as newer.)\n\
         Commit only the paths whose ancestry test prints nothing, with a clear message; \
         WIP-commit anything intentionally incomplete and say so. Don't leave the working tree \
         dirty — but don't commit a revert to clear it, either."
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
    if let Some(note) = append_only_note(dirty) {
        msg.push_str(&note);
    }
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

    // RESOLVE THE REPO ROOT AND COMPARE FROM THERE (AMUX-2947).
    //
    // `git status --porcelain` emits REPO-ROOT-relative paths, but `dir` is the
    // lane's CC_DIR, which is frequently a SUBDIRECTORY — creative-dna's is
    // /Users/ethan/Dev/mixpeek/gtm/creative-dna. Run from there,
    // `git hash-object -- .github/workflows/x.yml` resolves against the CWD and
    // dies with "could not open 'gtm/.github/workflows/x.yml'". The command
    // fails, the match arm below cannot compare, and its deliberately
    // conservative `_ => false` keeps the path.
    //
    // So the filter degraded to a NO-OP for every lane not sitting at a repo
    // root, and it did so silently while the provenance line kept saying
    // "compared against origin/main (just fetched)". Measured by creative-dna
    // across all 55 paths in one report: 34 byte-identical to origin, i.e. 62%
    // false, on a guard whose own text warns the reader those files may be a
    // peer's mid-edit work. A guard that is wrong most of the time trains
    // people to skim it, and the 19 real ones were in there.
    //
    // `cat-file -e origin/main:<path>` is why it was not obvious: that one
    // addresses the OBJECT DB and is root-relative, so it succeeds from any
    // subdir. Existence checks passed, content checks failed, everything was
    // kept.
    let root = tokio::process::Command::new("git")
        .args(["-C", dir, "rev-parse", "--show-toplevel"])
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| dir.to_string());
    let dir = root.as_str();

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
        // VALIDATE THE SHAPE, DO NOT TRUST THE EXIT CODE (creative-dna,
        // AMUX-2947 follow-up). Two reasons, and the second is the dangerous
        // one:
        //
        //   1. NOT because git's exit status is unreliable — it is. That was
        //      briefly believed here and is now settled: `hash-object` on an
        //      unreadable path exits 128, loudly and consistently. The 0 came
        //      from a piped measurement reading `head`'s status rather than
        //      git's (creative-dna measured it, then re-measured with
        //      PIPESTATUS and corrected the record). Recorded because a comment
        //      hinting that a core git command lies is a hazard note someone
        //      would act on, and it would be wrong.
        //
        //   2. The reason that stands, and it is the serious one: comparing two
        //      unvalidated strings has an inverse failure
        //      the conservative `_ => false` arm does NOT cover: if both sides
        //      came back empty, `"" == ""` is TRUE and the path is DROPPED — a
        //      real uncommitted file silently removed from the warning. Keeping
        //      a phantom is noise; dropping a genuine one loses somebody's work
        //      from the only notice that mentions it.
        //
        // A blob id is exactly 40 lowercase hex characters. Anything else is
        // not an answer, whatever the exit code said.
        let blob = |o: &std::process::Output| -> Option<String> {
            let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
            (t.len() == 40 && t.chars().all(|c| c.is_ascii_hexdigit())).then_some(t)
        };
        let same = match (local, remote) {
            (Ok(l), Ok(r)) => match (blob(&l), blob(&r)) {
                (Some(a), Some(b)) => a == b,
                // One side unreadable -> cannot compare -> keep it. A path we
                // failed to check is not a path we have cleared.
                _ => false,
            },
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

/// Per-path freshness for `paths`, computed against origin/main (MG-1467).
///
/// The second axis the guard never had. [`ownership_from_guard`] answers WHOSE a
/// file is; this answers WHICH DIRECTION it differs from origin. Both hazards are
/// real and need OPPOSITE handling (committing a peer's file vs committing a
/// stale copy that reverts origin), so one count cannot serve both.
///
/// Caller-side exactly like ownership, because it needs the repo and [`build`] is
/// deliberately repo-blind. It reads the origin/main ref that
/// [`drop_paths_identical_to_origin`] refreshed earlier in the same tick, so it
/// MUST run after that filter. It does not fetch again.
///
/// When it cannot classify a path (git error, no origin/main ref) it treats the
/// path as ordinary work, never STALE, so a failure degrades to today's
/// behaviour rather than inventing a revert warning.
async fn freshness_from_repo(dir: &str, paths: &[String]) -> Freshness {
    let mut fresh = Freshness::default();
    if paths.is_empty() {
        return fresh;
    }

    // Resolve the repo root and address paths from there. `git status` emits
    // repo-root-relative paths, but a lane's CC_DIR is often a SUBDIRECTORY; run
    // from there, `git hash-object -- <path>` resolves against the CWD and fails
    // (AMUX-2947). Same trap drop_paths_identical_to_origin documents.
    let root = tokio::process::Command::new("git")
        .args(["-C", dir, "rev-parse", "--show-toplevel"])
        .output()
        .await
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| dir.to_string());
    let dir = root.as_str();

    // A blob id is exactly 40 lowercase hex characters. Anything else, an error
    // message or an empty string, is NOT an answer whatever the exit code said.
    // This is the creative-dna trap: `"" == ""` is true, so two empty stdouts
    // would score as SAME and silently drop a real change.
    let blob = |o: &std::process::Output| -> Option<String> {
        let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
        (t.len() == 40 && t.chars().all(|c| c.is_ascii_hexdigit())).then_some(t)
    };

    for p in paths {
        // 1. NEW: absent from origin. TEST THE EXIT CODE, never stdout emptiness.
        //    Plain `git rev-parse` ECHOES an unresolvable revspec to stdout and
        //    still exits nonzero, so an is-empty check reports every path as
        //    present. Three sessions shipped that bug 2026-08-10. `--verify -q`
        //    keeps stdout clean; `.status.success()` is the only signal read.
        let present = tokio::process::Command::new("git")
            .args(["-C", dir, "rev-parse", "--verify", "-q", &format!("origin/main:{p}")])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !present {
            fresh.new.push(p.clone());
            continue;
        }

        // 2. SAME: worktree byte-identical to origin. Dirty only because local
        //    HEAD is behind; nothing is wrong, so build suppresses it.
        let local = tokio::process::Command::new("git")
            .args(["-C", dir, "hash-object", "--", p])
            .output()
            .await;
        let remote = tokio::process::Command::new("git")
            .args(["-C", dir, "rev-parse", &format!("origin/main:{p}")])
            .output()
            .await;
        let identical = match (local, remote) {
            (Ok(l), Ok(r)) => match (blob(&l), blob(&r)) {
                (Some(a), Some(b)) => a == b,
                // One side unreadable: cannot call it SAME, fall through.
                _ => false,
            },
            _ => false,
        };
        if identical {
            fresh.same.push(p.clone());
            continue;
        }

        // 3. STALE: origin has commits on this path that local HEAD lacks, so
        //    the worktree copy is older and committing it would REVERT origin.
        //    Here stdout emptiness IS the right test: this is `git log` output,
        //    not a revspec echo. Gated on the command succeeding.
        let stale = tokio::process::Command::new("git")
            .args(["-C", dir, "log", "--oneline", "HEAD..origin/main", "--", p])
            .output()
            .await
            .map(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            .unwrap_or(false);
        if stale {
            // 3b. DIVERGED: local HEAD ALSO has commits origin lacks on this
            //     path. Filing it STALE prescribes a restore that reverts the
            //     local side's landed work — and the restore-safety check
            //     passes while it happens, because locally-committed content
            //     is reachable-from-a-commit too (the mixpeek .githooks
            //     disarm, 2026-08-20). The missing cell, not a tighter test.
            let local_ahead = tokio::process::Command::new("git")
                .args(["-C", dir, "log", "--oneline", "origin/main..HEAD", "--", p])
                .output()
                .await
                .map(|o| {
                    o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty()
                })
                .unwrap_or(false);
            if local_ahead {
                fresh.diverged.push(p.clone());
            } else {
                fresh.stale.push(p.clone());
            }
            continue;
        }

        // 4. EDITED: differs, origin has not moved past HEAD on this path. A
        //    plausible in-flight edit; commit-worthy like today.
        fresh.edited.push(p.clone());
    }
    fresh
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
        // The freshness axis (MG-1467): WHICH DIRECTION each path differs from
        // origin. Computed here, caller-side, exactly as ownership is; build
        // stays repo-blind. Runs after drop_paths_identical_to_origin so it
        // reads the origin/main ref that filter just refreshed.
        let fresh = freshness_from_repo(dir, &dirty).await;
        // Provenance rides on EVERY nudge, not only the degraded ones. It is the
        // healthy line that makes the stale line legible: with a stamp present
        // in both states, "compared against a STALE origin/main" reads as a
        // difference. Printed only when degraded, it reads as ordinary prose and
        // gets skimmed exactly like the phantom paths it is warning about.
        let Some(msg) = build(dir, &dirty, &own, &fresh, &provenance) else { continue };

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
        let m = build("/repo", &dirty, &Ownership::default(), &Freshness::default(), "SCOPE-MARKER")
            .expect("mine branch");
        assert!(m.contains("SCOPE-MARKER"), "mine branch dropped its scope: {m}");

        let unknown = Ownership { unclaimed: vec!["a.rs".to_string()], ..Default::default() };
        let u = build("/repo", &dirty, &unknown, &Freshness::default(), "SCOPE-MARKER")
            .expect("unknown branch");
        assert!(u.contains("SCOPE-MARKER"), "unknown branch dropped its scope: {u}");
    }

    /// The advice a nudge PRINTS must use the same predicate the guard
    /// CLASSIFIES with, or the reader's own check contradicts the verdict.
    ///
    /// This shipped broken: the direction-unknown branches told the reader to
    /// run `git cat-file -e $(git hash-object <path>)` and treat "object
    /// exists" as STALE, while `freshness_from_repo` classifies with
    /// `git log HEAD..origin/main -- <path>`. Blob existence cannot separate an
    /// OLD revision from a CURRENT one that is merely unpushed, since both were
    /// committed at some point. On this checkout, which sits ~44 commits ahead
    /// of origin, EVERY committed file answered "exists", so the printed recipe
    /// reported the whole tree STALE and its remedy was a revert. Measured
    /// 2026-08-16: five committed-but-unpushed paths, five false STALEs, one of
    /// them a feature shipped that day.
    ///
    /// Asserted on the message TEXT because the text is the thing that was
    /// wrong; the classifier was right the whole time.
    #[test]
    fn printed_direction_test_matches_the_classifier() {
        let dirty = vec!["a.rs".to_string()];
        let unknown = Ownership { unclaimed: vec!["a.rs".to_string()], ..Default::default() };
        let msgs = [
            build("/repo", &dirty, &Ownership::default(), &Freshness::default(), "S")
                .expect("mine branch"),
            build("/repo", &dirty, &unknown, &Freshness::default(), "S")
                .expect("unknown branch"),
        ];
        for m in &msgs {
            assert!(
                m.contains("HEAD..origin/main"),
                "nudge must prescribe the ancestry test it classifies with: {m}"
            );
            // The old recipe may still appear as an explicit warning NOT to use
            // it, but never as the prescribed check.
            let prescribes_blob = m.contains("`git cat-file -e $(git hash-object <path>) 2>/dev/null`: object EXISTS")
                || m.contains("prove the direction with the OBJECT-EXISTENCE test");
            assert!(
                !prescribes_blob,
                "nudge prescribes blob-existence as the DIRECTION test; it reports STALE for \
                 every committed-but-unpushed file and its remedy reverts them: {m}"
            );
        }
    }

    /// The DIVERGED cell must render as its own section that forbids BOTH
    /// single-arm remedies (the mixpeek MG-1483 disarm, 2026-08-20): a path in
    /// `diverged` must never be listed as commit-worthy or STALE, and the text
    /// must name the merge as the only exit.
    #[test]
    fn diverged_paths_get_their_own_section_and_leave_both_recipes() {
        let dirty = vec!["hooks/pre-push".to_string()];
        let fresh =
            Freshness { diverged: vec!["hooks/pre-push".to_string()], ..Default::default() };
        let m = build("/repo", &dirty, &Ownership::default(), &fresh, "S")
            .expect("diverged section must render");
        assert!(m.contains("DIVERGED:"), "the cell must have its own section: {m}");
        assert!(
            m.contains("reachable-from-a-commit too"),
            "must say WHY the restore-safety check cannot catch this: {m}"
        );
        assert!(m.contains("MERGE the two versions"), "the only safe exit must be named: {m}");
        assert!(
            !m.contains("STALE:"),
            "a diverged path must not also render the STALE restore recipe: {m}"
        );
    }

    /// The STALE section's RESTORE-SAFETY check must be the reachable-from-a-commit
    /// test, never blob existence (AMUX-3264, cold-outbound near-miss 2026-08-17).
    ///
    /// 5b923db fixed the two DIRECTION branches but DELIBERATELY left this section
    /// using `git cat-file -e $(git hash-object <path>)`, reasoning it answered
    /// pure-old-copy vs mid-edit correctly. It does not: `git add` alone writes the
    /// blob into the object DB without committing, so the recipe returns EXISTS for
    /// a never-committed mid-edit and its `git checkout` remedy DELETES it. This
    /// asserts on the message TEXT so a reintroduction of the recipe fails here.
    #[test]
    fn stale_section_prescribes_find_object_never_blob_existence() {
        let dirty = vec!["a.rs".to_string()];
        let fresh = Freshness { stale: vec!["a.rs".to_string()], ..Default::default() };
        let m = build("/repo", &dirty, &Ownership::default(), &fresh, "S")
            .expect("stale section");
        assert!(m.contains("STALE:"), "premise: the stale section must render: {m}");
        // The correct restore-safety discriminator must be prescribed. A revert to
        // the old recipe removes this substring and fails the test.
        assert!(
            m.contains("--find-object=$(git hash-object <path>)"),
            "STALE section must prescribe find-object as the restore-safety check: {m}"
        );
        // The exact old prescription tied "object EXISTS" to "committed before" to
        // "safely restores". Blob existence may only appear inside a do-not warning.
        assert!(
            !m.contains("object EXISTS, this exact content was committed before"),
            "STALE section reintroduced blob-existence as the safe-to-restore check; git add \
             writes the blob uncommitted, so its restore deletes mid-edits: {m}"
        );
        assert!(
            m.contains("Do NOT substitute `git cat-file -e $(git hash-object <path>)`"),
            "STALE section must name blob-existence as the recipe NOT to use: {m}"
        );
    }

    /// cold-outbound's proof at the DISCRIMINATOR level (AMUX-3264, live 4-minute
    /// near-miss on .github/workflows/server-fast-checks.yml, 2026-08-17), run
    /// against a REAL repo because the whole defect is a disagreement between what
    /// the object DB holds and what any commit holds.
    ///
    /// The restore-safety check must separate a pure old copy (safe to
    /// `git checkout origin/main -- <path>`) from a NOVEL mid-edit (whose restore
    /// is an irreversible delete), and it must FAIL CLOSED: ANY inconclusive answer
    /// resolves to DO-NOT-RESTORE, never to safe. This asserts the PROPERTY across
    /// states, not one case, because failing OPEN on some third state is exactly
    /// how `git cat-file -e $(git hash-object <path>)` got here.
    #[test]
    fn find_object_restore_check_fails_closed_on_every_inconclusive_state() {
        let root = std::env::temp_dir().join(format!("amux-nudge-disc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let run = |args: &[&str]| -> std::process::Output {
            std::process::Command::new("git").arg("-C").arg(&root).args(args).output().unwrap()
        };
        let must = |args: &[&str]| {
            let out = run(args);
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        let blob_of = |name: &str| {
            String::from_utf8(run(&["hash-object", name]).stdout).unwrap().trim().to_string()
        };
        // The exact discriminator the advice prescribes, applied through the
        // fail-closed rule the advice states: RESTORE only on a non-empty print
        // from a command that SUCCEEDED; every other outcome (empty, error,
        // timeout) is DO-NOT-RESTORE.
        let safe_to_restore = |blob: &str, path: &str| -> bool {
            let out = run(&["log", "--all", "--oneline", &format!("--find-object={blob}"), "--", path]);
            out.status.success() && !String::from_utf8_lossy(&out.stdout).trim().is_empty()
        };

        must(&["init", "-b", "main"]);
        must(&["config", "user.email", "t@t"]);
        must(&["config", "user.name", "t"]);

        // (1) A committed baseline at f.yml: reachable from a commit => SAFE.
        std::fs::write(root.join("f.yml"), "name: ci\njobs:\n").unwrap();
        must(&["add", "f.yml"]);
        must(&["commit", "-m", "base"]);
        let committed_blob = blob_of("f.yml");
        assert!(
            safe_to_restore(&committed_blob, "f.yml"),
            "a genuine committed old copy must be classified safe to restore"
        );

        // (2) cold-outbound's specimen: a NEW line typed in, `git add`-ed, NEVER
        // committed. The blob is now in the object DB, so the OLD recipe answers
        // EXISTS, but it is in NO commit, so find-object is empty => DO-NOT-RESTORE.
        std::fs::write(root.join("f.yml"), "name: ci\njobs:\n  build: {}\n").unwrap();
        must(&["add", "f.yml"]);
        let staged_blob = blob_of("f.yml");
        assert_ne!(staged_blob, committed_blob, "premise: the mid-edit differs from the committed copy");
        assert!(
            run(&["cat-file", "-e", &staged_blob]).status.success(),
            "premise: git add wrote the blob, so the UNSOUND recipe cat-file -e reports EXISTS"
        );
        assert!(
            !safe_to_restore(&staged_blob, "f.yml"),
            "a staged-but-never-committed mid-edit must be DO-NOT-RESTORE (its restore is a delete)"
        );

        // (3) A blob committed under a DIFFERENT path, queried under THIS path with
        // the `-- <path>` pathspec: not-found here => DO-NOT-RESTORE. A false novel,
        // deliberately so: the pathspec biases toward the safe side.
        std::fs::write(root.join("moved.yml"), "shared: fixture\n").unwrap();
        must(&["add", "moved.yml"]);
        must(&["commit", "-m", "moved"]);
        let moved_blob = blob_of("moved.yml");
        assert!(
            !safe_to_restore(&moved_blob, "f.yml"),
            "content committed only under another path must be DO-NOT-RESTORE for THIS path"
        );

        // (4) A blob committed only on ANOTHER branch: a HEAD-only search reads it
        // as a false novel, which is why the advice uses `--all`. With `--all` it
        // is recognised as a genuine old copy => SAFE.
        must(&["checkout", "-b", "other"]);
        std::fs::write(root.join("f.yml"), "name: ci\njobs:\n  test: {}\n").unwrap();
        must(&["add", "f.yml"]);
        must(&["commit", "-m", "on other"]);
        let other_branch_blob = blob_of("f.yml");
        must(&["checkout", "main"]);
        let head_only = run(&["log", "--oneline", &format!("--find-object={other_branch_blob}"), "--", "f.yml"]);
        assert!(
            String::from_utf8_lossy(&head_only.stdout).trim().is_empty(),
            "premise: a HEAD-only search misses an other-branch blob, which is why --all is required"
        );
        assert!(
            safe_to_restore(&other_branch_blob, "f.yml"),
            "--all must recognise a blob committed on another branch as a genuine old copy"
        );

        // (5) A malformed object id: the command cannot answer => DO-NOT-RESTORE.
        // Fail closed on error, never fall through to safe.
        assert!(
            !safe_to_restore("zzz", "f.yml"),
            "an errored or inconclusive discriminator must resolve to DO-NOT-RESTORE"
        );

        let _ = std::fs::remove_dir_all(&root);
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

    /// TWO EMPTY ANSWERS ARE NOT A MATCH (creative-dna's follow-up).
    ///
    /// The comparison used to be `stdout == stdout` on two unvalidated strings,
    /// gated on exit status. The conservative arm covers "one side failed" —
    /// but NOT the case where both come back empty, where `"" == ""` is true
    /// and the path is DROPPED. Keeping a phantom is noise; dropping a real
    /// uncommitted file removes somebody's work from the only notice that
    /// mentions it, which is the failure worth engineering against.
    ///
    /// Asserts the shape rule directly, because provoking git into returning
    /// two empty stdouts on demand is not something a test should stage — the
    /// rule is what the fix rests on, so the rule is what is pinned.
    #[test]
    fn an_empty_or_malformed_blob_id_is_never_a_match() {
        let looks_like_blob = |t: &str| t.len() == 40 && t.chars().all(|c| c.is_ascii_hexdigit());
        assert!(!looks_like_blob(""), "empty must not pass as a blob id");
        assert!(!looks_like_blob("fatal: could not open"), "an error message must not pass");
        assert!(!looks_like_blob("c98e8388"), "a short hash must not pass");
        assert!(
            !looks_like_blob("z98e83889239f9ef1482196115f0edee24442b74"),
            "40 chars of non-hex must not pass"
        );
        assert!(
            looks_like_blob("c98e83889239f9ef1482196115f0edee24442b74"),
            "a real blob id must pass, or the filter drops everything and the guard goes silent"
        );
    }

    /// THE PHANTOM FILTER MUST WORK FROM A SUBDIRECTORY (AMUX-2947).
    ///
    /// `git status --porcelain` emits repo-root-relative paths, but a lane's
    /// CC_DIR is often a subdir. Run from there, `git hash-object -- <path>`
    /// resolves against the CWD and fails, the comparison cannot be made, and
    /// the conservative "keep it" arm keeps EVERYTHING — so the filter became a
    /// no-op while its provenance line still said "compared against
    /// origin/main". creative-dna measured 34 of 55 reported paths as
    /// byte-identical to origin: 62% false.
    ///
    /// Drives the real function against a real git repo, because the bug is
    /// entirely in how git resolves a path relative to a process CWD — a mocked
    /// git would have reproduced nothing.
    #[tokio::test]
    async fn phantom_filter_still_works_when_the_lane_dir_is_a_subdirectory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let sh = |args: &[&str], cwd: &std::path::Path| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(cwd)
                .output()
                .expect("git")
        };
        sh(&["init", "-q", "--initial-branch=main"], root);
        sh(&["config", "user.email", "t@t"], root);
        sh(&["config", "user.name", "t"], root);
        std::fs::create_dir_all(root.join("sub/lane")).unwrap();
        std::fs::write(root.join("tracked.txt"), "original\n").unwrap();
        sh(&["add", "-A"], root);
        sh(&["commit", "-qm", "seed"], root);
        // A local "origin/main" pointing at the same commit: the file is
        // therefore IDENTICAL to origin and must be filtered out.
        sh(&["update-ref", "refs/remotes/origin/main", "HEAD"], root);
        // Dirty the index only — worktree content still matches origin, which
        // is the phantom shape (a stale index on a graft-push checkout).
        sh(&["update-index", "--assume-unchanged", "tracked.txt"], root);
        std::fs::write(root.join("tracked.txt"), "original\n").unwrap();

        let subdir = root.join("sub/lane");
        let (kept, prov) = drop_paths_identical_to_origin(
            subdir.to_str().unwrap(),
            vec!["tracked.txt".to_string()],
        )
        .await;
        assert!(
            prov.contains("origin/main"),
            "provenance must still describe the comparison: {prov}"
        );
        assert!(
            kept.is_empty(),
            "a worktree file identical to origin must be dropped even when the lane dir is a \
             SUBDIRECTORY — got {kept:?}. Before the fix this kept everything, because \
             hash-object resolved the root-relative path against the subdir and failed."
        );
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
            build("/repo", &dirty, &own, &Freshness::default(), "test-provenance").is_none(),
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
        let msg = build("/repo", &dirty, &own, &Freshness::default(), "test-provenance")
            .expect("should nudge");
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
        let msg = build("/repo", &dirty, &own, &Freshness::default(), "test-provenance").unwrap();
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
        let msg = build("/repo", &dirty, &own, &Freshness::default(), "test-provenance")
            .expect("shared must not suppress");
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
        let msg = build("/repo", &dirty, &own, &Freshness::default(), "test-provenance")
            .expect("a dirty tree is still worth reporting");
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
        let msg = build("/repo", &dirty, &own, &Freshness::default(), "test-provenance").unwrap();
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
        let msg = build("/repo", &dirty, &own, &Freshness::default(), "test-provenance").unwrap();
        assert!(msg.contains("ATTRIBUTION IS PARTIAL"), "{msg}");
        assert!(msg.contains("amux-helper"), "name who is invisible: {msg}");
    }

    #[test]
    fn a_clean_tree_says_nothing() {
        assert!(
            build("/repo", &[], &Ownership::default(), &Freshness::default(), "test-provenance")
                .is_none()
        );
    }

    /// Ten shown, the rest elided — a nudge that pastes 82 paths is a nudge
    /// nobody reads.
    #[test]
    fn a_long_list_is_capped_but_the_count_is_honest() {
        let dirty: Vec<String> = (0..25).map(|i| format!("f{i}.rs")).collect();
        let msg = build("/repo", &dirty, &Ownership::default(), &Freshness::default(), "test-provenance")
            .unwrap();
        assert!(msg.contains("25 uncommitted change(s)"), "count must be the TRUE total: {msg}");
        assert!(msg.contains('…'), "the list must show it was truncated: {msg}");
        assert!(!msg.contains("f24.rs"), "only the first ten are listed: {msg}");
    }

    // -----------------------------------------------------------------------
    // FRESHNESS AXIS (MG-1467). build() renders it, and it is repo-blind, so
    // every case here is driven purely by the Freshness passed in. The two legs
    // that must both hold: STALE warns-not-commits, and SAME vanishes. A build
    // that renders everything fails the SAME legs; one that suppresses
    // everything fails the STALE legs.
    // -----------------------------------------------------------------------

    /// A STALE file is the INVERSE of the normal nudge. It is OLDER than origin,
    /// so committing it reverts origin. It must be rendered with the RESTORE
    /// command and NOT told to commit, and a stale-only tree carries no
    /// commit-worthy work at all.
    #[test]
    fn a_stale_file_is_told_to_restore_and_never_to_commit() {
        let dirty = s(&["stale.rs"]);
        let fresh = Freshness { stale: s(&["stale.rs"]), ..Default::default() };
        let msg = build("/repo", &dirty, &Ownership::default(), &fresh, "test-provenance")
            .expect("a stale file is worth warning about");
        assert!(msg.contains("git checkout origin/main -- "), "must give the RESTORE command: {msg}");
        assert!(msg.contains("DO NOT COMMIT"), "must say not to commit the stale copy: {msg}");
        assert!(msg.contains("stale.rs"), "must name the stale path: {msg}");
        assert!(
            !msg.contains("uncommitted change(s)"),
            "a stale-only tree has NO commit-worthy work; it must not render the commit nudge: {msg}"
        );
    }

    /// AMUX-3188 (social-media): behind-on-history does NOT prove the worktree is
    /// a pure old copy. A stale path can carry novel mid-edit content, and an
    /// unconditional `git checkout origin/main -- <path>` DELETES it irreversibly
    /// (social-media caught 16 such paths). AMUX-3264 (cold-outbound) then showed
    /// the guard this section USED to prescribe, blob existence, is itself unsound:
    /// `git add` writes the blob uncommitted, so it licenses the same destructive
    /// restore for a never-committed mid-edit. The section must prescribe the
    /// reachable-from-a-commit test and gate the restore on it, or it prescribes
    /// data loss.
    #[test]
    fn a_stale_file_warns_before_a_blind_restore_destroys_mid_edit_work() {
        let dirty = s(&["stale.rs"]);
        let fresh = Freshness { stale: s(&["stale.rs"]), ..Default::default() };
        let msg = build("/repo", &dirty, &Ownership::default(), &fresh, "test-provenance")
            .expect("a stale file is worth warning about");
        assert!(msg.contains("do NOT blind-restore"), "must not prescribe an unconditional restore: {msg}");
        assert!(msg.contains("mid-edit"), "must name the mid-edit case: {msg}");
        // The prove-it-first guard must be the reachable-from-a-commit test, and
        // the restore must be explicitly conditional on it printing a commit.
        assert!(
            msg.contains("--find-object=$(git hash-object <path>)"),
            "must give the reachable-from-a-commit guard, not blob existence: {msg}"
        );
        assert!(
            msg.contains("ONLY if it prints a commit"),
            "the restore must be gated on the guard, never unconditional: {msg}"
        );
        // The unsound recipe (AMUX-3264) must never be the prescribed check.
        assert!(
            !msg.contains("object EXISTS, this exact content was committed before"),
            "must not tie blob existence to a safe restore: {msg}"
        );
    }

    /// A STALE file among genuine work: the stale warning comes FIRST, and the
    /// commit count covers only the commit-worthy paths, never the stale one.
    #[test]
    fn stale_is_rendered_first_and_excluded_from_the_commit_count() {
        let dirty = s(&["stale.rs", "new.rs"]);
        let fresh = Freshness { stale: s(&["stale.rs"]), new: s(&["new.rs"]), ..Default::default() };
        let msg = build("/repo", &dirty, &Ownership::default(), &fresh, "test-provenance").unwrap();
        let stale_at = msg.find("are OLDER than origin/main").expect("stale block present");
        let commit_at = msg.find("uncommitted change(s)").expect("commit block present");
        assert!(stale_at < commit_at, "STALE must be rendered FIRST: {msg}");
        assert!(msg.contains("1 uncommitted change(s)"), "count MUST exclude the stale path: {msg}");
        assert!(msg.contains("new.rs"), "the commit-worthy path must be listed: {msg}");
    }

    /// SAME files are byte-identical to origin, dirty only because local HEAD is
    /// behind. They are pure noise and must be suppressed entirely, not counted.
    #[test]
    fn a_same_as_origin_file_is_suppressed_and_not_counted() {
        let dirty = s(&["real.rs", "same.rs"]);
        let fresh = Freshness { same: s(&["same.rs"]), ..Default::default() };
        let msg = build("/repo", &dirty, &Ownership::default(), &fresh, "test-provenance").unwrap();
        assert!(msg.contains("1 uncommitted change(s)"), "SAME must not be counted: {msg}");
        assert!(msg.contains("real.rs"), "the real change must remain: {msg}");
        assert!(!msg.contains("same.rs"), "a file identical to origin must be absent: {msg}");
    }

    /// If EVERY dirty path is identical to origin there is nothing to say, the
    /// same silence as a clean tree. 48 of 321 paths were this on the checkout
    /// that motivated MG-1467, and they are why the raw count read alarming.
    #[test]
    fn an_all_same_tree_says_nothing() {
        let dirty = s(&["a.rs", "b.rs"]);
        let fresh = Freshness { same: s(&["a.rs", "b.rs"]), ..Default::default() };
        assert!(
            build("/repo", &dirty, &Ownership::default(), &fresh, "test-provenance").is_none(),
            "an all-SAME tree is noise and must produce no nudge"
        );
    }

    /// NEW and EDITED are ordinary commit-worthy work: they keep today's
    /// messaging and are both counted. This is the "local HEAD not behind
    /// origin" world where nothing is STALE or SAME and the nudge reads exactly
    /// as it always did.
    #[test]
    fn new_and_edited_render_as_ordinary_commit_worthy_work() {
        let dirty = s(&["added.rs", "changed.rs"]);
        let fresh =
            Freshness { new: s(&["added.rs"]), edited: s(&["changed.rs"]), ..Default::default() };
        let msg = build("/repo", &dirty, &Ownership::default(), &fresh, "test-provenance").unwrap();
        assert!(msg.contains("2 uncommitted change(s)"), "both NEW and EDITED are commit-worthy: {msg}");
        assert!(msg.contains("added.rs") && msg.contains("changed.rs"), "{msg}");
    }

    /// AMUX-3367: an append-only multi-writer file (frustrations.md) can diverge
    /// BOTH ways at once, so the nudge's "commit the path" / "restore" advice both
    /// lose data on it. A dirty frustrations.md must therefore get an explicit
    /// UNION-MERGE directive, and a dirty set without one must not.
    #[test]
    fn an_append_only_shared_file_gets_a_union_merge_directive() {
        // Case-insensitive basename, any directory, ONLY frustrations.md.
        assert!(is_append_only_shared("frustrations.md"));
        assert!(is_append_only_shared("FRUSTRATIONS.md")); // macOS resolves to the same file
        assert!(is_append_only_shared("deep/path/frustrations.md"));
        assert!(!is_append_only_shared("frustrations.md.bak"));
        assert!(!is_append_only_shared("src/app.js"));

        let dirty = s(&["frustrations.md", "app.js"]);
        let msg = commit_worthy_body("/repo", &dirty, &Ownership::default()).unwrap();
        assert!(msg.contains("APPEND-ONLY SHARED FILE"), "{msg}");
        assert!(msg.contains("UNION-MERGE"), "{msg}");
        assert!(msg.contains("frustrations.md"), "{msg}");

        // No append-only file in the set -> no such block.
        let plain = s(&["app.js", "main.rs"]);
        let msg2 = commit_worthy_body("/repo", &plain, &Ownership::default()).unwrap();
        assert!(!msg2.contains("APPEND-ONLY SHARED FILE"), "{msg2}");
    }

    /// THE EXIT-CODE DISCRIMINATOR, against a real repo (MG-1467). The classifier
    /// shells git, so the one thing worth pinning end to end is that it tests the
    /// PROCESS EXIT CODE for absence, not stdout emptiness: plain `git rev-parse`
    /// echoes an unresolvable revspec to stdout and still exits nonzero, and
    /// three sessions shipped the is-empty bug on 2026-08-10. A checkout that is
    /// behind origin exercises all four classes at once.
    #[tokio::test]
    async fn freshness_from_repo_classifies_new_same_stale_and_edited() {
        let tmp = std::env::temp_dir().join(format!("amux-fresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (bare, work) = (tmp.join("origin.git"), tmp.join("work"));
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
        // Base commit (this stays HEAD): stale.txt=v0, same.txt=X, edited.txt=orig.
        std::fs::write(work.join("stale.txt"), "v0\n").unwrap();
        std::fs::write(work.join("same.txt"), "X\n").unwrap();
        std::fs::write(work.join("edited.txt"), "orig\n").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-m", "base"]);
        git(&work, &["push", "-q", "origin", "main"]);

        // A peer advances origin ONLY on stale.txt (v0 -> v2), pushes. Local
        // HEAD does not contain this commit.
        let peer = tmp.join("peer");
        std::process::Command::new("git")
            .args(["clone", "-q", bare.to_str().unwrap()])
            .arg(&peer)
            .output()
            .unwrap();
        git(&peer, &["config", "user.email", "t@t"]);
        git(&peer, &["config", "user.name", "t"]);
        std::fs::write(peer.join("stale.txt"), "v2\n").unwrap();
        git(&peer, &["add", "-A"]);
        git(&peer, &["commit", "-m", "peer moves stale.txt"]);
        git(&peer, &["push", "-q", "origin", "main"]);

        // Fetch the moved ref but do NOT merge: origin/main is now ahead of HEAD.
        git(&work, &["fetch", "-q", "origin"]);

        // Worktree edits that produce each class:
        //   stale.txt = v1  (differs from HEAD v0 AND from origin v2, origin moved) -> STALE
        //   edited.txt = changed (origin unmoved on this path)                      -> EDITED
        //   same.txt   = X  (byte-identical to origin)                              -> SAME
        //   new.txt    = fresh (absent from origin)                                 -> NEW
        std::fs::write(work.join("stale.txt"), "v1\n").unwrap();
        std::fs::write(work.join("edited.txt"), "changed\n").unwrap();
        std::fs::write(work.join("new.txt"), "fresh\n").unwrap();

        let dir = work.to_str().unwrap();
        let fresh = freshness_from_repo(
            dir,
            &s(&["new.txt", "same.txt", "stale.txt", "edited.txt"]),
        )
        .await;

        assert_eq!(fresh.new, s(&["new.txt"]), "absent-from-origin must be NEW (exit-code test)");
        assert_eq!(fresh.same, s(&["same.txt"]), "identical-to-origin must be SAME");
        assert_eq!(fresh.stale, s(&["stale.txt"]), "origin-ahead-on-path must be STALE");
        assert_eq!(fresh.edited, s(&["edited.txt"]), "differs, origin unmoved, must be EDITED");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
