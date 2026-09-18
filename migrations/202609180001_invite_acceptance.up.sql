-- No tenant rows are read or rewritten; FORCE remains enabled throughout.
-- xmin alone also proves a no-op UPDATE. Every unrevoked row version must instead
-- come from a bounded increment; inserts cannot manufacture an already-used invite.
CREATE FUNCTION fukulow_guard_invite_use() RETURNS trigger
LANGUAGE plpgsql SET search_path = pg_catalog, pg_temp
AS $$ BEGIN
    IF TG_OP = 'INSERT' THEN
        IF NEW.used_count <> 0 THEN
            RAISE EXCEPTION 'invalid initial invite use' USING ERRCODE = '23514';
        END IF;
    ELSIF NEW.used_count IS DISTINCT FROM OLD.used_count THEN
        -- A use changes the count and nothing else: revoking in the same statement would
        -- make one row version both the proof of a use and a revocation.
        IF NEW.used_count <> OLD.used_count + 1 OR OLD.used_count >= OLD.max_uses
           OR OLD.revoked_at IS NOT NULL OR OLD.expires_at <= now()
           OR NEW.revoked_at IS DISTINCT FROM OLD.revoked_at THEN
            RAISE EXCEPTION 'invalid invite use' USING ERRCODE = '42501';
        END IF;
    ELSIF OLD.revoked_at IS NOT NULL OR NEW.revoked_at IS NULL THEN
        -- The only other change is the first revocation of an unrevoked invite. A no-op,
        -- an un-revocation and a rewritten revocation time are all refused: the time is
        -- when the invite stopped admitting anyone, and the audit record names it.
        RAISE EXCEPTION 'invite update requires a use or a first revocation' USING ERRCODE = '42501';
    END IF;
    RETURN NEW;
END $$;
REVOKE ALL ON FUNCTION fukulow_guard_invite_use() FROM PUBLIC;
CREATE TRIGGER invites_guard_use BEFORE INSERT OR UPDATE ON invites
    FOR EACH ROW EXECUTE FUNCTION fukulow_guard_invite_use();

-- Measured on PostgreSQL 17: the outer increment's xmin survives create_person's
-- savepoint. A committed use, an initial row and a no-op cannot supply this proof.
CREATE POLICY organization_members_insert_invite ON organization_members FOR INSERT TO fukulow_app
    WITH CHECK (actor_id = nullif(current_setting('fukulow.actor_id', true), '')::uuid
        AND role = 'member' AND status = 'active'
        AND EXISTS (SELECT 1 FROM public.invites i
            WHERE i.organization_id = organization_members.organization_id
              AND i.token_hash = current_setting('fukulow.invite_token_hash', true)
              AND i.kind = 'organization' AND i.revoked_at IS NULL AND i.expires_at > now()
              AND i.used_count > 0 AND i.xmin = pg_current_xact_id()::xid));
