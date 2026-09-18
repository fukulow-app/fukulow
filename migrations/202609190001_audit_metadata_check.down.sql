ALTER TABLE public.audit_events DROP CONSTRAINT audit_events_metadata_check;
DROP FUNCTION public.fukulow_audit_metadata_valid(jsonb);
