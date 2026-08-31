# FOD Docker deployment

This is the end-state Docker deployment path for FOD. It is separate from benchmark compose files.

## Topology

The supported PostgreSQL topology is:

- exactly **1 writable primary** (`MASTERS=1`),
- **0..32 streaming replicas** (`SLAVES=N`),
- PostgreSQL 16 built with the FOD default `BLCKSZ=32K`,
- one persistent Docker volume per PostgreSQL node,
- one physical replication slot per replica.

`MASTERS>1` is rejected. Running multiple independent writable PostgreSQL servers is not a safe multi-master implementation for FOD. Automatic primary election/promotion is also intentionally outside this Docker-only scenario; real automatic HA should be added with a PostgreSQL HA layer such as Patroni plus a DCS/proxy rather than simulated by Compose.

## Public Make targets

Preview without changing Docker:

```bash
make docker-deploy-plan MASTERS=1 SLAVES=2
```

Render deployment state, compose and FOD configs:

```bash
make docker-deploy-render MASTERS=1 SLAVES=2
```

Complete first installation:

```bash
make docker-deploy-install MASTERS=1 SLAVES=2
```

The install operation renders state, pulls the PostgreSQL/FOD client images, starts the cluster, waits for the 32K primary and replicas, initializes the FOD schema when it does not yet exist, and runs a replication/block-size smoke test.

Lifecycle:

```bash
make docker-deploy-up MASTERS=1 SLAVES=2
make docker-deploy-status MASTERS=1 SLAVES=2
make docker-deploy-smoke MASTERS=1 SLAVES=2
make docker-deploy-logs MASTERS=1 SLAVES=2
make docker-deploy-down MASTERS=1 SLAVES=2
```

FOD schema operations:

```bash
make docker-deploy-fod-init MASTERS=1 SLAVES=2
make docker-deploy-fod-upgrade MASTERS=1 SLAVES=2
```

Destructive removal including PostgreSQL volumes requires an explicit guard:

```bash
make docker-deploy-destroy MASTERS=1 SLAVES=2 DESTROY=YES
```

To also remove the generated state/secrets after volume destruction:

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

These correspond to `1+0`, `1+1` and `1+2` primary/replica topologies.

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

The deployment smoke test requires PostgreSQL `SHOW block_size` to equal `32768` on primary and every replica.

## Persistent state and secrets

Generated deployment state is outside the repository by default:

```text
~/.local/state/fod/docker-deploy/
```

It contains:

- `compose.yml`
- `postgres.env`
- `fod-admin.env`
- `fod-host.ini`
- `fod-container.ini`
- copies of the primary/replica bootstrap scripts used by the rendered compose.

Secret files and generated FOD configuration files are mode `0600`. Database and replication passwords are not embedded in `compose.yml`.

Existing deployment secrets are reused. Supplying a different password against an already-rendered state is rejected; password rotation is intentionally not attempted implicitly because it also requires changing the live PostgreSQL roles.

## Generated FOD endpoint configuration

`fod-host.ini` is intended for a FOD process on the Docker host and uses host ports. `fod-container.ini` is intended for FOD tools running on the deployment Docker network and uses service DNS names such as `primary:5432` and `replica1:5432`.

Both files enable role-aware endpoint routing. Replica read routing remains disabled by default and can be enabled at render/install time with:

```bash
FOD_DOCKER_DEPLOY_REPLICA_READ_ROUTING=1 \
  make docker-deploy-install MASTERS=1 SLAVES=2
```

## Replication durability behavior

Each replica uses its own physical slot (`fod_replica_1`, `fod_replica_2`, ...). The primary sets `max_slot_wal_keep_size=4GB` so an abandoned replica cannot retain WAL without a bound indefinitely.

`docker-deploy-smoke` verifies:

- primary is writable (`pg_is_in_recovery() = false`),
- every configured replica is in recovery,
- every node reports PostgreSQL block size `32768`,
- the primary sees at least the requested number of streaming replicas.

## Policy test

```bash
make test-docker-deploy-policy
```

The test renders `1+0` and `1+2` topologies without requiring a running Docker daemon, checks secret permissions/configuration, checks the default 32K image, and verifies that `MASTERS>1` is rejected.
