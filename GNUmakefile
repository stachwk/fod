include Makefile

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

# Keep the selective-cleanup policy regression in the normal gate without
# modifying the existing Makefile target definition.
test-all: test-target-aux-clean-policy
