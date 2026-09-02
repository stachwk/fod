# FOD operations runbook

This runbook covers day-2 operation of the reference Docker/systemd deployment.

## Status

```bash
make docker-deploy-status MASTERS=1 SLAVES=2
make docker-deploy-smoke MASTERS=1 SLAVES=2
make docker-deploy-fod-status MASTERS=1 SLAVES=2
make docker-deploy-fod-smoke MASTERS=1 SLAVES=2
```

For an installed systemd service:

```bash
systemctl status fod-docker-deploy.service --no-pager -l
sudo make docker-deploy-systemd-smoke MASTERS=1 SLAVES=2
docker ps
```

## Upgrade the FOD client without restarting PostgreSQL

The normal release path pins the systemd environment to the exact FOD image derived from `fod_version.txt`.

Before reinstall, record PostgreSQL container identities:

```bash
docker inspect \
  fod-deploy-primary-1 \
  fod-deploy-replica1-1 \
  fod-deploy-replica2-1 \
  --format '{{.Name}} {{.Id}}' \
  > /tmp/fod-postgres-before.txt
```

Record the current FOD image:

```bash
docker inspect fod-deploy-fod \
  --format 'configured_image={{.Config.Image}} image_id={{.Image}} created={{.Created}}'
```

Install/reinstall the current release:

```bash
sudo make docker-deploy-systemd-install MASTERS=1 SLAVES=2
```

When the service is already active this uses `systemctl reload`, so `ExecStop` is not executed.

Verify PostgreSQL container identities:

```bash
docker inspect \
  fod-deploy-primary-1 \
  fod-deploy-replica1-1 \
  fod-deploy-replica2-1 \
  --format '{{.Name}} {{.Id}}' \
  > /tmp/fod-postgres-after.txt

diff -u /tmp/fod-postgres-before.txt /tmp/fod-postgres-after.txt
```

For an FOD-only upgrade the expected diff is empty.

Verify the FOD image:

```bash
docker inspect fod-deploy-fod \
  --format 'configured_image={{.Config.Image}} image_id={{.Image}} created={{.Created}}'

sudo grep '^FOD_DOCKER_DEPLOY_CLIENT_IMAGE=' /etc/fod/docker-deploy.env
```

Then run smoke:

```bash
sudo make docker-deploy-systemd-smoke MASTERS=1 SLAVES=2
```

## Full deployment restart

Use this only when a full lifecycle restart is intended:

```bash
make docker-deploy-systemd-restart MASTERS=1 SLAVES=2
```

This is deliberately different from reinstall/upgrade. The full restart executes the stop path and recreates running Compose containers while preserving persistent volumes.

## FOD-only lifecycle

```bash
make docker-deploy-fod-up MASTERS=1 SLAVES=2
make docker-deploy-fod-status MASTERS=1 SLAVES=2
make docker-deploy-fod-smoke MASTERS=1 SLAVES=2
make docker-deploy-fod-down MASTERS=1 SLAVES=2
```

`docker-deploy-fod-down` removes the FOD container and propagated FUSE views while preserving the underlying shared host bind.

## FUSE identity check

```bash
findmnt -T ~/.local/share/fod/mount \
  -o ID,PARENT,MAJ:MIN,TARGET,SOURCE,FSTYPE,PROPAGATION
```

More than one host row is acceptable when all FUSE rows represent one unique `MAJ:MIN` device and that device matches the mount seen inside the healthy FOD container.

## Reboot acceptance test

After a host reboot:

```bash
systemctl status fod-docker-deploy.service --no-pager -l
make docker-deploy-status MASTERS=1 SLAVES=2
make docker-deploy-smoke MASTERS=1 SLAVES=2
sudo make docker-deploy-systemd-smoke MASTERS=1 SLAVES=2
docker ps
findmnt -T ~/.local/share/fod/mount \
  -o ID,PARENT,MAJ:MIN,TARGET,SOURCE,FSTYPE,PROPAGATION
```

Expected state:

- systemd service active/exited with successful start,
- one healthy primary,
- requested healthy streaming replicas,
- healthy FOD client,
- matching FUSE identity across host/container namespaces.

## Destructive cleanup

Normal lifecycle commands preserve volumes.

Permanent deployment-volume removal is guarded:

```bash
make docker-deploy-destroy MASTERS=1 SLAVES=2 DESTROY=YES
```

Do not use destructive cleanup as a normal upgrade or troubleshooting step.
