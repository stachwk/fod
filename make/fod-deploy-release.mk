# Release-facing Docker deployment defaults and persistent boot integration.
# The normal `make` entry point pins the FOD client to the exact repository
# version. The :3.4 series alias remains available for manual/compatibility use.

FOD_RELEASE_VERSION := $(strip $(shell cat fod_version.txt))
FOD_DOCKER_DEPLOY_CLIENT_IMAGE ?= ghcr.io/stachwk/fod-client:$(FOD_RELEASE_VERSION)
FOD_CLIENT_IMAGE_VERSION ?= $(FOD_RELEASE_VERSION)
export FOD_DOCKER_DEPLOY_CLIENT_IMAGE
export FOD_CLIENT_IMAGE_VERSION

.PHONY: \
	docker-deploy-systemd-plan \
	docker-deploy-systemd-render \
	docker-deploy-systemd-install \
	docker-deploy-systemd-status \
	docker-deploy-systemd-smoke \
	docker-deploy-systemd-restart \
	docker-deploy-systemd-uninstall \
	test-cargo-lock-integrity \
	test-docker-deploy-systemd-policy \
	help-docker-deploy-release

docker-deploy-systemd-plan:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy_systemd.sh plan

docker-deploy-systemd-render:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy_systemd.sh render

docker-deploy-systemd-install:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy_systemd.sh install

docker-deploy-systemd-status:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy_systemd.sh status

docker-deploy-systemd-smoke:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy_systemd.sh smoke

docker-deploy-systemd-restart:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy_systemd.sh restart

docker-deploy-systemd-uninstall:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy_systemd.sh uninstall

test-cargo-lock-integrity:
	@python3 tests/test_cargo_lock_integrity.py

# `test-version` is defined by the public/internal Make interface. Add the
# lock integrity prerequisite here so hidden/non-ASCII checksum corruption is
# rejected before Cargo resolves dependencies.
test-version: test-cargo-lock-integrity

test-docker-deploy-systemd-policy:
	@bash tests/test_docker_deploy_systemd_policy.sh

test-all: test-cargo-lock-integrity test-docker-deploy-systemd-policy
test-all-full: test-cargo-lock-integrity test-docker-deploy-systemd-policy

help: help-docker-deploy-release
help-tests: help-docker-deploy-release-tests
.PHONY: help-docker-deploy-release-tests
help-docker-deploy-release-tests:
	@printf '%s\n' test-cargo-lock-integrity test-docker-deploy-systemd-policy

help-docker-deploy-release:
	@printf '%s\n' \
		'Docker release defaults:' \
		'  exact FOD image: $(FOD_DOCKER_DEPLOY_CLIENT_IMAGE)' \
		'  docker-deploy-systemd-plan | docker-deploy-systemd-install | docker-deploy-systemd-status' \
		'  docker-deploy-systemd-smoke | docker-deploy-systemd-restart | docker-deploy-systemd-uninstall' \
		''
