-- Channels and messages are tenant-owned; channel_members is cross-tenant.
CREATE TABLE channels (
    id uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations (id),
    team_id uuid,
    scope text NOT NULL,
    name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    next_message_seq bigint NOT NULL DEFAULT 0 CHECK (next_message_seq >= 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (id, organization_id),
    UNIQUE (id, scope),
    -- MATCH SIMPLE intentionally skips this key when team_id is NULL: organization channels have no team.
    FOREIGN KEY (team_id, organization_id) REFERENCES teams (id, organization_id),
    CHECK ((scope = 'organization' AND team_id IS NULL) OR (scope = 'team' AND team_id IS NOT NULL))
);
CREATE UNIQUE INDEX channels_team_name_key
    ON channels (team_id, lower(name COLLATE "C")) WHERE scope = 'team';
CREATE UNIQUE INDEX channels_organization_name_key
    ON channels (organization_id, lower(name COLLATE "C")) WHERE scope = 'organization';
CREATE TABLE channel_members (
    channel_id uuid NOT NULL,
    channel_scope text NOT NULL CHECK (channel_scope = 'team'),
    actor_id uuid NOT NULL REFERENCES actors (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, actor_id),
    FOREIGN KEY (channel_id, channel_scope) REFERENCES channels (id, scope)
);
-- The primary key leads with channel_id; leaving the service finds an actor's listings
-- across every channel, which would otherwise scan the whole table.
CREATE INDEX channel_members_actor_id_idx ON channel_members (actor_id);
CREATE TABLE messages (
    id uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations (id),
    channel_id uuid NOT NULL,
    channel_seq bigint NOT NULL CHECK (channel_seq >= 1),
    sender_actor_id uuid NOT NULL REFERENCES actors (id),
    body text NOT NULL CHECK (char_length(body) BETWEEN 1 AND 4000),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (channel_id, channel_seq),
    FOREIGN KEY (channel_id, organization_id) REFERENCES channels (id, organization_id)
);
ALTER TABLE audit_events DROP CONSTRAINT audit_events_target_type_check;
ALTER TABLE audit_events ADD CONSTRAINT audit_events_target_type_check
    CHECK (target_type IN ('organization', 'team', 'actor', 'channel'));
GRANT SELECT, INSERT ON channels, channel_members, messages TO fukulow_app;
GRANT UPDATE (next_message_seq) ON channels TO fukulow_app;
GRANT DELETE ON channel_members TO fukulow_app;
