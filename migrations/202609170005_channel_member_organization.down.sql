-- Only schema changes: FORCE remains enabled throughout.
DROP POLICY audit_events_insert ON audit_events;
CREATE POLICY audit_events_insert ON audit_events FOR INSERT TO fukulow_app
    WITH CHECK (organization_id IN (SELECT public.fukulow_active_organizations())
        OR (actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid AND target_type = 'actor' AND target_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid
            AND (action = 'organization.member.left'
                 OR (action IN ('team.member.removed', 'channel.member.removed')
                     AND metadata->>'reason' = 'left_service'))
            AND EXISTS (SELECT 1 FROM public.organization_members m
                        WHERE m.organization_id = audit_events.organization_id AND m.actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid)));
ALTER TABLE channel_members DROP CONSTRAINT channel_members_channel_organization_fkey;
ALTER TABLE channel_members ADD CONSTRAINT channel_members_channel_id_channel_scope_fkey
    FOREIGN KEY (channel_id, channel_scope) REFERENCES channels (id, scope);
ALTER TABLE channel_members DROP COLUMN organization_id;
ALTER TABLE channels DROP CONSTRAINT channels_id_scope_organization_id_key;
