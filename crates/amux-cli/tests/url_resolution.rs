//! AMUX-2672: a bare `amux-rs <verb>` must reach the server that is running.
//!
//! The default was `https://localhost:8823`, the port the Rust server used while
//! Python owned 8822. Python retired, the Rust server took over 8822 AND 8824,
//! and nothing has listened on 8823 since — so every bare invocation died on
//! connect. The failure mode is what made it expensive: a connection error looks
//! exactly like the server being down, so the CLI's own misconfiguration reads as
//! a server fault (it cost a wrong diagnosis on AMUX-2653).
//!
//! End-to-end rather than unit, because the bug was in the wiring — `resolve_url`
//! could be perfect and the arg still bound to the wrong default.

use std::process::Command;

/// Runs `amux-rs health` with a controlled env and returns stderr.
/// `--help`-style verbs would not exercise the URL at all.
fn health_stderr(env: &[(&str, Option<&str>)]) -> String {
    let mut c = Command::new(env!("CARGO_BIN_EXE_amux-rs"));
    c.arg("health");
    // Start from a clean slate: the test process inherits a real AMUX_URL when
    // run inside amux, which would silently decide the result.
    c.env_remove("AMUX_URL").env_remove("AMUX_RS_URL");
    for (k, v) in env {
        match v {
            Some(val) => { c.env(k, val); }
            None => { c.env_remove(k); }
        }
    }
    let out = c.output().expect("run amux-rs health");
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn bare_invocation_no_longer_targets_the_dead_8823() {
    let err = health_stderr(&[]);
    assert!(
        !err.contains(":8823"),
        "bare invocation must not target the retired port; stderr: {err}"
    );
    // It should be TRYING 8822. Asserting on the attempted URL keeps this
    // meaningful whether or not a server happens to be up on this machine —
    // otherwise the test would pass for the wrong reason in CI.
    assert!(
        err.is_empty() || err.contains(":8822"),
        "expected the default to be :8822; stderr: {err}"
    );
}

#[test]
fn amux_url_is_honoured_so_a_session_reaches_its_own_server() {
    // The whole point of the fallback: every running amux session has AMUX_URL
    // in its env, including when the server is not on localhost.
    let err = health_stderr(&[("AMUX_URL", Some("https://127.0.0.1:9999"))]);
    assert!(
        err.contains("9999"),
        "AMUX_URL must be used when set; stderr: {err}"
    );
}

#[test]
fn amux_rs_url_outranks_amux_url() {
    let err = health_stderr(&[
        ("AMUX_URL", Some("https://127.0.0.1:9998")),
        ("AMUX_RS_URL", Some("https://127.0.0.1:9997")),
    ]);
    assert!(
        err.contains("9997") && !err.contains("9998"),
        "the explicit override must win; stderr: {err}"
    );
}

#[test]
fn an_empty_env_value_falls_through_instead_of_targeting_nothing() {
    // An exported-but-empty AMUX_URL is ordinary shell breakage; it must not
    // produce a request to "" (which fails with a useless parse error).
    let err = health_stderr(&[("AMUX_URL", Some(""))]);
    assert!(
        err.is_empty() || err.contains(":8822"),
        "empty AMUX_URL must fall through to the default; stderr: {err}"
    );
}
