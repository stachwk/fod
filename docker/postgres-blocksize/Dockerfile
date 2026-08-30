# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# PostgreSQL image for FOD with a configurable compile-time table-page block
# size (BLCKSZ). The default base is pinned for reproducible published images;
# benchmark callers may override POSTGRES_BASE_IMAGE and POSTGRES_BLOCK_SIZE_KB.

ARG POSTGRES_BASE_IMAGE=postgres:16.15-alpine
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
        pkgconf \
        icu-dev \
        musl-locales \
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
        --with-icu \
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

ARG POSTGRES_BASE_IMAGE=postgres:16.15-alpine
FROM ${POSTGRES_BASE_IMAGE}

ARG POSTGRES_BLOCK_SIZE_KB=32
ARG FOD_IMAGE_SOURCE=https://github.com/stachwk/fod
ARG FOD_IMAGE_REVISION=unknown
ARG FOD_IMAGE_VERSION=16.15

COPY --from=builder /opt/postgresql-custom /opt/postgresql-custom

ENV PATH="/opt/postgresql-custom/bin:${PATH}"
ENV LD_LIBRARY_PATH="/opt/postgresql-custom/lib"

COPY docker/replica-read/primary-init.sh /docker-entrypoint-initdb.d/10-fod-replication.sh
COPY docker/replica-read/replica-entrypoint.sh /usr/local/bin/fod-replica-entrypoint.sh

RUN apk add --no-cache musl-locales \
    && locale -a | grep -F 'en_US.UTF-8' >/dev/null \
    && chmod 0755 \
        /docker-entrypoint-initdb.d/10-fod-replication.sh \
        /usr/local/bin/fod-replica-entrypoint.sh \
    && postgres --version \
    && pg_config --configure | grep -F -- "--with-blocksize=${POSTGRES_BLOCK_SIZE_KB}" >/dev/null \
    && pg_config --configure | grep -F -- "--with-icu" >/dev/null

LABEL org.opencontainers.image.title="PostgreSQL for FOD with 32 KiB pages"
LABEL org.opencontainers.image.description="PostgreSQL rebuilt for FOD with configurable BLCKSZ; published FOD image uses 32 KiB table pages"
LABEL org.opencontainers.image.source="${FOD_IMAGE_SOURCE}"
LABEL org.opencontainers.image.revision="${FOD_IMAGE_REVISION}"
LABEL org.opencontainers.image.version="${FOD_IMAGE_VERSION}"
LABEL org.opencontainers.image.licenses="BSL-1.1"
LABEL org.fod.postgresql.block-size-kb="${POSTGRES_BLOCK_SIZE_KB}"
LABEL org.fod.postgresql.purpose="FOD custom PostgreSQL BLCKSZ image"
