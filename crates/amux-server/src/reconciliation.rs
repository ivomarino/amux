//! Isolated, local-only candidate reconciliation and last-known-green refs.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use wait_timeout::ChildExt;

const MAX_GATE_TIMEOUT_SECS: u64 = 3_600;
const GATE_CAPTURE_BYTES: u64 = 128 * 1024;

const fn default_gate_timeout_secs() -> u64 {
    900
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateCommand {
    pub label: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_gate_timeout_secs")]
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GateResult {
    pub label: String,
    pub program: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub timeout_secs: u64,
    pub duration_ms: u64,
    pub output: String,
    pub output_truncated: bool,
    pub output_redactions: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReconciliationRun {
    pub candidate_sha: String,
    pub status: String,
    pub gates: Vec<GateResult>,
    pub failure_reason: Option<String>,
}

fn run_git(repo: &Path, args: &[&str]) -> anyhow::Result<Output> {
    Ok(Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()?)
}

struct WorktreeGuard {
    repo: PathBuf,
    path: PathBuf,
}

impl Drop for WorktreeGuard {
    fn drop(&mut self) {
        match Command::new("git")
            .arg("-C")
            .arg(&self.repo)
            .args(["worktree", "remove", "--force"])
            .arg(&self.path)
            .status()
        {
            Ok(status) if status.success() => {}
            Ok(status) => tracing::warn!(
                repo = %self.repo.display(),
                worktree = %self.path.display(),
                exit_code = ?status.code(),
                "failed to remove reconciliation worktree"
            ),
            Err(error) => tracing::warn!(
                repo = %self.repo.display(),
                worktree = %self.path.display(),
                error = %error,
                "failed to run reconciliation worktree cleanup"
            ),
        }
    }
}

fn captured(path: &Path) -> anyhow::Result<(String, bool)> {
    let file = std::fs::File::open(path)?;
    let truncated = file.metadata()?.len() > GATE_CAPTURE_BYTES;
    let mut raw = Vec::with_capacity(GATE_CAPTURE_BYTES as usize);
    file.take(GATE_CAPTURE_BYTES).read_to_end(&mut raw)?;
    Ok((String::from_utf8_lossy(&raw).into_owned(), truncated))
}

pub fn run_candidate(
    repo: &Path,
    candidate: &str,
    gates: &[GateCommand],
) -> anyhow::Result<ReconciliationRun> {
    if gates.is_empty() {
        anyhow::bail!("at least one reconciliation gate is required");
    }
    if gates.iter().any(|gate| {
        gate.label.trim().is_empty()
            || gate.program.trim().is_empty()
            || gate.timeout_secs == 0
            || gate.timeout_secs > MAX_GATE_TIMEOUT_SECS
    }) {
        anyhow::bail!(
            "gate labels/programs are required and timeout_secs must be between 1 and {MAX_GATE_TIMEOUT_SECS}"
        );
    }
    let repo = repo.canonicalize()?;
    let rev = run_git(
        &repo,
        &["rev-parse", "--verify", &format!("{candidate}^{{commit}}")],
    )?;
    if !rev.status.success() {
        anyhow::bail!(
            "candidate does not resolve: {}",
            String::from_utf8_lossy(&rev.stderr).trim()
        );
    }
    let sha = String::from_utf8_lossy(&rev.stdout).trim().to_string();
    let temp = tempfile::tempdir()?;
    let worktree = temp.path().join("candidate");
    let add = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["worktree", "add", "--detach"])
        .arg(&worktree)
        .arg(&sha)
        .output()?;
    if !add.status.success() {
        anyhow::bail!(
            "could not create isolated candidate worktree: {}",
            String::from_utf8_lossy(&add.stderr).trim()
        );
    }
    let _guard = WorktreeGuard {
        repo: repo.clone(),
        path: worktree.clone(),
    };
    let mut results = Vec::new();
    let mut failure = None;
    for gate in gates {
        let started = Instant::now();
        let stdout = tempfile::NamedTempFile::new()?;
        let stderr = tempfile::NamedTempFile::new()?;
        let child = Command::new(&gate.program)
            .args(&gate.args)
            .current_dir(&worktree)
            .stdout(Stdio::from(stdout.reopen()?))
            .stderr(Stdio::from(stderr.reopen()?))
            .spawn();
        let (success, exit_code, timed_out, launch_error) = match child {
            Ok(mut child) => match child.wait_timeout(Duration::from_secs(gate.timeout_secs))? {
                Some(status) => (status.success(), status.code(), false, None),
                None => {
                    child.kill()?;
                    let status = child.wait()?;
                    (false, status.code(), true, None)
                }
            },
            Err(error) => (false, None, false, Some(error.to_string())),
        };
        let elapsed = started.elapsed().as_millis() as u64;
        let (stdout_raw, stdout_truncated) = captured(stdout.path())?;
        let (stderr_raw, stderr_truncated) = captured(stderr.path())?;
        let raw = launch_error.unwrap_or_else(|| format!("{stdout_raw}{stderr_raw}"));
        let (output, output_truncated, output_redactions) =
            crate::db::trace_store::redact_and_bound(&raw);
        results.push(GateResult {
            label: gate.label.clone(),
            program: gate.program.clone(),
            success,
            exit_code,
            timed_out,
            timeout_secs: gate.timeout_secs,
            duration_ms: elapsed,
            output,
            output_truncated: output_truncated || stdout_truncated || stderr_truncated,
            output_redactions,
        });
        if !success {
            failure = Some(if timed_out {
                format!(
                    "gate {:?} timed out after {}s",
                    gate.label, gate.timeout_secs
                )
            } else {
                format!("gate {:?} failed", gate.label)
            });
            break;
        }
    }
    Ok(ReconciliationRun {
        candidate_sha: sha,
        status: if failure.is_some() { "failed" } else { "green" }.into(),
        gates: results,
        failure_reason: failure,
    })
}

/// Manual promotion updates a local ref only. Pushing or updating main remains
/// a distinct human-authorized action.
pub fn promote_local_ref(repo: &Path, candidate_sha: &str) -> anyhow::Result<String> {
    let repo = repo.canonicalize()?;
    let update = run_git(
        &repo,
        &[
            "update-ref",
            "refs/heads/amux/last-known-green",
            candidate_sha,
        ],
    )?;
    if !update.status.success() {
        anyhow::bail!(
            "could not update last-known-green ref: {}",
            String::from_utf8_lossy(&update.stderr).trim()
        );
    }
    let verify = run_git(
        &repo,
        &["rev-parse", "--verify", "refs/heads/amux/last-known-green"],
    )?;
    if !verify.status.success() {
        anyhow::bail!("last-known-green ref was not readable after update");
    }
    Ok(String::from_utf8_lossy(&verify.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn captured_is_binary_safe_when_the_byte_limit_splits_utf8() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let mut bytes = vec![b'a'; GATE_CAPTURE_BYTES as usize - 1];
        bytes.extend_from_slice("€".as_bytes());
        file.write_all(&bytes).unwrap();
        file.flush().unwrap();

        let (captured, truncated) = super::captured(file.path()).unwrap();

        assert!(truncated);
        assert!(captured.ends_with('\u{fffd}'));
        assert_eq!(
            captured.chars().filter(|ch| *ch == 'a').count(),
            bytes.len() - 3
        );
    }
}
