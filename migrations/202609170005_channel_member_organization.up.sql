-- No released installation has channel listings, so no data statements or FORCE exception are needed.
ALTER TABLE channels ADD CONSTRAINT channels_id_scope_organization_id_key
    UNIQUE (id, scope, organization_id);
ALTER TABLE channel_members ADD COLUMN organization_id uuid NOT NULL;
ALTER TABLE channel_members DROP CONSTRAINT channel_members_channel_id_channel_scope_fkey;
ALTER TABLE channel_members ADD CONSTRAINT channel_members_channel_organization_fkey
    FOREIGN KEY (channel_id, channel_scope, organization_id)
    REFERENCES channels (id, scope, organization_id);
COMMENT ON COLUMN channel_members.organization_id IS
    'The channel organization, not the actor organization; outside actors need no organization membership.';
