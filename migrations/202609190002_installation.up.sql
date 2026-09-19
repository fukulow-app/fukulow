-- Global deployment state: a primary key arbitrates bootstrap even for actors
-- whose row-security context cannot see any existing organization.
CREATE TABLE installation (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    bootstrapped_by_actor_id uuid NOT NULL REFERENCES actors (id),
    created_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE installation ENABLE ROW LEVEL SECURITY;
ALTER TABLE installation FORCE ROW LEVEL SECURITY;
CREATE POLICY installation_insert ON installation FOR INSERT TO fukulow_app
    WITH CHECK (bootstrapped_by_actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid);
GRANT INSERT ON installation TO fukulow_app;
