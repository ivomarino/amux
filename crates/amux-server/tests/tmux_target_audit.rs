//! Every tmux `-t` target in the crate must come from the exact-match L2
//! helpers (`session_target` / `pane_target`, reached as `st`/`pt`/`stq`/`ptq`).
//!
//! This is a SOURCE audit rather than a behavioural test on purpose: the
//! failure it guards cannot be reproduced on demand. tmux resolves a bare `-t
//! foo` by PREFIX, and `amux-amux` is a prefix of `amux-amux-frustrations`,
//! `amux-amux-rust`, `amux-amux-cloud` and five more. A non-exact target is
//! therefore correct every single time the exact session exists, and silently
//! addresses a SIBLING's pane only in the window where it does not — which is
//! precisely a restart, a rename, or a start/stop race. The 2026-08-09
//! `amux-frustrations.log` carried another session's launch command and a
//! third session's nudge text from exactly such a window (AMUX-1888 is the
//! same hazard class in the CLI).
//!
//! So the check is: you cannot merge a hand-spelled target. If you need a new
//! target shape, add it to the helpers in `backend/tmux.rs` and it is covered
//! everywhere at once.

use std::path::{Path, PathBuf};

/// Identifiers that are, by construction, `session_target()`/`pane_target()`
/// output. Deliberately a SHORT closed list — the point is that the set of
/// ways to name a pane stays small enough to audit by eye.
const ALLOWED: &[&str] = &["st", "pt", "stq", "ptq"];

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            // tests/ may legitimately build literal targets for throwaway
            // sessions it created itself.
            if p.file_name().and_then(|s| s.to_str()) == Some("tests") {
                continue;
            }
            rust_sources(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(p);
        }
    }
}

/// The expression following a `"-t",` in an argv array, normalised: leading
/// `&`, and a trailing `.as_str()` / `.clone()` accessor, are not part of the
/// identity of the value.
fn normalise(expr: &str) -> String {
    let e = expr.trim().trim_start_matches('&').trim();
    let e = e.split('.').next().unwrap_or(e);
    e.trim().to_string()
}

/// Positive evidence that this `-t` belongs to a program that is NOT tmux.
///
/// AEAB-20. The scanner searched every `.rs` under `src/` for the literal
/// `"-t"` and treated whatever followed as a tmux target, with no notion of
/// which program was being invoked. `integrations/email.rs` runs
/// `Command::new("touch").args(["-t", stamp, ...])` to backdate a fixture —
/// `touch -t` is a TIMESTAMP, not a pane — and the audit failed `main` over it,
/// which in turn made every open PR inherit a red `check` job and look broken.
///
/// Deliberately requires POSITIVE evidence and fails SAFE: only a literal
/// `Command::new("x")` with x != "tmux" exempts a site. Anything the scanner
/// cannot attribute stays audited, so this narrows the false positives without
/// opening a hole — `api/metrics.rs` reaches tmux through a
/// `cmd_output("tmux", ...)` helper with no `Command::new` at all, and must and
/// does remain covered.
///
/// The alternative shape — "skip anything whose statement does not mention
/// tmux" — was rejected for being the wrong polarity: it would exempt a real
/// `let args = ["-t", target]; tmux(&args)` split across two statements, which
/// is precisely the offender this file exists to catch.
fn non_tmux_program(src: &str, at: usize) -> Option<String> {
    // The enclosing statement, back to the nearest `;` / `{` / `}`.
    let start = src[..at].rfind([';', '{', '}']).map_or(0, |i| i + 1);
    let span = &src[start..at];
    const KEY: &str = "Command::new(\"";
    let i = span.rfind(KEY)?;
    let rest = &span[i + KEY.len()..];
    let end = rest.find('"')?;
    let prog = &rest[..end];
    (prog != "tmux").then(|| prog.to_string())
}

/// Walk one source text and return every non-exact `-t` target expression.
///
/// Extracted so `offenders()` and `the_audit_detects_a_planted_non_exact_target`
/// run the SAME code. They used to be two copies of this loop — the test
/// re-implemented ~30 lines of it inline — which meant the test could not
/// observe a change in the real scanner. Simulating what you believe a function
/// does cannot catch that function doing something else (ethos rule 7), and the
/// exemption added above would have been entirely untested under that shape.
fn scan(src: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = src[from..].find("\"-t\"") {
        let at = from + rel;
        from = at + 4;
        if non_tmux_program(src, at).is_some() {
            continue;
        }
        // Skip the separator after the literal, then take the argument up
        // to the next `,` / `]` / `)` at this nesting level.
        let rest = &src[from..];
        let Some(comma) = rest.find(',') else { continue };
        let tail = &rest[comma + 1..];
        let mut depth = 0i32;
        let mut end = tail.len();
        for (i, c) in tail.char_indices() {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => {
                    if depth == 0 {
                        end = i;
                        break;
                    }
                    depth -= 1;
                }
                ',' if depth == 0 => {
                    end = i;
                    break;
                }
                _ => {}
            }
        }
        let expr = normalise(&tail[..end]);
        if !ALLOWED.contains(&expr.as_str()) {
            found.push((at, expr));
        }
    }
    found
}

fn offenders() -> Vec<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_sources(&root, &mut files);
    assert!(!files.is_empty(), "found no sources to audit under {}", root.display());

    let mut bad = Vec::new();
    for f in files {
        let src = std::fs::read_to_string(&f).unwrap_or_default();
        for (at, expr) in scan(&src) {
            let line = src[..at].matches('\n').count() + 1;
            bad.push(format!(
                "{}:{line}: tmux -t target `{expr}` is not one of {ALLOWED:?} \
                 (build it with session_target()/pane_target())",
                f.display()
            ));
        }
    }
    bad
}

#[test]
fn every_tmux_target_uses_the_exact_match_helpers() {
    let bad = offenders();
    assert!(
        bad.is_empty(),
        "hand-spelled tmux target(s) found — a non-exact `-t` lands in a \
         SIBLING session's pane whenever the exact session is briefly absent:\n{}",
        bad.join("\n")
    );
}

/// The audit is only worth having if it can fail, and a source-scanning check
/// is exactly the kind that silently matches nothing after a refactor renames
/// something (ethos rule 7 — "can your check actually fail?"). So prove the
/// scanner FINDS a planted offender rather than trusting that it would.
#[test]
fn the_audit_detects_a_planted_non_exact_target() {
    // Same text shape the scanner walks in a real source file, including the
    // prefix-matching target that motivated the rule. Runs the SHIPPED `scan`
    // rather than a copy of it — this test used to re-implement the loop inline,
    // so it could not have observed the AEAB-20 exemption at all.
    let planted = r#"
        let _ = tmux(&["pipe-pane", "-t", &format!("amux-{name}"), &cmd]).await;
        let _ = tmux(&["send-keys", "-t", "amux-amux", "Enter"]).await;
        let _ = tmux(&["kill-session", "-t", &stq]).await;
    "#;
    let found: Vec<String> = scan(planted).into_iter().map(|(_, e)| e).collect();
    assert_eq!(
        found.len(),
        2,
        "the scanner must flag BOTH planted offenders and leave `stq` alone; got {found:?}"
    );
    assert!(found.iter().any(|f| f.contains("format!")), "missed the format! target: {found:?}");
    assert!(found.iter().any(|f| f.contains("\"amux-amux\"")), "missed the literal prefix target: {found:?}");
}

/// AEAB-20, both directions. The exemption must silence `touch -t` and MUST NOT
/// silence a tmux target — a fix that just stopped flagging things would satisfy
/// "main is green" while deleting the guard, which is the whole failure mode this
/// file was written to avoid.
#[test]
fn a_non_tmux_program_is_exempt_but_tmux_is_never_exempt() {
    // The real specimen, reduced from integrations/email.rs:1330. `touch -t` is a
    // timestamp; flagging it failed main and made every open PR look broken.
    let touch = r#"
        let set = |name: &str, stamp: &str| {
            std::process::Command::new("touch")
                .args(["-t", stamp, tok.join(format!("{name}.json")).to_str().unwrap()])
                .status()
                .unwrap();
        };
    "#;
    assert!(scan(touch).is_empty(), "touch -t is a timestamp, not a pane: {:?}", scan(touch));

    // THE CONTROL. Identical shape, program `tmux`: must still be caught.
    let tmux_cmd = r#"
        std::process::Command::new("tmux")
            .args(["send-keys", "-t", "amux-amux", "Enter"])
            .status()
            .unwrap();
    "#;
    let f: Vec<String> = scan(tmux_cmd).into_iter().map(|(_, e)| e).collect();
    assert_eq!(f.len(), 1, "a hand-spelled tmux target must still be flagged: {f:?}");
    assert!(f[0].contains("amux-amux"), "wrong expression captured: {f:?}");

    // And a helper that names tmux WITHOUT Command::new stays covered — this is
    // api/metrics.rs's shape (`cmd_output("tmux", &[...])`), where there is no
    // literal program to attribute, so the fail-safe keeps it audited.
    let helper = r#"
        let _ = cmd_output("tmux", &["list-panes", "-t", "amux-amux", "-F", "x"]);
    "#;
    assert_eq!(scan(helper).len(), 1, "a helper-invoked tmux target must stay audited");

    // A non-tmux program reached the same way is NOT exempt either, for the same
    // reason: no literal program means no positive evidence, so it stays flagged.
    // Recorded rather than "fixed" — erring toward a reviewable false positive is
    // the correct direction for this guard, and pretending otherwise would need a
    // real parser.
    let other_helper = r#"
        let _ = cmd_output("touch", &["-t", "202608161200", "f.json"]);
    "#;
    assert_eq!(
        scan(other_helper).len(),
        1,
        "documented limitation: without Command::new there is nothing to attribute"
    );
}
