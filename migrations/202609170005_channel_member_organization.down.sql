-- Only schema changes: FORCE remains enabled throughout.
ALTER TABLE channel_members DROP CONSTRAINT channel_members_channel_organization_fkey;
ALTER TABLE channel_members ADD CONSTRAINT channel_members_channel_id_channel_scope_fkey
    FOREIGN KEY (channel_id, channel_scope) REFERENCES channels (id, scope);
ALTER TABLE channel_members DROP COLUMN organization_id;
ALTER TABLE channels DROP CONSTRAINT channels_id_scope_organization_id_key;
