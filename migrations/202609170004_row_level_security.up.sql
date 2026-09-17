-- FORCE also subjects the owner: only these two fixed membership readers have an owner SELECT policy.
CREATE FUNCTION fukulow_active_organizations() RETURNS SETOF uuid
LANGUAGE sql STABLE PARALLEL RESTRICTED SECURITY DEFINER SET search_path = pg_catalog, pg_temp
AS $$ SELECT organization_id FROM public.organization_members
       WHERE actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid
         AND status = 'active' $$;
CREATE FUNCTION fukulow_organization_has_members(uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp
AS $$ SELECT EXISTS (SELECT 1 FROM public.organization_members WHERE organization_id = $1) $$;
REVOKE ALL ON FUNCTION fukulow_active_organizations() FROM PUBLIC;
REVOKE ALL ON FUNCTION fukulow_organization_has_members(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION fukulow_active_organizations() TO fukulow_app;
GRANT EXECUTE ON FUNCTION fukulow_organization_has_members(uuid) TO fukulow_app;

ALTER TABLE organizations ENABLE ROW LEVEL SECURITY;
ALTER TABLE organizations FORCE ROW LEVEL SECURITY;
ALTER TABLE organization_members ENABLE ROW LEVEL SECURITY;
ALTER TABLE organization_members FORCE ROW LEVEL SECURITY;
ALTER TABLE teams ENABLE ROW LEVEL SECURITY;
ALTER TABLE teams FORCE ROW LEVEL SECURITY;
ALTER TABLE team_members ENABLE ROW LEVEL SECURITY;
ALTER TABLE team_members FORCE ROW LEVEL SECURITY;
ALTER TABLE channels ENABLE ROW LEVEL SECURITY;
ALTER TABLE channels FORCE ROW LEVEL SECURITY;
ALTER TABLE channel_members ENABLE ROW LEVEL SECURITY;
ALTER TABLE channel_members FORCE ROW LEVEL SECURITY;
ALTER TABLE messages ENABLE ROW LEVEL SECURITY;
ALTER TABLE messages FORCE ROW LEVEL SECURITY;
ALTER TABLE invites ENABLE ROW LEVEL SECURITY;
ALTER TABLE invites FORCE ROW LEVEL SECURITY;
ALTER TABLE audit_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE audit_events FORCE ROW LEVEL SECURITY;
ALTER TABLE actors ENABLE ROW LEVEL SECURITY;
ALTER TABLE actors FORCE ROW LEVEL SECURITY;
ALTER TABLE users ENABLE ROW LEVEL SECURITY;
ALTER TABLE users FORCE ROW LEVEL SECURITY;
ALTER TABLE sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE sessions FORCE ROW LEVEL SECURITY;

CREATE POLICY organization_members_select_owner ON organization_members FOR SELECT TO fukulow_migrator
    USING (true);
CREATE POLICY organizations_select ON organizations FOR SELECT TO fukulow_app
    USING (id IN (SELECT public.fukulow_active_organizations()));
CREATE POLICY organizations_insert ON organizations FOR INSERT TO fukulow_app
    WITH CHECK (nullif(current_setting('fukulow.actor_id', true), '')::uuid IS NOT NULL);
CREATE POLICY organizations_update ON organizations FOR UPDATE TO fukulow_app
    USING (id IN (SELECT public.fukulow_active_organizations()))
    WITH CHECK (id IN (SELECT public.fukulow_active_organizations()));
CREATE POLICY organization_members_select ON organization_members FOR SELECT TO fukulow_app
    USING (organization_id IN (SELECT public.fukulow_active_organizations()) OR actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid);
CREATE POLICY organization_members_insert ON organization_members FOR INSERT TO fukulow_app
    WITH CHECK (organization_id IN (SELECT public.fukulow_active_organizations())
        OR (actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid AND role = 'owner' AND status = 'active'
            AND NOT public.fukulow_organization_has_members(organization_id)));
-- No invite branch yet: #3 adds it together with a database-side tie to the invite's use.
CREATE POLICY organization_members_update ON organization_members FOR UPDATE TO fukulow_app
    USING (organization_id IN (SELECT public.fukulow_active_organizations()) OR actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid)
    WITH CHECK (organization_id IN (SELECT public.fukulow_active_organizations())
        OR (actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid AND status = 'left' AND display_name IS NULL));

-- WITH CHECK cannot compare the old role; an inactive actor may only erase their own membership.
CREATE FUNCTION fukulow_guard_inactive_member_role() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp
AS $$ BEGIN
    IF OLD.status <> 'active'
       AND OLD.actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid
       AND NEW.role IS DISTINCT FROM OLD.role THEN
        RAISE EXCEPTION 'inactive members cannot change their own role' USING ERRCODE = '42501';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION fukulow_guard_inactive_member_role() FROM PUBLIC;
CREATE TRIGGER organization_members_guard_inactive_role BEFORE UPDATE ON organization_members
    FOR EACH ROW EXECUTE FUNCTION fukulow_guard_inactive_member_role();

CREATE POLICY teams_select ON teams FOR SELECT TO fukulow_app
    USING (organization_id IN (SELECT public.fukulow_active_organizations()));
CREATE POLICY teams_insert ON teams FOR INSERT TO fukulow_app
    WITH CHECK (organization_id IN (SELECT public.fukulow_active_organizations()));
CREATE POLICY channels_select ON channels FOR SELECT TO fukulow_app
    USING (organization_id IN (SELECT public.fukulow_active_organizations()));
CREATE POLICY channels_insert ON channels FOR INSERT TO fukulow_app
    WITH CHECK (organization_id IN (SELECT public.fukulow_active_organizations()));
CREATE POLICY channels_update ON channels FOR UPDATE TO fukulow_app
    USING (organization_id IN (SELECT public.fukulow_active_organizations()))
    WITH CHECK (organization_id IN (SELECT public.fukulow_active_organizations()));
CREATE POLICY messages_select ON messages FOR SELECT TO fukulow_app
    USING (organization_id = ANY (ARRAY(SELECT a.organization_id FROM public.fukulow_active_organizations() AS a(organization_id))));
CREATE POLICY messages_insert ON messages FOR INSERT TO fukulow_app
    WITH CHECK (organization_id IN (SELECT public.fukulow_active_organizations()));
CREATE POLICY team_members_select ON team_members FOR SELECT TO fukulow_app
    USING (organization_id IN (SELECT public.fukulow_active_organizations()) OR actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid);
CREATE POLICY team_members_insert ON team_members FOR INSERT TO fukulow_app
    WITH CHECK (organization_id IN (SELECT public.fukulow_active_organizations()));
CREATE POLICY team_members_update ON team_members FOR UPDATE TO fukulow_app
    USING (organization_id IN (SELECT public.fukulow_active_organizations()))
    WITH CHECK (organization_id IN (SELECT public.fukulow_active_organizations()));
CREATE POLICY team_members_delete ON team_members FOR DELETE TO fukulow_app
    USING (organization_id IN (SELECT public.fukulow_active_organizations()) OR actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid);
CREATE POLICY invites_select ON invites FOR SELECT TO fukulow_app
    USING (organization_id IN (SELECT public.fukulow_active_organizations()) OR token_hash = current_setting('fukulow.invite_token_hash', true));
CREATE POLICY invites_insert ON invites FOR INSERT TO fukulow_app
    WITH CHECK (organization_id IN (SELECT public.fukulow_active_organizations()));
CREATE POLICY invites_update ON invites FOR UPDATE TO fukulow_app
    USING (organization_id IN (SELECT public.fukulow_active_organizations()) OR token_hash = current_setting('fukulow.invite_token_hash', true))
    WITH CHECK (organization_id IN (SELECT public.fukulow_active_organizations()) OR token_hash = current_setting('fukulow.invite_token_hash', true));
CREATE POLICY audit_events_insert ON audit_events FOR INSERT TO fukulow_app
    WITH CHECK (organization_id IN (SELECT public.fukulow_active_organizations())
        OR (actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid AND target_type = 'actor' AND target_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid
            AND (action = 'organization.member.left'
                 OR (action IN ('team.member.removed', 'channel.member.removed')
                     AND metadata->>'reason' = 'left_service'))
            AND EXISTS (SELECT 1 FROM public.organization_members m
                        WHERE m.organization_id = audit_events.organization_id AND m.actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid)));
CREATE POLICY channel_members_select ON channel_members FOR SELECT TO fukulow_app
    USING (channel_id IN (SELECT id FROM public.channels) OR actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid);
CREATE POLICY channel_members_insert ON channel_members FOR INSERT TO fukulow_app
    WITH CHECK (channel_id IN (SELECT id FROM public.channels));
CREATE POLICY channel_members_delete ON channel_members FOR DELETE TO fukulow_app
    USING (channel_id IN (SELECT id FROM public.channels) OR actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid);
CREATE POLICY actors_select ON actors FOR SELECT TO fukulow_app
    USING (id = nullif(current_setting('fukulow.actor_id', true), '')::uuid OR id IN (SELECT actor_id FROM public.organization_members));
CREATE POLICY actors_insert ON actors FOR INSERT TO fukulow_app
    WITH CHECK (true);
CREATE POLICY actors_update ON actors FOR UPDATE TO fukulow_app
    USING (id = nullif(current_setting('fukulow.actor_id', true), '')::uuid OR id IN (SELECT actor_id FROM public.organization_members))
    WITH CHECK (id = nullif(current_setting('fukulow.actor_id', true), '')::uuid OR id IN (SELECT actor_id FROM public.organization_members));
CREATE POLICY users_select ON users FOR SELECT TO fukulow_app
    USING (actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid OR lower(email COLLATE "C") = lower(current_setting('fukulow.sign_in_email', true) COLLATE "C"));
CREATE POLICY users_insert ON users FOR INSERT TO fukulow_app
    WITH CHECK (actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid);
CREATE POLICY users_delete ON users FOR DELETE TO fukulow_app
    USING (actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid);
CREATE POLICY sessions_select ON sessions FOR SELECT TO fukulow_app
    USING (actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid OR token_hash = current_setting('fukulow.session_token_hash', true));
CREATE POLICY sessions_insert ON sessions FOR INSERT TO fukulow_app
    WITH CHECK (actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid);
CREATE POLICY sessions_update ON sessions FOR UPDATE TO fukulow_app
    USING (actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid)
    WITH CHECK (actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid);

-- Captured plans use the full indexes for both active and all-status actor lookups.
CREATE INDEX organization_members_actor_id_idx ON organization_members (actor_id);
CREATE INDEX team_members_actor_id_idx ON team_members (actor_id);
