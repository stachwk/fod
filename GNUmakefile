# Canonical FOD builds use Rust 1.98.0 and the ThinLTO production profile.
# Command-line/environment overrides still win over these defaults.
FOD_CARGO_PROFILE ?= release-lto
FOD_RUST_PRODUCTION_TOOLCHAIN ?= 1.98.0

include Makefile

.PHONY: rust-production-toolchain-check test-rust-release-defaults-policy

rust-production-toolchain-check:
	@set -eu; \
	actual="$$(rustc --version)"; \
	expected="rustc $(FOD_RUST_PRODUCTION_TOOLCHAIN) "; \
	case "$$actual " in \
		"$$expected"*) ;; \
		*) \
			printf 'FOD production build requires Rust %s; active compiler: %s\n' "$(FOD_RUST_PRODUCTION_TOOLCHAIN)" "$$actual" >&2; \
			printf 'Install/select the repository toolchain from rust-toolchain.toml before building release artifacts.\n' >&2; \
			exit 1 ;; \
	esac

# A toolchain change must invalidate an old debug build stamp, otherwise make
# could reuse binaries produced by the previous compiler.
$(FOD_DEBUG_BUILD_STAMP): rust-toolchain.toml

# Release artifacts installed by FOD are required to use the pinned production
# compiler. The C ABI is unchanged; only the compiler/profile defaults change.
build-libfod install-root-scripts: rust-production-toolchain-check

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
	@rustup run "$(FOD_RUST_BASELINE_TOOLCHAIN)" cargo check --workspace --locked

rust-candidate-check:
	@rustup run "$(FOD_RUST_CANDIDATE_TOOLCHAIN)" cargo check --workspace --locked

rust-candidate-clippy:
	@rustup run "$(FOD_RUST_CANDIDATE_TOOLCHAIN)" cargo clippy --workspace --all-targets --locked

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
test-all: test-target-aux-clean-policy test-rust-toolchain-benchmark-policy test-rust-release-defaults-policy
