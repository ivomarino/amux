-- Migration 0050: capture Google Calendar event details that were already
-- coming back from the API but never deserialized/stored — meeting link
-- (Google Meet/conferenceData) and the direct Google Calendar web link.
--
-- attendees and organizer already exist as columns on calendar_events
-- (migration 0049) but were never populated; this migration only adds the
-- two columns that genuinely didn't exist yet.

ALTER TABLE calendar_events ADD COLUMN html_link TEXT;
ALTER TABLE calendar_events ADD COLUMN meeting_url TEXT;
