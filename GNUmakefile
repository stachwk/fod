include Makefile

.PHONY: docker-perf-clean test-docker-perf-clean-policy test-rust-release-defaults-policy

# Safe cleanup for Docker resources created by the isolated primary/replica
# performance benchmark. Build cache pruning is global to the Docker builder,
# so it requires an explicit opt-in.
FOD_DOCKER_PERF_CLEAN_FORCE ?= 0
FOD_DOCKER_PERF_PRUNE_BUILD_CACHE ?= 0

docker-perf-clean:
	@FOD_DOCKER_PERF_CLEAN_FORCE="$(FOD_DOCKER_PERF_CLEAN_FORCE)" \
		FOD_DOCKER_PERF_PRUNE_BUILD_CACHE="$(FOD_DOCKER_PERF_PRUNE_BUILD_CACHE)" \
		bash scripts/perf/clean_replica_read_docker.sh

test-docker-perf-clean-policy:
	@bash tests/test_docker_perf_clean_policy.sh

# Selective cleanup for known auxiliary Cargo targets. The allowlist is
# enforced by scripts/fod-aux-target-clean.sh; these variables only tune the
# policy for the selected auxiliary target.
FOD_TARGET_AUX_NAME ?= test-locking
FOD_TARGET_AUX_CLEAN_MIN_SIZE_BYTES ?= 1073741824
FOD_TARGET_AUX_CLEAN_MIN_AGE_DAYS ?= 7
FOD_TARGET_AUX_CLEAN_CONFIRM ?=
FOD_TARGET_AUX_CLEAN_FORCE ?= 0

.PHONY: target-aux-status target-aux-clean-plan target-aux-clean test-target-aux-clean-policy

target-aux-status:
	@FOD_TARGET_AUX_NAME="$(FOD_TARGET_AUX_NAME)" \
		FOD_TARGET_AUX_CLEAN_MIN_SIZE_BYTES="$(FOD_TARGET_AUX_CLEAN_MIN_SIZE_BYTES)" \
		FOD_TARGET_AUX_CLEAN_MIN_AGE_DAYS="$(FOD_TARGET_AUX_CLEAN_MIN_AGE_DAYS)" \
		FOD_TARGET_AUX_CLEAN_FORCE="$(FOD_TARGET_AUX_CLEAN_FORCE)" \
		RUST_CARGO="$(RUST_CARGO)" \
		bash scripts/fod-aux-target-clean.sh status

target-aux-clean-plan:
	@FOD_TARGET_AUX_NAME="$(FOD_TARGET_AUX_NAME)" \
		FOD_TARGET_AUX_CLEAN_MIN_SIZE_BYTES="$(FOD_TARGET_AUX_CLEAN_MIN_SIZE_BYTES)" \
		FOD_TARGET_AUX_CLEAN_MIN_AGE_DAYS="$(FOD_TARGET_AUX_CLEAN_MIN_AGE_DAYS)" \
		FOD_TARGET_AUX_CLEAN_FORCE="$(FOD_TARGET_AUX_CLEAN_FORCE)" \
		RUST_CARGO="$(RUST_CARGO)" \
		bash scripts/fod-aux-target-clean.sh plan

target-aux-clean:
	@FOD_TARGET_AUX_NAME="$(FOD_TARGET_AUX_NAME)" \
		FOD_TARGET_AUX_CLEAN_MIN_SIZE_BYTES="$(FOD_TARGET_AUX_CLEAN_MIN_SIZE_BYTES)" \
		FOD_TARGET_AUX_CLEAN_MIN_AGE_DAYS="$(FOD_TARGET_AUX_CLEAN_MIN_AGE_DAYS)" \
		FOD_TARGET_AUX_CLEAN_CONFIRM="$(FOD_TARGET_AUX_CLEAN_CONFIRM)" \
		FOD_TARGET_AUX_CLEAN_FORCE="$(FOD_TARGET_AUX_CLEAN_FORCE)" \
		RUST_CARGO="$(RUST_CARGO)" \
		bash scripts/fod-aux-target-clean.sh clean

test-target-aux-clean-policy:
	@bash tests/test_aux_target_clean_policy.sh

# Rust toolchain comparison. rust-version remains the compatibility floor;
# official production builds are pinned separately by rust-toolchain.toml.
FOD_RUST_MSRV ?= 1.85
FOD_RUST_BASELINE_TOOLCHAIN ?= 1.85.0
FOD_RUST_CANDIDATE_TOOLCHAIN ?= 1.98.0
FOD_RUST_TOOLCHAIN_BENCH_ROOT ?= $(CURDIR)/target/toolchain-benchmark
FOD_RUST_TOOLCHAIN_BENCH_REPETITIONS ?= 3
FOD_RUST_TOOLCHAIN_BENCH_BLOCK_SIZE ?= 512K
FOD_RUST_TOOLCHAIN_BENCH_COUNT ?= 64
FOD_RUST_TOOLCHAIN_BENCH_SOURCE ?= pattern

.PHONY: rust-msrv-check rust-candidate-check rust-candidate-clippy rust-toolchain-benchmark-plan rust-toolchain-benchmark test-rust-toolchain-benchmark-policy

rust-msrv-check:
	@rustup run "$(FOD_RUST_BASELINE_TOOLCHAIN)" cargo check --workspace --locked --profile "$(FOD_CARGO_PROFILE)"

rust-candidate-check:
	@rustup run "$(FOD_RUST_CANDIDATE_TOOLCHAIN)" cargo check --workspace --locked --profile "$(FOD_CARGO_PROFILE)"

rust-candidate-clippy:
	@rustup run "$(FOD_RUST_CANDIDATE_TOOLCHAIN)" cargo clippy --workspace --all-targets --locked --profile "$(FOD_CARGO_PROFILE)"

rust-toolchain-benchmark-plan:
	@FOD_RUST_MSRV="$(FOD_RUST_MSRV)" \
		FOD_RUST_BASELINE_TOOLCHAIN="$(FOD_RUST_BASELINE_TOOLCHAIN)" \
		FOD_RUST_CANDIDATE_TOOLCHAIN="$(FOD_RUST_CANDIDATE_TOOLCHAIN)" \
		FOD_RUST_TOOLCHAIN_BENCH_ROOT="$(FOD_RUST_TOOLCHAIN_BENCH_ROOT)" \
		FOD_RUST_TOOLCHAIN_BENCH_REPETITIONS="$(FOD_RUST_TOOLCHAIN_BENCH_REPETITIONS)" \
		FOD_RUST_TOOLCHAIN_BENCH_BLOCK_SIZE="$(FOD_RUST_TOOLCHAIN_BENCH_BLOCK_SIZE)" \
		FOD_RUST_TOOLCHAIN_BENCH_COUNT="$(FOD_RUST_TOOLCHAIN_BENCH_COUNT)" \
		FOD_RUST_TOOLCHAIN_BENCH_SOURCE="$(FOD_RUST_TOOLCHAIN_BENCH_SOURCE)" \
		bash scripts/fod-rust-toolchain-benchmark.sh plan

rust-toolchain-benchmark: venv up
	@POSTGRES_DB="$(POSTGRES_DB)" \
		POSTGRES_USER="$(POSTGRES_USER)" \
		POSTGRES_PASSWORD="$(POSTGRES_PASSWORD)" \
		FOD_SCHEMA_ADMIN_PASSWORD="$(FOD_SCHEMA_ADMIN_PASSWORD)" \
		FOD_RUST_MSRV="$(FOD_RUST_MSRV)" \
		FOD_RUST_BASELINE_TOOLCHAIN="$(FOD_RUST_BASELINE_TOOLCHAIN)" \
		FOD_RUST_CANDIDATE_TOOLCHAIN="$(FOD_RUST_CANDIDATE_TOOLCHAIN)" \
		FOD_RUST_TOOLCHAIN_BENCH_ROOT="$(FOD_RUST_TOOLCHAIN_BENCH_ROOT)" \
		FOD_RUST_TOOLCHAIN_BENCH_REPETITIONS="$(FOD_RUST_TOOLCHAIN_BENCH_REPETITIONS)" \
		FOD_RUST_TOOLCHAIN_BENCH_BLOCK_SIZE="$(FOD_RUST_TOOLCHAIN_BENCH_BLOCK_SIZE)" \
		FOD_RUST_TOOLCHAIN_BENCH_COUNT="$(FOD_RUST_TOOLCHAIN_BENCH_COUNT)" \
		FOD_RUST_TOOLCHAIN_BENCH_SOURCE="$(FOD_RUST_TOOLCHAIN_BENCH_SOURCE)" \
		bash scripts/fod-rust-toolchain-benchmark.sh run

test-rust-toolchain-benchmark-policy:
	@bash tests/test_rust_toolchain_benchmark_policy.sh

test-rust-release-defaults-policy:
	@bash tests/test_rust_release_defaults_policy.sh

# Keep lightweight policy regressions in the normal gate without modifying the
# existing Makefile target definition.
test-all: test-docker-perf-clean-policy test-target-aux-clean-policy test-rust-toolchain-benchmark-policy test-rust-release-defaults-policy

# Test-only FUSE cleanup. The helper is deliberately restricted to temporary
# rust_fuse test workspaces under /tmp/fod-rust-fuse-*/mount.
.PHONY: test-fuse-test-cleanup test-fuse-test-cleanup-policy

test-fuse-test-cleanup:
	@bash scripts/fod-test-fuse-cleanup.sh clean

test-fuse-test-cleanup-policy:
	@bash tests/test_fuse_test_cleanup_policy.sh

# A stale Rust FUSE test mount must be removed before a destructive local DB
# restore, otherwise the restore guard correctly refuses to continue.
test-db-restore-local: test-fuse-test-cleanup

# Run the policy in the normal gate, pre-clean test mounts at the start, and
# assert/clean again after all regular test-all prerequisites complete.
test-all: test-fuse-test-cleanup test-fuse-test-cleanup-policy
	@bash scripts/fod-test-fuse-cleanup.sh clean

# test-all-full adds more FUSE-backed tests after test-all, so verify cleanup
# once more at the end of the extended gate.
test-all-full:
	@bash scripts/fod-test-fuse-cleanup.sh clean

# Docker image targets use one naming convention:
# docker-<component>-<variant>-<action>.
# PostgreSQL 32K is the accepted FOD default/target; 8K remains explicit for
# compatibility and regression comparisons.
.PHONY: docker-postgres-8k-build docker-postgres-8k-publish docker-postgres-32k-build docker-postgres-32k-publish docker-postgres-all-build docker-postgres-all-publish docker-postgres-test-policy

docker-postgres-8k-build:
	@FOD_CONTAINER_PUSH=0 bash scripts/publish_postgres_fod_8k.sh

docker-postgres-8k-publish:
	@FOD_CONTAINER_PUSH=1 bash scripts/publish_postgres_fod_8k.sh

docker-postgres-32k-build:
	@FOD_CONTAINER_PUSH=0 bash scripts/publish_postgres_fod_32k.sh

docker-postgres-32k-publish:
	@FOD_CONTAINER_PUSH=1 bash scripts/publish_postgres_fod_32k.sh

docker-postgres-all-build: docker-postgres-8k-build docker-postgres-32k-build

docker-postgres-all-publish: docker-postgres-8k-publish docker-postgres-32k-publish

docker-postgres-test-policy:
	@bash tests/test_postgres_container_publish_targets_policy.sh
	@bash tests/test_postgres_32k_default_policy.sh

test-all: docker-postgres-test-policy

# Standalone FOD client image follows the same docker-<component>-<action>
# convention. The image includes FOD/FUSE plus PostgreSQL client diagnostics,
# but no PostgreSQL server binaries.
.PHONY: docker-fod-client-build docker-fod-client-publish docker-fod-client-test-policy

docker-fod-client-build:
	@FOD_CONTAINER_PUSH=0 bash scripts/publish_fod_client.sh

docker-fod-client-publish:
	@FOD_CONTAINER_PUSH=1 bash scripts/publish_fod_client.sh

docker-fod-client-test-policy:
	@bash tests/test_fod_client_container_policy.sh

test-all: docker-fod-client-test-policy

include packaging/fod-packaging.mk
