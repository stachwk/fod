ARG FOD_POSTGRES_IMAGE=ghcr.io/stachwk/postgres-16-fod-32k:16.15
FROM ${FOD_POSTGRES_IMAGE}

COPY primary-init.sh /docker-entrypoint-initdb.d/10-fod-replication.sh
COPY replica-entrypoint.sh /usr/local/bin/fod-replica-entrypoint.sh

RUN chmod 0755 \
    /docker-entrypoint-initdb.d/10-fod-replication.sh \
    /usr/local/bin/fod-replica-entrypoint.sh
