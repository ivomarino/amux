-- Migration 0049: Google Calendar sync support
-- Adds tables for calendar events, sync metadata, and account management

-- Calendar accounts (one per connected Gmail account)
CREATE TABLE IF NOT EXISTS calendar_accounts (
  id TEXT PRIMARY KEY,
  email TEXT UNIQUE NOT NULL,
  service_name TEXT NOT NULL DEFAULT 'google-calendar',
  display_name TEXT,
  is_primary BOOLEAN NOT NULL DEFAULT 0,
  oauth_refresh_token TEXT NOT NULL,
  oauth_token_expiry TEXT,
  synced_at TEXT,
  next_sync_at TEXT,
  sync_status TEXT NOT NULL DEFAULT 'pending', -- pending, syncing, ok, error
  last_error TEXT,
  event_count INTEGER DEFAULT 0,
  calendar_list_synced_at TEXT,
  sync_window_days INTEGER DEFAULT 90,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_calendar_accounts_email ON calendar_accounts(email);
CREATE INDEX IF NOT EXISTS idx_calendar_accounts_primary ON calendar_accounts(is_primary);

-- Calendar events (unified from all accounts)
CREATE TABLE IF NOT EXISTS calendar_events (
  id TEXT PRIMARY KEY,
  event_id TEXT NOT NULL,
  account_id TEXT NOT NULL,
  calendar_id TEXT NOT NULL,
  title TEXT NOT NULL,
  description TEXT,
  start_time TEXT,
  end_time TEXT,
  all_day BOOLEAN DEFAULT 0,
  location TEXT,
  attendees TEXT, -- JSON array
  organizer TEXT,
  status TEXT, -- confirmed, tentative, cancelled
  event_type TEXT, -- default, focusTime, workingLocation, outOfOffice
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  synced_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(account_id, event_id)
);

CREATE INDEX IF NOT EXISTS idx_calendar_events_account ON calendar_events(account_id);
CREATE INDEX IF NOT EXISTS idx_calendar_events_start_time ON calendar_events(start_time);
CREATE INDEX IF NOT EXISTS idx_calendar_events_status ON calendar_events(status);

-- Calendar sync metadata
CREATE TABLE IF NOT EXISTS calendar_sync_metadata (
  id INTEGER PRIMARY KEY,
  account_id TEXT NOT NULL UNIQUE,
  total_events INTEGER DEFAULT 0,
  last_full_sync_at TEXT,
  last_incremental_sync_at TEXT,
  sync_window_days INTEGER DEFAULT 90, -- Look ahead/back window
  is_enabled BOOLEAN NOT NULL DEFAULT 1,
  sync_frequency_minutes INTEGER DEFAULT 15,
  owner TEXT DEFAULT 'platform-team',
  rotation_days INTEGER DEFAULT 90,
  purpose TEXT DEFAULT 'Calendar sync from Google Calendar API',
  used_by TEXT DEFAULT 'dashboard,ical-feed,agents',
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_calendar_sync_metadata_enabled ON calendar_sync_metadata(is_enabled);

-- Calendar subscriptions (for iCal feed)
CREATE TABLE IF NOT EXISTS calendar_subscriptions (
  id INTEGER PRIMARY KEY,
  feed_name TEXT UNIQUE NOT NULL,
  account_ids TEXT NOT NULL, -- Comma-separated or JSON
  include_all_events BOOLEAN DEFAULT 1,
  event_type_filters TEXT, -- JSON array of event types to include
  is_public BOOLEAN DEFAULT 0,
  secret_token TEXT, -- For access control
  last_generated_at TEXT,
  event_count INTEGER DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_calendar_subscriptions_feed ON calendar_subscriptions(feed_name);

-- Pre-populate with user's Gmail accounts - use environment-based config instead
-- See: crates/amux-server/src/db/calendar_init.rs
-- INSERT OR IGNORE INTO calendar_accounts (id, email, display_name, is_primary)
-- VALUES (?, ?, ?, ?);

-- Pre-populate sync metadata - use environment-based config instead
-- INSERT OR IGNORE INTO calendar_sync_metadata (account_id, owner, purpose)
-- VALUES (?, ?, ?);

-- Pre-populate default iCal subscription - use environment-based config instead
-- INSERT OR IGNORE INTO calendar_subscriptions (feed_name, account_ids, include_all_events)
-- VALUES (?, ?, ?);
