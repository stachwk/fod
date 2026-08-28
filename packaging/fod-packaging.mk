# Native installation packages. Binary packages are built only on their target
# distribution family so linked glibc/libpq/libfuse dependencies match the host.
FOD_PACKAGE_ROOT ?= $(CURDIR)/target/packages
FOD_PACKAGE_NAME ?= fod
FOD_PACKAGE_RELEASE ?= 2
FOD_PACKAGE_MAINTAINER ?= FOD Project <33524981+stachwk@users.noreply.github.com>
FOD_PACKAGE_URL ?= https://github.com/stachwk/fod
FOD_PACKAGE_CONFIG_SOURCE ?= $(CURDIR)/fod_config.example.ini
UBUNTU_PACKAGE_BUILD_DEPS ?= dpkg-dev
REDHAT_PACKAGE_BUILD_DEPS ?= rpm-build

.PHONY: package-artifacts package-plan package-info package-native package-ubuntu package-deb package-rocky package-redhat package-rpm package-clean package-deps-ubuntu package-deps-redhat test-native-package-policy

package-artifacts: rust-production-toolchain-check
	@set -eu; \
	if [ "$(FOD_CARGO_PROFILE)" != "release-lto" ]; then \
		printf 'Official FOD packages require FOD_CARGO_PROFILE=release-lto; got %s\n' "$(FOD_CARGO_PROFILE)" >&2; \
		exit 2; \
	fi
	$(CARGO_BUILD_INSTALL_ROOT)
	$(CARGO_BUILD_LIBFOD) $(FOD_RELEASE_FLAG) --lib
	@test -x "$(FOD_BOOTSTRAP_PROFILE_BIN)"
	@test -x "$(FOD_MKFS_PROFILE_BIN)"
	@test -x "$(FOD_CHANGE_PROFILE_BIN)"
	@test -x "$(FOD_INDEXER_PROFILE_BIN)"
	@test -x "$(FOD_MONITOR_PROFILE_BIN)"
	@test -x "$(FOD_FUSE_PROFILE_BIN)"
	@test -f "$(FOD_LIBFOD_PROFILE_SO)"

package-plan package-info:
	@FOD_PACKAGE_ROOT="$(FOD_PACKAGE_ROOT)" \
		FOD_PACKAGE_NAME="$(FOD_PACKAGE_NAME)" \
		FOD_PACKAGE_VERSION="$(FOD_VERSION)" \
		FOD_PACKAGE_RELEASE="$(FOD_PACKAGE_RELEASE)" \
		FOD_PACKAGE_CONFIG_SOURCE="$(FOD_PACKAGE_CONFIG_SOURCE)" \
		bash scripts/fod-native-package.sh plan native

package-native: package-artifacts
	@FOD_PACKAGE_ROOT="$(FOD_PACKAGE_ROOT)" \
		FOD_PACKAGE_NAME="$(FOD_PACKAGE_NAME)" \
		FOD_PACKAGE_VERSION="$(FOD_VERSION)" \
		FOD_PACKAGE_RELEASE="$(FOD_PACKAGE_RELEASE)" \
		FOD_PACKAGE_MAINTAINER="$(FOD_PACKAGE_MAINTAINER)" \
		FOD_PACKAGE_URL="$(FOD_PACKAGE_URL)" \
		FOD_PACKAGE_CONFIG_SOURCE="$(FOD_PACKAGE_CONFIG_SOURCE)" \
		FOD_PACKAGE_BOOTSTRAP_BIN="$(abspath $(FOD_BOOTSTRAP_PROFILE_BIN))" \
		FOD_PACKAGE_MKFS_BIN="$(abspath $(FOD_MKFS_PROFILE_BIN))" \
		FOD_PACKAGE_CHANGE_BIN="$(abspath $(FOD_CHANGE_PROFILE_BIN))" \
		FOD_PACKAGE_INDEXER_BIN="$(abspath $(FOD_INDEXER_PROFILE_BIN))" \
		FOD_PACKAGE_MONITOR_BIN="$(abspath $(FOD_MONITOR_PROFILE_BIN))" \
		FOD_PACKAGE_FUSE_BIN="$(abspath $(FOD_FUSE_PROFILE_BIN))" \
		FOD_PACKAGE_LIBFOD_SO="$(abspath $(FOD_LIBFOD_PROFILE_SO))" \
		FOD_PACKAGE_LIBFOD_HEADER="$(abspath $(FOD_LIBFOD_HEADER))" \
		bash scripts/fod-native-package.sh build native

package-ubuntu: package-artifacts
	@FOD_PACKAGE_ROOT="$(FOD_PACKAGE_ROOT)" \
		FOD_PACKAGE_NAME="$(FOD_PACKAGE_NAME)" \
		FOD_PACKAGE_VERSION="$(FOD_VERSION)" \
		FOD_PACKAGE_RELEASE="$(FOD_PACKAGE_RELEASE)" \
		FOD_PACKAGE_MAINTAINER="$(FOD_PACKAGE_MAINTAINER)" \
		FOD_PACKAGE_URL="$(FOD_PACKAGE_URL)" \
		FOD_PACKAGE_CONFIG_SOURCE="$(FOD_PACKAGE_CONFIG_SOURCE)" \
		FOD_PACKAGE_BOOTSTRAP_BIN="$(abspath $(FOD_BOOTSTRAP_PROFILE_BIN))" \
		FOD_PACKAGE_MKFS_BIN="$(abspath $(FOD_MKFS_PROFILE_BIN))" \
		FOD_PACKAGE_CHANGE_BIN="$(abspath $(FOD_CHANGE_PROFILE_BIN))" \
		FOD_PACKAGE_INDEXER_BIN="$(abspath $(FOD_INDEXER_PROFILE_BIN))" \
		FOD_PACKAGE_MONITOR_BIN="$(abspath $(FOD_MONITOR_PROFILE_BIN))" \
		FOD_PACKAGE_FUSE_BIN="$(abspath $(FOD_FUSE_PROFILE_BIN))" \
		FOD_PACKAGE_LIBFOD_SO="$(abspath $(FOD_LIBFOD_PROFILE_SO))" \
		FOD_PACKAGE_LIBFOD_HEADER="$(abspath $(FOD_LIBFOD_HEADER))" \
		bash scripts/fod-native-package.sh build deb

package-rocky: package-artifacts
	@FOD_PACKAGE_ROOT="$(FOD_PACKAGE_ROOT)" \
		FOD_PACKAGE_NAME="$(FOD_PACKAGE_NAME)" \
		FOD_PACKAGE_VERSION="$(FOD_VERSION)" \
		FOD_PACKAGE_RELEASE="$(FOD_PACKAGE_RELEASE)" \
		FOD_PACKAGE_MAINTAINER="$(FOD_PACKAGE_MAINTAINER)" \
		FOD_PACKAGE_URL="$(FOD_PACKAGE_URL)" \
		FOD_PACKAGE_CONFIG_SOURCE="$(FOD_PACKAGE_CONFIG_SOURCE)" \
		FOD_PACKAGE_BOOTSTRAP_BIN="$(abspath $(FOD_BOOTSTRAP_PROFILE_BIN))" \
		FOD_PACKAGE_MKFS_BIN="$(abspath $(FOD_MKFS_PROFILE_BIN))" \
		FOD_PACKAGE_CHANGE_BIN="$(abspath $(FOD_CHANGE_PROFILE_BIN))" \
		FOD_PACKAGE_INDEXER_BIN="$(abspath $(FOD_INDEXER_PROFILE_BIN))" \
		FOD_PACKAGE_MONITOR_BIN="$(abspath $(FOD_MONITOR_PROFILE_BIN))" \
		FOD_PACKAGE_FUSE_BIN="$(abspath $(FOD_FUSE_PROFILE_BIN))" \
		FOD_PACKAGE_LIBFOD_SO="$(abspath $(FOD_LIBFOD_PROFILE_SO))" \
		FOD_PACKAGE_LIBFOD_HEADER="$(abspath $(FOD_LIBFOD_HEADER))" \
		bash scripts/fod-native-package.sh build rpm

package-deb: package-ubuntu
package-redhat package-rpm: package-rocky

package-clean:
	@FOD_PACKAGE_ROOT="$(FOD_PACKAGE_ROOT)" bash scripts/fod-native-package.sh clean

package-deps-ubuntu:
	@printf '%s\n' "sudo apt install -y $(UBUNTU_PACKAGE_BUILD_DEPS)"

package-deps-redhat:
	@printf '%s\n' "sudo dnf install -y $(REDHAT_PACKAGE_BUILD_DEPS)"

test-native-package-policy:
	@bash tests/test_native_package_policy.sh

test-all: test-native-package-policy
