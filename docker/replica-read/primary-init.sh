#!/bin/sh
set -eu

psql -v ON_ERROR_STOP=1 \
    --username "$POSTGRES_USER" \
    --dbname "$POSTGRES_DB" <<'SQL'
DO $fod$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_roles WHERE rolname = 'fod_repl'
    ) THEN
        CREATE ROLE fod_repl
            WITH REPLICATION LOGIN PASSWORD 'fod_repl_pass';
    END IF;
END
$fod$;
SQL

printf '%s\n' \
    'host replication fod_repl 0.0.0.0/0 scram-sha-256' \
    'host replication fod_repl ::/0 scram-sha-256' \
    >> "$PGDATA/pg_hba.conf"
