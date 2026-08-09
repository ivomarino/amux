//! StructuredCliProtocol (RR-0030): [`AgentProtocol`] over headless
//! structured CLI runs — the orchestrator's ExecuteTask path.
//!
//! One prompt = one child process run in the worker's cwd, speaking the
//! provider's structured stdout format (RR-0028e coverage matrix,
//! docs/provider-coverage.csv):
//!
//! - Claude Code: `claude --print <prompt> --output-format stream-json --verbose`
//! - Gemini CLI:  `gemini -p <prompt> --output-format stream-json`
//! - Codex CLI:   `codex exec --json <prompt>`
//!
//! A reader task per child parses stdout lines via [`super::events`] and
//! broadcasts [`WorkerEvent`]s on the worker's channel. Claude and Gemini
//! streams have no explicit turn-start marker, so the reader inserts
//! `TurnStarted` before the first in-turn event; Codex's literal
//! `turn.started` is passed through and deduplicated.
//!
//! Honest limits of the headless shape, stated rather than faked:
//!
//! - `pause`/`resume` return [`ProtocolError::Rejected`]: a `--print`-style
//!   run has no suspend concept. Faking it (e.g. SIGSTOP silently) would be
//!   state the harness reports but the agent does not have.
//! - `deliver_message` mid-turn returns `Rejected`: there is no stdin
//!   session to inject into. The command queue's `AtTurnBoundary` timing is
//!   the sanctioned retry path (Invariant 34). When idle, a message delivery
//!   IS a new turn, keyed by the durable `MessageId` (Invariant 29), so a
//!   redelivered message never double-runs.
//! - `cancel` sends a real SIGINT via `/bin/kill` (graceful — the CLIs
//!   checkpoint on SIGINT per the spike), falling back to
//!   [`tokio::process::Child::start_kill`] (SIGKILL) only if that fails.
//!   nix/libc are not workspace deps and adding one for a single signal is
//!   not worth the surface; `kill(1)` is POSIX and already everywhere amux
//!   runs. `kill_on_drop(true)` reaps children if the server itself dies.
//!
//! Idempotency (Invariant 9): a `send_prompt` whose `idempotency_key` was
//! already seen for that worker returns `Ok` without re-spawning — restart
//! reconciliation replays enqueues, and this is what makes the replay safe.

use super::{events, AgentProtocol, AgentState, Prompt, ProtocolError, Result};
use amux_core::ids::{MessageId, TurnId, WorkerId};
use amux_core::protocol::{ExitStatus, Failure, WorkerEvent};
use async_trait::async_trait;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdout, Command};
use tokio::sync::broadcast;

/// Which structured CLI a worker speaks. Closed here (unlike the open
/// `ProviderId`) because each variant IS a concrete argv shape this module
/// owns; an unlisted provider simply is not spawnable by this protocol and
/// belongs to the terminal adapter until its structured shape is captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliProvider {
    ClaudeCode,
    GeminiCli,
    CodexCli,
}

impl CliProvider {
    /// Binary name resolved via PATH (overridable per worker for tests).
    pub fn binary(&self) -> &'static str {
        match self {
            CliProvider::ClaudeCode => "claude",
            CliProvider::GeminiCli => "gemini",
            CliProvider::CodexCli => "codex",
        }
    }

    /// Argv for one headless prompt run. Flags verified against the
    /// installed binaries on 2026-08-09 (claude 2.1.226, gemini 0.53.1,
    /// codex 0.141.0) — see events.rs for the captured output they produce.
    fn args(&self, prompt: &str) -> Vec<String> {
        match self {
            // --verbose is required for stream-json with --print.
            CliProvider::ClaudeCode => vec![
                "--print".into(),
                prompt.into(),
                "--output-format".into(),
                "stream-json".into(),
                "--verbose".into(),
            ],
            CliProvider::GeminiCli => vec![
                "-p".into(),
                prompt.into(),
                "--output-format".into(),
                "stream-json".into(),
            ],
            // --skip-git-repo-check: codex refuses to run outside a git repo
            // otherwise. The orchestrator chose the cwd deliberately, and a
            // headless run cannot answer the interactive refusal.
            CliProvider::CodexCli => vec![
                "exec".into(),
                "--json".into(),
                "--skip-git-repo-check".into(),
                prompt.into(),
            ],
        }
    }

    /// Extra env for headless runs. Gemini refuses untrusted directories
    /// with an interactive prompt a headless run cannot answer; the cwd is
    /// the worker's own workspace, chosen by the orchestrator, so trusting
    /// it is stating a fact, not bypassing a control.
    fn envs(&self) -> &'static [(&'static str, &'static str)] {
        match self {
            CliProvider::GeminiCli => &[("GEMINI_CLI_TRUST_WORKSPACE", "true")],
            _ => &[],
        }
    }

    fn translate(&self, line: &str, turn: &TurnId) -> Vec<WorkerEvent> {
        match self {
            CliProvider::ClaudeCode => events::translate_claude(line, turn),
            CliProvider::GeminiCli => events::translate_gemini(line, turn),
            CliProvider::CodexCli => events::translate_codex(line, turn),
        }
    }
}

/// Per-worker spawn configuration, registered once per worker.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub provider: CliProvider,
    /// Working directory for every run — the worker's workspace.
    pub cwd: PathBuf,
    /// Override the binary path (tests point this at a fixture-replaying
    /// script; production leaves it `None` and resolves via PATH).
    pub binary: Option<PathBuf>,
}

struct WorkerShared {
    config: WorkerConfig,
    events: broadcast::Sender<WorkerEvent>,
    state: Mutex<AgentState>,
    /// Idempotency keys already accepted (Invariant 9). Inserted only after
    /// a successful spawn, so a failed spawn does not burn the key.
    seen_keys: Mutex<HashSet<String>>,
    /// The live child, present while a turn is running. Held for `cancel`;
    /// the reader task takes it back at EOF to reap the exit status.
    child: Mutex<Option<Child>>,
}

impl WorkerShared {
    fn emit(&self, ev: WorkerEvent) {
        // No receivers is fine — broadcast keeps working when they arrive.
        let _ = self.events.send(ev);
    }

    fn set_state(&self, s: AgentState) {
        *self.state.lock().unwrap() = s;
    }
}

/// The structured-CLI implementation of [`AgentProtocol`].
#[derive(Default)]
pub struct StructuredCliProtocol {
    workers: Mutex<BTreeMap<WorkerId, Arc<WorkerShared>>>,
}

impl StructuredCliProtocol {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a worker. Without registration every call fails with
    /// `NoSession`, matching [`super::mock::MockProtocol`]'s contract for a
    /// worker whose process is gone.
    pub fn register(&self, worker: WorkerId, config: WorkerConfig) {
        let (tx, _) = broadcast::channel(256);
        self.workers.lock().unwrap().insert(
            worker,
            Arc::new(WorkerShared {
                config,
                events: tx,
                state: Mutex::new(AgentState::Idle),
                seen_keys: Mutex::new(HashSet::new()),
                child: Mutex::new(None),
            }),
        );
    }

    fn shared(&self, worker: &WorkerId) -> Result<Arc<WorkerShared>> {
        self.workers
            .lock()
            .unwrap()
            .get(worker)
            .cloned()
            .ok_or_else(|| ProtocolError::NoSession(worker.to_string()))
    }

    /// The one spawn path for both prompts and idle-time message delivery.
    async fn run_prompt(&self, worker: &WorkerId, text: &str, key: String) -> Result<()> {
        let shared = self.shared(worker)?;

        // Dedupe + busy-check + spawn under the per-worker key lock so two
        // concurrent sends with the same key cannot both spawn. Nothing in
        // this block awaits.
        let (stdout, stderr, turn) = {
            let mut keys = shared.seen_keys.lock().unwrap();
            if keys.contains(&key) {
                return Ok(()); // Invariant 9: redelivery must not double-run.
            }
            if matches!(*shared.state.lock().unwrap(), AgentState::Working { .. }) {
                return Err(ProtocolError::Rejected(format!(
                    "worker {worker} has a headless turn in progress; \
                     redeliver at the turn boundary"
                )));
            }

            let cfg = &shared.config;
            let program: PathBuf = cfg
                .binary
                .clone()
                .unwrap_or_else(|| PathBuf::from(cfg.provider.binary()));
            let mut cmd = Command::new(&program);
            cmd.args(cfg.provider.args(text))
                .current_dir(&cfg.cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            for (k, v) in cfg.provider.envs() {
                cmd.env(k, v);
            }
            let mut child = cmd.spawn().map_err(|e| {
                ProtocolError::Transport(format!("spawn {}: {e}", program.display()))
            })?;
            let stdout = child.stdout.take().ok_or_else(|| {
                ProtocolError::Transport("child stdout not captured".to_string())
            })?;
            let stderr = child.stderr.take();
            *shared.child.lock().unwrap() = Some(child);

            let turn = TurnId::from_ulid(ulid::Ulid::new());
            shared.set_state(AgentState::Working {
                turn: Some(turn.clone()),
                progress: None,
            });
            keys.insert(key);
            (stdout, stderr, turn)
        };

        tokio::spawn(read_stream(shared, stdout, stderr, turn));
        Ok(())
    }
}

/// Keep the last few stderr lines so a nonzero exit is diagnosable from the
/// Failure it produces (ethos rule 4: never discard the only evidence).
async fn stderr_tail(stderr: Option<ChildStderr>) -> String {
    let Some(stderr) = stderr else {
        return String::new();
    };
    let mut lines = BufReader::new(stderr).lines();
    let mut tail: VecDeque<String> = VecDeque::with_capacity(5);
    while let Ok(Some(line)) = lines.next_line().await {
        if tail.len() == 5 {
            tail.pop_front();
        }
        tail.push_back(line);
    }
    tail.into_iter().collect::<Vec<_>>().join(" | ")
}

/// Reader task: translate stdout lines into events, then reap the child and
/// settle the worker's state.
async fn read_stream(
    shared: Arc<WorkerShared>,
    stdout: ChildStdout,
    stderr: Option<ChildStderr>,
    turn: TurnId,
) {
    let stderr_task = tokio::spawn(stderr_tail(stderr));

    let mut lines = BufReader::new(stdout).lines();
    let mut turn_started = false;
    let mut turn_completed = false;
    while let Ok(Some(line)) = lines.next_line().await {
        for ev in shared.config.provider.translate(&line, &turn) {
            match &ev {
                WorkerEvent::Started => {}
                WorkerEvent::TurnStarted { .. } => {
                    if turn_started {
                        continue; // provider marker after our insertion
                    }
                    turn_started = true;
                }
                _ => {
                    // Claude/Gemini have no explicit turn-start marker: the
                    // first in-turn event implies it (Invariant 6 — the turn
                    // is first-class, so its start must be an event).
                    if !turn_started {
                        turn_started = true;
                        shared.emit(WorkerEvent::TurnStarted {
                            turn_id: turn.clone(),
                        });
                    }
                }
            }
            match &ev {
                WorkerEvent::Progress(p) => shared.set_state(AgentState::Working {
                    turn: Some(turn.clone()),
                    progress: Some(p.clone()),
                }),
                WorkerEvent::TurnCompleted(_) => {
                    turn_completed = true;
                    shared.set_state(AgentState::Idle);
                }
                _ => {}
            }
            shared.emit(ev);
        }
    }

    // stdout closed: reap the child for the real exit status.
    let child = shared.child.lock().unwrap().take();
    let status = match child {
        Some(mut c) => c.wait().await.ok(),
        None => None, // cancel() raced us and the child is being torn down
    };
    let tail = stderr_task.await.unwrap_or_default();

    match status {
        Some(st) if st.success() => {
            if !turn_completed {
                // Exit 0 with no result event (e.g. a clean SIGINT). The
                // turn still ENDED — leaving it open would hang the harness
                // on a boundary that already happened.
                if !turn_started {
                    shared.emit(WorkerEvent::TurnStarted {
                        turn_id: turn.clone(),
                    });
                }
                shared.emit(WorkerEvent::TurnCompleted(amux_core::protocol::TurnResult {
                    turn_id: turn.clone(),
                    outcome: "stream ended without a result event".to_string(),
                }));
            }
            shared.set_state(AgentState::Idle);
        }
        Some(st) => {
            let code = st.code();
            #[cfg(unix)]
            let signal = std::os::unix::process::ExitStatusExt::signal(&st);
            #[cfg(not(unix))]
            let signal = None;
            let reason = if tail.is_empty() {
                format!("{} exited with {st}", shared.config.provider.binary())
            } else {
                format!(
                    "{} exited with {st}; stderr: {tail}",
                    shared.config.provider.binary()
                )
            };
            shared.emit(WorkerEvent::Failed(Failure {
                reason,
                retryable: true,
            }));
            shared.emit(WorkerEvent::Exited(ExitStatus { code, signal }));
            shared.set_state(AgentState::Exited { code });
        }
        None => {
            // We could not observe the exit (Invariant 20: never invent a
            // code that was not reported).
            shared.emit(WorkerEvent::Exited(ExitStatus {
                code: None,
                signal: None,
            }));
            shared.set_state(AgentState::Exited { code: None });
        }
    }
}

#[async_trait]
impl AgentProtocol for StructuredCliProtocol {
    async fn send_prompt(&self, worker: &WorkerId, prompt: Prompt) -> Result<()> {
        self.run_prompt(worker, &prompt.text, prompt.idempotency_key)
            .await
    }

    /// Deliver a durable message. Headless CLIs have no mid-turn stdin, so
    /// an idle worker gets the message as a new turn (keyed by the durable
    /// `MessageId`, so redelivery is idempotent) and a busy worker rejects —
    /// the queue's `AtTurnBoundary` timing retries at the boundary.
    async fn deliver_message(&self, worker: &WorkerId, msg: MessageId, body: String) -> Result<()> {
        self.run_prompt(worker, &body, msg.as_str().to_string())
            .await
    }

    /// Graceful SIGINT via `/bin/kill`; hard-kill fallback. A cancel with
    /// nothing running is Ok — the absence of a turn IS the cancelled state.
    async fn cancel(&self, worker: &WorkerId) -> Result<()> {
        let shared = self.shared(worker)?;
        let pid = shared
            .child
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|c| c.id());
        let Some(pid) = pid else {
            return Ok(());
        };
        let sigint_ok = matches!(
            Command::new("kill")
                .args(["-INT", &pid.to_string()])
                .status()
                .await,
            Ok(st) if st.success()
        );
        if !sigint_ok {
            if let Some(child) = shared.child.lock().unwrap().as_mut() {
                child
                    .start_kill()
                    .map_err(|e| ProtocolError::Transport(format!("kill: {e}")))?;
            }
        }
        Ok(())
    }

    /// Not supported, honestly (ethos rule 3): a headless one-shot run has
    /// no suspend state to enter. The caller gets a truthful refusal, not a
    /// fake acknowledgement.
    async fn pause(&self, worker: &WorkerId) -> Result<()> {
        self.shared(worker)?;
        Err(ProtocolError::Rejected(
            "pause is not supported by headless structured-CLI sessions".to_string(),
        ))
    }

    async fn resume(&self, worker: &WorkerId) -> Result<()> {
        self.shared(worker)?;
        Err(ProtocolError::Rejected(
            "resume is not supported by headless structured-CLI sessions".to_string(),
        ))
    }

    async fn state(&self, worker: &WorkerId) -> Result<AgentState> {
        Ok(self.shared(worker)?.state.lock().unwrap().clone())
    }

    fn events(&self, worker: &WorkerId) -> broadcast::Receiver<WorkerEvent> {
        match self.workers.lock().unwrap().get(worker) {
            Some(shared) => shared.events.subscribe(),
            None => {
                // Same contract as MockProtocol: a dead worker yields a
                // closed channel, so the subscriber sees Closed immediately
                // instead of hanging.
                let (tx, rx) = broadcast::channel(1);
                drop(tx);
                rx
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Conformance tests (RR-0030). The fake-binary tests replay the REAL
// captured provider lines from events.rs through the full spawn -> read ->
// translate -> broadcast path, so they exercise the shipped code path (ethos
// rule 7) without needing provider auth. The real-claude integration test is
// #[ignore]d for CI (no claude auth there); run it locally with:
//   cargo test -p amux-server opencode -- --ignored
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    fn worker_id(tag: &str) -> WorkerId {
        // Distinct fixed ids per test for determinism.
        let base = "01JGXV000000000000000";
        WorkerId::from_ulid(format!("{base}{tag}").parse().unwrap())
    }

    /// Write an executable script that logs each run (for idempotency
    /// assertions) and replays a fixture stream on stdout.
    fn fixture_script(dir: &std::path::Path, name: &str, lines: &[&str], exit_code: i32) -> PathBuf {
        let path = dir.join(name);
        let runs = dir.join(format!("{name}.runs"));
        let mut body = String::from("#!/bin/sh\n");
        body.push_str(&format!("printf x >> '{}'\n", runs.display()));
        body.push_str("cat <<'FIXTURE_EOF'\n");
        for line in lines {
            body.push_str(line);
            body.push('\n');
        }
        body.push_str("FIXTURE_EOF\n");
        body.push_str(&format!("exit {exit_code}\n"));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn run_count(dir: &std::path::Path, name: &str) -> usize {
        std::fs::read(dir.join(format!("{name}.runs")))
            .map(|b| b.len())
            .unwrap_or(0)
    }

    async fn collect_until_terminal(
        rx: &mut broadcast::Receiver<WorkerEvent>,
        deadline: Duration,
    ) -> Vec<WorkerEvent> {
        let mut out = Vec::new();
        let end = tokio::time::Instant::now() + deadline;
        loop {
            let now = tokio::time::Instant::now();
            if now >= end {
                break;
            }
            match tokio::time::timeout(end - now, rx.recv()).await {
                Ok(Ok(ev)) => {
                    let terminal = matches!(
                        ev,
                        WorkerEvent::TurnCompleted(_) | WorkerEvent::Exited(_)
                    );
                    out.push(ev);
                    if terminal {
                        break;
                    }
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => break,
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Err(_) => break,
            }
        }
        out
    }

    async fn wait_for_state(
        proto: &StructuredCliProtocol,
        worker: &WorkerId,
        pred: impl Fn(&AgentState) -> bool,
    ) -> AgentState {
        for _ in 0..100 {
            let s = proto.state(worker).await.unwrap();
            if pred(&s) {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        proto.state(worker).await.unwrap()
    }

    fn prompt(key: &str) -> Prompt {
        Prompt {
            text: "say hi".to_string(),
            idempotency_key: key.to_string(),
        }
    }

    fn kind(ev: &WorkerEvent) -> &'static str {
        match ev {
            WorkerEvent::Started => "started",
            WorkerEvent::TurnStarted { .. } => "turn_started",
            WorkerEvent::Progress(_) => "progress",
            WorkerEvent::Waiting(_) => "waiting",
            WorkerEvent::ToolUsed(_) => "tool_used",
            WorkerEvent::TaskUpdated(_) => "task_updated",
            WorkerEvent::TurnCompleted(_) => "turn_completed",
            WorkerEvent::RateLimited(_) => "rate_limited",
            WorkerEvent::ContextLow(_) => "context_low",
            WorkerEvent::Failed(_) => "failed",
            WorkerEvent::Exited(_) => "exited",
        }
    }

    #[tokio::test]
    async fn codex_shaped_stream_full_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let script = fixture_script(
            dir.path(),
            "fake-codex",
            &[
                events::tests::CODEX_THREAD_STARTED,
                events::tests::CODEX_TURN_STARTED,
                events::tests::CODEX_AGENT_MESSAGE,
                events::tests::CODEX_TURN_COMPLETED,
            ],
            0,
        );
        let proto = StructuredCliProtocol::new();
        let w = worker_id("00001");
        proto.register(
            w.clone(),
            WorkerConfig {
                provider: CliProvider::CodexCli,
                cwd: dir.path().to_path_buf(),
                binary: Some(script),
            },
        );
        let mut rx = proto.events(&w);
        proto.send_prompt(&w, prompt("k1")).await.unwrap();

        let evs = collect_until_terminal(&mut rx, Duration::from_secs(10)).await;
        let kinds: Vec<_> = evs.iter().map(kind).collect();
        assert_eq!(
            kinds,
            vec!["started", "turn_started", "progress", "turn_completed"],
            "{evs:?}"
        );
        let settled = wait_for_state(&proto, &w, |s| *s == AgentState::Idle).await;
        assert_eq!(settled, AgentState::Idle);
    }

    #[tokio::test]
    async fn turn_started_inserted_for_streams_without_marker() {
        // Gemini's stream has no turn-start line; the reader must insert it
        // before the first in-turn event (Invariant 6).
        let dir = tempfile::tempdir().unwrap();
        let script = fixture_script(
            dir.path(),
            "fake-gemini",
            &[
                events::tests::GEMINI_INIT,
                events::tests::GEMINI_ASSISTANT_DELTA,
                events::tests::GEMINI_RESULT,
            ],
            0,
        );
        let proto = StructuredCliProtocol::new();
        let w = worker_id("00002");
        proto.register(
            w.clone(),
            WorkerConfig {
                provider: CliProvider::GeminiCli,
                cwd: dir.path().to_path_buf(),
                binary: Some(script),
            },
        );
        let mut rx = proto.events(&w);
        proto.send_prompt(&w, prompt("k1")).await.unwrap();

        let evs = collect_until_terminal(&mut rx, Duration::from_secs(10)).await;
        let kinds: Vec<_> = evs.iter().map(kind).collect();
        assert_eq!(
            kinds,
            vec![
                "started",
                "turn_started",
                "progress", // assistant delta
                "progress", // final token accounting
                "turn_completed"
            ],
            "{evs:?}"
        );
    }

    #[tokio::test]
    async fn claude_shaped_stream_translates_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let script = fixture_script(
            dir.path(),
            "fake-claude",
            &[
                events::tests::CLAUDE_INIT,
                events::tests::CLAUDE_TEXT,
                events::tests::CLAUDE_RESULT,
            ],
            0,
        );
        let proto = StructuredCliProtocol::new();
        let w = worker_id("00003");
        proto.register(
            w.clone(),
            WorkerConfig {
                provider: CliProvider::ClaudeCode,
                cwd: dir.path().to_path_buf(),
                binary: Some(script),
            },
        );
        let mut rx = proto.events(&w);
        proto.send_prompt(&w, prompt("k1")).await.unwrap();

        let evs = collect_until_terminal(&mut rx, Duration::from_secs(10)).await;
        let kinds: Vec<_> = evs.iter().map(kind).collect();
        assert_eq!(
            kinds,
            vec!["started", "turn_started", "progress", "turn_completed"],
            "{evs:?}"
        );
        match evs.last().unwrap() {
            WorkerEvent::TurnCompleted(r) => assert_eq!(r.outcome, "success: done"),
            other => panic!("expected TurnCompleted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn idempotent_send_prompt_does_not_respawn() {
        let dir = tempfile::tempdir().unwrap();
        let script = fixture_script(
            dir.path(),
            "fake-codex",
            &[
                events::tests::CODEX_THREAD_STARTED,
                events::tests::CODEX_TURN_STARTED,
                events::tests::CODEX_TURN_COMPLETED,
            ],
            0,
        );
        let proto = StructuredCliProtocol::new();
        let w = worker_id("00004");
        proto.register(
            w.clone(),
            WorkerConfig {
                provider: CliProvider::CodexCli,
                cwd: dir.path().to_path_buf(),
                binary: Some(script),
            },
        );
        let mut rx = proto.events(&w);
        proto.send_prompt(&w, prompt("dup")).await.unwrap();
        collect_until_terminal(&mut rx, Duration::from_secs(10)).await;
        wait_for_state(&proto, &w, |s| *s == AgentState::Idle).await;
        assert_eq!(run_count(dir.path(), "fake-codex"), 1);

        // Same key again: Ok, no second run (Invariant 9).
        proto.send_prompt(&w, prompt("dup")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(run_count(dir.path(), "fake-codex"), 1);

        // A fresh key runs again.
        let mut rx2 = proto.events(&w);
        proto.send_prompt(&w, prompt("fresh")).await.unwrap();
        collect_until_terminal(&mut rx2, Duration::from_secs(10)).await;
        wait_for_state(&proto, &w, |s| *s == AgentState::Idle).await;
        assert_eq!(run_count(dir.path(), "fake-codex"), 2);
    }

    #[tokio::test]
    async fn nonzero_exit_emits_failed_then_exited() {
        let dir = tempfile::tempdir().unwrap();
        let script = fixture_script(
            dir.path(),
            "fake-broken",
            &[events::tests::CODEX_THREAD_STARTED],
            3,
        );
        let proto = StructuredCliProtocol::new();
        let w = worker_id("00005");
        proto.register(
            w.clone(),
            WorkerConfig {
                provider: CliProvider::CodexCli,
                cwd: dir.path().to_path_buf(),
                binary: Some(script),
            },
        );
        let mut rx = proto.events(&w);
        proto.send_prompt(&w, prompt("k1")).await.unwrap();

        let evs = collect_until_terminal(&mut rx, Duration::from_secs(10)).await;
        let kinds: Vec<_> = evs.iter().map(kind).collect();
        assert_eq!(kinds, vec!["started", "failed", "exited"], "{evs:?}");
        match &evs[2] {
            WorkerEvent::Exited(st) => assert_eq!(st.code, Some(3)),
            other => panic!("expected Exited, got {other:?}"),
        }
        let settled =
            wait_for_state(&proto, &w, |s| matches!(s, AgentState::Exited { .. })).await;
        assert_eq!(settled, AgentState::Exited { code: Some(3) });
    }

    #[tokio::test]
    async fn pause_and_resume_are_honestly_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let proto = StructuredCliProtocol::new();
        let w = worker_id("00006");
        proto.register(
            w.clone(),
            WorkerConfig {
                provider: CliProvider::ClaudeCode,
                cwd: dir.path().to_path_buf(),
                binary: None,
            },
        );
        assert!(matches!(
            proto.pause(&w).await,
            Err(ProtocolError::Rejected(_))
        ));
        assert!(matches!(
            proto.resume(&w).await,
            Err(ProtocolError::Rejected(_))
        ));
        // Unregistered workers still get NoSession, not Rejected.
        let ghost = worker_id("00007");
        assert!(matches!(
            proto.pause(&ghost).await,
            Err(ProtocolError::NoSession(_))
        ));
    }

    #[tokio::test]
    async fn unregistered_worker_is_no_session_and_closed_channel() {
        let proto = StructuredCliProtocol::new();
        let ghost = worker_id("00008");
        assert!(matches!(
            proto.send_prompt(&ghost, prompt("k")).await,
            Err(ProtocolError::NoSession(_))
        ));
        assert!(matches!(
            proto.state(&ghost).await,
            Err(ProtocolError::NoSession(_))
        ));
        assert!(matches!(proto.cancel(&ghost).await, Err(ProtocolError::NoSession(_))));
        let mut rx = proto.events(&ghost);
        assert!(matches!(
            rx.recv().await,
            Err(broadcast::error::RecvError::Closed)
        ));
    }

    #[tokio::test]
    async fn cancel_with_nothing_running_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let proto = StructuredCliProtocol::new();
        let w = worker_id("00009");
        proto.register(
            w.clone(),
            WorkerConfig {
                provider: CliProvider::CodexCli,
                cwd: dir.path().to_path_buf(),
                binary: None,
            },
        );
        proto.cancel(&w).await.unwrap();
    }

    /// Real end-to-end run against the installed claude CLI. #[ignore]d
    /// because CI has no claude binary or auth; locally:
    ///   cargo test -p amux-server opencode -- --ignored
    #[tokio::test]
    #[ignore = "requires installed+authenticated claude CLI; run with -- --ignored"]
    async fn real_claude_turn_end_to_end() {
        // Gate on `which claude` so an --ignored sweep on a claude-less
        // machine skips instead of failing on spawn.
        let have_claude = std::process::Command::new("which")
            .arg("claude")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !have_claude {
            eprintln!("skipping: claude not on PATH");
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let proto = StructuredCliProtocol::new();
        let w = worker_id("0000A");
        proto.register(
            w.clone(),
            WorkerConfig {
                provider: CliProvider::ClaudeCode,
                cwd: dir.path().to_path_buf(),
                binary: None,
            },
        );
        let mut rx = proto.events(&w);
        proto
            .send_prompt(
                &w,
                Prompt {
                    text: "Reply with exactly: ok".to_string(),
                    idempotency_key: "real-claude-1".to_string(),
                },
            )
            .await
            .unwrap();

        let evs = collect_until_terminal(&mut rx, Duration::from_secs(120)).await;
        let kinds: Vec<_> = evs.iter().map(kind).collect();
        let pos = |k: &str| kinds.iter().position(|x| *x == k);
        let (started, turn_started, turn_completed) = (
            pos("started"),
            pos("turn_started"),
            pos("turn_completed"),
        );
        assert!(started.is_some(), "no Started in {kinds:?}");
        assert!(turn_started.is_some(), "no TurnStarted in {kinds:?}");
        assert!(
            turn_completed.is_some(),
            "no TurnCompleted within 120s: {kinds:?}"
        );
        assert!(started < turn_started && turn_started < turn_completed, "{kinds:?}");
        let settled = wait_for_state(&proto, &w, |s| *s == AgentState::Idle).await;
        assert_eq!(settled, AgentState::Idle);
    }
}
