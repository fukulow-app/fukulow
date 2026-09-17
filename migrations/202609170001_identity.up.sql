CREATE TABLE actors (
    id uuid PRIMARY KEY,
    type text NOT NULL CHECK (type IN ('human', 'bot', 'integration', 'system')),
    display_name text,
    deleted_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (id, type)
);
CREATE TABLE users (
    actor_id uuid PRIMARY KEY,
    actor_type text NOT NULL CHECK (actor_type = 'human'),
    email text NOT NULL,
    password_hash text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (actor_id, actor_type) REFERENCES actors (id, type)
);
CREATE UNIQUE INDEX users_email_lower_key ON users (lower(email COLLATE "C"));
CREATE TABLE organizations (
    id uuid PRIMARY KEY,
    name text NOT NULL,
    slug text NOT NULL UNIQUE CONSTRAINT organizations_slug_format CHECK (slug ~ '^[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?$'),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE organization_members (
    organization_id uuid NOT NULL REFERENCES organizations (id),
    actor_id uuid NOT NULL REFERENCES actors (id),
    role text NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'member')),
    status text NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended', 'left')),
    display_name text,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (organization_id, actor_id)
);
CREATE TABLE teams (
    id uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations (id),
    name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (id, organization_id)
);
CREATE TABLE team_members (
    team_id uuid NOT NULL,
    actor_id uuid NOT NULL REFERENCES actors (id),
    organization_id uuid NOT NULL REFERENCES organizations (id),
    role text NOT NULL DEFAULT 'member' CHECK (role IN ('manager', 'member')),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (team_id, actor_id),
    FOREIGN KEY (team_id, organization_id) REFERENCES teams (id, organization_id),
    FOREIGN KEY (organization_id, actor_id) REFERENCES organization_members (organization_id, actor_id)
);
CREATE TABLE invites (
    id uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations (id),
    kind text NOT NULL,
    target_team_id uuid,
    token_hash text NOT NULL UNIQUE,
    expires_at timestamptz NOT NULL,
    max_uses integer NOT NULL CHECK (max_uses > 0),
    used_count integer NOT NULL DEFAULT 0 CHECK (used_count >= 0 AND used_count <= max_uses),
    revoked_at timestamptz,
    created_by_actor_id uuid NOT NULL REFERENCES actors (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    -- MATCH SIMPLE intentionally skips this key for organization invites; the CHECK requires their target to be NULL.
    FOREIGN KEY (target_team_id, organization_id) REFERENCES teams (id, organization_id),
    CHECK ((kind = 'organization' AND target_team_id IS NULL) OR (kind = 'team' AND target_team_id IS NOT NULL))
);
CREATE TABLE sessions (
    id uuid PRIMARY KEY,
    actor_id uuid NOT NULL REFERENCES users (actor_id) ON DELETE CASCADE,
    token_hash text NOT NULL UNIQUE,
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE audit_events (
    id uuid PRIMARY KEY,
    organization_id uuid NOT NULL REFERENCES organizations (id),
    actor_id uuid NOT NULL REFERENCES actors (id),
    action text NOT NULL CHECK (action ~ '^[a-z]+(\.[a-z_]+){1,2}$'),
    target_type text NOT NULL CHECK (target_type IN ('organization', 'team', 'actor')),
    target_id uuid NOT NULL,
    metadata jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
GRANT SELECT, INSERT ON actors, users, organizations, organization_members, teams, team_members, invites, sessions TO fukulow_app;
GRANT INSERT ON audit_events TO fukulow_app;
GRANT UPDATE (display_name, deleted_at) ON actors TO fukulow_app;
-- Row locks require UPDATE on at least one column, even when no value is changed.
GRANT UPDATE (name) ON organizations TO fukulow_app;
GRANT UPDATE (role, status, display_name) ON organization_members TO fukulow_app;
GRANT UPDATE (role) ON team_members TO fukulow_app;
GRANT UPDATE (used_count, revoked_at) ON invites TO fukulow_app;
GRANT UPDATE (revoked_at) ON sessions TO fukulow_app;
GRANT DELETE ON users, team_members TO fukulow_app;
