\set ON_ERROR_STOP on
-- Runs once, for a new volume, as the administrator. Passwords come from the
-- environment and are inserted with :'…', which quotes them as SQL literals, so
-- any printable character works and none is interpolated into SQL text.
\getenv migrator_password FUKULOW_MIGRATOR_PASSWORD
\getenv app_password FUKULOW_APP_PASSWORD
CREATE ROLE fukulow_migrator LOGIN PASSWORD :'migrator_password' NOSUPERUSER NOBYPASSRLS NOCREATEDB NOCREATEROLE;
-- The runtime cannot create objects or assume the migration role.
CREATE ROLE fukulow_app LOGIN PASSWORD :'app_password' NOSUPERUSER NOBYPASSRLS NOCREATEDB NOCREATEROLE;
GRANT CONNECT ON DATABASE fukulow TO fukulow_migrator, fukulow_app;
GRANT USAGE, CREATE ON SCHEMA public TO fukulow_migrator;
GRANT USAGE ON SCHEMA public TO fukulow_app;
