//! The bash CLI must PARSE. It ships on save, so a parse error is an outage.
//!
//! AMUX-3464, 2026-08-26. `~/.local/bin/amux` is a 26-byte symlink to
//! `<repo>/amux`, which means an uncommitted edit to that file is live to
//! every session in the fleet the moment it is written — no install step, no
//! builder cycle, no CI in between. A comment containing an apostrophe was
//! added inside a `python3 -c '...'` block; the apostrophe closed the quote
//! early, bash reinterpreted the remainder, and the script died at load with
//! `syntax error near unexpected token ';;'`. Every subcommand, every
//! session, until a peer reported it over the HTTP API.
//!
//! What makes this worth its own guard rather than a note: the failure is at
//! LOAD, so the CLI cannot print its own help. It is not degraded, it is
//! mute — and a session that has only ever reached other lanes through
//! `amux send` has no way left to discover that POST /api/sessions/<n>/send
//! exists. The blast radius is every lane at once and the recovery path is
//! invisible from inside it.
//!
//! `.claude/check-and-commit.sh` is the gate that fires in TIME (at edit,
//! before the fleet sees it). This one is the backstop for an edit made
//! without that hook: another machine, a rebase, a merge resolution, CI.
//! Both are cheap; neither existed while the repo gated dashboard JS with
//! `node --check` on every save.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root above crates/amux-server")
        .to_path_buf()
}

fn parse_check(path: &std::path::Path) -> Result<(), String> {
    let out = Command::new("bash")
        .arg("-n")
        .arg(path)
        .output()
        .expect("run bash -n");
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

#[test]
fn the_bash_cli_parses() {
    let cli = repo_root().join("amux");
    if let Err(e) = parse_check(&cli) {
        panic!(
            "`bash -n amux` FAILED — this file is the symlink target of \
             ~/.local/bin/amux, so it is already live to the whole fleet and \
             every subcommand is dying at load:\n{e}\n\
             Common cause: an apostrophe or single quote inside a python3 -c '...' block."
        );
    }
}

#[test]
fn the_parse_check_can_actually_fail() {
    // The test above is a `bash -n` that passes, and a `bash -n` that passes
    // looks identical to a `bash -n` that was never reached — a wrong path, a
    // missing binary, a helper that swallows its own error. Ethos rule 7: a
    // green check that cannot detect the bug is theatre. So construct the
    // real specimen and confirm the checker rejects it.
    let cli = repo_root().join("amux");
    let src = std::fs::read_to_string(&cli).expect("read amux");

    // The actual break, not a paraphrase of one: an apostrophe inside the
    // single-quoted python block that builds the board transition body.
    let anchor = r#"if os.environ.get("F"):"#;
    assert!(
        src.contains(anchor),
        "mutation anchor missing — the specimen was never constructed, so this \
         test would pass without proving anything (an unapplied mutation and a \
         working check produce the same green)"
    );
    let broken = src.replacen(anchor, &format!("# the card's ledger\n{anchor}"), 1);
    assert_ne!(broken, src, "mutation did not change the source");

    let tmp = std::env::temp_dir().join(format!("amux-syntax-guard-{}.sh", std::process::id()));
    std::fs::write(&tmp, &broken).expect("write specimen");
    let verdict = parse_check(&tmp);
    let _ = std::fs::remove_file(&tmp);

    let err = verdict.expect_err("bash -n must REJECT an apostrophe inside python3 -c '...'");
    assert!(
        err.contains("syntax error"),
        "expected a parse error naming the syntax fault, got: {err}"
    );
}


// ---------------------------------------------------------------------------
// AMUX-3892 — a help text is DATA, and bash must not re-parse it.
// ---------------------------------------------------------------------------
//
// `bash -n` above cannot see this class and passed the whole time. The board
// help block is built with an UNQUOTED heredoc, because it interpolates
// ${BOLD}/${RESET}, which means bash also expands backticks and $(...) inside
// it. Every backtick in that block is hand-escaped, so the convention is
// "remember to escape" — and somebody forgot within a day of the surrounding
// text being written. Two lines documenting the new evidence flag,
// `done` and `none: <reason>`, shipped unescaped:
//
//   amux: command substitution: line 2938: syntax error near unexpected token `done'
//   amux: command substitution: line 2938: `none: <reason>'
//
// stdout was correct and rc was 0, which is why it survived: only a caller that
// merges stderr sees it, and then it reads as a failed command. The CLI ships on
// SAVE through a symlink, so this was live to all ~57 lanes the moment the line
// was typed.
//
// The guard is stderr, not the text. Asserting "these two lines are escaped"
// would pin the instance; asserting the help paths are silent pins the class,
// including the next unescaped `$(...)` nobody has written yet.

/// Run a CLI verb in a SANITIZED env and hand back (rc, stderr).
///
/// `env -i`-equivalent on purpose: the help paths must not need a server, a
/// session, or an AMUX_URL, and a test that inherited this lane's environment
/// would pass for reasons that have nothing to do with the CLI.
fn run_quiet(args: &[&str]) -> (i32, String) {
    let cli = repo_root().join("amux");
    let out = Command::new("bash")
        .arg(&cli)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/local/bin")
        .env("HOME", std::env::var("HOME").unwrap_or_default())
        .output()
        .unwrap_or_else(|e| panic!("run amux {args:?}: {e}"));
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn help_prints_nothing_to_stderr() {
    for args in [vec!["help"], vec!["board", "help"]] {
        let (rc, err) = run_quiet(&args);
        assert_eq!(rc, 0, "`amux {}` must succeed; stderr: {err}", args.join(" "));
        assert!(
            err.is_empty(),
            "`amux {}` wrote to stderr. A help text is data and bash must not \
             re-parse it — an unescaped backtick or $(...) inside the UNQUOTED \
             heredoc is the usual cause (AMUX-3892). stdout and rc stay correct, \
             so nothing else catches this:\n{err}",
            args.join(" ")
        );
    }
}

#[test]
fn the_stderr_check_can_actually_fail() {
    // Same discipline as `the_parse_check_can_actually_fail`: a silent stderr
    // looks identical whether the guard works or the command never ran. So
    // reconstruct the real defect — an unescaped backtick pair inside the board
    // help heredoc — and confirm it is caught.
    let cli = repo_root().join("amux");
    let src = std::fs::read_to_string(&cli).expect("read amux");

    // The line the live incident landed on, now escaped. If this anchor ever
    // stops matching, the mutation was never applied and this test proves
    // nothing, so its absence is a failure rather than a skip.
    let anchor = r"\`none: <reason>\`";
    assert!(
        src.contains(anchor),
        "mutation anchor missing — an unapplied mutation and a working check are \
         both green, so this must fail loudly instead"
    );
    let broken = src.replacen(anchor, "`none: <reason>`", 1);
    assert_ne!(broken, src, "mutation did not change the source");

    let tmp = std::env::temp_dir().join(format!("amux-help-stderr-{}.sh", std::process::id()));
    std::fs::write(&tmp, &broken).expect("write specimen");
    let out = Command::new("bash")
        .arg(&tmp)
        .arg("board")
        .arg("help")
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/local/bin")
        .env("HOME", std::env::var("HOME").unwrap_or_default())
        .output()
        .expect("run specimen");
    let _ = std::fs::remove_file(&tmp);

    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("command substitution"),
        "an unescaped backtick in the help heredoc must reach stderr, or the \
         guard above is theatre. got: {err:?}"
    );
    // AND the thing that hid it for a day: the command still SUCCEEDS.
    assert_eq!(
        out.status.code(),
        Some(0),
        "the broken specimen still exits 0 — which is why rc is not a usable \
         signal here and stderr has to be"
    );
}
