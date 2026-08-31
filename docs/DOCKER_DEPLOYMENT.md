# FOD Docker deployment

This is the end-state Docker deployment path for FOD. It is separate from benchmark compose files.

The final installation consists of three layers:

1. PostgreSQL 16 / `BLCKSZ=32K` primary plus optional streaming replicas,
2. the FOD schema in the primary database,
3. a persistent `fod-client` container that mounts FOD through `/dev/fuse` and exposes the mount on the Docker host.

See `docs/DOCKER_FOD_INSTALL.md` for the FOD/FUSE-client layer in detail.

## Topology

The supported PostgreSQL topology is:

- exactly **1 writable primary** (`MASTERS=1`),
- **0..32 streaming replicas** (`SLAVES=N`),
- PostgreSQL 16 built with the FOD default `BLCKSZ=32K`,
- one persistent Docker volume per PostgreSQL node,
- one physical replication slot per replica,
- one persistent FOD/FUSE client container.

`MASTERS>1` is rejected. Running multiple independent writable PostgreSQL servers is not a safe multi-master implementation for FOD. Automatic primary election/promotion is also intentionally outside this Docker-only scenario; real automatic HA should be added with a PostgreSQL HA layer such as Patroni plus a DCS/proxy rather than simulated by Compose.

## Complete installation

Preview without changing Docker:

```bash
make docker-deploy-plan MASTERS=1 SLAVES=2
```

Prepare the host FUSE mount propagation when required:

```bash
make docker-deploy-fod-preflight MASTERS=1 SLAVES=2
make docker-deploy-fod-host-prepare MASTERS=1 SLAVES=2
```

Complete first installation:

```bash
make docker-deploy-install MASTERS=1 SLAVES=2
```

The install operation:

1. renders state, secrets, PostgreSQL Compose and FOD endpoint configuration,
2. pulls and starts the PostgreSQL 32K cluster,
3. waits for the primary and requested replicas,
4. initializes the FOD schema when it does not yet exist,
5. renders the FOD Compose overlay,
6. starts the persistent FOD/FUSE client,
7. waits for the FOD mount to become healthy,
8. runs PostgreSQL and FOD smoke checks.

## Lifecycle

Whole deployment:

```bash
make docker-deploy-up MASTERS=1 SLAVES=2
make docker-deploy-status MASTERS=1 SLAVES=2
make docker-deploy-smoke MASTERS=1 SLAVES=2
make docker-deploy-logs MASTERS=1 SLAVES=2
make docker-deploy-down MASTERS=1 SLAVES=2
```

FOD container only:

```bash
make docker-deploy-fod-install MASTERS=1 SLAVES=2
make docker-deploy-fod-up MASTERS=1 SLAVES=2
make docker-deploy-fod-status MASTERS=1 SLAVES=2
make docker-deploy-fod-smoke MASTERS=1 SLAVES=2
make docker-deploy-fod-logs MASTERS=1 SLAVES=2
make docker-deploy-fod-shell MASTERS=1 SLAVES=2
make docker-deploy-fod-down MASTERS=1 SLAVES=2
```

Schema lifecycle is deliberately separate from the FOD-container lifecycle:

```bash
make docker-deploy-schema-init MASTERS=1 SLAVES=2
make docker-deploy-schema-upgrade MASTERS=1 SLAVES=2
```

Destructive removal including PostgreSQL volumes requires an explicit guard:

```bash
make docker-deploy-destroy MASTERS=1 SLAVES=2 DESTROY=YES
```

To also remove generated database deployment state/secrets after volume destruction:

```bash
FOD_DOCKER_DEPLOY_PURGE_STATE=1 \
  make docker-deploy-destroy MASTERS=1 SLAVES=2 DESTROY=YES
```

## Presets

```bash
make docker-deploy-single-install
make docker-deploy-one-replica-install
make docker-deploy-two-replicas-install
```

These correspond to `1+0`, `1+1` and `1+2` PostgreSQL topologies and each preset also installs the FOD client.

## Default ports

| Node | Host endpoint |
| --- | --- |
| primary | `127.0.0.1:55441` |
| replica1 | `127.0.0.1:55442` |
| replica2 | `127.0.0.1:55443` |
| replicaN | consecutive ports from `55442` |

Override the bind/ports with:

- `FOD_DOCKER_DEPLOY_BIND_ADDRESS`
- `FOD_DOCKER_DEPLOY_CLIENT_HOST`
- `FOD_DOCKER_DEPLOY_PRIMARY_PORT`
- `FOD_DOCKER_DEPLOY_REPLICA_PORT_BASE`

## Images

Defaults:

```text
ghcr.io/stachwk/postgres-16-fod-32k:16.15
ghcr.io/stachwk/fod-client:3.4
```

Override them with `FOD_DOCKER_DEPLOY_POSTGRES_IMAGE` and `FOD_DOCKER_DEPLOY_CLIENT_IMAGE`.

The PostgreSQL deployment smoke test requires `SHOW block_size = 32768` on primary and every replica.

## FOD host mount

The persistent client mounts FOD inside the container at:

```text
/mnt/fod
```

The default host-visible bind mount is:

```text
~/.local/share/fod/mount
```

Override it with:

```bash
FOD_DOCKER_DEPLOY_FOD_MOUNT_DIR=/srv/fod \
  make docker-deploy-fod-install MASTERS=1 SLAVES=2
```

The client container receives `/dev/fuse` and `CAP_SYS_ADMIN`, not full `privileged` mode. The bind mount uses `rshared` propagation so the FUSE submount can propagate from the container namespace back to the Docker host.

Check host readiness with:

```bash
make docker-deploy-fod-preflight MASTERS=1 SLAVES=2
```

If the source mount is private, prepare it explicitly:

```bash
make docker-deploy-fod-host-prepare MASTERS=1 SLAVES=2
```

This host propagation setting normally needs a persistent systemd/mount configuration if FOD should start unattended after a host reboot.

## Persistent state and secrets

Generated deployment state is outside the repository by default:

```text
~/.local/state/fod/docker-deploy/
```

It contains:

- `compose.yml` — PostgreSQL topology,
- `compose-fod.yml` — persistent FOD client overlay,
- `postgres.env`,
- `fod-admin.env`,
- `fod-host.ini`,
- `fod-container.ini`,
- copies of the primary/replica bootstrap scripts used by the rendered PostgreSQL compose.

Secret files and generated FOD configuration files are mode `0600`. Database and replication passwords are not embedded in the Compose files.

Existing deployment secrets are reused. Supplying a different password against an already-rendered state is rejected; password rotation is intentionally not attempted implicitly because it also requires changing the live PostgreSQL roles.

## Generated FOD endpoint configuration

`fod-host.ini` is intended for a FOD process on the Docker host and uses host ports. `fod-container.ini` is used by the persistent FOD container and uses service DNS names such as `primary:5432` and `replica1:5432`.

Both files enable role-aware endpoint routing. Replica read routing remains disabled by default and can be enabled at render/install time with:

```bash
FOD_DOCKER_DEPLOY_REPLICA_READ_ROUTING=1 \
  make docker-deploy-install MASTERS=1 SLAVES=2
```

## Replication durability behavior

Each replica uses its own physical slot (`fod_replica_1`, `fod_replica_2`, ...). The primary sets `max_slot_wal_keep_size=4GB` so an abandoned replica cannot retain WAL without a bound indefinitely.

`docker-deploy-smoke` verifies the database topology and then the FOD client:

- primary is writable (`pg_is_in_recovery() = false`),
- every configured replica is in recovery,
- every PostgreSQL node reports block size `32768`,
- the primary sees at least the requested number of streaming replicas,
- the FOD container is healthy,
- `/mnt/fod` is mounted inside the FOD container,
- the FOD container can reach the PostgreSQL primary.

## Policy tests

```bash
make test-docker-deploy-policy
make test-docker-fod-install-policy
```

The database policy renders `1+0` and `1+2` topologies without requiring a running Docker daemon, checks secret permissions/configuration, checks the default 32K image and verifies that `MASTERS>1` is rejected.

The FOD policy checks the persistent client overlay, `/dev/fuse`, `SYS_ADMIN`, `rshared` bind propagation, read-only FOD configuration and the public Make target contract.
