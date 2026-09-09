-- Local/Tailscale invitees are real users, not bearer-token owners.  Persist
-- the resource boundary on both the invitation (what was granted) and member
-- (what every subsequent request is authorized against).  Existing rows stay
-- global for backward compatibility with links accepted before scopes existed.
ALTER TABLE org_invites ADD COLUMN scope_level TEXT NOT NULL DEFAULT 'global';
ALTER TABLE org_invites ADD COLUMN scope_name TEXT NOT NULL DEFAULT '';

ALTER TABLE org_members ADD COLUMN scope_level TEXT NOT NULL DEFAULT 'global';
ALTER TABLE org_members ADD COLUMN scope_name TEXT NOT NULL DEFAULT '';
