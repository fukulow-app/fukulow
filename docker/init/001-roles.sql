\set ON_ERROR_STOP on
-- Development credentials only; self-hosters must replace both passwords. gitleaks:allow
CREATE ROLE fukulow_migrator LOGIN PASSWORD 'fukulow_migrator' NOSUPERUSER NOBYPASSRLS CREATEDB NOCREATEROLE;
-- The runtime cannot create objects or assume the migration role. gitleaks:allow
CREATE ROLE fukulow_app LOGIN PASSWORD 'fukulow_app' NOSUPERUSER NOBYPASSRLS NOCREATEDB NOCREATEROLE;
GRANT CONNECT ON DATABASE fukulow_dev TO fukulow_migrator, fukulow_app;
GRANT USAGE, CREATE ON SCHEMA public TO fukulow_migrator;
GRANT USAGE ON SCHEMA public TO fukulow_app;
