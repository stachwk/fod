FROM rust:1.85-bookworm AS libfuse-builder

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    meson \
    ninja-build \
    pkg-config \
 && rm -rf /var/lib/apt/lists/*

COPY docker/libfuse/build_libfuse3.sh /usr/local/src/build_libfuse3.sh
ARG FOD_LIBFUSE_VERSION=3.18.3
RUN FOD_LIBFUSE_VERSION="${FOD_LIBFUSE_VERSION}" sh /usr/local/src/build_libfuse3.sh

FROM rust:1.85-bookworm

ENV DEBIAN_FRONTEND=noninteractive
ENV PATH="/usr/local/cargo/bin:${PATH}"
ENV PKG_CONFIG_PATH="/usr/local/lib/pkgconfig"
ENV LD_LIBRARY_PATH="/usr/local/lib"

COPY --from=libfuse-builder /usr/local/ /usr/local/

RUN apt-get update && apt-get install -y --no-install-recommends \
    acl \
    attr \
    bash \
    build-essential \
    ca-certificates \
    curl \
    fuse3 \
    git \
    libfuse3-dev \
    libpq-dev \
    make \
    pkg-config \
    policycoreutils \
    python3 \
    python3-venv \
    selinux-utils \
    sudo \
 && rm -rf /var/lib/apt/lists/*

RUN ldconfig && test "$(PKG_CONFIG_PATH="${PKG_CONFIG_PATH}" pkg-config --modversion fuse3)" = "3.18.3"

RUN if [ -f /etc/fuse.conf ] && ! grep -q '^user_allow_other$' /etc/fuse.conf; then \
      printf '%s\n' user_allow_other >> /etc/fuse.conf; \
    fi

WORKDIR /workspace/fod

CMD ["sleep", "infinity"]
