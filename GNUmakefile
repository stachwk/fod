ifeq ($(FOD_INTERNAL_MAKE),1)
include make/fod-internal-entry.mk
else
include Makefile
include make/fod-deploy-public.mk
include make/fod-deploy-release.mk
endif
