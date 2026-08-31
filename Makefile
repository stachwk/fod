# Public FOD Make interface.
#
# Naming convention:
#   <area>-<variant>-<action>
#
# The historical implementation is kept verbatim under make/fod-internal.mk.
# Public commands delegate to the internal dispatcher so recipe behaviour stays
# unchanged while the user-facing target namespace remains small and coherent.

.DEFAULT_GOAL := help

FOD_INTERNAL_MAKEFILE := make/fod-internal-entry.mk
FOD_INTERNAL_MAKE := $(MAKE) -f $(FOD_INTERNAL_MAKEFILE)

# Forward one public target to one internal implementation target.
define FOD_FORWARD_TARGET
.PHONY: $(1)
$(1):
	@$(FOD_INTERNAL_MAKE) $(2)
endef

# Build and Rust toolchain.
$(eval $(call FOD_FORWARD_TARGET,rust-toolchain-production-check,rust-production-toolchain-check))
$(eval $(call FOD_FORWARD_TARGET,rust-profile-show,cargo-profile-show))
$(eval $(call FOD_FORWARD_TARGET,build-fod-runtime,build-runtime))
$(eval $(call FOD_FORWARD_TARGET,build-fod-debug,build-debug))
$(eval $(call FOD_FORWARD_TARGET,build-fod-runtime-shm,build-runtime-shm))
$(eval $(call FOD_FORWARD_TARGET,build-fod-debug-shm,build-debug-shm))
$(eval $(call FOD_FORWARD_TARGET,build-libfod,build-libfod))

# Cargo target/cache management.
$(eval $(call FOD_FORWARD_TARGET,target-cargo-info,cargo-target-info))
$(eval $(call FOD_FORWARD_TARGET,target-cargo-preflight,cargo-target-preflight))
$(eval $(call FOD_FORWARD_TARGET,target-shm-status,shm-target-status))
$(eval $(call FOD_FORWARD_TARGET,target-shm-clean,shm-target-clean))
$(eval $(call FOD_FORWARD_TARGET,target-disk-status,target-disk-status))
$(eval $(call FOD_FORWARD_TARGET,target-disk-clean-plan,target-disk-clean-plan))
$(eval $(call FOD_FORWARD_TARGET,target-disk-clean,target-disk-clean))
$(eval $(call FOD_FORWARD_TARGET,target-aux-status,target-aux-status))
$(eval $(call FOD_FORWARD_TARGET,target-aux-clean-plan,target-aux-clean-plan))
$(eval $(call FOD_FORWARD_TARGET,target-aux-clean,target-aux-clean))

# Test/development environment and build dependency discovery.
$(eval $(call FOD_FORWARD_TARGET,env-test-prepare,deps))
$(eval $(call FOD_FORWARD_TARGET,env-test-clean,clean))
$(eval $(call FOD_FORWARD_TARGET,deps-build-debian-show,deps-ubuntu))
$(eval $(call FOD_FORWARD_TARGET,deps-build-redhat-show,deps-redhat))

# PostgreSQL lifecycle. Use QNAP=1 with the same targets for the QNAP backend.
$(eval $(call FOD_FORWARD_TARGET,postgres-up,up))
$(eval $(call FOD_FORWARD_TARGET,postgres-down,down))
$(eval $(call FOD_FORWARD_TARGET,postgres-restart,restart))
$(eval $(call FOD_FORWARD_TARGET,postgres-logs,logs))
$(eval $(call FOD_FORWARD_TARGET,postgres-wait,wait))
$(eval $(call FOD_FORWARD_TARGET,postgres-wait-client,wait-client))
$(eval $(call FOD_FORWARD_TARGET,postgres-reset,reset))
$(eval $(call FOD_FORWARD_TARGET,postgres-smoke,smoke))
$(eval $(call FOD_FORWARD_TARGET,postgres-shell,db-shell))
$(eval $(call FOD_FORWARD_TARGET,postgres-config-show,postgres-config-show))
$(eval $(call FOD_FORWARD_TARGET,postgres-qnap-config-show,qnap-config-show))
$(eval $(call FOD_FORWARD_TARGET,postgres-enable-pg-stat-statements,enable-pg-stat-statements))

# FOD schema and mount lifecycle.
$(eval $(call FOD_FORWARD_TARGET,fod-init,init))
$(eval $(call FOD_FORWARD_TARGET,fod-remote-init,init-qnap))
$(eval $(call FOD_FORWARD_TARGET,fod-mount,mount))
$(eval $(call FOD_FORWARD_TARGET,fod-remote-mount,mount-qnap))
$(eval $(call FOD_FORWARD_TARGET,fod-user-mount,mount-user))
$(eval $(call FOD_FORWARD_TARGET,fod-demo,demo))
$(eval $(call FOD_FORWARD_TARGET,fod-unmount,unmount))

# Live runtime configuration.
$(eval $(call FOD_FORWARD_TARGET,runtime-config-path-show,config-show))
$(eval $(call FOD_FORWARD_TARGET,runtime-config-list,change-runtime-list))
$(eval $(call FOD_FORWARD_TARGET,runtime-config-get,change-runtime-get))
$(eval $(call FOD_FORWARD_TARGET,runtime-config-set,change-runtime-set))
$(eval $(call FOD_FORWARD_TARGET,runtime-config-reload,reload-runtime))

# Installation.
$(eval $(call FOD_FORWARD_TARGET,config-secret-check,warn-config-secret))
$(eval $(call FOD_FORWARD_TARGET,install-config-system,install-config))
$(eval $(call FOD_FORWARD_TARGET,install-config-user,install-config-user))
$(eval $(call FOD_FORWARD_TARGET,install-mount-helper,install-mount-helper))
$(eval $(call FOD_FORWARD_TARGET,install-root-binaries,install-root-scripts))
$(eval $(call FOD_FORWARD_TARGET,install-root,install-on-root))
$(eval $(call FOD_FORWARD_TARGET,uninstall-root,uninstall-on-root))
$(eval $(call FOD_FORWARD_TARGET,install-root-venv,install-on-root-venv))

# Indexer.
$(eval $(call FOD_FORWARD_TARGET,indexer-run,indexer))
$(eval $(call FOD_FORWARD_TARGET,indexer-materialize,indexer-import))

# Benchmarks.
$(eval $(call FOD_FORWARD_TARGET,benchmark-all,benchmarks))
$(eval $(call FOD_FORWARD_TARGET,benchmark-postgres,postgres-benchmarks))
$(eval $(call FOD_FORWARD_TARGET,benchmark-postgres-local,postgres-benchmarks-local))
$(eval $(call FOD_FORWARD_TARGET,benchmark-postgres-qnap,postgres-benchmarks-qnap))
$(eval $(call FOD_FORWARD_TARGET,benchmark-postgres-checkpoint,postgres-benchmarks-checkpoint))
$(eval $(call FOD_FORWARD_TARGET,benchmark-postgres-compare,postgres-benchmarks-compare))
$(eval $(call FOD_FORWARD_TARGET,benchmark-postgres-wal-preset,postgres-benchmarks-wal-preset))
$(eval $(call FOD_FORWARD_TARGET,benchmark-postgres-planner-preset,postgres-benchmarks-planner-preset))

# Docker labs and cleanup.
$(eval $(call FOD_FORWARD_TARGET,docker-perf-clean,docker-perf-clean))
$(eval $(call FOD_FORWARD_TARGET,docker-selinux-acl-up,docker-selinux-acl-up))
$(eval $(call FOD_FORWARD_TARGET,docker-selinux-acl-wait,docker-selinux-acl-wait))
$(eval $(call FOD_FORWARD_TARGET,docker-selinux-acl-down,docker-selinux-acl-down))
$(eval $(call FOD_FORWARD_TARGET,docker-selinux-acl-shell,docker-selinux-acl-shell))
$(eval $(call FOD_FORWARD_TARGET,docker-selinux-acl-smoke,docker-selinux-acl-smoke))

# Docker images.
$(eval $(call FOD_FORWARD_TARGET,docker-postgres-8k-build,docker-postgres-8k-build))
$(eval $(call FOD_FORWARD_TARGET,docker-postgres-8k-publish,docker-postgres-8k-publish))
$(eval $(call FOD_FORWARD_TARGET,docker-postgres-32k-build,docker-postgres-32k-build))
$(eval $(call FOD_FORWARD_TARGET,docker-postgres-32k-publish,docker-postgres-32k-publish))
$(eval $(call FOD_FORWARD_TARGET,docker-postgres-all-build,docker-postgres-all-build))
$(eval $(call FOD_FORWARD_TARGET,docker-postgres-all-publish,docker-postgres-all-publish))
$(eval $(call FOD_FORWARD_TARGET,docker-fod-client-build,docker-fod-client-build))
$(eval $(call FOD_FORWARD_TARGET,docker-fod-client-publish,docker-fod-client-publish))

# Rust comparison/diagnostic tools.
$(eval $(call FOD_FORWARD_TARGET,rust-toolchain-msrv-check,rust-msrv-check))
$(eval $(call FOD_FORWARD_TARGET,rust-toolchain-candidate-check,rust-candidate-check))
$(eval $(call FOD_FORWARD_TARGET,rust-toolchain-candidate-clippy,rust-candidate-clippy))
$(eval $(call FOD_FORWARD_TARGET,rust-toolchain-benchmark-plan,rust-toolchain-benchmark-plan))
$(eval $(call FOD_FORWARD_TARGET,rust-toolchain-benchmark-run,rust-toolchain-benchmark))

# Rocky Linux SELinux validation.
$(eval $(call FOD_FORWARD_TARGET,selinux-rocky-deps-show,rocky-selinux-deps))
$(eval $(call FOD_FORWARD_TARGET,selinux-rocky-install-deps,rocky-selinux-install-deps))
$(eval $(call FOD_FORWARD_TARGET,selinux-rocky-postgres-prepare,rocky-selinux-postgres-prepare))
$(eval $(call FOD_FORWARD_TARGET,selinux-rocky-preflight,rocky-selinux-preflight))
$(eval $(call FOD_FORWARD_TARGET,selinux-rocky-prepare,rocky-selinux-prepare))
$(eval $(call FOD_FORWARD_TARGET,selinux-rocky-test-strict,rocky-selinux-test-strict))
$(eval $(call FOD_FORWARD_TARGET,selinux-rocky-test-operational,rocky-selinux-test-operational))
$(eval $(call FOD_FORWARD_TARGET,selinux-rocky-remote-install-deps,remote-rocky-selinux-install-deps))
$(eval $(call FOD_FORWARD_TARGET,selinux-rocky-remote-sync,remote-rocky-selinux-sync))
$(eval $(call FOD_FORWARD_TARGET,selinux-rocky-remote-postgres-prepare,remote-rocky-selinux-postgres-prepare))
$(eval $(call FOD_FORWARD_TARGET,selinux-rocky-remote-preflight,remote-rocky-selinux-preflight))
$(eval $(call FOD_FORWARD_TARGET,selinux-rocky-remote-prepare,remote-rocky-selinux-prepare))
$(eval $(call FOD_FORWARD_TARGET,selinux-rocky-remote-test-strict,remote-rocky-selinux-test-strict))
$(eval $(call FOD_FORWARD_TARGET,selinux-rocky-remote-test-operational,remote-rocky-selinux-test-operational))

# Native packages. Public names describe format and action explicitly.
$(eval $(call FOD_FORWARD_TARGET,package-build-artifacts,package-artifacts))
$(eval $(call FOD_FORWARD_TARGET,package-plan,package-plan))
$(eval $(call FOD_FORWARD_TARGET,package-native-build,package-native))
$(eval $(call FOD_FORWARD_TARGET,package-deb-build,package-ubuntu))
$(eval $(call FOD_FORWARD_TARGET,package-rpm-build,package-rocky))
$(eval $(call FOD_FORWARD_TARGET,package-clean,package-clean))
$(eval $(call FOD_FORWARD_TARGET,package-deb-deps-show,package-deps-ubuntu))
$(eval $(call FOD_FORWARD_TARGET,package-rpm-deps-show,package-deps-redhat))

# Existing test-* and profile-* targets are already consistently named. Export
# real targets automatically, while deliberately excluding historical aliases
# and targets renamed above.
FOD_INTERNAL_TEST_TARGETS := $(shell awk '/^\.PHONY:/ {for (i=2; i<=NF; i++) if ($$i ~ /^test-[[:alnum:]_.-]+$$/) print $$i} /^test-[[:alnum:]_.-]+:/ {t=$$1; sub(/:.*/, "", t); print t}' make/fod-internal.mk make/fod-extra-internal.mk packaging/fod-packaging.mk | sort -u)
FOD_OBSOLETE_TEST_TARGETS := \
	test-all-shm \
	test-all-full-shm \
	test-copy-dedupe-benchmark \
	test-ext4-vs-fod-permissions \
	test-fod-indexer-materialize \
	test-fuse-test-cleanup \
	test-fuse-test-cleanup-policy \
	test-native-package-policy \
	test-postgresql-requirements
FOD_SPECIAL_TEST_TARGETS := test-all test-all-full
FOD_PUBLIC_TEST_TARGETS := $(filter-out $(FOD_OBSOLETE_TEST_TARGETS) $(FOD_SPECIAL_TEST_TARGETS),$(FOD_INTERNAL_TEST_TARGETS))
$(foreach target,$(FOD_PUBLIC_TEST_TARGETS),$(eval $(call FOD_FORWARD_TARGET,$(target),$(target))))

FOD_INTERNAL_PROFILE_TARGETS := $(shell awk '/^\.PHONY:/ {for (i=2; i<=NF; i++) if ($$i ~ /^profile-[[:alnum:]_.-]+$$/) print $$i} /^profile-[[:alnum:]_.-]+:/ {t=$$1; sub(/:.*/, "", t); print t}' make/fod-internal.mk | sort -u)
$(foreach target,$(FOD_INTERNAL_PROFILE_TARGETS),$(eval $(call FOD_FORWARD_TARGET,$(target),$(target))))

# Renamed test/policy entry points.
.PHONY: test-shm-all test-shm-all-full test-fuse-cleanup test-fuse-cleanup-policy test-package-native-policy test-docker-postgres-policy test-docker-fod-client-policy test-make-target-naming-policy test-all test-all-full

test-shm-all:
	@$(FOD_INTERNAL_MAKE) test-all-shm

test-shm-all-full:
	@$(FOD_INTERNAL_MAKE) test-all-full-shm

test-fuse-cleanup:
	@$(FOD_INTERNAL_MAKE) test-fuse-test-cleanup

test-fuse-cleanup-policy:
	@$(FOD_INTERNAL_MAKE) test-fuse-test-cleanup-policy

test-package-native-policy:
	@$(FOD_INTERNAL_MAKE) test-native-package-policy

test-docker-postgres-policy:
	@$(FOD_INTERNAL_MAKE) docker-postgres-test-policy

test-docker-fod-client-policy:
	@$(FOD_INTERNAL_MAKE) docker-fod-client-test-policy

test-make-target-naming-policy:
	@bash tests/test_make_target_naming_policy.sh

test-all: test-make-target-naming-policy
	@$(FOD_INTERNAL_MAKE) test-all

test-all-full: test-make-target-naming-policy
	@$(FOD_INTERNAL_MAKE) test-all-full

.PHONY: help help-tests help-profiles
help:
	@printf '%s\n' \
		'FOD Make targets use: <area>-<variant>-<action>' \
		'' \
		'Build:' \
		'  build-fod-runtime | build-fod-debug | build-libfod' \
		'  rust-toolchain-production-check | rust-profile-show' \
		'' \
		'PostgreSQL (add QNAP=1 to select QNAP):' \
		'  postgres-up | postgres-down | postgres-restart | postgres-logs' \
		'  postgres-wait | postgres-wait-client | postgres-smoke | postgres-reset' \
		'  postgres-shell | postgres-config-show | postgres-qnap-config-show' \
		'' \
		'FOD:' \
		'  fod-init | fod-remote-init | fod-mount | fod-remote-mount' \
		'  fod-user-mount | fod-demo | fod-unmount' \
		'' \
		'Runtime config:' \
		'  runtime-config-path-show | runtime-config-list | runtime-config-get' \
		'  runtime-config-set | runtime-config-reload' \
		'' \
		'Benchmarks:' \
		'  benchmark-all | benchmark-postgres | benchmark-postgres-local' \
		'  benchmark-postgres-qnap | benchmark-postgres-compare' \
		'' \
		'Docker images:' \
		'  docker-postgres-32k-build | docker-postgres-32k-publish' \
		'  docker-postgres-8k-build | docker-postgres-8k-publish' \
		'  docker-fod-client-build | docker-fod-client-publish' \
		'' \
		'Packages:' \
		'  package-plan | package-native-build | package-deb-build | package-rpm-build' \
		'' \
		'Installation:' \
		'  install-config-system | install-config-user | install-root | uninstall-root' \
		'' \
		'Target cache:' \
		'  target-cargo-info | target-disk-status | target-shm-status | target-aux-status' \
		'' \
		'Tests/profiles:' \
		'  test-all | test-all-full | help-tests | help-profiles'

help-tests:
	@printf '%s\n' $(sort $(FOD_PUBLIC_TEST_TARGETS) test-shm-all test-shm-all-full test-fuse-cleanup test-fuse-cleanup-policy test-package-native-policy test-docker-postgres-policy test-docker-fod-client-policy test-make-target-naming-policy test-all test-all-full)

help-profiles:
	@printf '%s\n' $(sort $(FOD_INTERNAL_PROFILE_TARGETS))
