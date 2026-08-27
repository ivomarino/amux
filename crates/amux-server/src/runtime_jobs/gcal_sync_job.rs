//! Google Calendar sync background job (Phase 5)
//!
//! Runs every 15 minutes to sync events from both Gmail accounts.
//! Uses existing OAuth refresh tokens from ~/.amux/connectors/google/

use crate::db::{SharedStore, WriteOutcome, calendar};
use crate::integrations::gcal_sync;
use std::time::Duration;

/// Run periodic Google Calendar sync (every 15 minutes)
pub async fn sync_google_calendars(store: SharedStore) -> anyhow::Result<()> {
    loop {
        tokio::time::sleep(Duration::from_secs(15 * 60)).await;

        if let Err(e) = run_sync(&store).await {
            tracing::warn!("Google Calendar sync failed: {}", e);
        }
    }
}

/// Execute one sync cycle for all configured accounts
async fn run_sync(store: &SharedStore) -> anyhow::Result<()> {
    // Get all accounts from database
    let conn = store.read()?;
    let accounts = calendar::get_accounts(&conn)?;
    drop(conn);

    for account in accounts {
        if let Err(e) = sync_account(store, &account.id, &account.email).await {
            tracing::error!("Failed to sync {}: {}", account.email, e);
            // Mark as errored but continue with other accounts
            let account_id = account.id.clone();
            let err_msg = e.to_string();
            let _ = store
                .write_async(move |conn| {
                    let _ = calendar::update_sync_status(conn, &account_id, "error", Some(&err_msg));
                    Ok(WriteOutcome { applied: true, events: vec![] })
                })
                .await;
        }
    }

    Ok(())
}

/// Sync one account's calendars and events.
///
/// `pub` so `POST /api/gcal/sync` (`api::gcal::trigger_sync`) can run the
/// real fetch instead of the account-loop stub it used to have (which never
/// called Google's API and just echoed back the DB's existing event_count).
pub async fn sync_account(store: &SharedStore, account_id: &str, email: &str) -> anyhow::Result<()> {
    // Mark as syncing. store.read() is a read-only pool connection (amux's
    // single-writer pattern, crate::db::Store::write/write_async) — every
    // mutation below used to run through it and fail with "attempt to write
    // a readonly database" the first time this path actually ran, since it
    // previously only got exercised by the background timer where OAuth was
    // failing first and this write was never reached.
    let aid = account_id.to_string();
    store
        .write_async(move |conn| {
            let _ = calendar::update_sync_status(conn, &aid, "syncing", None);
            Ok(WriteOutcome { applied: true, events: vec![] })
        })
        .await?;

    // Load OAuth token — prefers the Gmail connector's token (keyed by email,
    // already scoped for calendar under the shared OAuth client), falls back
    // to an account-id-keyed token if no Gmail connector token exists.
    let refresh_token = gcal_sync::load_refresh_token(account_id, email)
        .map_err(|e| anyhow::anyhow!("Failed to load token for {}: {}", account_id, e))?;

    // Fetch events from Google Calendar API
    let events = gcal_sync::fetch_calendar_events(&refresh_token, account_id)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch events from Google Calendar: {}", e))?;

    tracing::info!("Fetched {} events from {}", events.len(), email);

    // Store events + prune stale ones + mark sync complete, in one write.
    // event_count reflects rows actually stored, not events.len() — a
    // mismatch here previously hid every insert failing silently behind a
    // nonzero "stored" count.
    let fetched_keys: std::collections::HashSet<(String, String)> =
        events.iter().map(|e| (e.calendar_id.clone(), e.event_id.clone())).collect();
    let aid = account_id.to_string();
    let stored: std::sync::Arc<std::sync::atomic::AtomicI32> =
        std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0));
    let stored_w = stored.clone();
    let pruned: std::sync::Arc<std::sync::atomic::AtomicI32> =
        std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0));
    let pruned_w = pruned.clone();
    store
        .write_async(move |conn| {
            for event in &events {
                match calendar::upsert_event(conn, event) {
                    Ok(()) => { stored_w.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }
                    Err(e) => tracing::warn!("Failed to store event: {}", e),
                }
            }

            // Prune local rows Google no longer returns — the fetch was a
            // full replace within [now-90d, now+90d], so anything in that
            // window we already had locally but did NOT see this round is
            // gone on Google's side (deleted directly in Google, or via our
            // own DELETE /api/gcal/events/:id, which only calls Google's API
            // and never touched the local row itself). A row outside the
            // window is left alone — its absence from this fetch says
            // nothing about whether it still exists.
            match calendar::get_events_for_account(conn, &aid, None, None) {
                Ok(existing) => {
                    for row in existing {
                        if event_in_sync_window(&row.start_time)
                            && !fetched_keys.contains(&(row.calendar_id.clone(), row.event_id.clone()))
                        {
                            match calendar::delete_event(conn, &row.id) {
                                Ok(()) => { pruned_w.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }
                                Err(e) => tracing::warn!("Failed to prune stale event {}: {}", row.id, e),
                            }
                        }
                    }
                }
                // Previously silent (`if let Ok(...)`) — a query error here
                // meant pruning silently did nothing, every sync, forever,
                // with no trace anywhere that it had even been attempted.
                Err(e) => tracing::warn!("Prune step failed to load existing events for {}: {}", aid, e),
            }

            let count = stored_w.load(std::sync::atomic::Ordering::Relaxed);
            let _ = calendar::mark_sync_complete(conn, &aid, count);
            // Keeps `calendar_sync_metadata` (owner/purpose/total_events,
            // surfaced by GET /api/gcal/status) current — it was written
            // once at boot by calendar_init and never touched again
            // otherwise, so total_events/last_full_sync_at would silently
            // go stale forever. `mark_sync_complete` above updates the
            // separate `calendar_accounts` row this loop actually reads
            // its own state from; this is additive, not a replacement.
            let _ = calendar::update_sync_metadata(conn, &aid, count, true);
            Ok(WriteOutcome { applied: true, events: vec![] })
        })
        .await?;
    let event_count = stored.load(std::sync::atomic::Ordering::Relaxed);
    let pruned_count = pruned.load(std::sync::atomic::Ordering::Relaxed);

    tracing::info!(
        "Sync complete for {}: {} events stored, {} stale local event(s) pruned",
        email, event_count, pruned_count
    );

    Ok(())
}

/// True if `start_time` (RFC 3339 for timed events, "YYYYMMDD" for all-day —
/// see parse_google_datetime) falls inside the same window
/// fetch_events_for_calendar just queried. Deliberately parses rather than
/// comparing as TEXT: the two stored formats don't share a lexical
/// ordering with each other (an all-day "20260825" and a timed
/// "2026-08-25T10:00:00+00:00" do not compare correctly as strings), so a
/// SQL-level BETWEEN across both would silently mis-scope the prune.
fn event_in_sync_window(start_time: &Option<String>) -> bool {
    let now = chrono::Utc::now();
    let lo = now - chrono::Duration::days(gcal_sync::SYNC_WINDOW_DAYS);
    let hi = now + chrono::Duration::days(gcal_sync::SYNC_WINDOW_DAYS);
    let Some(s) = start_time else { return false };

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        let dt = dt.with_timezone(&chrono::Utc);
        return dt >= lo && dt <= hi;
    }
    if s.len() == 8 {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y%m%d") {
            if let Some(ndt) = d.and_hms_opt(0, 0, 0) {
                let dt = ndt.and_utc();
                return dt >= lo && dt <= hi;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires live OAuth token
    async fn test_load_token() {
        // This would need ~/.amux/connectors/google/user@example.com.json
        let token = gcal_sync::load_refresh_token("account-1", "user@example.com");
        assert!(token.is_ok());
    }
}
