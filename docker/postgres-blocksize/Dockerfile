# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# PostgreSQL image for FOD with a configurable compile-time table-page block
# size (BLCKSZ). The default base is pinned for reproducible published images;
# benchmark callers may override POSTGRES_BASE_IMAGE and POSTGRES_BLOCK_SIZE_KB.
#
# C extensions are always compiled against this custom PostgreSQL build. Do not
# install binary extension packages built against the standard 8 KiB PostgreSQL
# headers into this image.

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
        ca-certificates \
        readline-dev \
        zlib-dev \
        openssl-dev \
        libxml2-dev \
        libxslt-dev \
        lz4-dev \
        zstd-dev \
        util-linux-dev; \
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
        --with-openssl \
        --with-libxml \
        --with-libxslt \
        --with-lz4 \
        --with-zstd \
        --with-uuid=e2fs; \
    jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)"; \
    make -j"${jobs}"; \
    make install; \
    make -C contrib -j"${jobs}"; \
    make -C contrib install; \
    install -d -o postgres -g postgres /tmp/pg-blocksize-verify; \
    su-exec postgres /opt/postgresql-custom/bin/initdb \
        -D /tmp/pg-blocksize-verify >/dev/null; \
    actual="$(su-exec postgres /opt/postgresql-custom/bin/postgres \
        -D /tmp/pg-blocksize-verify -C block_size)"; \
    expected="$((POSTGRES_BLOCK_SIZE_KB * 1024))"; \
    echo "PostgreSQL compile-time block_size=${actual} expected=${expected}"; \
    test "${actual}" = "${expected}"; \
    for ext in pg_stat_statements pgcrypto hstore citext pg_trgm amcheck postgres_fdw uuid-ossp xml2; do \
        test -f "/opt/postgresql-custom/share/extension/${ext}.control"; \
    done

ENV PATH="/opt/postgresql-custom/bin:${PATH}"
ENV LD_LIBRARY_PATH="/opt/postgresql-custom/lib"

# Keep external extensions in separate layers. If a later extension fails after
# an upstream update, Docker can reuse the already compiled PostgreSQL/contrib
# and previous extension layers instead of rebuilding everything from scratch.
ARG PGVECTOR_VERSION=0.8.6
RUN set -eux; \
    mkdir -p /tmp/pgvector; \
    wget -qO /tmp/pgvector.tar.gz \
        "https://github.com/pgvector/pgvector/archive/refs/tags/v${PGVECTOR_VERSION}.tar.gz"; \
    tar -xzf /tmp/pgvector.tar.gz --strip-components=1 -C /tmp/pgvector; \
    make -C /tmp/pgvector -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)" PG_CONFIG=/opt/postgresql-custom/bin/pg_config; \
    make -C /tmp/pgvector PG_CONFIG=/opt/postgresql-custom/bin/pg_config install; \
    test -f /opt/postgresql-custom/share/extension/vector.control; \
    rm -rf /tmp/pgvector /tmp/pgvector.tar.gz

ARG PGAUDIT_VERSION=16.1
RUN set -eux; \
    mkdir -p /tmp/pgaudit; \
    wget -qO /tmp/pgaudit.tar.gz \
        "https://github.com/pgaudit/pgaudit/archive/refs/tags/${PGAUDIT_VERSION}.tar.gz"; \
    tar -xzf /tmp/pgaudit.tar.gz --strip-components=1 -C /tmp/pgaudit; \
    make -C /tmp/pgaudit USE_PGXS=1 PG_CONFIG=/opt/postgresql-custom/bin/pg_config install; \
    test -f /opt/postgresql-custom/share/extension/pgaudit.control; \
    rm -rf /tmp/pgaudit /tmp/pgaudit.tar.gz

ARG PG_CRON_VERSION=1.6.7
RUN set -eux; \
    mkdir -p /tmp/pg_cron; \
    wget -qO /tmp/pg_cron.tar.gz \
        "https://github.com/citusdata/pg_cron/archive/refs/tags/v${PG_CRON_VERSION}.tar.gz"; \
    tar -xzf /tmp/pg_cron.tar.gz --strip-components=1 -C /tmp/pg_cron; \
    make -C /tmp/pg_cron -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)" PG_CONFIG=/opt/postgresql-custom/bin/pg_config; \
    make -C /tmp/pg_cron PG_CONFIG=/opt/postgresql-custom/bin/pg_config install; \
    test -f /opt/postgresql-custom/share/extension/pg_cron.control; \
    rm -rf /tmp/pg_cron /tmp/pg_cron.tar.gz

ARG PG_REPACK_VERSION=1.5.3
RUN set -eux; \
    mkdir -p /tmp/pg_repack; \
    wget -qO /tmp/pg_repack.tar.gz \
        "https://github.com/reorg/pg_repack/archive/refs/tags/ver_${PG_REPACK_VERSION}.tar.gz"; \
    tar -xzf /tmp/pg_repack.tar.gz --strip-components=1 -C /tmp/pg_repack; \
    make -C /tmp/pg_repack -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)" PG_CONFIG=/opt/postgresql-custom/bin/pg_config; \
    make -C /tmp/pg_repack PG_CONFIG=/opt/postgresql-custom/bin/pg_config install; \
    test -f /opt/postgresql-custom/share/extension/pg_repack.control; \
    rm -rf /tmp/pg_repack /tmp/pg_repack.tar.gz

ARG HYPOPG_VERSION=1.4.3
RUN set -eux; \
    mkdir -p /tmp/hypopg; \
    wget -qO /tmp/hypopg.tar.gz \
        "https://github.com/HypoPG/hypopg/archive/refs/tags/${HYPOPG_VERSION}.tar.gz"; \
    tar -xzf /tmp/hypopg.tar.gz --strip-components=1 -C /tmp/hypopg; \
    make -C /tmp/hypopg -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)" PG_CONFIG=/opt/postgresql-custom/bin/pg_config; \
    make -C /tmp/hypopg PG_CONFIG=/opt/postgresql-custom/bin/pg_config install; \
    test -f /opt/postgresql-custom/share/extension/hypopg.control; \
    rm -rf /tmp/hypopg /tmp/hypopg.tar.gz

ARG PG_STAT_KCACHE_VERSION=2.3.2
RUN set -eux; \
    mkdir -p /tmp/pg_stat_kcache; \
    wget -qO /tmp/pg_stat_kcache.tar.gz \
        "https://github.com/powa-team/pg_stat_kcache/archive/refs/tags/REL2_3_2.tar.gz"; \
    tar -xzf /tmp/pg_stat_kcache.tar.gz --strip-components=1 -C /tmp/pg_stat_kcache; \
    make -C /tmp/pg_stat_kcache -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)" PG_CONFIG=/opt/postgresql-custom/bin/pg_config; \
    make -C /tmp/pg_stat_kcache PG_CONFIG=/opt/postgresql-custom/bin/pg_config install; \
    test -f /opt/postgresql-custom/share/extension/pg_stat_kcache.control; \
    rm -rf /tmp/pg_stat_kcache /tmp/pg_stat_kcache.tar.gz

ARG PG_HINT_PLAN_VERSION=1.6.2
RUN set -eux; \
    mkdir -p /tmp/pg_hint_plan; \
    wget -qO /tmp/pg_hint_plan.tar.gz \
        "https://github.com/ossc-db/pg_hint_plan/archive/refs/tags/REL16_1_6_2.tar.gz"; \
    tar -xzf /tmp/pg_hint_plan.tar.gz --strip-components=1 -C /tmp/pg_hint_plan; \
    make -C /tmp/pg_hint_plan -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)" PG_CONFIG=/opt/postgresql-custom/bin/pg_config; \
    make -C /tmp/pg_hint_plan PG_CONFIG=/opt/postgresql-custom/bin/pg_config install; \
    test -f /opt/postgresql-custom/share/extension/pg_hint_plan.control; \
    rm -rf /tmp/pg_hint_plan /tmp/pg_hint_plan.tar.gz

RUN set -eux; \
    mkdir -p /opt/postgresql-custom/share/fod; \
    for control in /opt/postgresql-custom/share/extension/*.control; do \
        basename "${control}" .control; \
    done | sort > /opt/postgresql-custom/share/fod/available-extensions.txt; \
    cat /opt/postgresql-custom/share/fod/available-extensions.txt; \
    count="$(wc -l < /opt/postgresql-custom/share/fod/available-extensions.txt)"; \
    echo "available_extension_count=${count}"; \
    test "${count}" -ge 45

ARG POSTGRES_BASE_IMAGE=postgres:16.15-alpine
FROM ${POSTGRES_BASE_IMAGE}

ARG POSTGRES_BLOCK_SIZE_KB=32
ARG FOD_IMAGE_SOURCE=https://github.com/stachwk/fod
ARG FOD_IMAGE_REVISION=unknown
ARG FOD_IMAGE_VERSION=16.15
ARG PGVECTOR_VERSION=0.8.6
ARG PGAUDIT_VERSION=16.1
ARG PG_CRON_VERSION=1.6.7
ARG PG_REPACK_VERSION=1.5.3
ARG HYPOPG_VERSION=1.4.3
ARG PG_STAT_KCACHE_VERSION=2.3.2
ARG PG_HINT_PLAN_VERSION=1.6.2

COPY --from=builder /opt/postgresql-custom /opt/postgresql-custom

ENV PATH="/opt/postgresql-custom/bin:${PATH}"
ENV LD_LIBRARY_PATH="/opt/postgresql-custom/lib"
ENV MUSL_LOCPATH="/usr/share/i18n/locales/musl"
ENV LANG="C.UTF-8"

COPY docker/replica-read/primary-init.sh /docker-entrypoint-initdb.d/10-fod-replication.sh
COPY docker/replica-read/replica-entrypoint.sh /usr/local/bin/fod-replica-entrypoint.sh

RUN apk add --no-cache \
        musl-locales \
        musl-locales-lang \
        icu-data-full \
        icu-libs \
        readline \
        zlib \
        openssl \
        libxml2 \
        libxslt \
        lz4-libs \
        zstd-libs \
        libuuid \
    && locale -a | sort -u >/tmp/fod-locales.txt \
    && cat /tmp/fod-locales.txt \
    && test "$(awk 'NF {n++} END {print n + 0}' /tmp/fod-locales.txt)" -ge 100 \
    && for loc in C.UTF-8 en_US en_GB de_DE fr_FR es_ES it_IT pt_BR ru_RU cs_CZ nl_NL sv_SE pl_PL hu_HU uk_UA ja_JP ko_KR zh_CN ar_SA; do \
        grep -Fx "${loc}" /tmp/fod-locales.txt >/dev/null; \
    done \
    && test "$(locale charmap)" = "UTF-8" \
    && icu_dir="$(mktemp -d)" \
    && chown postgres:postgres "${icu_dir}" \
    && su-exec postgres initdb --no-sync --locale-provider=icu --icu-locale=pl-PL -D "${icu_dir}" >/dev/null \
    && rm -rf "${icu_dir}" /tmp/fod-locales.txt \
    && chmod 0755 \
        /docker-entrypoint-initdb.d/10-fod-replication.sh \
        /usr/local/bin/fod-replica-entrypoint.sh \
    && postgres --version \
    && pg_config --configure | grep -F -- "--with-blocksize=${POSTGRES_BLOCK_SIZE_KB}" >/dev/null \
    && pg_config --configure | grep -F -- "--with-icu" >/dev/null \
    && pg_config --configure | grep -F -- "--with-openssl" >/dev/null \
    && pg_config --configure | grep -F -- "--with-libxml" >/dev/null \
    && pg_config --configure | grep -F -- "--with-libxslt" >/dev/null \
    && pg_config --configure | grep -F -- "--with-lz4" >/dev/null \
    && pg_config --configure | grep -F -- "--with-zstd" >/dev/null \
    && for ext in vector pgaudit pg_cron pg_repack hypopg pg_stat_kcache pg_hint_plan pg_stat_statements pgcrypto uuid-ossp xml2; do \
        test -f "/opt/postgresql-custom/share/extension/${ext}.control"; \
    done

LABEL org.opencontainers.image.title="PostgreSQL for FOD with 32 KiB pages"
LABEL org.opencontainers.image.description="PostgreSQL rebuilt for FOD with 32 KiB table pages, full PostgreSQL contrib, multilingual locale/ICU data and a curated external extension bundle"
LABEL org.opencontainers.image.source="${FOD_IMAGE_SOURCE}"
LABEL org.opencontainers.image.revision="${FOD_IMAGE_REVISION}"
LABEL org.opencontainers.image.version="${FOD_IMAGE_VERSION}"
LABEL org.opencontainers.image.licenses="BSL-1.1"
LABEL org.fod.postgresql.block-size-kb="${POSTGRES_BLOCK_SIZE_KB}"
LABEL org.fod.postgresql.purpose="FOD custom PostgreSQL BLCKSZ image"
LABEL org.fod.extensions.pgvector="${PGVECTOR_VERSION}"
LABEL org.fod.extensions.pgaudit="${PGAUDIT_VERSION}"
LABEL org.fod.extensions.pg-cron="${PG_CRON_VERSION}"
LABEL org.fod.extensions.pg-repack="${PG_REPACK_VERSION}"
LABEL org.fod.extensions.hypopg="${HYPOPG_VERSION}"
LABEL org.fod.extensions.pg-stat-kcache="${PG_STAT_KCACHE_VERSION}"
LABEL org.fod.extensions.pg-hint-plan="${PG_HINT_PLAN_VERSION}"
