CREATE FUNCTION public.fukulow_audit_metadata_valid(metadata jsonb) RETURNS boolean
LANGUAGE plpgsql IMMUTABLE SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    entry record;
    valid boolean;
BEGIN
    IF NOT coalesce(jsonb_typeof(metadata) = 'object', false) THEN
        RETURN false;
    END IF;
    FOR entry IN SELECT key, value FROM jsonb_each(metadata) LOOP
        valid := CASE
            WHEN entry.key IN ('team_id', 'channel_id', 'invite_id') THEN
                jsonb_typeof(entry.value) = 'string'
                AND (entry.value #>> '{}') COLLATE "C" ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
            WHEN entry.key IN ('role', 'from', 'to') THEN
                jsonb_typeof(entry.value) = 'string'
                AND entry.value #>> '{}' IN ('owner', 'admin', 'member', 'manager')
            WHEN entry.key = 'reason' THEN
                jsonb_typeof(entry.value) = 'string'
                AND entry.value #>> '{}' IN ('removed', 'left_service')
            WHEN entry.key = 'scope' THEN
                jsonb_typeof(entry.value) = 'string'
                AND entry.value #>> '{}' IN ('organization', 'team')
            WHEN entry.key = 'kind' THEN
                jsonb_typeof(entry.value) = 'string'
                AND entry.value #>> '{}' IN ('organization', 'team')
            WHEN entry.key = 'max_uses' THEN
                CASE WHEN jsonb_typeof(entry.value) = 'number' THEN
                    (entry.value #>> '{}')::numeric BETWEEN 1 AND 100
                    AND (entry.value #>> '{}')::numeric = trunc((entry.value #>> '{}')::numeric)
                ELSE false END
            ELSE false
        END;
        -- A NULL result would let a CHECK silently admit an invalid value.
        IF NOT coalesce(valid, false) THEN
            RETURN false;
        END IF;
    END LOOP;
    RETURN true;
END $$;
REVOKE ALL ON FUNCTION public.fukulow_audit_metadata_valid(jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.fukulow_audit_metadata_valid(jsonb) TO fukulow_app;

ALTER TABLE public.audit_events ADD CONSTRAINT audit_events_metadata_check
    CHECK (public.fukulow_audit_metadata_valid(metadata)) NOT VALID;
-- Pre-release audit history is empty or small, so validation's scan is bounded.
-- Existing records stay append-only: invalid history fails migration, never rewrites it.
ALTER TABLE public.audit_events VALIDATE CONSTRAINT audit_events_metadata_check;
