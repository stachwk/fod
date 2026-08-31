#!/bin/sh
set -eu

mode="${1:---runtime}"

fail() {
    echo "FOD container preflight: ERROR: $*" >&2
    exit 2
}

warn() {
    echo "FOD container preflight: WARNING: $*" >&2
}

check_image() {
    for cmd in fod-bootstrap fod-rust-fuse mkfs.fod mount.fod psql pg_isready capsh; do
        command -v "$cmd" >/dev/null 2>&1 || fail "missing required command: $cmd"
    done

    if [ ! -r /etc/fuse.conf ]; then
        fail "/etc/fuse.conf is missing"
    fi
    grep -Fxq user_allow_other /etc/fuse.conf || fail "/etc/fuse.conf does not enable user_allow_other"
}

apparmor_profile() {
    if [ -r /proc/self/attr/current ]; then
        cat /proc/self/attr/current 2>/dev/null || true
    fi
}

check_runtime() {
    [ -e /dev/fuse ] || fail "/dev/fuse is not passed into the container; use --device /dev/fuse"
    [ -c /dev/fuse ] || fail "/dev/fuse exists but is not a character device"

    capsh --has-p=cap_sys_admin >/dev/null 2>&1 || \
        fail "CAP_SYS_ADMIN is missing; use --cap-add SYS_ADMIN"

    profile="$(apparmor_profile)"
    case "$profile" in
        ""|unconfined|unconfined*)
            ;;
        docker-default*)
            fail "AppArmor profile '$profile' blocks mount operations needed by FUSE; use --security-opt apparmor=unconfined or a host-loaded FOD-specific profile"
            ;;
        *)
            warn "custom AppArmor profile active: $profile; it must allow /dev/fuse, CAP_SYS_ADMIN and FUSE mount operations"
            ;;
    esac

    if [ -d /mnt/fod ] && command -v findmnt >/dev/null 2>&1; then
        propagation="$(findmnt -n -o PROPAGATION -T /mnt/fod 2>/dev/null || true)"
        case "$propagation" in
            shared|rshared|"") ;;
            *) warn "/mnt/fod propagation is '$propagation'; host-visible FUSE mounts normally require rshared propagation" ;;
        esac
    fi
}

case "$mode" in
    --image-only)
        check_image
        echo "OK: FOD client image prerequisites"
        ;;
    --runtime)
        check_image
        check_runtime
        echo "OK: FOD container FUSE/AppArmor prerequisites"
        ;;
    *)
        echo "Usage: fod-container-preflight [--image-only|--runtime]" >&2
        exit 2
        ;;
esac
