-- Deleting a users row cascades to its sessions, and PostgreSQL does not index the
-- referencing column. Without this, every departure scans the whole sessions table.
CREATE INDEX sessions_actor_id_idx ON sessions (actor_id);
