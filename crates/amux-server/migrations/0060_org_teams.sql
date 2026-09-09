-- A workspace member belongs to one team, and the team owns the reusable
-- global/group/worker boundary.  Keep the scope columns added in 0059 as a
-- denormalized compatibility/readback copy, but make team_id the assignment
-- primitive for every new invite and accepted member.
CREATE TABLE IF NOT EXISTS org_teams (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE COLLATE NOCASE,
    scope_level TEXT NOT NULL CHECK (scope_level IN ('global','group','worker')),
    scope_name  TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL,
    CHECK (
        (scope_level = 'global' AND scope_name = '') OR
        (scope_level IN ('group','worker') AND scope_name <> '')
    )
);

-- ADDCOL: org_members team_id TEXT
-- ADDCOL: org_invites team_id TEXT

INSERT OR IGNORE INTO org_teams (id,name,scope_level,scope_name,created_at)
VALUES ('team_global','Everyone','global','',CAST(strftime('%s','now') AS INTEGER));

-- Preserve every grant made before teams existed.  A scoped member must never
-- become global just because its row predates this migration.  Distinct grants
-- become explicit legacy teams that the owner can rename or consolidate later.
INSERT OR IGNORE INTO org_teams (id,name,scope_level,scope_name,created_at)
SELECT
    'team_legacy_' || printf('%04d', ROW_NUMBER() OVER (ORDER BY scope_level,scope_name)),
    CASE scope_level WHEN 'group' THEN 'Group: ' ELSE 'Worker: ' END || scope_name,
    scope_level,
    scope_name,
    CAST(strftime('%s','now') AS INTEGER)
FROM (
    SELECT DISTINCT scope_level,scope_name FROM org_members
    WHERE scope_level IN ('group','worker') AND scope_name <> ''
    UNION
    SELECT DISTINCT scope_level,scope_name FROM org_invites
    WHERE scope_level IN ('group','worker') AND scope_name <> ''
);

UPDATE org_members SET team_id='team_global'
WHERE scope_level='global' AND (team_id IS NULL OR team_id='');
UPDATE org_invites SET team_id='team_global'
WHERE scope_level='global' AND (team_id IS NULL OR team_id='');

UPDATE org_members
SET team_id=(SELECT id FROM org_teams t
             WHERE t.scope_level=org_members.scope_level
               AND t.scope_name=org_members.scope_name
             ORDER BY id LIMIT 1)
WHERE (team_id IS NULL OR team_id='') AND scope_level IN ('group','worker');
UPDATE org_invites
SET team_id=(SELECT id FROM org_teams t
             WHERE t.scope_level=org_invites.scope_level
               AND t.scope_name=org_invites.scope_name
             ORDER BY id LIMIT 1)
WHERE (team_id IS NULL OR team_id='') AND scope_level IN ('group','worker');

CREATE INDEX IF NOT EXISTS idx_org_members_team ON org_members(team_id);
CREATE INDEX IF NOT EXISTS idx_org_invites_team ON org_invites(team_id);
