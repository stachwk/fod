FROM rust:1.85-bookworm

ENV DEBIAN_FRONTEND=noninteractive

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

RUN if [ -f /etc/fuse.conf ] && ! grep -q '^user_allow_other$' /etc/fuse.conf; then \
      printf '%s\n' user_allow_other >> /etc/fuse.conf; \
    fi

WORKDIR /workspace/fod

CMD ["sleep", "infinity"]
