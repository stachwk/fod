#!/bin/sh
set -eu

case "${1:-}" in
    mount.fod|fod-rust-fuse|fod-bootstrap)
        # Mount/bootstrap paths may create or manage the FUSE mount. Validate
        # the host-provided container privileges first so failures are explicit.
        if [ "${1:-}" = "mount.fod" ] || [ "${1:-}" = "fod-rust-fuse" ] || [ "${FOD_CONTAINER_PREFLIGHT_BOOTSTRAP:-0}" = "1" ]; then
            fod-container-preflight --runtime
        fi
        ;;
esac

exec "$@"
