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
    /// Nobody's edit record claims it.
    pub unclaimed: Vec<String>,
}

/// The nudge, or None when there is nothing honest to say.
///
/// `dirty` is the list of paths `git status` reported under the session's
/// working directory — a LIST, carrying no ownership claim.
pub fn build(dir: &str, dirty: &[String], own: &Ownership) -> Option<String> {
    if dirty.is_empty() {
        return None;
    }
    let foreign_paths: BTreeSet<&str> = own.foreign.iter().map(|(p, _)| p.as_str()).collect();

    // MINE = dirty MINUS foreign. Not "dirty", which is the bug this port
    // exists to not reintroduce.
    let mine: Vec<&String> = dirty.iter().filter(|p| !foreign_paths.contains(p.as_str())).collect();
    if mine.is_empty() {
        // Every dirty file belongs to someone else. Telling this session to
        // "commit completed work" here is precisely the instruction that swept
        // three peers' work in one day.
        return None;
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
    Some(msg)
}

#[cfg(test)]
mod tests {
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
            build("/repo", &dirty, &own).is_none(),
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
        let msg = build("/repo", &dirty, &own).expect("should nudge");
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
        let msg = build("/repo", &dirty, &own).unwrap();
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
        let msg = build("/repo", &dirty, &own).expect("shared must not suppress");
        assert!(msg.contains("CONTESTED") && msg.contains("peer"), "{msg}");
        assert!(msg.contains("git add -p"), "per-hunk is the actionable advice: {msg}");
    }

    /// Unclaimed files are still MINE to consider: nobody's edit record claims
    /// them, which is not the same as someone else owning them. Treating
    /// unclaimed as foreign would silence a session that genuinely has work.
    #[test]
    fn unclaimed_files_still_count_as_work_to_commit() {
        let dirty = s(&["orphan.rs"]);
        let own = Ownership { unclaimed: s(&["orphan.rs"]), ..Default::default() };
        let msg = build("/repo", &dirty, &own).expect("unclaimed is not foreign");
        assert!(msg.contains("1 uncommitted change(s)"), "{msg}");
    }

    #[test]
    fn a_clean_tree_says_nothing() {
        assert!(build("/repo", &[], &Ownership::default()).is_none());
    }

    /// Ten shown, the rest elided — a nudge that pastes 82 paths is a nudge
    /// nobody reads.
    #[test]
    fn a_long_list_is_capped_but_the_count_is_honest() {
        let dirty: Vec<String> = (0..25).map(|i| format!("f{i}.rs")).collect();
        let msg = build("/repo", &dirty, &Ownership::default()).unwrap();
        assert!(msg.contains("25 uncommitted change(s)"), "count must be the TRUE total: {msg}");
        assert!(msg.contains('…'), "the list must show it was truncated: {msg}");
        assert!(!msg.contains("f24.rs"), "only the first ten are listed: {msg}");
    }
}
