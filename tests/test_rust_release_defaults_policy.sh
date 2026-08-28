#!/usr/bin/env bash
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"

grep -Eq '^channel[[:space:]]*=[[:space:]]*"1\.98\.0"' "${repo_root}/rust-toolchain.toml"
grep -Eq '^profile[[:space:]]*=[[:space:]]*"minimal"' "${repo_root}/rust-toolchain.toml"
grep -Fq '"clippy"' "${repo_root}/rust-toolchain.toml"
grep -Fq '"rustfmt"' "${repo_root}/rust-toolchain.toml"

grep -Eq '^rust-version[[:space:]]*=[[:space:]]*"1\.85"' "${repo_root}/Cargo.toml"
grep -Eq '^FOD_CARGO_PROFILE[[:space:]]*\?=[[:space:]]*release-lto$' "${repo_root}/Makefile"
grep -Fq 'FOD_CARGO_TEST_PROFILE ?= $(FOD_CARGO_PROFILE)' "${repo_root}/Makefile"
grep -Fq 'FOD_TEST_FLAG := --profile $(FOD_CARGO_TEST_PROFILE)' "${repo_root}/Makefile"
grep -Fq 'FOD_RUNTIME_FLAG := --profile $(FOD_RUNTIME_PROFILE)' "${repo_root}/Makefile"
grep -Fq 'FOD_RUNTIME_ARTIFACT_PROFILE := $(if $(filter dev,$(FOD_RUNTIME_PROFILE)),debug,$(FOD_RUNTIME_PROFILE))' "${repo_root}/Makefile"
grep -Eq '^FOD_RUST_PRODUCTION_TOOLCHAIN[[:space:]]*\?=[[:space:]]*1\.98\.0$' "${repo_root}/Makefile"
grep -Fq 'build-runtime: rust-production-toolchain-check $(FOD_RUNTIME_BUILD_STAMP)' "${repo_root}/Makefile"
grep -Fq 'FOD_RUNTIME_BUILD_STAMP := $(RUST_MKFS_TARGET_DIR)/.fod-runtime-$(FOD_RUNTIME_PROFILE)-build.stamp' "${repo_root}/Makefile"
grep -Fq '$(CARGO_BUILD_MKFS) $(FOD_RUNTIME_FLAG) --bins' "${repo_root}/Makefile"
grep -Fq 'FOD_BOOTSTRAP_RUNTIME_BIN ?= $(RUST_MKFS_TARGET_DIR)/$(FOD_RUNTIME_ARTIFACT_PROFILE)/fod-bootstrap' "${repo_root}/Makefile"
grep -Fq '$(CARGO_BUILD_LIBFOD) $(FOD_RELEASE_FLAG) --lib' "${repo_root}/Makefile"
grep -Fq 'build-libfod install-root-scripts: rust-production-toolchain-check' "${repo_root}/Makefile"
grep -Fq '$(FOD_DEBUG_BUILD_STAMP): Makefile GNUmakefile rust-toolchain.toml' "${repo_root}/Makefile"
grep -Fq 'CARGO_TEST_FUSE := $(RUST_CARGO) test --manifest-path $(CARGO_ROOT_MANIFEST) $(FOD_TEST_FLAG) -p $(FOD_FUSE_PACKAGE)' "${repo_root}/Makefile"

grep -Fq 'target/release-lto/fod-rust-fuse' "${repo_root}/rust_mkfs/src/bin/fod-bootstrap.rs"
grep -Fq 'sibling_rust_fuse_binary' "${repo_root}/rust_mkfs/src/bin/fod-bootstrap.rs"
grep -Fq 'pub unsafe extern "C" fn fod_program_find' "${repo_root}/rust_libfod/src/lib.rs"
if grep -Fq '${project_root}/target/release/fod-bootstrap' "${repo_root}/mount.fod"; then
  echo "normal mount.fod checkout path must not fall back to target/release/fod-bootstrap" >&2
  exit 1
fi

bash -n "${repo_root}/mount.fod"

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/fod-release-defaults.XXXXXX")"
trap 'rm -rf "${tmpdir}"' EXIT
checkout="${tmpdir}/checkout"
mkdir -p "${checkout}/target/release-lto" "${checkout}/target/release" "${checkout}/target/debug"
cp "${repo_root}/mount.fod" "${checkout}/mount.fod"
chmod +x "${checkout}/mount.fod"
printf '[workspace]\n' >"${checkout}/Cargo.toml"
cp "${repo_root}/fod_version.txt" "${checkout}/fod_version.txt"
printf '[database]\nhost = 127.0.0.1\n' >"${checkout}/test.ini"

cat >"${checkout}/target/release-lto/fod-bootstrap" <<'EOF'
#!/usr/bin/env bash
printf 'BOOTSTRAP=%s\n' "$0"
printf 'FOD_RUST_FUSE_BIN=%s\n' "${FOD_RUST_FUSE_BIN:-unset}"
printf 'ARGS=%s\n' "$*"
EOF
chmod +x "${checkout}/target/release-lto/fod-bootstrap"

cat >"${checkout}/target/release-lto/fod-rust-fuse" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "${checkout}/target/release-lto/fod-rust-fuse"

# Stale alternatives must not win for a normal (non-debug) mount.
for profile in release debug; do
  cat >"${checkout}/target/${profile}/fod-bootstrap" <<'EOF'
#!/usr/bin/env bash
echo 'STALE_BOOTSTRAP_SELECTED' >&2
exit 97
EOF
  chmod +x "${checkout}/target/${profile}/fod-bootstrap"
done

output="$({
  cd "${checkout}"
  ./mount.fod none "${checkout}/mnt" -o "ini=${checkout}/test.ini,selinux=off,acl=off"
})"

grep -Fq "BOOTSTRAP=${checkout}/target/release-lto/fod-bootstrap" <<<"${output}"
grep -Fq "FOD_RUST_FUSE_BIN=${checkout}/target/release-lto/fod-rust-fuse" <<<"${output}"
if grep -Fq 'STALE_BOOTSTRAP_SELECTED' <<<"${output}"; then
  echo "mount wrapper selected a stale non-LTO bootstrap" >&2
  exit 1
fi

build_plan="$(
  cd "${repo_root}"
  make --no-print-directory -nB build-runtime FOD_CARGO_PROFILE=release FOD_RUNTIME_PROFILE=release-lto
)"

grep -Eq 'cargo build .*--profile release-lto .*--bins' <<<"${build_plan}"
grep -Eq 'cargo build .*--profile release-lto .*--bin fod-rust-fuse' <<<"${build_plan}"
grep -Eq 'cargo build .*--profile release-lto .*--lib' <<<"${build_plan}"
grep -Fq '.fod-runtime-release-lto-build.stamp' <<<"${build_plan}"
if grep -Eq 'cargo build .*--profile release .*--bins' <<<"${build_plan}"; then
  echo "build-runtime must not use FOD_CARGO_PROFILE when FOD_RUNTIME_PROFILE differs" >&2
  exit 1
fi

dev_profile="$(
  cd "${repo_root}"
  make --no-print-directory cargo-profile-show FOD_RUNTIME_PROFILE=dev
)"

grep -Fq 'FOD_RUNTIME_PROFILE=dev' <<<"${dev_profile}"
grep -Fq 'FOD_RUNTIME_FLAG=--profile dev' <<<"${dev_profile}"
grep -Fq 'FOD_RUNTIME_ARTIFACT_PROFILE=debug' <<<"${dev_profile}"
grep -Fq 'FOD_RUNTIME_BUILD_STAMP=target/.fod-runtime-dev-build.stamp' <<<"${dev_profile}"

dev_build_plan="$(
  cd "${repo_root}"
  make --no-print-directory -nB build-runtime FOD_RUNTIME_PROFILE=dev
)"

grep -Eq 'cargo build .*--profile dev .*--bins' <<<"${dev_build_plan}"
grep -Eq 'cargo build .*--profile dev .*--bin fod-rust-fuse' <<<"${dev_build_plan}"
grep -Eq 'cargo build .*--profile dev .*--lib' <<<"${dev_build_plan}"
grep -Fq '.fod-runtime-dev-build.stamp' <<<"${dev_build_plan}"
if grep -Eq 'cargo build .*--profile debug' <<<"${dev_build_plan}"; then
  echo "build-runtime must use Cargo profile dev, not debug" >&2
  exit 1
fi

runtime_bin_plan="$(
  cd "${repo_root}"
  make --no-print-directory -n mount FOD_RUNTIME_PROFILE=dev
)"
grep -Fq 'target/debug/fod-bootstrap' <<<"${runtime_bin_plan}"
if grep -Fq 'target/dev/fod-bootstrap' <<<"${runtime_bin_plan}"; then
  echo "runtime bin lookup must use target/debug for FOD_RUNTIME_PROFILE=dev" >&2
  exit 1
fi

echo "OK rust-release-defaults-policy"
