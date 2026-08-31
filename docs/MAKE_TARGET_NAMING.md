# FOD Make target naming

## Status

This document defines the public Make target namespace for FOD.

The normal user interface is the repository `make` command. `GNUmakefile` loads the main public `Makefile` plus focused public modules such as `make/fod-deploy-public.mk`. Historical recipe implementations live under `make/fod-internal.mk` and `make/fod-extra-internal.mk`; those implementation target names are not a compatibility interface and must not be documented as user commands.

## Naming rule

Public operational targets use:

```text
<area>-<variant>-<action>
```

The variant is omitted when it adds no information.

Examples:

```text
postgres-up
docker-postgres-32k-publish
docker-deploy-two-replicas-install
build-fod-runtime
runtime-config-reload
package-deb-build
selinux-rocky-remote-preflight
```

Use nouns first and the action last. Prefer one established area name instead of adding synonyms.

## Canonical areas

- `build-*` — FOD build artifacts.
- `rust-*` — Rust toolchain and Rust diagnostics.
- `target-*` — Cargo target/cache inspection and cleanup.
- `env-*` — local development/test environment.
- `deps-*` — dependency discovery.
- `postgres-*` — PostgreSQL lifecycle and diagnostics.
- `fod-*` — FOD schema/mount lifecycle.
- `runtime-config-*` — live FOD configuration.
- `install-*` / `uninstall-*` — native installation lifecycle.
- `indexer-*` — fod-indexer commands.
- `benchmark-*` — benchmark orchestration.
- `docker-*` — Docker labs, images, cleanup and final deployment scenarios.
- `selinux-*` — SELinux validation environments.
- `package-*` — native package generation.
- `test-*` — tests and policy checks.
- `profile-*` — profiling and diagnostic captures.

`test-*` and `profile-*` already use stable purpose-first namespaces and stay under those prefixes rather than being mechanically renamed to the operational pattern.

## Backend selection

Do not duplicate the PostgreSQL lifecycle with `qnap-*` aliases. The same target selects QNAP explicitly through `QNAP=1`:

```bash
make postgres-up
QNAP=1 make postgres-up

make fod-init
QNAP=1 make fod-init
```

A dedicated target may contain `remote` or `qnap` only when its behavior is genuinely different, for example a remote-only FOD operation that intentionally does not start Docker.

Docker deployment topology follows the same rule: topology is parameterized with `MASTERS=1 SLAVES=N`, while explicit preset targets such as `docker-deploy-two-replicas-install` are allowed because they encode a useful fixed scenario rather than preserving an obsolete alias.

## No compatibility aliases

Removed public target names are not retained as aliases. A renamed command has one public spelling.

Examples:

| Removed name | Canonical public name |
| --- | --- |
| `up` | `postgres-up` |
| `down` | `postgres-down` |
| `reset` | `postgres-reset` |
| `db-shell` | `postgres-shell` |
| `init` | `fod-init` |
| `mount` | `fod-mount` |
| `unmount` | `fod-unmount` |
| `change-runtime-list` | `runtime-config-list` |
| `reload-runtime` | `runtime-config-reload` |
| `indexer` | `indexer-run` |
| `indexer-import` | `indexer-materialize` |
| `benchmarks` | `benchmark-all` |
| `postgres-benchmarks-local` | `benchmark-postgres-local` |
| `package-ubuntu` | `package-deb-build` |
| `package-rocky` | `package-rpm-build` |
| `install-on-root` | `install-root` |
| `uninstall-on-root` | `uninstall-root` |

The legacy implementation target can remain inside the private dispatcher while the recipe is being refactored, but it must not resolve through normal public `make` invocation.

## Discoverability

Use:

```bash
make help
make help-tests
make help-profiles
make help-docker-deploy
```

The naming policy is guarded by:

```bash
make test-make-target-naming-policy
```

New public targets should be added to the facade or a focused public module using the canonical convention and should extend the relevant policy test when they introduce a new area or naming form.
