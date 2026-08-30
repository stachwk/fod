# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Benchmark-only PostgreSQL image. It rebuilds the same PostgreSQL major/version
# shipped by the selected official base image with a configurable table-page
# block size (BLCKSZ). The runtime remains based on the same official Alpine
# image so the 8 KiB and 32 KiB variants differ primarily in the compile-time
# block-size setting.

ARG POSTGRES_BASE_IMAGE=postgres:16-alpine
FROM ${POSTGRES_BASE_IMAGE} AS builder

ARG POSTGRES_BLOCK_SIZE_KB=32

RUN set -eux; \
    case "${POSTGRES_BLOCK_SIZE_KB}" in \
        1|2|4|8|16|32) ;; \
        *) echo "POSTGRES_BLOCK_SIZE_KB must be one of 1,2,4,8,16,32" >&2; exit 2 ;; \
    esac; \
    apk add --no-cache --virtual .pg-build-deps \
        build-base \
        linux-headers \
        perl \
        bison \
        flex \
        wget \
        ca-certificates; \
    pg_version="$(postgres --version | awk '{print $3}')"; \
    wget -O /tmp/postgresql.tar.gz \
        "https://ftp.postgresql.org/pub/source/v${pg_version}/postgresql-${pg_version}.tar.gz"; \
    mkdir -p /tmp/postgresql-src; \
    tar -xzf /tmp/postgresql.tar.gz --strip-components=1 -C /tmp/postgresql-src; \
    cd /tmp/postgresql-src; \
    ./configure \
        --prefix=/opt/postgresql-custom \
        --with-blocksize="${POSTGRES_BLOCK_SIZE_KB}" \
        --without-readline \
        --without-zlib; \
    jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)"; \
    make -j"${jobs}"; \
    make install; \
    make -C contrib/pg_stat_statements install; \
    install -d -o postgres -g postgres /tmp/pg-blocksize-verify; \
    su-exec postgres /opt/postgresql-custom/bin/initdb \
        -D /tmp/pg-blocksize-verify >/dev/null; \
    actual="$(su-exec postgres /opt/postgresql-custom/bin/postgres \
        -D /tmp/pg-blocksize-verify -C block_size)"; \
    expected="$((POSTGRES_BLOCK_SIZE_KB * 1024))"; \
    echo "PostgreSQL compile-time block_size=${actual} expected=${expected}"; \
    test "${actual}" = "${expected}"

ARG POSTGRES_BASE_IMAGE=postgres:16-alpine
FROM ${POSTGRES_BASE_IMAGE}

ARG POSTGRES_BLOCK_SIZE_KB=32

COPY --from=builder /opt/postgresql-custom /opt/postgresql-custom

ENV PATH="/opt/postgresql-custom/bin:${PATH}"
ENV LD_LIBRARY_PATH="/opt/postgresql-custom/lib"

COPY docker/replica-read/primary-init.sh /docker-entrypoint-initdb.d/10-fod-replication.sh
COPY docker/replica-read/replica-entrypoint.sh /usr/local/bin/fod-replica-entrypoint.sh

RUN chmod 0755 \
        /docker-entrypoint-initdb.d/10-fod-replication.sh \
        /usr/local/bin/fod-replica-entrypoint.sh \
    && postgres --version \
    && pg_config --configure | grep -F -- "--with-blocksize=${POSTGRES_BLOCK_SIZE_KB}" >/dev/null

LABEL org.fod.postgresql.block-size-kb="${POSTGRES_BLOCK_SIZE_KB}"
LABEL org.fod.postgresql.purpose="FOD benchmark-only custom BLCKSZ experiment"
