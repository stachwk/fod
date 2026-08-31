# Internal Make dispatcher. Not part of the public target namespace.
FOD_INTERNAL_MAKE := 1
export FOD_INTERNAL_MAKE
_FOD_BASE_MAKE := $(MAKE)
override MAKE := $(_FOD_BASE_MAKE) -f make/fod-internal-entry.mk

include make/fod-internal.mk
include make/fod-extra-internal.mk
include packaging/fod-packaging.mk
