FROM postgres:16-alpine

COPY primary-init.sh /docker-entrypoint-initdb.d/10-fod-replication.sh
COPY replica-entrypoint.sh /usr/local/bin/fod-replica-entrypoint.sh

RUN chmod 0755 \
    /docker-entrypoint-initdb.d/10-fod-replication.sh \
    /usr/local/bin/fod-replica-entrypoint.sh
