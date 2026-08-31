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
	docker-deploy-fod-init \
	docker-deploy-fod-upgrade \
	docker-deploy-destroy \
	docker-deploy-single-install \
	docker-deploy-one-replica-install \
	docker-deploy-two-replicas-install \
	test-docker-deploy-policy \
	help-docker-deploy \
	help-docker-deploy-summary \
	help-docker-deploy-tests

docker-deploy-plan:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh plan

docker-deploy-render:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh render

docker-deploy-pull:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh pull

docker-deploy-install:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh install

docker-deploy-up:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh up

docker-deploy-down:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh down

docker-deploy-status:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh status

docker-deploy-smoke:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh smoke

docker-deploy-logs:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh logs

docker-deploy-fod-init:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh fod-init

docker-deploy-fod-upgrade:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" bash scripts/docker_deploy.sh fod-upgrade

docker-deploy-destroy:
	@MASTERS="$(MASTERS)" SLAVES="$(SLAVES)" DESTROY="$(DESTROY)" bash scripts/docker_deploy.sh destroy

docker-deploy-single-install:
	@$(MAKE) --no-print-directory docker-deploy-install MASTERS=1 SLAVES=0

docker-deploy-one-replica-install:
	@$(MAKE) --no-print-directory docker-deploy-install MASTERS=1 SLAVES=1

docker-deploy-two-replicas-install:
	@$(MAKE) --no-print-directory docker-deploy-install MASTERS=1 SLAVES=2

test-docker-deploy-policy:
	@bash tests/test_docker_deploy_policy.sh

# Keep the deployment policy in the normal local gates.
test-all: test-docker-deploy-policy
test-all-full: test-docker-deploy-policy

# Extend existing help targets without replacing their recipes.
help: help-docker-deploy-summary
help-tests: help-docker-deploy-tests

help-docker-deploy-summary:
	@printf '%s\n' \
		'Docker deployment:' \
		'  docker-deploy-plan | docker-deploy-install | docker-deploy-status | docker-deploy-smoke' \
		'  use MASTERS=1 SLAVES=N; see: make help-docker-deploy' \
		''

help-docker-deploy-tests:
	@printf '%s\n' test-docker-deploy-policy

help-docker-deploy:
	@printf '%s\n' \
		'FOD final Docker deployment (PostgreSQL 16 / BLCKSZ=32K):' \
		'  make docker-deploy-plan MASTERS=1 SLAVES=2' \
		'  make docker-deploy-install MASTERS=1 SLAVES=2' \
		'  make docker-deploy-up MASTERS=1 SLAVES=2' \
		'  make docker-deploy-status MASTERS=1 SLAVES=2' \
		'  make docker-deploy-smoke MASTERS=1 SLAVES=2' \
		'  make docker-deploy-logs MASTERS=1 SLAVES=2' \
		'  make docker-deploy-down MASTERS=1 SLAVES=2' \
		'  make docker-deploy-fod-init MASTERS=1 SLAVES=2' \
		'  make docker-deploy-fod-upgrade MASTERS=1 SLAVES=2' \
		'  make docker-deploy-destroy MASTERS=1 SLAVES=2 DESTROY=YES' \
		'' \
		'Presets:' \
		'  docker-deploy-single-install        = 1 primary + 0 replicas' \
		'  docker-deploy-one-replica-install   = 1 primary + 1 replica' \
		'  docker-deploy-two-replicas-install  = 1 primary + 2 replicas' \
		'' \
		'MASTERS must equal 1; PostgreSQL multi-master is intentionally unsupported.'
