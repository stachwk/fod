#!/bin/sh
set -eu

: "${POSTGRES_USER:?POSTGRES_USER is required}"
: "${POSTGRES_DB:?POSTGRES_DB is required}"
: "${FOD_REPLICATION_USER:=fod_repl}"
: "${FOD_REPLICATION_PASSWORD:?FOD_REPLICATION_PASSWORD is required}"
: "${FOD_REPLICA_COUNT:=0}"

case "${FOD_REPLICATION_USER}" in
  ''|*[!A-Za-z0-9_]*)
    echo "FOD_REPLICATION_USER must contain only letters, digits and underscore" >&2
    exit 2
    ;;
esac
case "${FOD_REPLICA_COUNT}" in
  ''|*[!0-9]*)
    echo "FOD_REPLICA_COUNT must be a non-negative integer" >&2
    exit 2
    ;;
esac

psql -v ON_ERROR_STOP=1 \
  --username "${POSTGRES_USER}" \
  --dbname "${POSTGRES_DB}" \
  --set=repl_user="${FOD_REPLICATION_USER}" \
  --set=repl_password="${FOD_REPLICATION_PASSWORD}" <<'SQL'
SELECT format('CREATE ROLE %I WITH REPLICATION LOGIN PASSWORD %L', :'repl_user', :'repl_password')
WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'repl_user')
\gexec
SELECT format('ALTER ROLE %I WITH REPLICATION LOGIN PASSWORD %L', :'repl_user', :'repl_password')
\gexec
SQL

hba_v4="host replication ${FOD_REPLICATION_USER} 0.0.0.0/0 scram-sha-256"
hba_v6="host replication ${FOD_REPLICATION_USER} ::/0 scram-sha-256"
grep -Fqx "${hba_v4}" "${PGDATA}/pg_hba.conf" || printf '%s\n' "${hba_v4}" >> "${PGDATA}/pg_hba.conf"
grep -Fqx "${hba_v6}" "${PGDATA}/pg_hba.conf" || printf '%s\n' "${hba_v6}" >> "${PGDATA}/pg_hba.conf"

index=1
while [ "${index}" -le "${FOD_REPLICA_COUNT}" ]; do
  slot="fod_replica_${index}"
  psql -v ON_ERROR_STOP=1 \
    --username "${POSTGRES_USER}" \
    --dbname "${POSTGRES_DB}" \
    --set=slot="${slot}" <<'SQL'
SELECT pg_create_physical_replication_slot(:'slot')
WHERE NOT EXISTS (SELECT 1 FROM pg_replication_slots WHERE slot_name = :'slot');
SQL
  index=$((index + 1))
done
