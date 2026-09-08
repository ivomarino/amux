//! Name synchronous job polls that hold a Tokio worker through health/TLS deadlines.
use std::{future::Future, task::Poll, time::{Duration, Instant}};

pub(super) async fn watch<T>(job: &str, future: impl Future<Output = T>) -> T {
    observe(future, |elapsed| {
        if elapsed >= Duration::from_millis(250) {
            tracing::warn!(target: "runtime", verdict = "runtime_job_blocking_poll", job,
                elapsed_ms = elapsed.as_millis() as u64, pid = std::process::id(),
                commit = env!("AMUX_BUILD_COMMIT_FULL"),
                "job held an async runtime thread without yielding; health and TLS may stall");
        }
    }).await
}

async fn observe<T>(future: impl Future<Output = T>, mut report: impl FnMut(Duration)) -> T {
    let mut future = std::pin::pin!(future);
    std::future::poll_fn(|cx| {
        let started = Instant::now();
        let result: Poll<T> = future.as_mut().poll(cx);
        report(started.elapsed());
        result
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reports_time_inside_poll_instead_of_time_awaiting_io() {
        let mut blocking = Duration::ZERO;
        observe(async { std::thread::sleep(Duration::from_millis(80)); }, |d| blocking = blocking.max(d)).await;
        assert!(blocking >= Duration::from_millis(80));
        let mut yielding = Duration::ZERO;
        observe(tokio::time::sleep(Duration::from_millis(80)), |d| yielding = yielding.max(d)).await;
        assert!(yielding < Duration::from_millis(40), "awaited IO must not be blamed as a blocking poll: {yielding:?}");
    }
}
