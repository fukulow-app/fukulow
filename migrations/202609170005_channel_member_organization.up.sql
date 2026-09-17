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
-- The departure audit for an outside listing needs the listing's organization.
DROP POLICY audit_events_insert ON audit_events;
CREATE POLICY audit_events_insert ON audit_events FOR INSERT TO fukulow_app
    WITH CHECK (organization_id IN (SELECT public.fukulow_active_organizations())
        OR (actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid AND target_type = 'actor' AND target_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid
            AND (action = 'organization.member.left'
                 OR (action IN ('team.member.removed', 'channel.member.removed')
                     AND metadata->>'reason' = 'left_service'))
            AND EXISTS (SELECT 1 FROM public.organization_members m
                        WHERE m.organization_id = audit_events.organization_id AND m.actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid))
        -- An outside actor leaving removes listings in organizations it has no membership in;
        -- only this audit is admitted, not access (#43).
        OR (actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid AND target_type = 'actor' AND target_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid
            AND action = 'channel.member.removed' AND metadata->>'reason' = 'left_service'
            AND EXISTS (SELECT 1 FROM public.channel_members cm
                        WHERE cm.actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid
                          AND cm.organization_id = audit_events.organization_id
                          AND cm.channel_id::text = audit_events.metadata->>'channel_id')));
