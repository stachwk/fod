#!/bin/sh
set -eu

: "${PGDATA:=/var/lib/postgresql/data/pgdata}"
: "${PRIMARY_HOST:=primary}"
: "${PRIMARY_PORT:=5432}"
: "${FOD_REPLICATION_USER:=fod_repl}"
: "${FOD_REPLICATION_PASSWORD:?FOD_REPLICATION_PASSWORD is required}"
: "${FOD_REPLICATION_SLOT:?FOD_REPLICATION_SLOT is required}"
: "${FOD_REPLICATION_APPLICATION_NAME:=${FOD_REPLICATION_SLOT}}"

case "${FOD_REPLICATION_USER}:${FOD_REPLICATION_SLOT}:${FOD_REPLICATION_APPLICATION_NAME}" in
  *[!A-Za-z0-9_:.-]*)
    echo "replication identifiers contain unsupported characters" >&2
    exit 2
    ;;
esac

mkdir -p "${PGDATA}"

if [ ! -s "${PGDATA}/PG_VERSION" ]; then
  until PGPASSWORD="${FOD_REPLICATION_PASSWORD}" pg_isready \
    -h "${PRIMARY_HOST}" -p "${PRIMARY_PORT}" \
    -U "${FOD_REPLICATION_USER}" >/dev/null 2>&1; do
    sleep 1
  done

  find "${PGDATA}" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
  export PGPASSWORD="${FOD_REPLICATION_PASSWORD}"
  pg_basebackup \
    -h "${PRIMARY_HOST}" \
    -p "${PRIMARY_PORT}" \
    -U "${FOD_REPLICATION_USER}" \
    -D "${PGDATA}" \
    -Fp \
    -Xs \
    -P

  cat >> "${PGDATA}/postgresql.auto.conf" <<EOF
primary_conninfo = 'host=${PRIMARY_HOST} port=${PRIMARY_PORT} user=${FOD_REPLICATION_USER} password=${FOD_REPLICATION_PASSWORD} application_name=${FOD_REPLICATION_APPLICATION_NAME}'
primary_slot_name = '${FOD_REPLICATION_SLOT}'
EOF
  touch "${PGDATA}/standby.signal"
fi

chown -R postgres:postgres /var/lib/postgresql/data
chmod 700 "${PGDATA}"

exec /usr/local/bin/docker-entrypoint.sh postgres \
  -c listen_addresses='*' \
  -c hot_standby=on \
  -c hot_standby_feedback=off \
  -c shared_preload_libraries=pg_stat_statements
