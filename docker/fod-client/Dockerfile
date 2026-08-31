# syntax=docker/dockerfile:1
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
#
# Runtime-only FOD client image. PostgreSQL server binaries are intentionally
# excluded; only the client-side libpq runtime required by FOD is installed.

ARG FOD_BUILDER_IMAGE=rust:1.98.0-bookworm
ARG FOD_RUNTIME_IMAGE=debian:bookworm-slim

FROM ${FOD_BUILDER_IMAGE} AS builder

ENV DEBIAN_FRONTEND=noninteractive
WORKDIR /src/fod

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential \
      ca-certificates \
      dpkg-dev \
      fuse3 \
      git \
      libfuse3-dev \
      libpq-dev \
      make \
      pkg-config \
 && rm -rf /var/lib/apt/lists/*

COPY . .

ARG FOD_CARGO_PROFILE=release-lto
RUN set -eux; \
    make \
      FOD_CARGO_PROFILE="${FOD_CARGO_PROFILE}" \
      FOD_PACKAGE_ROOT=/tmp/fod-packages \
      package-ubuntu; \
    package_path="$(find /tmp/fod-packages/deb -maxdepth 1 -type f -name 'fod_*.deb' -print -quit)"; \
    test -n "${package_path}"; \
    cp "${package_path}" /tmp/fod-client.deb

FROM ${FOD_RUNTIME_IMAGE}

ARG FOD_IMAGE_SOURCE=https://github.com/stachwk/fod
ARG FOD_IMAGE_REVISION=unknown
ARG FOD_IMAGE_VERSION=unknown

ENV DEBIAN_FRONTEND=noninteractive

COPY --from=builder /tmp/fod-client.deb /tmp/fod-client.deb

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates /tmp/fod-client.deb; \
    rm -f /tmp/fod-client.deb; \
    rm -rf /var/lib/apt/lists/*; \
    command -v fod-bootstrap; \
    command -v fod-change; \
    command -v fod-indexer; \
    command -v fod-monitor; \
    command -v fod-rust-fuse; \
    command -v mkfs.fod; \
    command -v mount.fod; \
    dpkg-query -W -f='${db:Status-Abbrev}\n' libpq5 | grep -Fx 'ii '; \
    command -v postgres >/dev/null 2>&1 && exit 1 || true; \
    command -v initdb >/dev/null 2>&1 && exit 1 || true; \
    command -v pg_ctl >/dev/null 2>&1 && exit 1 || true; \
    command -v psql >/dev/null 2>&1 && exit 1 || true; \
    for binary in \
      /usr/bin/fod-bootstrap \
      /usr/bin/fod-change \
      /usr/bin/fod-indexer \
      /usr/bin/fod-monitor \
      /usr/bin/fod-rust-fuse \
      /usr/sbin/mkfs.fod; do \
        ldd "${binary}" | tee /tmp/fod-ldd.txt; \
        if grep -Fq 'not found' /tmp/fod-ldd.txt; then exit 1; fi; \
    done; \
    rm -f /tmp/fod-ldd.txt; \
    if [ -f /etc/fuse.conf ] && ! grep -q '^user_allow_other$' /etc/fuse.conf; then \
      printf '%s\n' user_allow_other >> /etc/fuse.conf; \
    fi

WORKDIR /work
VOLUME ["/mnt/fod"]

LABEL org.opencontainers.image.title="FOD client"
LABEL org.opencontainers.image.description="FOD PostgreSQL-backed FUSE client with libpq runtime only; no PostgreSQL server"
LABEL org.opencontainers.image.source="${FOD_IMAGE_SOURCE}"
LABEL org.opencontainers.image.revision="${FOD_IMAGE_REVISION}"
LABEL org.opencontainers.image.version="${FOD_IMAGE_VERSION}"
LABEL org.opencontainers.image.licenses="BSL-1.1"
LABEL org.fod.container.role="client"
LABEL org.fod.postgresql.runtime="libpq-only"

CMD ["fod-bootstrap", "--help"]
