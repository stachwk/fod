PYTHON ?= python3
RUST_CARGO ?= cargo

# If the root Cargo.toml exists, use the workspace.
# If it does not, keep the legacy split-manifest mode.
CARGO_ROOT_MANIFEST ?= Cargo.toml

# Keep these package names aligned with Cargo.toml and CI.
# Legacy RUST_* overrides still work by feeding the canonical FOD_* names.
FOD_MKFS_PACKAGE ?= $(or $(RUST_MKFS_PACKAGE),fod-rust-mkfs)
FOD_FUSE_PACKAGE ?= $(or $(RUST_FUSE_PACKAGE),fod-rust-fuse)
FOD_HOTPATH_PACKAGE ?= $(or $(RUST_HOTPATH_PACKAGE),fod-rust-hotpath)
FOD_INDEXER_PACKAGE ?= $(or $(RUST_INDEXER_PACKAGE),fod-rust-indexer)
FOD_BOOTSTRAP_BIN ?= fod-bootstrap
FOD_MKFS_BIN ?= fod-rust-mkfs
FOD_CONFIG_BIN ?= fod-config
FOD_CHANGE_BIN ?= fod-change
FOD_FUSE_BIN ?= fod-rust-fuse
FOD_INDEXER_BIN ?= fod-indexer
FOD_VERSION_FILE ?= fod_version.txt
FOD_VERSION := $(shell cat $(FOD_VERSION_FILE))
FOD_CARGO_PROFILE ?= release
FOD_RELEASE_FLAG := --profile $(FOD_CARGO_PROFILE)

ifeq ($(wildcard $(CARGO_ROOT_MANIFEST)),)
CARGO_BUILD_MKFS := $(RUST_CARGO) build --manifest-path rust_mkfs/Cargo.toml
CARGO_BUILD_FUSE := $(RUST_CARGO) build --manifest-path rust_fuse/Cargo.toml
CARGO_BUILD_HOTPATH := $(RUST_CARGO) build --manifest-path rust_hotpath/Cargo.toml
CARGO_BUILD_INDEXER := $(RUST_CARGO) build --manifest-path rust_indexer/Cargo.toml

CARGO_RUN_MKFS := $(RUST_CARGO) run --manifest-path rust_mkfs/Cargo.toml
CARGO_RUN_INDEXER := $(RUST_CARGO) run --manifest-path rust_indexer/Cargo.toml

CARGO_TEST_MKFS := $(RUST_CARGO) test --manifest-path rust_mkfs/Cargo.toml
CARGO_TEST_FUSE := $(RUST_CARGO) test --manifest-path rust_fuse/Cargo.toml
CARGO_TEST_HOTPATH := $(RUST_CARGO) test --manifest-path rust_hotpath/Cargo.toml

RUST_MKFS_TARGET_DIR := rust_mkfs/target
RUST_FUSE_TARGET_DIR := rust_fuse/target
RUST_HOTPATH_TARGET_DIR := rust_hotpath/target
RUST_INDEXER_TARGET_DIR := rust_indexer/target
else
CARGO_BUILD_MKFS := $(RUST_CARGO) build --manifest-path $(CARGO_ROOT_MANIFEST) -p $(FOD_MKFS_PACKAGE)
CARGO_BUILD_FUSE := $(RUST_CARGO) build --manifest-path $(CARGO_ROOT_MANIFEST) -p $(FOD_FUSE_PACKAGE)
CARGO_BUILD_HOTPATH := $(RUST_CARGO) build --manifest-path $(CARGO_ROOT_MANIFEST) -p $(FOD_HOTPATH_PACKAGE)
CARGO_BUILD_INDEXER := $(RUST_CARGO) build --manifest-path $(CARGO_ROOT_MANIFEST) -p $(FOD_INDEXER_PACKAGE)
CARGO_BUILD_INSTALL_ROOT := $(RUST_CARGO) build --manifest-path $(CARGO_ROOT_MANIFEST) $(FOD_RELEASE_FLAG) -p $(FOD_MKFS_PACKAGE) --bins -p $(FOD_FUSE_PACKAGE) --bin $(FOD_FUSE_BIN) -p $(FOD_INDEXER_PACKAGE) --bin $(FOD_INDEXER_BIN)

CARGO_RUN_MKFS := $(RUST_CARGO) run --manifest-path $(CARGO_ROOT_MANIFEST) -p $(FOD_MKFS_PACKAGE)
CARGO_RUN_INDEXER := $(RUST_CARGO) run --manifest-path $(CARGO_ROOT_MANIFEST) -p $(FOD_INDEXER_PACKAGE)

CARGO_TEST_MKFS := $(RUST_CARGO) test --manifest-path $(CARGO_ROOT_MANIFEST) -p $(FOD_MKFS_PACKAGE)
CARGO_TEST_FUSE := $(RUST_CARGO) test --manifest-path $(CARGO_ROOT_MANIFEST) -p $(FOD_FUSE_PACKAGE)
CARGO_TEST_HOTPATH := $(RUST_CARGO) test --manifest-path $(CARGO_ROOT_MANIFEST) -p $(FOD_HOTPATH_PACKAGE)

RUST_MKFS_TARGET_DIR := target
RUST_FUSE_TARGET_DIR := target
RUST_HOTPATH_TARGET_DIR := target
RUST_INDEXER_TARGET_DIR := target
endif

FOD_BOOTSTRAP_DEBUG_BIN := $(RUST_MKFS_TARGET_DIR)/debug/fod-bootstrap
FOD_MKFS_DEBUG_BIN := $(RUST_MKFS_TARGET_DIR)/debug/fod-rust-mkfs
FOD_CONFIG_DEBUG_BIN := $(RUST_MKFS_TARGET_DIR)/debug/fod-config
FOD_CHANGE_DEBUG_BIN := $(RUST_MKFS_TARGET_DIR)/debug/fod-change
FOD_FUSE_DEBUG_BIN := $(RUST_FUSE_TARGET_DIR)/debug/fod-rust-fuse
FOD_INDEXER_DEBUG_BIN := $(RUST_INDEXER_TARGET_DIR)/debug/fod-indexer
FOD_DEBUG_BUILD_STAMP := target/.fod-debug-build.stamp
FOD_RUST_INPUT_ROOTS := Cargo.toml Cargo.lock fod_version.txt rust_mkfs rust_fuse rust_hotpath rust_runtime rust_indexer migrations
FOD_RUST_INPUTS := $(shell find $(FOD_RUST_INPUT_ROOTS) -type f \( -name '*.rs' -o -name 'Cargo.toml' -o -name 'Cargo.lock' -o -name '*.sql' -o -name '*.txt' \) 2>/dev/null)

FOD_BOOTSTRAP_PROFILE_BIN := $(RUST_MKFS_TARGET_DIR)/$(FOD_CARGO_PROFILE)/fod-bootstrap
FOD_MKFS_PROFILE_BIN := $(RUST_MKFS_TARGET_DIR)/$(FOD_CARGO_PROFILE)/fod-rust-mkfs
FOD_FUSE_PROFILE_BIN := $(RUST_FUSE_TARGET_DIR)/$(FOD_CARGO_PROFILE)/fod-rust-fuse
FOD_CHANGE_PROFILE_BIN := $(RUST_MKFS_TARGET_DIR)/$(FOD_CARGO_PROFILE)/fod-change
FOD_INDEXER_PROFILE_BIN := $(RUST_INDEXER_TARGET_DIR)/$(FOD_CARGO_PROFILE)/fod-indexer
FOD_HOTPATH_PROFILE_LIB := $(RUST_HOTPATH_TARGET_DIR)/$(FOD_CARGO_PROFILE)/libfod_rust_hotpath.so

ifeq ($(wildcard $(CARGO_ROOT_MANIFEST)),)
CARGO_BUILD_INSTALL_ROOT := $(CARGO_BUILD_MKFS) $(FOD_RELEASE_FLAG) --bins && $(CARGO_BUILD_FUSE) $(FOD_RELEASE_FLAG) --bin $(FOD_FUSE_BIN)
endif

build-debug: $(FOD_DEBUG_BUILD_STAMP)

$(FOD_DEBUG_BUILD_STAMP): Makefile $(FOD_RUST_INPUTS)
	$(CARGO_BUILD_MKFS) --bins
	$(CARGO_BUILD_FUSE) --bin $(FOD_FUSE_BIN)
	$(CARGO_BUILD_INDEXER) --bin $(FOD_INDEXER_BIN)
	mkdir -p $(dir $@)
	touch $@

.PHONY: build-debug

# Benchmark targets are run sequentially because they share the same local
# Docker/PostgreSQL state and often rebuild the same binaries.
BENCHMARK_TARGETS := \
	test-copy-dedupe-benchmark \
	test-atime-benchmark \
	test-read-cache-benchmark \
	test-throughput \
	test-throughput-sync \
	test-flush-release-profile \
	test-truncate-release-profile \
	test-large-copy-benchmark \
	test-large-file-multiblock-benchmark \
	test-remount-durability-benchmark \
	test-tree-scale \
	test-fio-sequential-io \
	test-fio-mixed-io \
	test-fio-random-mixed-io \
	test-rust-hotpath-copy-dedupe-benchmark \
	test-rust-hotpath-extent-poc-benchmark

POSTGRES_BENCHMARK_TARGETS := \
	test-postgresql-wal-pressure \
	test-postgresql-connection-churn

POSTGRES_BENCHMARK_CHECKPOINT_TARGETS := \
	test-postgresql-wal-pressure-checkpoint

PG_WAL_PRESSURE_COUNT ?= 128
POSTGRES_BENCHMARK_WAL_PRESET_MAX_WAL_SIZE ?= 8GB
POSTGRES_BENCHMARK_WAL_PRESET_CHECKPOINT_TIMEOUT ?= 15min
POSTGRES_BENCHMARK_WAL_PRESET_WAL_COMPRESSION ?= pglz
POSTGRES_BENCHMARK_REPEAT ?= 1
POSTGRES_BENCHMARK_PLANNER_PRESET_SHARED_BUFFERS ?= 512MB
POSTGRES_BENCHMARK_PLANNER_PRESET_RANDOM_PAGE_COST ?= 1.1
POSTGRES_BENCHMARK_PLANNER_PRESET_EFFECTIVE_CACHE_SIZE ?= 4GB
POSTGRES_BENCHMARK_PLANNER_PRESET_MAINTENANCE_WORK_MEM ?= 512MB
POSTGRES_BENCHMARK_PLANNER_PRESET_AUTOVACUUM_MAX_WORKERS ?= 3
POSTGRES_BENCHMARK_PLANNER_PRESET_AUTOVACUUM_WORK_MEM ?= 256MB
PROFILE_RUN_ID ?= $(shell date -u +%Y%m%dT%H%M%SZ)
PROFILE_HOST ?= $(shell hostname -s 2>/dev/null || hostname 2>/dev/null || echo unknown-host)
ARTIFACTS_DIR ?= artifacts/perf/$(shell git rev-parse --short HEAD 2>/dev/null || echo unknown)/$(PROFILE_HOST)-$(PROFILE_RUN_ID)
PROFILE_CAPTURE_LABEL ?=
PROFILE_CAPTURE_SUFFIX = $(if $(PROFILE_CAPTURE_LABEL),-$(PROFILE_CAPTURE_LABEL),)
PERF_FREQ ?= 99
PROFILE_SECONDS ?= 60
PROFILE_WORKLOAD ?= test-large-copy-benchmark
PROFILE_PID ?=
PROFILE_MAKE ?= make
PROFILE_SUDO ?= sudo -n

define RUN_POSTGRES_BENCHMARK_REPEAT
	@set -eu; \
	repeat="$(POSTGRES_BENCHMARK_REPEAT)"; \
	case "$$repeat" in \
		''|*[!0-9]*) \
			echo "POSTGRES_BENCHMARK_REPEAT must be a positive integer, got: $$repeat" >&2; \
			exit 1 ;; \
	esac; \
	if [ "$$repeat" -lt 1 ]; then \
		echo "POSTGRES_BENCHMARK_REPEAT must be at least 1, got: $$repeat" >&2; \
		exit 1; \
	fi; \
	i=1; \
	while [ "$$i" -le "$$repeat" ]; do \
		printf '%s\n' "PostgreSQL benchmark run $$i/$$repeat"; \
		$(MAKE) --no-print-directory $(1); \
		i=$$((i + 1)); \
	done
endef

define RUN_POSTGRES_BENCHMARKS
	@set -eu; \
	for target in $(1); do \
		$(MAKE) --no-print-directory $(2) QNAP=$(3) $$target; \
	done
endef

VENV_DIR ?= .venv
VENV_PYTHON := $(VENV_DIR)/bin/python
VENV_PIP := $(VENV_DIR)/bin/pip
VENV_STAMP := $(VENV_DIR)/.fod-venv.stamp
SYSTEM_SITE_PACKAGES := $(shell $(PYTHON) -c 'import site; print(":".join(site.getsitepackages()))')
COMPOSE ?= docker compose
COMPOSE_FILE ?= docker-compose.yml
SELINUX_ACL_COMPOSE_FILE ?= docker-compose.selinux-acl.yml
FOD_CONFIG_SOURCE ?= fod_config.ini
FOD_CONFIG_DEST ?= /etc/fod/fod_config.ini
CONTAINER_POSTGRES_NAME ?= fod-postgres
CONTAINER_POSTGRES_SELINUX_ACL_NAME ?= fod-postgres-selinux-acl
CONTAINER_FOD_SELINUX_ACL_NAME ?= fod-selinux-acl
POSTGRES_DB_BASE ?= foddbname
POSTGRES_USER_BASE ?= foduser
POSTGRES_PASSWORD_BASE ?= cichosza
POSTGRES_PORT_BASE ?= 5432
POSTGRES_SHARED_PRELOAD_LIBRARIES ?= pg_stat_statements
POSTGRES_SHARED_BUFFERS ?=
POSTGRES_MAX_CONNECTIONS ?=
POSTGRES_MAX_WAL_SIZE ?=
POSTGRES_CHECKPOINT_TIMEOUT ?=
POSTGRES_CHECKPOINT_COMPLETION_TARGET ?=
POSTGRES_WAL_COMPRESSION ?=
POSTGRES_RANDOM_PAGE_COST ?=
POSTGRES_EFFECTIVE_CACHE_SIZE ?=
POSTGRES_MAINTENANCE_WORK_MEM ?=
POSTGRES_AUTOVACUUM_MAX_WORKERS ?=
POSTGRES_AUTOVACUUM_WORK_MEM ?=
POSTGRES_SERVER_TUNING_ENV := POSTGRES_SHARED_PRELOAD_LIBRARIES=$(POSTGRES_SHARED_PRELOAD_LIBRARIES) POSTGRES_SHARED_BUFFERS=$(POSTGRES_SHARED_BUFFERS) POSTGRES_MAX_CONNECTIONS=$(POSTGRES_MAX_CONNECTIONS) POSTGRES_MAX_WAL_SIZE=$(POSTGRES_MAX_WAL_SIZE) POSTGRES_CHECKPOINT_TIMEOUT=$(POSTGRES_CHECKPOINT_TIMEOUT) POSTGRES_CHECKPOINT_COMPLETION_TARGET=$(POSTGRES_CHECKPOINT_COMPLETION_TARGET) POSTGRES_WAL_COMPRESSION=$(POSTGRES_WAL_COMPRESSION) POSTGRES_RANDOM_PAGE_COST=$(POSTGRES_RANDOM_PAGE_COST) POSTGRES_EFFECTIVE_CACHE_SIZE=$(POSTGRES_EFFECTIVE_CACHE_SIZE) POSTGRES_MAINTENANCE_WORK_MEM=$(POSTGRES_MAINTENANCE_WORK_MEM) POSTGRES_AUTOVACUUM_MAX_WORKERS=$(POSTGRES_AUTOVACUUM_MAX_WORKERS) POSTGRES_AUTOVACUUM_WORK_MEM=$(POSTGRES_AUTOVACUUM_WORK_MEM)
QNAP ?= 0
QNAP_ENABLED := $(filter 1 true yes on,$(QNAP))
QNAP_DOCKER_HOST ?= tcp://192.168.1.11:2376
QNAP_DOCKER_TLS_VERIFY ?= 1
QNAP_DOCKER_CERT_PATH ?= $(HOME)/.docker
QNAP_PG_HOST ?= 192.168.1.11
QNAP_PG_PORT ?= 5432
QNAP_PG_DBNAME ?= $(POSTGRES_DB_BASE)
QNAP_PG_USER ?= postgresql
QNAP_PG_PASSWORD ?= postgresqlfod
COMPOSE_RUN := $(if $(QNAP_ENABLED),DOCKER_HOST=$(QNAP_DOCKER_HOST) DOCKER_TLS_VERIFY=$(QNAP_DOCKER_TLS_VERIFY) DOCKER_CERT_PATH=$(QNAP_DOCKER_CERT_PATH),) $(POSTGRES_SERVER_TUNING_ENV) $(COMPOSE)
FOD_REMOTE_PG_HOST ?= $(QNAP_PG_HOST)
FOD_REMOTE_PG_PORT ?= $(QNAP_PG_PORT)
FOD_REMOTE_PG_DBNAME ?= $(QNAP_PG_DBNAME)
FOD_REMOTE_PG_USER ?= $(QNAP_PG_USER)
FOD_REMOTE_PG_PASSWORD ?= $(QNAP_PG_PASSWORD)
FOD_REMOTE_PG_ENV := FOD_PG_HOST=$(FOD_REMOTE_PG_HOST) FOD_PG_PORT=$(FOD_REMOTE_PG_PORT) FOD_PG_DBNAME=$(FOD_REMOTE_PG_DBNAME) FOD_PG_USER=$(FOD_REMOTE_PG_USER) FOD_PG_PASSWORD=$(FOD_REMOTE_PG_PASSWORD)
FOD_PG_HOST ?= $(if $(QNAP_ENABLED),$(QNAP_PG_HOST),127.0.0.1)
FOD_PG_PORT ?= $(if $(QNAP_ENABLED),$(QNAP_PG_PORT),$(POSTGRES_PORT))
FOD_PG_DBNAME ?= $(if $(QNAP_ENABLED),$(QNAP_PG_DBNAME),$(POSTGRES_DB))
FOD_PG_USER ?= $(if $(QNAP_ENABLED),$(QNAP_PG_USER),$(POSTGRES_USER))
FOD_PG_PASSWORD ?= $(if $(QNAP_ENABLED),$(QNAP_PG_PASSWORD),$(POSTGRES_PASSWORD))
POSTGRES_DB := $(if $(QNAP_ENABLED),$(QNAP_PG_DBNAME),$(POSTGRES_DB_BASE))
POSTGRES_USER := $(if $(QNAP_ENABLED),$(QNAP_PG_USER),$(POSTGRES_USER_BASE))
POSTGRES_PASSWORD := $(if $(QNAP_ENABLED),$(QNAP_PG_PASSWORD),$(POSTGRES_PASSWORD_BASE))
POSTGRES_PORT := $(if $(QNAP_ENABLED),$(QNAP_PG_PORT),$(POSTGRES_PORT_BASE))
export FOD_PG_HOST
export FOD_PG_PORT
export FOD_PG_DBNAME
export FOD_PG_USER
export FOD_PG_PASSWORD
PSQL ?= PGPASSWORD="$(FOD_PG_PASSWORD)" psql -v ON_ERROR_STOP=1 -h $(FOD_PG_HOST) -p $(FOD_PG_PORT) -U $(FOD_PG_USER) -d $(FOD_PG_DBNAME)
MOUNTPOINT ?= /tmp/fod-mount
FOD_SELINUX ?= auto
FOD_DEFAULT_PERMISSIONS ?= 1
FOD_ATIME_POLICY ?= default
FOD_ROLE ?= auto
FOD_PROFILE ?=
ADMP_TRACE_INI ?= admpanch_trace.fod.local.ini
ADMP_TRACE_TARGET ?= test-fio-sequential-io-strace
ADMP_TRACE_INI_ABS := $(abspath $(ADMP_TRACE_INI))
ADMP_TRACE_ENV ?=
export ADMP_TRACE_ENV
FOD_CHANGE_CONFIG_PATH ?= $(FOD_CONFIG_SOURCE)
FOD_SCHEMA_ADMIN_PASSWORD_FILE ?= .fod/schema-admin-password
FOD_CHANGE_KEY ?=
FOD_CHANGE_VALUE ?=
FOD_CHANGE_PASSWORD ?=
FOD_LOG_LEVEL ?= INFO
FOD_ACL ?= off
ifndef FOD_SCHEMA_ADMIN_PASSWORD
FOD_SCHEMA_ADMIN_PASSWORD := $(shell $(PYTHON) -c 'import secrets; print("fod-" + secrets.token_urlsafe(24))')
endif
export FOD_SCHEMA_ADMIN_PASSWORD
FOD_SELINUX_CONTEXT ?=
FOD_SELINUX_FSCONTEXT ?=
FOD_SELINUX_DEFCONTEXT ?=
FOD_SELINUX_ROOTCONTEXT ?=
FOD_LAZYTIME ?= 0
FOD_SYNC ?= 0
FOD_DIRSYNC ?= 0
export CONTAINER_POSTGRES_NAME
export CONTAINER_POSTGRES_SELINUX_ACL_NAME
export CONTAINER_FOD_SELINUX_ACL_NAME
MOUNT_HELPER_DEST ?= /usr/local/sbin/mount.fod
STRIP ?= strip
STRIP_FLAGS ?= --strip-unneeded
UBUNTU_BUILD_DEPS := cargo rustc build-essential pkg-config libpq-dev libfuse3-dev python3 openssl
UBUNTU_LEGACY_PYTHON_DEPS := python3-venv python3-pip
REDHAT_BUILD_DEPS := cargo rustc gcc make pkgconf-pkg-config libpq-devel fuse3-devel python3 openssl
REDHAT_LEGACY_PYTHON_DEPS := python3-pip

.PHONY: help benchmark benchmarks postgres-benchmarks postgres-benchmarks-local postgres-benchmarks-qnap postgres-benchmarks-checkpoint postgres-benchmarks-compare postgres-benchmarks-wal-preset postgres-benchmarks-planner-preset venv deps deps-ubuntu deps-redhat up down restart logs wait init init-qnap reset smoke enable-pg-stat-statements mount mount-qnap mount-user demo unmount db-shell cargo-profile-show reload-runtime change-runtime change-runtime-list change-runtime-get change-runtime-set install-config install-config-user install-mount-helper install-root-scripts install-rust-hotpath install-on-root install-on-root-venv pip-build pip-install pip-install-editable config-show postgres-config-show qnap-config-show qnap-config-show-inner qnap-up qnap-down qnap-restart qnap-logs qnap-wait qnap-init qnap-smoke qnap-reset qnap-mount warn-config-secret docker-selinux-acl-up docker-selinux-acl-wait docker-selinux-acl-down docker-selinux-acl-shell docker-selinux-acl-smoke test-integration test-xattr test-df test-locking test-pg-lock-manager test-permissions test-journal test-destroy test-dirhooks test-hardlink test-fallocate test-copy-file-range test-copy-dedupe-benchmark test-copy-block-crc-table test-worker-thresholds-block-size test-rust-hotpath-copy-plan test-rust-hotpath-crc32 test-rust-hotpath-read-ahead test-rust-hotpath-read-sequence test-rust-hotpath-read-fetch-bounds test-rust-hotpath-read-slice-plan test-rust-hotpath-read-missing-range-worker-count test-rust-hotpath-block-count test-rust-hotpath-dirty-block-size test-rust-hotpath-logical-resize-plan test-rust-hotpath-persist-layout-plan test-rust-hotpath-persist-block-plan test-rust-hotpath-persist-block-crc-plan test-rust-hotpath-write-copy-worker-count test-rust-hotpath-parallel-worker-count test-rust-hotpath-missing-ranges test-rust-hotpath-copy-dedupe test-rust-hotpath-copy-dedupe-benchmark test-rust-hotpath-extent-poc-benchmark test-rust-hotpath-copy-pack test-rust-hotpath-persist-pad test-rust-hotpath-read-assemble test-rust-pg-query test-rust-hotpath-runtime-size-limits test-ioctl test-mknod test-lseek test-poll test-access-groups test-inode-model test-ownership-inheritance test-rename-root-conflict test-statfs-use-ino test-mount-workflow test-mount-root-permissions test-mount-wrapper-options test-fuse-context-identity test-files test-directories test-metadata test-symlink test-pool-connections test-postgresql-requirements test-postgresql-requirements-autocommit-off test-postgresql-requirements-autocommit-on test-runtime-profile test-runtime-reload test-metadata-cache test-truncate-shrink-block-boundary test-mount-suite test-fio-sequential-io test-fio-sequential-io-strace test-admpanch-trace test-fio-mixed-io test-fio-random-mixed-io test-atime-noatime test-atime-relatime test-atime-benchmark test-timestamp-touch-once test-read-ahead-sequence test-read-cache-benchmark test-workers-read-parallel test-workers-write-parallel-copy test-runtime-config test-runtime-validation test-schema-upgrade test-schema-status test-throughput test-throughput-sync test-large-copy-benchmark test-large-file-multiblock-benchmark test-remount-durability-benchmark test-tree-scale test-flush-release-profile test-truncate-release-profile test-persist-buffer-chunking test-write-flush-threshold test-utimens-noop test-write-noop test-unlink-after-write test-local-vs-fod-permissions test-ext4-vs-fod-permissions test-root-owned-permissions test-allow-other-visibility test-multi-open-unique-handles test-version test-block-read test-connection-recovery test-postgresql-wal-pressure test-postgresql-wal-pressure-checkpoint test-postgresql-connection-churn test-all test-all-full clean test-rust-hotpath-helper-parity test-rust-hotpath-block-transfer-plan test-rust-hotpath-write-copy-plan test-mkfs-pg-tls test-mkfs-config-suite test-rust-mkfs-suite test-fod-indexer-parallel-smoke

help:
	@printf '%s\n' \
		'Targets:' \
	'  make cargo-profile-show - print the active Cargo build profile used by Makefile install targets' \
		'  make qnap-config-show - print the resolved QNAP Docker, PostgreSQL endpoint, and server tuning preset' \
		'  make postgres-config-show - print the resolved PostgreSQL server tuning preset' \
		'  make change-runtime-list - show the effective live reloadable snapshot via fod.change' \
		'  make change-runtime-get - print one live reloadable key via fod.change (set FOD_CHANGE_KEY=...)' \
		'  make reload-runtime - apply reloadable FOD_* values from the current config via fod.change (no remount needed)' \
		'  make change-runtime-set - persist one live reloadable key via fod.change (set FOD_CHANGE_KEY, FOD_CHANGE_VALUE, and FOD_CHANGE_PASSWORD)' \
		'  make venv       - create .venv for legacy Python integration tests' \
		'  make deps       - refresh the legacy Python test dependencies in .venv' \
		'  make deps-ubuntu - print the Ubuntu/Debian packages needed to build FOD' \
		'  make deps-redhat - print the Fedora/RHEL packages needed to build FOD' \
		'  make up         - start local PostgreSQL in Docker' \
		'  make qnap-up    - start PostgreSQL in Docker using QNAP=1' \
		'  make docker-selinux-acl-up - start the SELinux/ACL test lab in Docker' \
		'  make docker-selinux-acl-wait - wait until the SELinux/ACL lab PostgreSQL is ready' \
		'  make down       - stop local PostgreSQL' \
		'  make qnap-down  - stop PostgreSQL using QNAP=1' \
		'  make docker-selinux-acl-down - stop the SELinux/ACL test lab' \
	'  make restart    - restart local PostgreSQL' \
		'  make qnap-restart - restart PostgreSQL using QNAP=1' \
		'  make logs       - show local PostgreSQL logs' \
		'  make qnap-logs  - show PostgreSQL logs using QNAP=1' \
		'  make wait       - wait until PostgreSQL is ready' \
		'  make qnap-wait  - wait until PostgreSQL is ready using QNAP=1' \
		'  make init       - create the FOD schema in local PostgreSQL with --schema-admin-password' \
		'  make qnap-init  - create the FOD schema using the QNAP transport preset' \
		'  make init-qnap  - create the FOD schema using the remote QNAP PostgreSQL preset' \
		'  make qnap-smoke - run the PostgreSQL smoke check using QNAP=1' \
		'  make reset      - down -v / up / wait / init for a clean start' \
		'  make qnap-reset - run reset using QNAP=1' \
		'  make enable-pg-stat-statements - create pg_stat_statements in the local PostgreSQL database for diagnostics' \
		'  make install-config - install fod_config.ini to /etc/fod/fod_config.ini (warns if password is still cichosza)' \
		'  make install-config-user - install fod_config.ini to $$HOME/.config/fod/fod_config.ini without sudo (warns if password is still cichosza)' \
		'  make test-config-warning - verify the install-config password warning behavior' \
		'  make install-mount-helper - install mount.fod to $(MOUNT_HELPER_DEST)' \
	'  make install-root-scripts - install fod-bootstrap, mkfs.fod, fod-change/fod.change, and fod-rust-fuse Rust binaries to /usr/local/bin (use FOD_CARGO_PROFILE=release-lto for final builds)' \
	'  make install-rust-hotpath - build and install the Rust hot-path shared library (respects FOD_CARGO_PROFILE)' \
		'  make install-on-root - install system config, Rust binaries, mount helper, and Rust hot-path artifacts' \
		'  make install-on-root-venv - create .venv for legacy tests, then run the full root-style install' \
		'  make pip-build - removed; Rust binaries are built directly' \
		'  make pip-install - removed; Rust binaries are built directly' \
		'  make pip-install-editable - legacy Python test helper install' \
		'  make config-show - show which file FOD uses for configuration' \
		'  make indexer - run fod-indexer with INDEXER_ARGS="..."' \
		'  make indexer-import - materialize a source into FOD (set INDEXER_SOURCE=...)' \
		'  make test-fod-indexer-smoke - smoke the fod-indexer materialize pipeline' \
		'  make test-fod-indexer-materialize - alias for make test-fod-indexer-smoke' \
		'  make test-fod-indexer-materialize-rollback - smoke automatic rollback for failed materialize' \
		'  make test-fod-indexer-usability - smoke help, browse, progress, dry-run, and clean UX' \
		'  make test-fod-indexer-json-output - smoke JSON output and snapshot exports for fod-indexer' \
		'  make test-fod-indexer-plan-import-scope - smoke the fod-indexer plan-import source scoping' \
		'  make test-fod-indexer-cleanup-failed - smoke cleanup for failed fod-indexer materialization' \
		'  make test-fod-indexer-parallel-smoke - run selected fod-indexer smokes concurrently' \
		'  make smoke      - quick database connectivity test' \
		'  make benchmarks - run the benchmark suite sequentially' \
		'  make benchmark  - alias for make benchmarks' \
		'  make postgres-benchmarks - run PostgreSQL optimization benchmark targets sequentially on the selected backend' \
		'  make postgres-benchmarks-local - run PostgreSQL optimization benchmarks on local Docker' \
		'  make postgres-benchmarks-qnap - run PostgreSQL optimization benchmarks on QNAP' \
		'  make postgres-benchmarks-checkpoint - run the checkpoint-forcing PostgreSQL WAL benchmark on the selected backend' \
		'  make postgres-benchmarks-wal-preset - run the WAL/checkpoint benchmark preset across local Docker and QNAP; set POSTGRES_BENCHMARK_REPEAT=N to repeat the full preset' \
		'  make postgres-benchmarks-planner-preset - run the planner/autovacuum benchmark preset across local Docker and QNAP' \
		'  make postgres-benchmarks-compare - run the PostgreSQL optimization benchmarks on local Docker and QNAP' \
		'  make profile-env - capture local environment fingerprint under artifacts/perf/<commit>' \
		'  make profile-local-baseline - run PROFILE_WORKLOAD with pg_stat capture before/after' \
		'  make profile-perf-stat - run perf stat around PROFILE_WORKLOAD' \
		'  make profile-perf-record - record perf samples around PROFILE_WORKLOAD' \
		'  make profile-sudo-perf-stat-system - run system-wide sudo perf while PROFILE_WORKLOAD runs as the current user' \
		'  make profile-sudo-bpftrace-syscalls-workload - run sudo bpftrace syscall sampling while PROFILE_WORKLOAD runs as the current user' \
		'  make profile-pg-data-blocks-merge-explain - capture temp-table EXPLAIN for the current data_blocks merge shape' \
		'  make profile-pg-data-blocks-bloat - capture real data_blocks table/index size and churn diagnostics' \
		'  make profile-fuse-attach PROFILE_PID=<pid> - attach perf to a running fod-rust-fuse process' \
		'  make mount      - mount FOD at $(MOUNTPOINT)' \
		'  make qnap-mount - mount FOD at $(MOUNTPOINT) using QNAP=1' \
		'  make mount-qnap - mount using the remote QNAP PostgreSQL preset (no local Docker)' \
	'  make mount-user - prefer $$HOME/.config/fod/fod_config.ini and fall back to local ./fod_config.ini' \
		'  make demo       - up/init and then mount FOD at $(MOUNTPOINT)' \
	'  make docker-selinux-acl-shell - enter the SELinux/ACL Docker lab container' \
	'  make docker-selinux-acl-smoke - run the SELinux/ACL lab smoke checks inside the Docker container' \
		'  make unmount    - unmount FOD from $(MOUNTPOINT)' \
		'  make test-integration - run mkdir/create/write/read tests against local PostgreSQL' \
		'  make test-role-autodetect - verify runtime role and lock autodetection logic' \
		'  make test-postgresql-requirements - alias for autocommit=off PostgreSQL requirements smoke' \
		'  make test-postgresql-requirements-autocommit-off - verify PostgreSQL version, time zone, connection budget, and autocommit=off' \
		'  make test-postgresql-requirements-autocommit-on - verify PostgreSQL version, time zone, connection budget, and autocommit=on' \
		'  make test-xattr - run xattr/SELinux backend tests' \
		'  make test-df   - verify df -Ph and df -Phi on a mounted FOD' \
		'  make test-locking - verify FOD lock backends and replica behavior' \
		'  make test-pg-lock-manager - verify PostgreSQL-backed flock and range leases in Rust' \
		'  make test-permissions - verify sticky bit and chown permission semantics' \
		'  make test-journal - verify journal entries for mutating operations' \
		'  make test-destroy - verify the destroy cleanup hook' \
		'  make test-dirhooks - verify opendir/releasedir/fsyncdir on a directory' \
		'  make test-hardlink - verify hardlinks through the FOD backend' \
		'  make test-fallocate - verify fallocate through the FOD backend' \
		'  make test-copy-file-range - verify copy_file_range through the FOD backend' \
		'  make test-copy-dedupe-benchmark - benchmark repeated copy dedupe in Rust hotpath' \
		'  make test-copy-block-crc-table - verify CRC cache population for unchanged-block dedupe' \
		'  make test-worker-thresholds-block-size - verify worker thresholds against block-sized transfers' \
		'  make test-ioctl - verify ioctl/FIONREAD through the FOD backend' \
		'  make test-mknod - verify FIFO mknod through the FOD backend' \
		'  make test-lseek - verify backend lseek through the FOD backend' \
		'  make test-poll - verify backend poll through the FOD backend' \
		'  make test-utimens-noop - verify utimens same-timestamp no-op behavior' \
		'  make test-write-noop - verify zero-length write no-op behavior' \
		'  make test-unlink-after-write - verify unlink after a flushed write' \
		'  make test-local-vs-fod-permissions - compare local filesystem and FOD permission behavior' \
		'  make test-root-owned-permissions - compare root-owned file handling on ext4 and FOD' \
		'  make test-allow-other-visibility - verify allow_other visibility between users (host-dependent skip if not exposed)' \
		'  make test-multi-open-unique-handles - verify independent fh values for concurrent opens' \
		'  make test-version - verify the published FOD version string from Rust' \
		'  make test-access-groups - verify access() for owner, primary group, and supplementary groups' \
		'  make test-inode-model - verify a stable inode model after FS restart' \
		'  make test-ownership-inheritance - verify gid inheritance after parent chmod/chown' \
		'  make test-rename-root-conflict - verify rename replace semantics and edge cases' \
		'  make test-statfs-use-ino - verify statfs and use_ino behavior on a mount' \
		'  make test-atime-noatime - smoke test for FOD atime behavior (noatime)' \
		'  make test-atime-relatime - smoke test for FOD atime behavior (relatime)' \
		'  make test-atime-benchmark - benchmark FOD atime behavior (file and directory reads)' \
		'  make test-timestamp-touch-once - relatime-style one-touch-at-a-time timestamp regression' \
		'  make test-read-ahead-sequence - regression for sequential read-ahead cache behavior' \
		'  make test-read-cache-benchmark - benchmark FOD block cache size under sequential reads' \
		'  make test-workers-read-parallel - verify workers_read only parallelize disjoint read gaps' \
		'  make test-workers-write-parallel-copy - verify small copy stays sequential and large copy threads' \
		'  make test-runtime-config - verify fod_config.ini runtime tuning values in Rust' \
		'  make test-runtime-validation - verify runtime config rejects invalid values in Rust' \
		'  make test-runtime-profile - verify named runtime profiles against fod_config.ini' \
		'  make test-runtime-reload - verify live reload accepts safe knobs and rejects mount-only ones' \
		'  make test-runtime-profile-extents - verify named runtime profiles with the extents preset' \
		'  make change-runtime - alias for make change-runtime-set' \
		'  make change-runtime-sync - alias for make reload-runtime' \
		'  make test-mkfs-pg-tls - verify PostgreSQL TLS path resolution and generated client pair handling' \
		'  make test-schema-upgrade - verify schema version reporting for upgrade flow' \
		'  make test-files - files: create/write/truncate/rename/unlink' \
		'  make test-block-read - range reads, block cache, and read-ahead' \
		'  make test-truncate-shrink-block-boundary - verify truncate shrink/extend boundaries stay zero-filled' \
		'  make test-directories - directories: mkdir/rmdir/rename/stat/ls' \
		'  make test-metadata - metadata: stat/chmod/chown/access' \
		'  make test-mount-workflow - mount + dd + stat + ls + rename + chown + chmod + access' \
		'  make test-mount-root-permissions - fresh mount + directory chmod/chown/write smoke' \
		'  make test-mount-wrapper-options - verify mount.fod wrapper option parsing and PATH/ro handling' \
		'  make test-fuse-context-identity - verify FUSE uid/gid context handling' \
		'  make test-symlink - mount + ln -s + readlink + rename symlink + orphaned symlink ls on the symlink path' \
		'  make test-throughput - benchmark FOD writes with dd if=/dev/zero' \
		'  make test-throughput-sync - benchmark FOD writes with conv=fsync' \
		'  make test-postgresql-wal-pressure - benchmark WAL pressure during mounted write bursts' \
		'  make test-postgresql-wal-pressure-checkpoint - benchmark WAL pressure with a forced CHECKPOINT' \
		'  make test-postgresql-connection-churn - benchmark repeated short PostgreSQL connections' \
		'  make test-rust-hotpath-helper-parity - run the shared Rust hot-path helper parity test suite once' \
		'  make test-rust-hotpath-copy-plan - Rust helper parity tests for copy planner and related helpers' \
		'  make test-rust-hotpath-copy-dedupe - Rust helper parity tests for changed-copy dedupe' \
		'  make test-rust-hotpath-copy-dedupe-benchmark - benchmark repeated copy dedupe in Rust hotpath' \
		'  make test-rust-hotpath-extent-poc-benchmark - benchmark the sequential-only extent PoC planner' \
		'  make test-rust-hotpath-copy-pack - Rust helper parity tests for changed-run packing' \
		'  make test-rust-hotpath-persist-pad - Rust helper parity tests for block padding' \
		'  make test-rust-hotpath-read-assemble - Rust helper parity tests for read assembly' \
		'  make test-rust-pg-query - verify PostgreSQL query paths and metadata helpers through Rust' \
		'  make test-rust-hotpath-runtime-size-limits - verify config size parsing and PG-visible fs cap in Rust' \
		'  make test-rust-hotpath-read-ahead - Rust helper parity tests for read-ahead formulas' \
		'  make test-rust-hotpath-read-sequence - Rust helper parity tests for read-sequence helpers' \
		'  make test-rust-hotpath-read-fetch-bounds - Rust helper parity tests for read fetch planning' \
		'  make test-rust-hotpath-read-slice-plan - Rust helper parity tests for read slice planning' \
		'  make test-rust-hotpath-read-missing-range-worker-count - Rust helper parity tests for missing-range parallelism' \
		'  make test-rust-hotpath-block-count - Rust helper parity tests for block counting' \
		'  make test-rust-hotpath-dirty-block-size - Rust helper parity tests for dirty block sizing' \
		'  make test-rust-hotpath-logical-resize-plan - Rust helper parity tests for logical resize planning' \
		'  make test-rust-hotpath-persist-layout-plan - Rust helper parity tests for persist layout planning' \
		'  make test-rust-hotpath-persist-block-plan - Rust helper parity tests for persist block planning' \
		'  make test-rust-hotpath-persist-block-crc-plan - Rust helper parity tests for persist block CRC planning' \
		'  make test-rust-hotpath-write-copy-worker-count - Rust helper parity tests for write copy worker counting' \
		'  make test-rust-hotpath-block-transfer-plan - Rust helper parity tests for block transfer planning' \
		'  make test-rust-hotpath-write-copy-plan - Rust helper parity tests for write copy planning' \
		'  make test-rust-hotpath-parallel-worker-count - Rust helper parity tests for shared worker counting' \
		'  make test-rust-hotpath-missing-ranges - Rust helper parity tests for missing-range handling' \
		'  make test-large-copy-benchmark - benchmark large copy_file_range transfers' \
		'  make test-large-file-multiblock-benchmark - benchmark large multi-block file writes' \
		'  make test-remount-durability-benchmark - benchmark data survival across remounts' \
		'  make test-tree-scale - benchmark getattr/readdir on a larger tree' \
		'  make test-flush-release-profile - verify clean flush/release and dirty flush regression handling' \
		'  make test-truncate-release-profile - benchmark truncate-only flush/release on large files' \
		'  make test-write-flush-threshold - verify automatic flush when the write buffer threshold is exceeded' \
		'  make test-all-full - full integration suite + atime checks' \
		'  make test-pool-connections - verify ThreadedConnectionPool configuration' \
		'  make test-metadata-cache - verify short-TTL metadata and statfs cache behavior' \
		'  make test-mount-suite - shared Python mount smoke runner' \
		'  make test-fio-sequential-io - fio sequential read/write smoke for block and extent paths' \
		'  make test-fio-sequential-io-strace - fio sequential smoke with strace syscall tables for block and extent paths' \
		'  make test-admpanch-trace - run ADMP_TRACE_TARGET with ADMP_INI=$(ADMP_TRACE_INI_ABS) (override ADMP_TRACE_TARGET=...)' \
		'  make test-fio-mixed-io - fio mixed sequential rw smoke for block and extent paths' \
		'  make test-fio-random-mixed-io - fio random mixed rw negative control for block and extent paths' \
		'  make test-all   - smoke + current integration suite' \
		'  make db-shell   - open psql on local PostgreSQL' \
		'  make clean      - remove .venv'

$(VENV_PYTHON):
	$(PYTHON) -m venv $(VENV_DIR)

$(VENV_STAMP): requirements-test.txt $(VENV_PYTHON)
	$(VENV_PYTHON) -m ensurepip --upgrade
	$(VENV_PIP) install -r requirements-test.txt
	@touch $@

venv: $(VENV_STAMP)

deps: venv

deps-ubuntu:
	@printf '%s\n' \
		'Ubuntu/Debian build prerequisites for FOD:' \
		'  sudo apt-get update' \
		"  sudo apt-get install -y $(UBUNTU_BUILD_DEPS)" \
		"  Optional legacy Python helpers/tests: sudo apt-get install -y $(UBUNTU_LEGACY_PYTHON_DEPS)"

deps-redhat:
	@printf '%s\n' \
		'Fedora/RHEL build prerequisites for FOD:' \
		"  sudo dnf install -y $(REDHAT_BUILD_DEPS)" \
		"  Optional legacy Python helpers/tests: sudo dnf install -y $(REDHAT_LEGACY_PYTHON_DEPS)"

up:
	COMPOSE_PROJECT_NAME=fod POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) POSTGRES_PORT=$(POSTGRES_PORT) \
	$(COMPOSE_RUN) -f $(COMPOSE_FILE) up -d postgres
	@$(MAKE) wait QNAP=$(QNAP)

docker-selinux-acl-up:
	COMPOSE_PROJECT_NAME=fod-selinux-acl POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) POSTGRES_PORT=$(POSTGRES_PORT) FOD_ROLE=auto FOD_PROFILE=bulk_write FOD_SELINUX=on FOD_ACL=on FOD_LOG_LEVEL=DEBUG FOD_ALLOW_OTHER=1 \
	$(COMPOSE_RUN) -f $(SELINUX_ACL_COMPOSE_FILE) up -d postgres fod-selinux-acl
	@$(MAKE) docker-selinux-acl-wait

down:
	COMPOSE_PROJECT_NAME=fod POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) POSTGRES_PORT=$(POSTGRES_PORT) \
	$(COMPOSE_RUN) -f $(COMPOSE_FILE) down

docker-selinux-acl-down:
	COMPOSE_PROJECT_NAME=fod-selinux-acl POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) POSTGRES_PORT=$(POSTGRES_PORT) \
	$(COMPOSE_RUN) -f $(SELINUX_ACL_COMPOSE_FILE) down -v

docker-selinux-acl-wait:
	@set -eu; \
	echo "Waiting for PostgreSQL in the SELinux/ACL Docker lab..."; \
	for i in $$(seq 1 60); do \
		if COMPOSE_PROJECT_NAME=fod-selinux-acl $(COMPOSE_RUN) -f $(SELINUX_ACL_COMPOSE_FILE) exec -T postgres pg_isready -U $(POSTGRES_USER) -d $(POSTGRES_DB) >/dev/null 2>&1; then \
			echo "SELinux/ACL lab PostgreSQL ready."; \
			exit 0; \
		fi; \
		sleep 1; \
	done; \
	echo "SELinux/ACL lab PostgreSQL did not start within the expected time."; \
	exit 1

docker-selinux-acl-shell:
	COMPOSE_PROJECT_NAME=fod-selinux-acl \
	$(COMPOSE_RUN) -f $(SELINUX_ACL_COMPOSE_FILE) exec fod-selinux-acl bash

docker-selinux-acl-smoke: docker-selinux-acl-up
	# This lab builds inside the container because it validates the container-local FUSE/SELinux toolchain.
	COMPOSE_PROJECT_NAME=fod-selinux-acl $(COMPOSE_RUN) -f $(SELINUX_ACL_COMPOSE_FILE) exec -T fod-selinux-acl bash -lc 'set -euo pipefail; $(CARGO_BUILD_MKFS) --bin fod-bootstrap --bin fod-rust-mkfs; $(CARGO_BUILD_FUSE) --bin fod-rust-fuse; ./.venv/bin/python tests/integration/test_fuse_context_identity.py; ./.venv/bin/python tests/integration/test_xattr.py; $(CARGO_TEST_FUSE) --test root_permissions_smoke -- --nocapture'

restart: down up

logs:
	COMPOSE_PROJECT_NAME=fod POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) POSTGRES_PORT=$(POSTGRES_PORT) \
	$(COMPOSE_RUN) -f $(COMPOSE_FILE) logs -f postgres

wait:
	@set -eu; \
	echo "Waiting for PostgreSQL in Docker..."; \
	for i in $$(seq 1 60); do \
		if COMPOSE_PROJECT_NAME=fod $(COMPOSE_RUN) -f $(COMPOSE_FILE) exec -T postgres pg_isready -U $(POSTGRES_USER) -d $(POSTGRES_DB) >/dev/null 2>&1; then \
			echo "PostgreSQL ready."; \
			exit 0; \
		fi; \
		sleep 1; \
	done; \
	echo "PostgreSQL did not start within the expected time."; \
	exit 1


init: build-debug up
	@set -eu; \
	status_output="$$($(FOD_MKFS_DEBUG_BIN) status 2>/dev/null || true)"; \
	if printf '%s\n' "$$status_output" | grep -Fq 'FOD ready: yes'; then \
		echo 'FOD schema already initialized; skipping init.'; \
	else \
		POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(FOD_MKFS_DEBUG_BIN) init --schema-admin-password "$(FOD_SCHEMA_ADMIN_PASSWORD)"; \
		mkdir -p .fod; \
		printf '%s\n' "$(FOD_SCHEMA_ADMIN_PASSWORD)" > "$(FOD_SCHEMA_ADMIN_PASSWORD_FILE)"; \
	fi

init-qnap: build-debug
	@set -eu; \
	status_output="$$($(FOD_REMOTE_PG_ENV) $(FOD_MKFS_DEBUG_BIN) status 2>/dev/null || true)"; \
	if printf '%s\n' "$$status_output" | grep -Fq 'FOD ready: yes'; then \
		echo 'FOD schema already initialized; skipping qnap init.'; \
	else \
		$(FOD_REMOTE_PG_ENV) $(FOD_MKFS_DEBUG_BIN) init --schema-admin-password "$(FOD_SCHEMA_ADMIN_PASSWORD)"; \
		mkdir -p .fod; \
		printf '%s\n' "$(FOD_SCHEMA_ADMIN_PASSWORD)" > "$(FOD_SCHEMA_ADMIN_PASSWORD_FILE)"; \
	fi


reset: build-debug
	COMPOSE_PROJECT_NAME=fod POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) POSTGRES_PORT=$(POSTGRES_PORT) \
	$(COMPOSE_RUN) -f $(COMPOSE_FILE) down -v
	$(MAKE) up QNAP=$(QNAP)
	sleep 2
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(FOD_MKFS_DEBUG_BIN) init --schema-admin-password "$(FOD_SCHEMA_ADMIN_PASSWORD)"
	mkdir -p .fod
	printf '%s\n' "$(FOD_SCHEMA_ADMIN_PASSWORD)" > "$(FOD_SCHEMA_ADMIN_PASSWORD_FILE)"

warn-config-secret:
	@set -eu; \
	if [ -f "$(FOD_CONFIG_SOURCE)" ] && grep -Eq '^[[:space:]]*password[[:space:]]*=[[:space:]]*cichosza([[:space:]]*([#;].*)?)?$$' "$(FOD_CONFIG_SOURCE)"; then \
		printf '%s\n' "Warning: $(FOD_CONFIG_SOURCE) still contains password = cichosza."; \
		printf '%s\n' "Warning: use fod_config.example.ini for shared installs and keep fod_config.ini local."; \
	fi

install-config:
	$(MAKE) warn-config-secret
	@printf '%s\n' "Installing $(FOD_CONFIG_SOURCE) -> $(FOD_CONFIG_DEST)"
	sudo install -D -m 0644 $(FOD_CONFIG_SOURCE) $(FOD_CONFIG_DEST)

install-config-user:
	$(MAKE) warn-config-secret
	@printf '%s\n' "Installing $(FOD_CONFIG_SOURCE) -> $$HOME/.config/fod/fod_config.ini"
	install -D -m 0644 $(FOD_CONFIG_SOURCE) $$HOME/.config/fod/fod_config.ini

test-config-warning:
	tests/integration/test_config_warning.sh

install-mount-helper:
	@printf '%s\n' "Installing mount.fod -> $(MOUNT_HELPER_DEST)"
	sudo install -D -m 0755 mount.fod $(MOUNT_HELPER_DEST)


install-root-scripts:
	@printf '%s\n' "Installing FOD $(FOD_VERSION): fod-bootstrap, mkfs.fod, fod-change/fod.change, fod-indexer, and fod-rust-fuse -> /usr/local/bin"
	$(CARGO_BUILD_INSTALL_ROOT)
	sudo install -D -m 0755 "$(FOD_BOOTSTRAP_PROFILE_BIN)" /usr/local/bin/fod-bootstrap
	sudo install -D -m 0755 "$(FOD_MKFS_PROFILE_BIN)" /usr/local/bin/mkfs.fod
	sudo install -D -m 0755 "$(FOD_CHANGE_PROFILE_BIN)" /usr/local/bin/fod-change
	sudo ln -sf fod-change /usr/local/bin/fod.change
	sudo install -D -m 0755 "$(FOD_INDEXER_PROFILE_BIN)" /usr/local/bin/fod-indexer
	sudo install -D -m 0755 "$(FOD_FUSE_PROFILE_BIN)" /usr/local/bin/fod-rust-fuse
	sudo $(STRIP) $(STRIP_FLAGS) /usr/local/bin/fod-bootstrap
	sudo $(STRIP) $(STRIP_FLAGS) /usr/local/bin/mkfs.fod
	sudo $(STRIP) $(STRIP_FLAGS) /usr/local/bin/fod-change
	sudo $(STRIP) $(STRIP_FLAGS) /usr/local/bin/fod-indexer
	sudo $(STRIP) $(STRIP_FLAGS) /usr/local/bin/fod-rust-fuse


install-rust-hotpath:
	@printf '%s\n' "Building Rust hot-path artifacts"
	@$(CARGO_BUILD_HOTPATH) $(FOD_RELEASE_FLAG) --lib
	@printf '%s\n' "Installing Rust hot-path shared library -> /usr/local/lib"
	@sudo install -D -m 0755 "$(FOD_HOTPATH_PROFILE_LIB)" /usr/local/lib/libfod-2.so
	@sudo $(STRIP) $(STRIP_FLAGS) /usr/local/lib/libfod-2.so

install-on-root: install-config install-root-scripts install-rust-hotpath install-mount-helper
	@printf '%s\n' "FOD installed for root-style use: config, Rust binaries including fod-indexer, mount helper, and Rust hot-path library"

install-on-root-venv: venv install-on-root
	@printf '%s\n' "FOD root-style install ready in $(VENV_DIR): config, legacy test venv, Rust binaries, mount helper, and Rust hot-path library"

pip-build:
	@printf '%s\n' "Python packaging has been removed; build the Rust binaries directly." >&2
	@exit 1

pip-install:
	@printf '%s\n' "Python packaging has been removed; use the Rust binaries directly." >&2
	@exit 1

pip-install-editable:
	@printf '%s\n' "Python packaging has been removed; use the Rust binaries directly." >&2
	@exit 1


config-show: build-debug
	$(FOD_CONFIG_DEBUG_BIN) --config-path . resolve-path

indexer: build-debug
	@set -eu; \
	if [ -z "$(strip $(INDEXER_ARGS))" ]; then \
		echo 'Set INDEXER_ARGS=...'; \
		exit 1; \
	fi; \
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(FOD_INDEXER_DEBUG_BIN) $(INDEXER_ARGS)

indexer-import: build-debug init
	@set -eu; \
	if [ -z "$(strip $(INDEXER_SOURCE))" ]; then \
		echo 'Set INDEXER_SOURCE=...'; \
		exit 1; \
	fi; \
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(FOD_INDEXER_DEBUG_BIN) materialize --source "$(INDEXER_SOURCE)"

.PHONY: indexer indexer-import

test-fod-indexer-smoke: venv init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) POSTGRES_PORT=$(POSTGRES_PORT) $(VENV_PYTHON) tests/integration/test_fod_indexer_materialize.py

.PHONY: test-fod-indexer-smoke

test-fod-indexer-materialize: test-fod-indexer-smoke

.PHONY: test-fod-indexer-materialize

test-fod-indexer-materialize-rollback: venv init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) POSTGRES_PORT=$(POSTGRES_PORT) $(VENV_PYTHON) tests/integration/test_fod_indexer_materialize_rollback.py

.PHONY: test-fod-indexer-materialize-rollback

test-fod-indexer-usability: venv init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) POSTGRES_PORT=$(POSTGRES_PORT) $(VENV_PYTHON) tests/integration/test_fod_indexer_usability.py

.PHONY: test-fod-indexer-usability

test-fod-indexer-json-output: venv init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) POSTGRES_PORT=$(POSTGRES_PORT) $(VENV_PYTHON) tests/integration/test_fod_indexer_json_output.py

.PHONY: test-fod-indexer-json-output

test-fod-indexer-plan-import-scope: venv init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) POSTGRES_PORT=$(POSTGRES_PORT) $(VENV_PYTHON) tests/integration/test_fod_indexer_plan_import_scope.py

.PHONY: test-fod-indexer-plan-import-scope

test-fod-indexer-cleanup-failed: venv init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) POSTGRES_PORT=$(POSTGRES_PORT) $(VENV_PYTHON) tests/integration/test_fod_indexer_cleanup_failed.py

.PHONY: test-fod-indexer-cleanup-failed

test-fod-indexer-parallel-smoke: venv init
	@set -u; \
	$(MAKE) --no-print-directory test-fod-indexer-plan-import-scope & \
	pid_plan=$$!; \
	$(MAKE) --no-print-directory test-fod-indexer-cleanup-failed & \
	pid_cleanup=$$!; \
	wait $$pid_plan; \
	status_plan=$$?; \
	wait $$pid_cleanup; \
	status_cleanup=$$?; \
	if [ "$$status_plan" -ne 0 ] || [ "$$status_cleanup" -ne 0 ]; then \
		echo "fod-indexer parallel smoke failed: plan-import-scope=$$status_plan cleanup-failed=$$status_cleanup" >&2; \
		exit 1; \
	fi

.PHONY: test-fod-indexer-parallel-smoke

cargo-profile-show:
	@printf '%s\n' "FOD_VERSION=$(FOD_VERSION)"
	@printf '%s\n' "FOD_CARGO_PROFILE=$(FOD_CARGO_PROFILE)"
	@printf '%s\n' "FOD_RELEASE_FLAG=$(FOD_RELEASE_FLAG)"
	@printf '%s\n' "install-root-scripts outputs: $(FOD_BOOTSTRAP_PROFILE_BIN), $(FOD_MKFS_PROFILE_BIN), $(FOD_CHANGE_PROFILE_BIN), $(FOD_INDEXER_PROFILE_BIN), $(FOD_FUSE_PROFILE_BIN)"

smoke: up
	@set -eu; \
	for attempt in 1 2 3 4 5; do \
		if PGPASSWORD=$(POSTGRES_PASSWORD) psql -h $(FOD_PG_HOST) -p $(FOD_PG_PORT) -U $(POSTGRES_USER) -d $(POSTGRES_DB) -tAc 'SELECT 1' | grep -qx 1; then \
			exit 0; \
		fi; \
		sleep 1; \
	done; \
	exit 1

enable-pg-stat-statements: up
	COMPOSE_PROJECT_NAME=fod POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) POSTGRES_PORT=$(POSTGRES_PORT) \
	$(COMPOSE_RUN) -f $(COMPOSE_FILE) exec -T postgres sh -lc 'PGPASSWORD="$$POSTGRES_PASSWORD" psql -v ON_ERROR_STOP=1 -h 127.0.0.1 -U "$$POSTGRES_USER" -d "$$POSTGRES_DB" -c "CREATE EXTENSION IF NOT EXISTS pg_stat_statements;"'

mount: build-debug up
	mkdir -p $(MOUNTPOINT)
	@printf '%s\n' "Using FOD config file: /etc/fod/fod_config.ini (fallback: ./fod_config.ini)"
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_ROLE=$(FOD_ROLE) FOD_PROFILE=$(FOD_PROFILE) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_LOG_LEVEL=$(FOD_LOG_LEVEL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) FOD_SELINUX_CONTEXT=$(FOD_SELINUX_CONTEXT) FOD_SELINUX_FSCONTEXT=$(FOD_SELINUX_FSCONTEXT) FOD_SELINUX_DEFCONTEXT=$(FOD_SELINUX_DEFCONTEXT) FOD_SELINUX_ROOTCONTEXT=$(FOD_SELINUX_ROOTCONTEXT) $(FOD_BOOTSTRAP_DEBUG_BIN) --role $(FOD_ROLE) $(if $(strip $(FOD_PROFILE)),--profile $(FOD_PROFILE)) --selinux $(FOD_SELINUX) --acl $(FOD_ACL) --atime-policy $(FOD_ATIME_POLICY) $(if $(filter 0 false False no,$(FOD_DEFAULT_PERMISSIONS)),--no-default-permissions,--default-permissions) -f $(MOUNTPOINT)

mount-qnap: build-debug
	mkdir -p $(MOUNTPOINT)
	@printf '%s\n' "Using remote PostgreSQL at $(FOD_REMOTE_PG_HOST):$(FOD_REMOTE_PG_PORT) (db=$(FOD_REMOTE_PG_DBNAME), user=$(FOD_REMOTE_PG_USER))"
	$(FOD_REMOTE_PG_ENV) FOD_ROLE=$(FOD_ROLE) FOD_PROFILE=$(FOD_PROFILE) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_LOG_LEVEL=$(FOD_LOG_LEVEL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) FOD_SELINUX_CONTEXT=$(FOD_SELINUX_CONTEXT) FOD_SELINUX_FSCONTEXT=$(FOD_SELINUX_FSCONTEXT) FOD_SELINUX_DEFCONTEXT=$(FOD_SELINUX_DEFCONTEXT) FOD_SELINUX_ROOTCONTEXT=$(FOD_SELINUX_ROOTCONTEXT) $(FOD_BOOTSTRAP_DEBUG_BIN) --role $(FOD_ROLE) $(if $(strip $(FOD_PROFILE)),--profile $(FOD_PROFILE)) --selinux $(FOD_SELINUX) --acl $(FOD_ACL) --atime-policy $(FOD_ATIME_POLICY) $(if $(filter 0 false False no,$(FOD_DEFAULT_PERMISSIONS)),--no-default-permissions,--default-permissions) -f $(MOUNTPOINT)

mount-user: build-debug up
	mkdir -p $(MOUNTPOINT)
	@set -eu; \
	config_path="$$HOME/.config/fod/fod_config.ini"; \
	if [ -f "$$config_path" ]; then \
		export FOD_CONFIG="$$config_path"; \
		echo "Using FOD config file: $$config_path"; \
	else \
		unset FOD_CONFIG; \
		echo "Using local ./fod_config.ini if present (user config $$config_path not found)"; \
	fi; \
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_ROLE=$(FOD_ROLE) FOD_PROFILE=$(FOD_PROFILE) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_LOG_LEVEL=$(FOD_LOG_LEVEL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) FOD_SELINUX_CONTEXT=$(FOD_SELINUX_CONTEXT) FOD_SELINUX_FSCONTEXT=$(FOD_SELINUX_FSCONTEXT) FOD_SELINUX_DEFCONTEXT=$(FOD_SELINUX_DEFCONTEXT) FOD_SELINUX_ROOTCONTEXT=$(FOD_SELINUX_ROOTCONTEXT) $(FOD_BOOTSTRAP_DEBUG_BIN) --role $(FOD_ROLE) $(if $(strip $(FOD_PROFILE)),--profile $(FOD_PROFILE)) --selinux $(FOD_SELINUX) --acl $(FOD_ACL) --atime-policy $(FOD_ATIME_POLICY) $(if $(filter 0 false False no,$(FOD_DEFAULT_PERMISSIONS)),--no-default-permissions,--default-permissions) -f $(MOUNTPOINT)

demo: build-debug init
	mkdir -p $(MOUNTPOINT)
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_ROLE=$(FOD_ROLE) FOD_PROFILE=$(FOD_PROFILE) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_LOG_LEVEL=$(FOD_LOG_LEVEL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) FOD_SELINUX_CONTEXT=$(FOD_SELINUX_CONTEXT) FOD_SELINUX_FSCONTEXT=$(FOD_SELINUX_FSCONTEXT) FOD_SELINUX_DEFCONTEXT=$(FOD_SELINUX_DEFCONTEXT) FOD_SELINUX_ROOTCONTEXT=$(FOD_SELINUX_ROOTCONTEXT) $(FOD_BOOTSTRAP_DEBUG_BIN) --role $(FOD_ROLE) $(if $(strip $(FOD_PROFILE)),--profile $(FOD_PROFILE)) --selinux $(FOD_SELINUX) --acl $(FOD_ACL) --atime-policy $(FOD_ATIME_POLICY) $(if $(filter 0 false False no,$(FOD_DEFAULT_PERMISSIONS)),--no-default-permissions,--default-permissions) -f $(MOUNTPOINT)

unmount:
	@set -eu; \
	if command -v fusermount3 >/dev/null 2>&1; then \
		fusermount3 -u $(MOUNTPOINT); \
	elif command -v fusermount >/dev/null 2>&1; then \
		fusermount -u $(MOUNTPOINT); \
	else \
		umount $(MOUNTPOINT); \
	fi

test-integration: venv reset test-persist-buffer-chunking test-write-flush-threshold test-utimens-noop test-write-noop test-unlink-after-write test-local-vs-fod-permissions test-copy-block-crc-table test-multi-open-unique-handles test-workers-read-parallel test-workers-write-parallel-copy test-worker-thresholds-block-size test-rust-hotpath-copy-plan test-rust-hotpath-crc32 test-rust-hotpath-read-ahead test-rust-hotpath-read-sequence test-rust-hotpath-read-fetch-bounds test-rust-hotpath-read-slice-plan test-rust-hotpath-read-missing-range-worker-count test-rust-hotpath-block-count test-rust-hotpath-dirty-block-size test-rust-hotpath-logical-resize-plan test-rust-hotpath-persist-layout-plan test-rust-hotpath-write-copy-worker-count test-rust-hotpath-block-transfer-plan test-rust-hotpath-write-copy-plan test-rust-hotpath-parallel-worker-count test-rust-hotpath-missing-ranges test-rust-hotpath-copy-dedupe test-rust-hotpath-copy-pack test-rust-hotpath-persist-pad test-rust-hotpath-read-assemble test-rust-pg-query test-rust-hotpath-runtime-size-limits test-version test-timestamp-touch-once test-read-ahead-sequence test-runtime-config test-runtime-validation test-schema-upgrade test-block-read test-pg-lock-manager test-mount-root-permissions test-mount-wrapper-options test-connection-recovery test-fuse-context-identity test-postgresql-requirements test-runtime-profile test-mkfs-pg-tls test-metadata-cache test-truncate-shrink-block-boundary
test-integration: test-rust-hotpath-persist-block-plan
test-integration: test-rust-hotpath-persist-block-crc-plan
test-integration: test-config-warning
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_mkdir_create_write_read.py
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(CARGO_TEST_FUSE) --test mount_smoke mkdir_parent_missing --offline
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(CARGO_TEST_FUSE) --test mount_smoke truncate_rename --offline
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_chmod_rmdir.py
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) tests/integration/test_rename_root_conflict.sh
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_destroy.py
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) tests/integration/test_dirhooks.sh
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) tests/integration/test_hardlink.sh
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_fallocate.py
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_copy_file_range.py
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_ioctl.py
	sudo env $(ADMP_TRACE_ENV) POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_mknod.py
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(CARGO_TEST_FUSE) --test mount_smoke --offline -- --nocapture --test-threads=1
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_lseek.py
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_poll.py
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_access_groups.py
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_inode_model.py
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_ownership_inheritance.py
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_permissions.py
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_xattr.py

test-role-autodetect:
	cargo test --manifest-path rust_runtime/Cargo.toml --lib resolves_auto_and_replica_lock_roles --offline

test-xattr: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_xattr.py

test-locking: init
	sudo env $(ADMP_TRACE_ENV) POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(CARGO_TEST_FUSE) --test lock_backend_smoke -- --nocapture

test-pg-lock-manager: init
	$(CARGO_TEST_HOTPATH) --test lock_manager

test-permissions: up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_permissions.py

test-journal: up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_journal.py

test-destroy: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_destroy.py

test-dirhooks: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) tests/integration/test_dirhooks.sh

test-hardlink: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) tests/integration/test_hardlink.sh

test-fallocate: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_fallocate.py

test-copy-file-range: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_copy_file_range.py


test-copy-dedupe-benchmark: test-rust-hotpath-copy-dedupe-benchmark
	@:
test-copy-block-crc-table: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_BOOTSTRAP_BIN=$(CURDIR)/$(FOD_BOOTSTRAP_DEBUG_BIN) $(CARGO_TEST_FUSE) --test profile_smoke copy_block_crc_table --offline

test-worker-thresholds-block-size: init
	$(CARGO_TEST_HOTPATH) --test helper_parity write_worker_thresholds_block_size_plan_matches_expected_values

test-ioctl: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_ioctl.py

test-mknod: init
	sudo env $(ADMP_TRACE_ENV) POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) POSTGRES_PORT=$(POSTGRES_PORT) FOD_SCHEMA_ADMIN_PASSWORD=$(FOD_SCHEMA_ADMIN_PASSWORD) VENV_PYTHON=$(VENV_PYTHON) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_ROLE=$(FOD_ROLE) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) FOD_SELINUX_CONTEXT=$(FOD_SELINUX_CONTEXT) FOD_SELINUX_FSCONTEXT=$(FOD_SELINUX_FSCONTEXT) FOD_SELINUX_DEFCONTEXT=$(FOD_SELINUX_DEFCONTEXT) FOD_SELINUX_ROOTCONTEXT=$(FOD_SELINUX_ROOTCONTEXT) $(VENV_PYTHON) tests/integration/test_mknod.py


test-lseek: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_lseek.py

test-poll: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_poll.py

test-access-groups: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_access_groups.py

test-inode-model: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_inode_model.py

test-ownership-inheritance: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_ownership_inheritance.py

test-rename-root-conflict: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) tests/integration/test_rename_root_conflict.sh

test-statfs-use-ino: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SCHEMA_ADMIN_PASSWORD=$(FOD_SCHEMA_ADMIN_PASSWORD) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) VENV_PYTHON=$(VENV_PYTHON) bash tests/integration/test_statfs_use_ino.sh

test-atime-noatime: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SCHEMA_ADMIN_PASSWORD=$(FOD_SCHEMA_ADMIN_PASSWORD) FOD_ATIME_POLICY=noatime FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) VENV_PYTHON=$(VENV_PYTHON) bash tests/integration/test_atime_policy.sh

test-atime-relatime: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SCHEMA_ADMIN_PASSWORD=$(FOD_SCHEMA_ADMIN_PASSWORD) FOD_ATIME_POLICY=relatime FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) VENV_PYTHON=$(VENV_PYTHON) bash tests/integration/test_atime_policy.sh

test-atime-benchmark: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SCHEMA_ADMIN_PASSWORD=$(FOD_SCHEMA_ADMIN_PASSWORD) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) VENV_PYTHON=$(VENV_PYTHON) ATIME_BENCH_KIND=file bash tests/integration/test_atime_benchmark.sh
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SCHEMA_ADMIN_PASSWORD=$(FOD_SCHEMA_ADMIN_PASSWORD) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) VENV_PYTHON=$(VENV_PYTHON) ATIME_BENCH_KIND=dir bash tests/integration/test_atime_benchmark.sh

test-timestamp-touch-once: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SCHEMA_ADMIN_PASSWORD=$(FOD_SCHEMA_ADMIN_PASSWORD) FOD_ATIME_POLICY=relatime FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) bash tests/integration/test_timestamp_touch_once.sh

test-read-ahead-sequence: init
	$(CARGO_TEST_HOTPATH) --test helper_parity read_ahead_sequence_plan_matches_expected_values

test-read-cache-benchmark: init
	$(CARGO_TEST_HOTPATH) --test helper_parity read_cache_benchmark_plan_matches_expected_values

test-workers-read-parallel: init
	$(CARGO_TEST_HOTPATH) --test helper_parity read_workers_parallel_plan_matches_expected_values

test-workers-write-parallel-copy: init
	$(CARGO_TEST_HOTPATH) --test helper_parity write_workers_parallel_copy_plan_matches_expected_values


test-mkfs-config-suite:
	$(CARGO_TEST_MKFS) --test fod_config

test-runtime-config: init test-mkfs-config-suite
	@:

test-rust-mkfs-suite:
	$(CARGO_TEST_MKFS)

test-runtime-validation: test-rust-mkfs-suite
	@:

test-rust-hotpath-runtime-size-limits: test-rust-mkfs-suite
	@:
test-schema-upgrade: up
	$(CARGO_TEST_MKFS) --test schema_upgrade schema_upgrade_non_destructive_password_protected --offline

test-schema-status: up
	$(CARGO_TEST_MKFS) --test schema_upgrade schema_status_reports_version_secret_and_pending_migrations --offline

test-df: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) VENV_PYTHON=$(VENV_PYTHON) bash tests/integration/test_df.sh

test-mount-workflow: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) FOD_SELINUX_CONTEXT=$(FOD_SELINUX_CONTEXT) FOD_SELINUX_FSCONTEXT=$(FOD_SELINUX_FSCONTEXT) FOD_SELINUX_DEFCONTEXT=$(FOD_SELINUX_DEFCONTEXT) FOD_SELINUX_ROOTCONTEXT=$(FOD_SELINUX_ROOTCONTEXT) VENV_PYTHON=$(VENV_PYTHON) bash tests/integration/test_mount_workflow.sh

test-mount-root-permissions: reset
	ADMP_TRACE_ENV="$(ADMP_TRACE_ENV)" POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SCHEMA_ADMIN_PASSWORD=$(FOD_SCHEMA_ADMIN_PASSWORD) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) VENV_PYTHON=$(VENV_PYTHON) bash tests/integration/test_mount_root_permissions.sh

test-mount-wrapper-options:
	bash tests/integration/test_mount_wrapper_options.sh
	bash tests/integration/test_mount_wrapper_path_and_ro.sh

test-fuse-context-identity: venv
	$(VENV_PYTHON) tests/integration/test_fuse_context_identity.py

test-files: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) VENV_PYTHON=$(VENV_PYTHON) bash tests/integration/test_files.sh

test-block-read: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(CARGO_TEST_FUSE) --test mount_smoke block_read_range --offline

test-directories: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) VENV_PYTHON=$(VENV_PYTHON) bash tests/integration/test_directories.sh

test-metadata: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) VENV_PYTHON=$(VENV_PYTHON) bash tests/integration/test_metadata.sh

test-metadata-cache: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) $(VENV_PYTHON) tests/integration/test_metadata_cache.py

test-truncate-shrink-block-boundary: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) $(VENV_PYTHON) tests/integration/test_truncate_shrink_block_boundary.py

test-symlink: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) VENV_PYTHON=$(VENV_PYTHON) bash tests/integration/test_symlink.sh

test-throughput: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) VENV_PYTHON=$(VENV_PYTHON) THROUGHPUT_BLOCK_SIZE=$(THROUGHPUT_BLOCK_SIZE) THROUGHPUT_COUNT=$(THROUGHPUT_COUNT) THROUGHPUT_SYNC=$(THROUGHPUT_SYNC) bash tests/integration/test_throughput.sh

test-throughput-sync: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) VENV_PYTHON=$(VENV_PYTHON) THROUGHPUT_BLOCK_SIZE=$(THROUGHPUT_BLOCK_SIZE) THROUGHPUT_COUNT=$(THROUGHPUT_COUNT) THROUGHPUT_SYNC=1 bash tests/integration/test_throughput.sh

test-postgresql-wal-pressure: venv init
	POSTGRES_BENCHMARK_LABEL=$(if $(QNAP_ENABLED),qnap,local) PG_WAL_PRESSURE_COUNT=$(PG_WAL_PRESSURE_COUNT) POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_PG_HOST=$(FOD_PG_HOST) FOD_PG_PORT=$(FOD_PG_PORT) FOD_PG_DBNAME=$(FOD_PG_DBNAME) FOD_PG_USER=$(FOD_PG_USER) FOD_PG_PASSWORD=$(FOD_PG_PASSWORD) FOD_PG_SSLMODE=$(FOD_PG_SSLMODE) FOD_PG_SSLROOTCERT=$(FOD_PG_SSLROOTCERT) FOD_PG_SSLCERT=$(FOD_PG_SSLCERT) FOD_PG_SSLKEY=$(FOD_PG_SSLKEY) $(VENV_PYTHON) tests/integration/test_postgresql_wal_pressure.py

test-postgresql-wal-pressure-checkpoint: venv init
	POSTGRES_BENCHMARK_LABEL=$(if $(QNAP_ENABLED),qnap,local) PG_WAL_PRESSURE_COUNT=$(PG_WAL_PRESSURE_COUNT) PG_WAL_PRESSURE_FORCE_CHECKPOINT=1 POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_PG_HOST=$(FOD_PG_HOST) FOD_PG_PORT=$(FOD_PG_PORT) FOD_PG_DBNAME=$(FOD_PG_DBNAME) FOD_PG_USER=$(FOD_PG_USER) FOD_PG_PASSWORD=$(FOD_PG_PASSWORD) FOD_PG_SSLMODE=$(FOD_PG_SSLMODE) FOD_PG_SSLROOTCERT=$(FOD_PG_SSLROOTCERT) FOD_PG_SSLCERT=$(FOD_PG_SSLCERT) FOD_PG_SSLKEY=$(FOD_PG_SSLKEY) $(VENV_PYTHON) tests/integration/test_postgresql_wal_pressure.py

test-postgresql-connection-churn: venv init
	POSTGRES_BENCHMARK_LABEL=$(if $(QNAP_ENABLED),qnap,local) POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_PG_HOST=$(FOD_PG_HOST) FOD_PG_PORT=$(FOD_PG_PORT) FOD_PG_DBNAME=$(FOD_PG_DBNAME) FOD_PG_USER=$(FOD_PG_USER) FOD_PG_PASSWORD=$(FOD_PG_PASSWORD) FOD_PG_SSLMODE=$(FOD_PG_SSLMODE) FOD_PG_SSLROOTCERT=$(FOD_PG_SSLROOTCERT) FOD_PG_SSLCERT=$(FOD_PG_SSLCERT) FOD_PG_SSLKEY=$(FOD_PG_SSLKEY) $(VENV_PYTHON) tests/integration/test_postgresql_connection_churn.py

test-fio-sequential-io: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SCHEMA_ADMIN_PASSWORD=$(FOD_SCHEMA_ADMIN_PASSWORD) bash tests/integration/test_fio_sequential_io.sh

test-fio-sequential-io-strace: init
	sudo env $(ADMP_TRACE_ENV) POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SCHEMA_ADMIN_PASSWORD=$(FOD_SCHEMA_ADMIN_PASSWORD) FOD_PROFILE_IO=1 FOD_FOPEN_DIRECT_IO=1 FOD_STRACE=1 FIO_FILE_SIZE=$(FIO_FILE_SIZE) bash tests/integration/test_fio_sequential_io.sh

test-admpanch-trace:
	@printf '%s\n' "Running $(ADMP_TRACE_TARGET) with ADMP_INI=$(ADMP_TRACE_INI_ABS)"
	ADMP_INI="$(ADMP_TRACE_INI_ABS)" ADMP_TRACE_ENV="ADMP_INI=$(ADMP_TRACE_INI_ABS)" $(MAKE) $(ADMP_TRACE_TARGET)

test-fio-mixed-io: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SCHEMA_ADMIN_PASSWORD=$(FOD_SCHEMA_ADMIN_PASSWORD) bash tests/integration/test_fio_mixed_io.sh

test-fio-random-mixed-io: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SCHEMA_ADMIN_PASSWORD=$(FOD_SCHEMA_ADMIN_PASSWORD) FIO_RW_MODE=randrw FIO_RWMIXREAD=50 bash tests/integration/test_fio_mixed_io.sh


test-rust-hotpath-helper-parity:
	$(CARGO_TEST_HOTPATH) --test helper_parity

test-rust-hotpath-copy-plan \
test-rust-hotpath-crc32 \
test-rust-hotpath-read-ahead \
test-rust-hotpath-read-sequence \
test-rust-hotpath-read-fetch-bounds \
test-rust-hotpath-read-slice-plan \
test-rust-hotpath-read-missing-range-worker-count \
test-rust-hotpath-block-count \
test-rust-hotpath-dirty-block-size \
test-rust-hotpath-logical-resize-plan \
test-rust-hotpath-persist-layout-plan \
test-rust-hotpath-persist-block-plan \
test-rust-hotpath-persist-block-crc-plan \
test-rust-hotpath-write-copy-worker-count \
test-rust-hotpath-block-transfer-plan \
test-rust-hotpath-write-copy-plan \
test-rust-hotpath-parallel-worker-count \
test-rust-hotpath-missing-ranges \
test-rust-hotpath-copy-dedupe \
test-rust-hotpath-copy-pack \
test-rust-hotpath-persist-pad \
test-rust-hotpath-read-assemble: test-rust-hotpath-helper-parity
	@:

test-rust-hotpath-copy-dedupe-benchmark:
	$(CARGO_TEST_HOTPATH) --test copy_dedupe_benchmark -- --nocapture

test-rust-hotpath-extent-poc-benchmark:
	$(CARGO_TEST_HOTPATH) --test extent_poc_benchmark -- --nocapture

test-rust-pg-query: init
	$(CARGO_TEST_HOTPATH) --test pg_query

test-large-copy-benchmark: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_BOOTSTRAP_BIN=$(CURDIR)/$(FOD_BOOTSTRAP_DEBUG_BIN) $(CARGO_TEST_FUSE) --test large_copy_benchmark --offline -- --nocapture

test-large-file-multiblock-benchmark: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SCHEMA_ADMIN_PASSWORD=$(FOD_SCHEMA_ADMIN_PASSWORD) FOD_BOOTSTRAP_BIN=$(CURDIR)/$(FOD_BOOTSTRAP_DEBUG_BIN) $(CARGO_TEST_FUSE) --test large_file_multiblock_benchmark --offline -- --nocapture

test-remount-durability-benchmark: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_BOOTSTRAP_BIN=$(CURDIR)/$(FOD_BOOTSTRAP_DEBUG_BIN) $(CARGO_TEST_FUSE) --test remount_durability_benchmark --offline -- --nocapture

test-tree-scale: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) VENV_PYTHON=$(VENV_PYTHON) TREE_SCALE_DIRS=$(TREE_SCALE_DIRS) TREE_SCALE_FILES=$(TREE_SCALE_FILES) bash tests/integration/test_tree_scale.sh

test-flush-release-profile: reset
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_BOOTSTRAP_BIN=$(CURDIR)/$(FOD_BOOTSTRAP_DEBUG_BIN) $(CARGO_TEST_FUSE) --test profile_smoke flush_release_profile --offline

test-truncate-release-profile: reset
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_BOOTSTRAP_BIN=$(CURDIR)/$(FOD_BOOTSTRAP_DEBUG_BIN) $(CARGO_TEST_FUSE) --test profile_smoke truncate_release_profile --offline

test-persist-buffer-chunking: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_BOOTSTRAP_BIN=$(CURDIR)/$(FOD_BOOTSTRAP_DEBUG_BIN) $(CARGO_TEST_FUSE) --test profile_smoke persist_buffer_chunking --offline

test-write-flush-threshold: init
	$(CARGO_TEST_FUSE) --test profile_smoke write_flush_threshold --offline -- --nocapture

test-utimens-noop: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_BOOTSTRAP_BIN=$(CURDIR)/$(FOD_BOOTSTRAP_DEBUG_BIN) $(CARGO_TEST_FUSE) --test profile_smoke utimens_noop --offline

test-write-noop: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_BOOTSTRAP_BIN=$(CURDIR)/$(FOD_BOOTSTRAP_DEBUG_BIN) $(CARGO_TEST_FUSE) --test mount_smoke write_noop

test-unlink-after-write: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_BOOTSTRAP_BIN=$(CURDIR)/$(FOD_BOOTSTRAP_DEBUG_BIN) $(CARGO_TEST_FUSE) --test mount_smoke unlink_after_write

test-local-vs-fod-permissions: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_local_vs_fod_permissions.py

test-ext4-vs-fod-permissions: test-local-vs-fod-permissions

test-root-owned-permissions: init
	ADMP_TRACE_ENV="$(ADMP_TRACE_ENV)" POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(CARGO_TEST_FUSE) --test root_permissions_smoke -- --nocapture

test-allow-other-visibility: init
	bash tests/integration/test_allow_other_visibility.sh

test-multi-open-unique-handles: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_BOOTSTRAP_BIN=$(CURDIR)/$(FOD_BOOTSTRAP_DEBUG_BIN) $(CARGO_TEST_FUSE) --test mount_smoke multi_open_unique_handles


test-version: test-mkfs-config-suite
	@:
test-connection-recovery: init
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(CARGO_TEST_HOTPATH) --test connection_recovery --offline

test-pool-connections: venv
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_pool_connections.py

test-postgresql-requirements-autocommit-off: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_POSTGRES_AUTOCOMMIT=off $(VENV_PYTHON) tests/integration/test_postgresql_requirements.py

test-postgresql-requirements-autocommit-on: venv up
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_POSTGRES_AUTOCOMMIT=on $(VENV_PYTHON) tests/integration/test_postgresql_requirements.py

test-postgresql-requirements: test-postgresql-requirements-autocommit-off
	@:

test-runtime-profile: venv build-debug up
	sudo env $(ADMP_TRACE_ENV) POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) $(VENV_PYTHON) tests/integration/test_runtime_profile.py

test-runtime-reload: venv build-debug
	$(MAKE) reset
	sudo env $(ADMP_TRACE_ENV) POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_SCHEMA_ADMIN_PASSWORD=$(FOD_SCHEMA_ADMIN_PASSWORD) $(VENV_PYTHON) tests/integration/test_runtime_reload.py

test-runtime-profile-extents: venv build-debug up
	sudo env $(ADMP_TRACE_ENV) POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) FOD_PROFILE=extents FOD_ENABLE_EXTENTS=1 $(VENV_PYTHON) tests/integration/test_runtime_profile.py

.PHONY: test-runtime-profile-extents test-runtime-reload

change-runtime-list: build-debug up wait
	$(FOD_CHANGE_DEBUG_BIN) --config-path $(FOD_CHANGE_CONFIG_PATH) --list

change-runtime-get: build-debug up wait
	@set -eu; \
	if [ -z "$(strip $(FOD_CHANGE_KEY))" ]; then \
		echo 'Set FOD_CHANGE_KEY=...'; \
		exit 1; \
	fi; \
	$(FOD_CHANGE_DEBUG_BIN) --config-path $(FOD_CHANGE_CONFIG_PATH) --get $(FOD_CHANGE_KEY)

change-runtime-set: build-debug up wait
	@set -eu; \
	if [ -z "$(strip $(FOD_CHANGE_KEY))" ]; then \
		echo 'Set FOD_CHANGE_KEY=...'; \
		exit 1; \
	fi; \
	if [ -z "$(strip $(FOD_CHANGE_VALUE))" ]; then \
		echo 'Set FOD_CHANGE_VALUE=...'; \
		exit 1; \
	fi; \
	if [ -z "$(strip $(FOD_CHANGE_PASSWORD))" ]; then \
		echo 'Set FOD_CHANGE_PASSWORD=...'; \
		exit 1; \
	fi; \
	$(FOD_CHANGE_DEBUG_BIN) --config-path $(FOD_CHANGE_CONFIG_PATH) --password $(FOD_CHANGE_PASSWORD) --set $(FOD_CHANGE_KEY)=$(FOD_CHANGE_VALUE)

change-runtime: change-runtime-set
	@:

change-runtime-sync: reload-runtime

reload-runtime: build-debug up wait
	@set -eu; \
	$(FOD_CHANGE_DEBUG_BIN) --config-path $(FOD_CHANGE_CONFIG_PATH) --sync-config

.PHONY: reload-runtime change-runtime change-runtime-sync change-runtime-list change-runtime-get change-runtime-set

test-mkfs-pg-tls: test-mkfs-config-suite
	@:

test-mount-suite: venv
	$(MAKE) reset
	POSTGRES_DB=$(POSTGRES_DB) POSTGRES_USER=$(POSTGRES_USER) POSTGRES_PASSWORD=$(POSTGRES_PASSWORD) VENV_PYTHON=$(VENV_PYTHON) FOD_SELINUX=$(FOD_SELINUX) FOD_ACL=$(FOD_ACL) FOD_DEFAULT_PERMISSIONS=$(FOD_DEFAULT_PERMISSIONS) FOD_ATIME_POLICY=$(FOD_ATIME_POLICY) FOD_ROLE=$(FOD_ROLE) FOD_LAZYTIME=$(FOD_LAZYTIME) FOD_SYNC=$(FOD_SYNC) FOD_DIRSYNC=$(FOD_DIRSYNC) FOD_SELINUX_CONTEXT=$(FOD_SELINUX_CONTEXT) FOD_SELINUX_FSCONTEXT=$(FOD_SELINUX_FSCONTEXT) FOD_SELINUX_DEFCONTEXT=$(FOD_SELINUX_DEFCONTEXT) FOD_SELINUX_ROOTCONTEXT=$(FOD_SELINUX_ROOTCONTEXT) $(VENV_PYTHON) tests/integration/test_mount_suite.py

test-all: smoke test-integration test-mount-suite test-locking test-journal test-rename-root-conflict test-pool-connections
test-all-full: test-all test-files test-directories test-metadata test-symlink test-mount-workflow test-statfs-use-ino test-atime-noatime test-atime-relatime test-fod-indexer-smoke test-fod-indexer-materialize-rollback test-fod-indexer-usability test-fod-indexer-plan-import-scope test-fod-indexer-cleanup-failed
test-integration: test-runtime-profile-extents

benchmark: benchmarks

benchmarks:
	@set -eu; \
	for target in $(BENCHMARK_TARGETS); do \
		$(MAKE) --no-print-directory $$target; \
	done

postgres-benchmarks:
	$(call RUN_POSTGRES_BENCHMARKS,$(POSTGRES_BENCHMARK_TARGETS),$(if $(QNAP_ENABLED),$(FOD_REMOTE_PG_ENV),),$(QNAP))

postgres-benchmarks-local:
	$(call RUN_POSTGRES_BENCHMARKS,$(POSTGRES_BENCHMARK_TARGETS),,0)

postgres-benchmarks-qnap:
	$(call RUN_POSTGRES_BENCHMARKS,$(POSTGRES_BENCHMARK_TARGETS),$(FOD_REMOTE_PG_ENV),1)

postgres-benchmarks-checkpoint:
	$(call RUN_POSTGRES_BENCHMARKS,$(POSTGRES_BENCHMARK_CHECKPOINT_TARGETS),$(if $(QNAP_ENABLED),$(FOD_REMOTE_PG_ENV),),$(QNAP))

postgres-benchmarks-compare:
	@set -eu; \
	$(MAKE) --no-print-directory postgres-benchmarks-local; \
	$(MAKE) --no-print-directory postgres-benchmarks-qnap; \
	$(MAKE) --no-print-directory QNAP=0 postgres-benchmarks-checkpoint; \
	$(MAKE) --no-print-directory QNAP=1 postgres-benchmarks-checkpoint

postgres-benchmarks-wal-preset:
	$(call RUN_POSTGRES_BENCHMARK_REPEAT,POSTGRES_MAX_WAL_SIZE=$(POSTGRES_BENCHMARK_WAL_PRESET_MAX_WAL_SIZE) POSTGRES_CHECKPOINT_TIMEOUT=$(POSTGRES_BENCHMARK_WAL_PRESET_CHECKPOINT_TIMEOUT) POSTGRES_WAL_COMPRESSION=$(POSTGRES_BENCHMARK_WAL_PRESET_WAL_COMPRESSION) postgres-benchmarks-compare)

postgres-benchmarks-planner-preset:
	@$(MAKE) --no-print-directory \
		POSTGRES_SHARED_BUFFERS=$(POSTGRES_BENCHMARK_PLANNER_PRESET_SHARED_BUFFERS) \
		POSTGRES_RANDOM_PAGE_COST=$(POSTGRES_BENCHMARK_PLANNER_PRESET_RANDOM_PAGE_COST) \
		POSTGRES_EFFECTIVE_CACHE_SIZE=$(POSTGRES_BENCHMARK_PLANNER_PRESET_EFFECTIVE_CACHE_SIZE) \
		POSTGRES_MAINTENANCE_WORK_MEM=$(POSTGRES_BENCHMARK_PLANNER_PRESET_MAINTENANCE_WORK_MEM) \
		POSTGRES_AUTOVACUUM_MAX_WORKERS=$(POSTGRES_BENCHMARK_PLANNER_PRESET_AUTOVACUUM_MAX_WORKERS) \
		POSTGRES_AUTOVACUUM_WORK_MEM=$(POSTGRES_BENCHMARK_PLANNER_PRESET_AUTOVACUUM_WORK_MEM) \
		postgres-benchmarks-compare

.PHONY: profile-env profile-pg-reset profile-pg-top profile-pg-wal profile-pg-io profile-pg-activity profile-perf-stat profile-perf-record profile-sudo-perf-stat-system profile-sudo-bpftrace-syscalls-workload profile-fuse-attach profile-indexer-attach profile-bpftrace-syscalls profile-bpftrace-read-hist profile-bpftrace-write-hist profile-local-baseline

profile-env:
	@mkdir -p $(ARTIFACTS_DIR)
	@{ \
		echo "commit=$$(git rev-parse HEAD 2>/dev/null || true)"; \
		echo "fod_version=$$(cat fod_version.txt 2>/dev/null || true)"; \
		echo "date=$$(date -Is)"; \
		echo "uname=$$(uname -a)"; \
		echo "cargo=$$(cargo --version 2>/dev/null || true)"; \
		echo "rustc=$$(rustc --version 2>/dev/null || true)"; \
		echo "psql=$$(psql --version 2>/dev/null || true)"; \
		echo "--- lscpu ---"; lscpu 2>/dev/null || true; \
		echo "--- free -h ---"; free -h 2>/dev/null || true; \
		echo "--- df -hT ---"; df -hT 2>/dev/null || true; \
	} > $(ARTIFACTS_DIR)/env.txt
	@printf '%s\n' "Wrote $(ARTIFACTS_DIR)/env.txt"

profile-pg-reset:
	$(PSQL) -f scripts/perf/pg/reset.sql

profile-pg-top:
	@mkdir -p $(ARTIFACTS_DIR)
	$(PSQL) -f scripts/perf/pg/top_statements.sql > $(ARTIFACTS_DIR)/pg_top_statements$(PROFILE_CAPTURE_SUFFIX).txt
	@cat $(ARTIFACTS_DIR)/pg_top_statements$(PROFILE_CAPTURE_SUFFIX).txt

profile-pg-wal:
	@mkdir -p $(ARTIFACTS_DIR)
	$(PSQL) -f scripts/perf/pg/wal_checkpointer.sql > $(ARTIFACTS_DIR)/pg_wal_checkpointer$(PROFILE_CAPTURE_SUFFIX).txt
	@cat $(ARTIFACTS_DIR)/pg_wal_checkpointer$(PROFILE_CAPTURE_SUFFIX).txt

profile-pg-io:
	@mkdir -p $(ARTIFACTS_DIR)
	@set +e; \
	$(PSQL) -f scripts/perf/pg/io_stats.sql > $(ARTIFACTS_DIR)/pg_io_stats$(PROFILE_CAPTURE_SUFFIX).txt 2>&1; \
	status=$$?; \
	cat $(ARTIFACTS_DIR)/pg_io_stats$(PROFILE_CAPTURE_SUFFIX).txt; \
	if [ "$$status" -ne 0 ]; then \
		if grep -q "pg_stat_io" $(ARTIFACTS_DIR)/pg_io_stats$(PROFILE_CAPTURE_SUFFIX).txt; then \
			echo "Optional pg_stat_io capture failed with status $$status; this usually means PostgreSQL does not expose pg_stat_io."; \
			exit 0; \
		fi; \
		exit "$$status"; \
	fi; \
	exit 0

profile-pg-activity:
	@mkdir -p $(ARTIFACTS_DIR)
	$(PSQL) -f scripts/perf/pg/activity.sql > $(ARTIFACTS_DIR)/pg_activity$(PROFILE_CAPTURE_SUFFIX).txt
	@cat $(ARTIFACTS_DIR)/pg_activity$(PROFILE_CAPTURE_SUFFIX).txt

profile-perf-stat:
	@mkdir -p $(ARTIFACTS_DIR)
	perf stat -d -d -d -r 5 -o $(ARTIFACTS_DIR)/perf-stat-$(PROFILE_WORKLOAD).txt -- $(PROFILE_MAKE) --no-print-directory $(PROFILE_WORKLOAD)

profile-perf-record:
	@mkdir -p $(ARTIFACTS_DIR)
	perf record -F $(PERF_FREQ) -g --call-graph dwarf,16384 -o $(ARTIFACTS_DIR)/perf-$(PROFILE_WORKLOAD).data -- $(PROFILE_MAKE) --no-print-directory $(PROFILE_WORKLOAD)
	@printf '%s\n' "Run: perf report -i $(ARTIFACTS_DIR)/perf-$(PROFILE_WORKLOAD).data"

profile-sudo-perf-stat-system:
	@mkdir -p $(ARTIFACTS_DIR)
	@set +e; \
	out="$(ARTIFACTS_DIR)/perf-stat-system-$(PROFILE_WORKLOAD)$(PROFILE_CAPTURE_SUFFIX).txt"; \
	status_file="$(ARTIFACTS_DIR)/.profile-workload-status-$$$$"; \
	rm -f "$$status_file"; \
	( sleep 1; $(PROFILE_MAKE) --no-print-directory $(PROFILE_WORKLOAD); echo "$$?" > "$$status_file" ) & \
	workload_pid="$$!"; \
	$(PROFILE_SUDO) perf stat -a -d -d -d -o "$$out" -- sh -c 'sleep 1; while [ ! -f "$$1" ]; do sleep 0.2; done' sh "$$status_file"; \
	perf_status="$$?"; \
	wait "$$workload_pid"; \
	workload_status="$$(cat "$$status_file" 2>/dev/null || echo 1)"; \
	rm -f "$$status_file"; \
	$(PROFILE_SUDO) chown "$$(id -u):$$(id -g)" "$$out" 2>/dev/null || true; \
	printf 'workload_status=%s\nperf_status=%s\n' "$$workload_status" "$$perf_status" >> "$$out"; \
	cat "$$out"; \
	if [ "$$workload_status" -ne 0 ]; then exit "$$workload_status"; fi; \
	if [ "$$perf_status" -ne 0 ]; then exit "$$perf_status"; fi

profile-sudo-bpftrace-syscalls-workload:
	@mkdir -p $(ARTIFACTS_DIR)
	@set +e; \
	out="$(ARTIFACTS_DIR)/bpftrace-syscalls-$(PROFILE_WORKLOAD)$(PROFILE_CAPTURE_SUFFIX).txt"; \
	$(PROFILE_SUDO) timeout $(PROFILE_SECONDS)s bpftrace scripts/perf/bpftrace/syscalls_by_comm.bt > "$$out" 2>&1 & \
	trace_pid="$$!"; \
	sleep 1; \
	$(PROFILE_MAKE) --no-print-directory $(PROFILE_WORKLOAD); \
	workload_status="$$?"; \
	wait "$$trace_pid"; \
	trace_status="$$?"; \
	printf 'workload_status=%s\nbpftrace_status=%s\n' "$$workload_status" "$$trace_status" >> "$$out"; \
	cat "$$out"; \
	if [ "$$workload_status" -ne 0 ]; then exit "$$workload_status"; fi; \
	if [ "$$trace_status" -ne 0 ] && [ "$$trace_status" -ne 124 ]; then exit "$$trace_status"; fi

profile-fuse-attach:
	@test -n "$(PROFILE_PID)" || { echo "Set PROFILE_PID to fod-rust-fuse PID"; exit 2; }
	@mkdir -p $(ARTIFACTS_DIR)
	sudo perf record -F $(PERF_FREQ) -g --call-graph dwarf,16384 -p $(PROFILE_PID) -o $(ARTIFACTS_DIR)/perf-fuse-attach.data -- sleep $(PROFILE_SECONDS)

profile-indexer-attach:
	@test -n "$(PROFILE_PID)" || { echo "Set PROFILE_PID to fod-indexer PID"; exit 2; }
	@mkdir -p $(ARTIFACTS_DIR)
	sudo perf record -F $(PERF_FREQ) -g --call-graph dwarf,16384 -p $(PROFILE_PID) -o $(ARTIFACTS_DIR)/perf-indexer-attach.data -- sleep $(PROFILE_SECONDS)

profile-bpftrace-syscalls:
	@mkdir -p $(ARTIFACTS_DIR)
	sudo timeout $(PROFILE_SECONDS)s bpftrace scripts/perf/bpftrace/syscalls_by_comm.bt | tee $(ARTIFACTS_DIR)/bpftrace-syscalls.txt

profile-bpftrace-read-hist:
	@mkdir -p $(ARTIFACTS_DIR)
	sudo timeout $(PROFILE_SECONDS)s bpftrace scripts/perf/bpftrace/read_size_hist.bt | tee $(ARTIFACTS_DIR)/bpftrace-read-hist.txt

profile-bpftrace-write-hist:
	@mkdir -p $(ARTIFACTS_DIR)
	sudo timeout $(PROFILE_SECONDS)s bpftrace scripts/perf/bpftrace/write_size_hist.bt | tee $(ARTIFACTS_DIR)/bpftrace-write-hist.txt

profile-local-baseline: profile-env profile-pg-reset
	$(PROFILE_MAKE) --no-print-directory PROFILE_RUN_ID=$(PROFILE_RUN_ID) PROFILE_HOST=$(PROFILE_HOST) PROFILE_WORKLOAD=$(PROFILE_WORKLOAD) $(PROFILE_WORKLOAD)
	$(PROFILE_MAKE) --no-print-directory PROFILE_RUN_ID=$(PROFILE_RUN_ID) PROFILE_HOST=$(PROFILE_HOST) PROFILE_CAPTURE_LABEL=$(PROFILE_CAPTURE_LABEL) profile-pg-top
	$(PROFILE_MAKE) --no-print-directory PROFILE_RUN_ID=$(PROFILE_RUN_ID) PROFILE_HOST=$(PROFILE_HOST) PROFILE_CAPTURE_LABEL=$(PROFILE_CAPTURE_LABEL) profile-pg-wal

db-shell:
	$(COMPOSE_RUN) -f $(COMPOSE_FILE) exec postgres psql -U $(POSTGRES_USER) -d $(POSTGRES_DB)

qnap-config-show:
	@$(MAKE) QNAP=1 $(FOD_REMOTE_PG_ENV) qnap-config-show-inner

qnap-config-show-inner:
	@printf '%s\n' \
		'QNAP transport preset:' \
		"  QNAP=$(if $(QNAP_ENABLED),1,0)" \
		"  DOCKER_HOST=$(if $(QNAP_ENABLED),$(QNAP_DOCKER_HOST),<local docker>)" \
		"  DOCKER_TLS_VERIFY=$(if $(QNAP_ENABLED),$(QNAP_DOCKER_TLS_VERIFY),<default>)" \
		"  DOCKER_CERT_PATH=$(if $(QNAP_ENABLED),$(QNAP_DOCKER_CERT_PATH),<default>)" \
		"  FOD_PG_HOST=$(FOD_PG_HOST)" \
		"  FOD_PG_PORT=$(FOD_PG_PORT)" \
		"  FOD_PG_DBNAME=$(FOD_PG_DBNAME)" \
		"  FOD_PG_USER=$(FOD_PG_USER)" \
		"  FOD_PG_PASSWORD=$(FOD_PG_PASSWORD)" \
		'PostgreSQL server tuning preset:' \
		"  POSTGRES_SHARED_PRELOAD_LIBRARIES=$(POSTGRES_SHARED_PRELOAD_LIBRARIES)" \
		"  POSTGRES_SHARED_BUFFERS=$(if $(strip $(POSTGRES_SHARED_BUFFERS)),$(POSTGRES_SHARED_BUFFERS),<default>)" \
		"  POSTGRES_MAX_CONNECTIONS=$(if $(strip $(POSTGRES_MAX_CONNECTIONS)),$(POSTGRES_MAX_CONNECTIONS),<default>)" \
		"  POSTGRES_MAX_WAL_SIZE=$(if $(strip $(POSTGRES_MAX_WAL_SIZE)),$(POSTGRES_MAX_WAL_SIZE),<default>)" \
		"  POSTGRES_CHECKPOINT_TIMEOUT=$(if $(strip $(POSTGRES_CHECKPOINT_TIMEOUT)),$(POSTGRES_CHECKPOINT_TIMEOUT),<default>)" \
		"  POSTGRES_CHECKPOINT_COMPLETION_TARGET=$(if $(strip $(POSTGRES_CHECKPOINT_COMPLETION_TARGET)),$(POSTGRES_CHECKPOINT_COMPLETION_TARGET),<default>)" \
		"  POSTGRES_WAL_COMPRESSION=$(if $(strip $(POSTGRES_WAL_COMPRESSION)),$(POSTGRES_WAL_COMPRESSION),<default>)" \
		"  POSTGRES_RANDOM_PAGE_COST=$(if $(strip $(POSTGRES_RANDOM_PAGE_COST)),$(POSTGRES_RANDOM_PAGE_COST),<default>)" \
		"  POSTGRES_EFFECTIVE_CACHE_SIZE=$(if $(strip $(POSTGRES_EFFECTIVE_CACHE_SIZE)),$(POSTGRES_EFFECTIVE_CACHE_SIZE),<default>)" \
		"  POSTGRES_MAINTENANCE_WORK_MEM=$(if $(strip $(POSTGRES_MAINTENANCE_WORK_MEM)),$(POSTGRES_MAINTENANCE_WORK_MEM),<default>)" \
		"  POSTGRES_AUTOVACUUM_MAX_WORKERS=$(if $(strip $(POSTGRES_AUTOVACUUM_MAX_WORKERS)),$(POSTGRES_AUTOVACUUM_MAX_WORKERS),<default>)" \
		"  POSTGRES_AUTOVACUUM_WORK_MEM=$(if $(strip $(POSTGRES_AUTOVACUUM_WORK_MEM)),$(POSTGRES_AUTOVACUUM_WORK_MEM),<default>)"

postgres-config-show:
	@printf '%s\n' \
		'PostgreSQL server tuning preset:' \
		"  POSTGRES_SHARED_PRELOAD_LIBRARIES=$(POSTGRES_SHARED_PRELOAD_LIBRARIES)" \
		"  POSTGRES_SHARED_BUFFERS=$(if $(strip $(POSTGRES_SHARED_BUFFERS)),$(POSTGRES_SHARED_BUFFERS),<default>)" \
		"  POSTGRES_MAX_CONNECTIONS=$(if $(strip $(POSTGRES_MAX_CONNECTIONS)),$(POSTGRES_MAX_CONNECTIONS),<default>)" \
		"  POSTGRES_MAX_WAL_SIZE=$(if $(strip $(POSTGRES_MAX_WAL_SIZE)),$(POSTGRES_MAX_WAL_SIZE),<default>)" \
		"  POSTGRES_CHECKPOINT_TIMEOUT=$(if $(strip $(POSTGRES_CHECKPOINT_TIMEOUT)),$(POSTGRES_CHECKPOINT_TIMEOUT),<default>)" \
		"  POSTGRES_CHECKPOINT_COMPLETION_TARGET=$(if $(strip $(POSTGRES_CHECKPOINT_COMPLETION_TARGET)),$(POSTGRES_CHECKPOINT_COMPLETION_TARGET),<default>)" \
		"  POSTGRES_WAL_COMPRESSION=$(if $(strip $(POSTGRES_WAL_COMPRESSION)),$(POSTGRES_WAL_COMPRESSION),<default>)" \
		"  POSTGRES_RANDOM_PAGE_COST=$(if $(strip $(POSTGRES_RANDOM_PAGE_COST)),$(POSTGRES_RANDOM_PAGE_COST),<default>)" \
		"  POSTGRES_EFFECTIVE_CACHE_SIZE=$(if $(strip $(POSTGRES_EFFECTIVE_CACHE_SIZE)),$(POSTGRES_EFFECTIVE_CACHE_SIZE),<default>)" \
		"  POSTGRES_MAINTENANCE_WORK_MEM=$(if $(strip $(POSTGRES_MAINTENANCE_WORK_MEM)),$(POSTGRES_MAINTENANCE_WORK_MEM),<default>)" \
		"  POSTGRES_AUTOVACUUM_MAX_WORKERS=$(if $(strip $(POSTGRES_AUTOVACUUM_MAX_WORKERS)),$(POSTGRES_AUTOVACUUM_MAX_WORKERS),<default>)" \
		"  POSTGRES_AUTOVACUUM_WORK_MEM=$(if $(strip $(POSTGRES_AUTOVACUUM_WORK_MEM)),$(POSTGRES_AUTOVACUUM_WORK_MEM),<default>)"

qnap-up:
	@$(MAKE) QNAP=1 $(FOD_REMOTE_PG_ENV) up

qnap-down:
	@$(MAKE) QNAP=1 $(FOD_REMOTE_PG_ENV) down

qnap-restart:
	@$(MAKE) QNAP=1 $(FOD_REMOTE_PG_ENV) restart

qnap-logs:
	@$(MAKE) QNAP=1 $(FOD_REMOTE_PG_ENV) logs

qnap-wait:
	@$(MAKE) QNAP=1 $(FOD_REMOTE_PG_ENV) wait

qnap-init:
	@$(MAKE) QNAP=1 $(FOD_REMOTE_PG_ENV) init

qnap-smoke:
	@$(MAKE) QNAP=1 $(FOD_REMOTE_PG_ENV) smoke

qnap-reset:
	@$(MAKE) QNAP=1 $(FOD_REMOTE_PG_ENV) reset

qnap-mount:
	@$(MAKE) QNAP=1 $(FOD_REMOTE_PG_ENV) mount

clean:
	rm -rf $(VENV_DIR)

.PHONY: profile-pg-data-blocks-semantics profile-pg-data-blocks-merge-explain profile-pg-data-blocks-bloat

profile-pg-data-blocks-semantics:
	@mkdir -p $(ARTIFACTS_DIR)
	$(PSQL) -f scripts/perf/pg/data_blocks_semantics.sql > $(ARTIFACTS_DIR)/pg_data_blocks_semantics$(PROFILE_CAPTURE_SUFFIX).txt
	@cat $(ARTIFACTS_DIR)/pg_data_blocks_semantics$(PROFILE_CAPTURE_SUFFIX).txt

profile-pg-data-blocks-merge-explain:
	@mkdir -p $(ARTIFACTS_DIR)
	$(PSQL) -f scripts/perf/pg/explain_data_blocks_merge.sql > $(ARTIFACTS_DIR)/pg_data_blocks_merge_explain$(PROFILE_CAPTURE_SUFFIX).txt
	@cat $(ARTIFACTS_DIR)/pg_data_blocks_merge_explain$(PROFILE_CAPTURE_SUFFIX).txt

profile-pg-data-blocks-bloat:
	@mkdir -p $(ARTIFACTS_DIR)
	$(PSQL) -f scripts/perf/pg/data_blocks_bloat.sql > $(ARTIFACTS_DIR)/pg_data_blocks_bloat$(PROFILE_CAPTURE_SUFFIX).txt
	@cat $(ARTIFACTS_DIR)/pg_data_blocks_bloat$(PROFILE_CAPTURE_SUFFIX).txt
