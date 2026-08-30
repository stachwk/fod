#!/bin/sh
set -eu

: "${PGDATA:=/var/lib/postgresql/data/pgdata}"
: "${FOD_REPLICATION_USER:=fod_repl}"
: "${FOD_REPLICATION_PASSWORD:=fod_repl_pass}"

mkdir -p "$PGDATA"

if [ ! -s "$PGDATA/PG_VERSION" ]; then
    until pg_isready -h primary -p 5432 -U "$POSTGRES_USER" \
        -d "$POSTGRES_DB" >/dev/null 2>&1; do
        sleep 1
    done

    rm -rf "${PGDATA:?}/"*
    export PGPASSWORD="$FOD_REPLICATION_PASSWORD"

    pg_basebackup \
        -h primary \
        -p 5432 \
        -U "$FOD_REPLICATION_USER" \
        -D "$PGDATA" \
        -Fp \
        -Xs \
        -P

    cat >> "$PGDATA/postgresql.auto.conf" <<EOF
primary_conninfo = 'host=primary port=5432 user=${FOD_REPLICATION_USER} password=${FOD_REPLICATION_PASSWORD} application_name=fod_replica_read_test'
EOF
    touch "$PGDATA/standby.signal"
fi

chown -R postgres:postgres /var/lib/postgresql/data
chmod 700 "$PGDATA"

exec /usr/local/bin/docker-entrypoint.sh postgres \
    -c listen_addresses='*' \
    -c hot_standby=on \
    -c hot_standby_feedback=off
