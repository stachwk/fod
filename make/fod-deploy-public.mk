# Public Docker deployment targets.
# Loaded by GNUmakefile for normal user-facing `make` invocations.

MASTERS ?= 1
SLAVES ?= 1

.PHONY: \
	docker-deploy-plan \
	docker-deploy-render \
	docker-deploy-pull \
	docker-deploy-install \
	docker-deploy-up \
	docker-deploy-down \
	docker-deploy-status \
	docker-deploy-smoke \
	docker-deploy-logs \
	docker-deploy-schema-init \
	docker-deploy-schema-upgrade \
	docker-deploy-destroy \
	docker-deploy-fod-plan \
	docker-deploy-fod-render \
	docker-deploy-fod-preflight \
	docker-deploy-fod-host-prepare \
	docker-deploy-fod-install \
	docker-deploy-fod-up \
	docker-deploy-fod-down \
	docker-deploy-fod-status \
	docker-deploy-fod-smoke \
	docker-deploy-fod-logs \
	docker-deploy-fod-shell \
	docker-deploy-fod-diagnostics \
	docker-deploy-single-install \
	docker-deploy-one-replica-install \
	docker-deploy-two-replicas-install \
	test-docker-deploy-policy \
	test-docker-fod-install-policy \
	help-docker-deploy \
	help-docker-deploy-summary \
	help-docker-deploy-tests

docker-deploy-plan:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh plan
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_install.sh plan

docker-deploy-render:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh render
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_install.sh render

docker-deploy-pull:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh pull

# Final installation: PostgreSQL cluster + FOD schema + persistent FOD/FUSE client.
docker-deploy-install:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh install
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_start_guard.sh start

docker-deploy-up:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh up
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_start_guard.sh start

docker-deploy-down:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_install.sh down || true
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh down

docker-deploy-status:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh status
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_install.sh status

docker-deploy-smoke:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh smoke
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_install.sh smoke

docker-deploy-logs:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh logs

docker-deploy-schema-init:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh fod-init

docker-deploy-schema-upgrade:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh fod-upgrade

docker-deploy-destroy:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_install.sh down || true
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" DESTROY="$(DESTROY)" bash scripts/docker_deploy.sh destroy

# FOD container lifecycle, independently of the PostgreSQL lifecycle.
docker-deploy-fod-plan:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_install.sh plan

docker-deploy-fod-render:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_install.sh render

docker-deploy-fod-preflight:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_install.sh preflight

docker-deploy-fod-host-prepare:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_install.sh host-prepare

docker-deploy-fod-install:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_start_guard.sh install

docker-deploy-fod-up:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_start_guard.sh up

docker-deploy-fod-down:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_install.sh down

docker-deploy-fod-status:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_install.sh status

docker-deploy-fod-smoke:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_install.sh smoke

docker-deploy-fod-logs:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_install.sh logs

docker-deploy-fod-shell:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_install.sh shell

docker-deploy-fod-diagnostics:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_fod_start_guard.sh diagnostics

docker-deploy-single-install:
	@$(MAKE) --no-print-directory docker-deploy-install MASTERS=1 SLAVES=0

docker-deploy-one-replica-install:
	@$(MAKE) --no-print-directory docker-deploy-install MASTERS=1 SLAVES=1

docker-deploy-two-replicas-install:
	@$(MAKE) --no-print-directory docker-deploy-install MASTERS=1 SLAVES=2

test-docker-deploy-policy:
	@bash tests/test_docker_deploy_policy.sh

test-docker-fod-install-policy:
	@bash tests/test_docker_fod_install_policy.sh

# Keep deployment policies in the normal local gates.
test-all: test-docker-deploy-policy test-docker-fod-install-policy
test-all-full: test-docker-deploy-policy test-docker-fod-install-policy

# Extend existing help targets without replacing their recipes.
help: help-docker-deploy-summary
help-tests: help-docker-deploy-tests

help-docker-deploy-summary:
	@printf '%s\n' \
		'Docker deployment:' \
		'  docker-deploy-plan | docker-deploy-install | docker-deploy-status | docker-deploy-smoke' \
		'  docker-deploy-fod-install | docker-deploy-fod-status | docker-deploy-fod-logs' \
		'  use MASTERS=1 SLAVES=N; see: make help-docker-deploy' \
		''

help-docker-deploy-tests:
	@printf '%s\n' test-docker-deploy-policy test-docker-fod-install-policy

help-docker-deploy:
	@printf '%s\n' \
		'FOD final Docker deployment (PostgreSQL 16 / BLCKSZ=32K + FOD/FUSE client):' \
		'  make docker-deploy-plan MASTERS=1 SLAVES=2' \
		'  make docker-deploy-install MASTERS=1 SLAVES=2' \
		'  make docker-deploy-up MASTERS=1 SLAVES=2' \
		'  make docker-deploy-status MASTERS=1 SLAVES=2' \
		'  make docker-deploy-smoke MASTERS=1 SLAVES=2' \
		'  make docker-deploy-down MASTERS=1 SLAVES=2' \
		'  make docker-deploy-schema-init MASTERS=1 SLAVES=2' \
		'  make docker-deploy-schema-upgrade MASTERS=1 SLAVES=2' \
		'  make docker-deploy-destroy MASTERS=1 SLAVES=2 DESTROY=YES' \
		'' \
		'FOD container only:' \
		'  make docker-deploy-fod-plan MASTERS=1 SLAVES=2' \
		'  make docker-deploy-fod-preflight MASTERS=1 SLAVES=2' \
		'  make docker-deploy-fod-host-prepare MASTERS=1 SLAVES=2' \
		'  make docker-deploy-fod-install MASTERS=1 SLAVES=2' \
		'  make docker-deploy-fod-up MASTERS=1 SLAVES=2' \
		'  make docker-deploy-fod-status MASTERS=1 SLAVES=2' \
		'  make docker-deploy-fod-smoke MASTERS=1 SLAVES=2' \
		'  make docker-deploy-fod-logs MASTERS=1 SLAVES=2' \
		'  make docker-deploy-fod-diagnostics MASTERS=1 SLAVES=2' \
		'  make docker-deploy-fod-shell MASTERS=1 SLAVES=2' \
		'  make docker-deploy-fod-down MASTERS=1 SLAVES=2' \
		'' \
		'FOD startup guard:' \
		'  FOD_DOCKER_DEPLOY_FOD_START_TIMEOUT_SECONDS=30' \
		'  FOD_DOCKER_DEPLOY_FOD_HEALTH_TIMEOUT_SECONDS=90' \
		'  FOD_DOCKER_DEPLOY_FOD_APPARMOR=auto|unconfined|default' \
		'' \
		'FOD host mount:' \
		'  FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR=/path/on/host' \
		'  source mount propagation must be shared/rshared for FUSE propagation' \
		'' \
		'Presets:' \
		'  docker-deploy-single-install        = 1 primary + 0 replicas + FOD' \
		'  docker-deploy-one-replica-install   = 1 primary + 1 replica + FOD' \
		'  docker-deploy-two-replicas-install  = 1 primary + 2 replicas + FOD' \
		'' \
		'MASTERS must equal 1; PostgreSQL multi-master is intentionally unsupported.'