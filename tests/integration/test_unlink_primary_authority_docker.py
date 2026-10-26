#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1
"""Namespace mutations must preserve primary authority despite replica lag."""

from __future__ import annotations

import datetime
import errno
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import threading
import time

import psycopg2

from fod_mount import FODMount

ROOT = Path(__file__).resolve().parents[2]
PAYLOAD = b"surviving-hardlink-payload\n" * 2048


def query(db, sql, params=()):
    with db.cursor() as cursor:
        cursor.execute(sql, params or None)
        return cursor.fetchall()


def wait_for(predicate, label, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.05)
    raise AssertionError(f"timed out: {label}")


def main():
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    artifacts = ROOT / "artifacts/integration" / f"unlink-primary-authority-{stamp}"
    artifacts.mkdir(parents=True)
    primary_port = os.environ.get("UNLINK_PRIMARY_PORT", "55511")
    replica_port = os.environ.get("UNLINK_REPLICA_PORT", "55512")
    os.environ.update(
        POSTGRES_DB="fod_unlink_authority_test", POSTGRES_USER="foduser",
        POSTGRES_PASSWORD="cichosza", POSTGRES_HOST="127.0.0.1",
        POSTGRES_PORT=primary_port, FOD_PG_HOST="127.0.0.1", FOD_PG_PORT=primary_port,
        FOD_PG_DBNAME="fod_unlink_authority_test", FOD_PG_USER="foduser",
        FOD_PG_PASSWORD="cichosza", REPLICA_READ_PRIMARY_PORT=primary_port,
        REPLICA_READ_REPLICA_PORT=replica_port, REPLICA_READ_BIND_ADDRESS="127.0.0.1",
        FOD_RUNTIME_PROFILE=os.environ.get("FOD_RUNTIME_PROFILE", "release-lto"),
        FOD_PG_ENDPOINT_ROUTING_ENABLED="1", FOD_PG_RUNTIME_FAILOVER_ENABLED="0",
        FOD_PG_PRIMARY_HOSTS=f"127.0.0.1:{primary_port}",
        FOD_PG_REPLICA_HOSTS=f"127.0.0.1:{replica_port}",
        FOD_PG_REPLICA_READ_ROUTING_ENABLED="0", FOD_ROLE="primary",
        FOD_ATIME_POLICY="noatime", FOD_METADATA_CACHE_TTL_SECONDS="0",
        FOD_READ_CACHE_BLOCKS="0", FOD_FOPEN_DIRECT_IO="1",
        FOD_SCHEMA_ADMIN_PASSWORD="unlink-authority-test-admin",
        FOD_CONFIG=str(ROOT / "fod_config.ini"), FOD_LOG_LEVEL="info",
    )
    for key in ("FOD_DEGRADED_READONLY_ENABLED", "FOD_TELEMETRY_DSN", "FOD_MONITOR_DSN"):
        os.environ.pop(key, None)
    compose_command = [
        "docker", "compose", "-p", f"fod-unlink-authority-{os.getpid()}",
        "-f", str(ROOT / "docker-compose.replica-read.yml"),
    ]

    def compose(*args, check=True):
        return subprocess.run(
            compose_command + list(args), cwd=ROOT, check=check,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        )

    def connect(port):
        db = psycopg2.connect(
            host="127.0.0.1", port=port, dbname=os.environ["POSTGRES_DB"],
            user="foduser", password="cichosza",
        )
        db.autocommit = True
        return db

    mounts, connections, results = [], [], {}
    temp = Path(tempfile.mkdtemp(prefix="fod-unlink-authority-"))
    try:
        started = compose("up", "-d", "--wait", "--wait-timeout", "90")
        (artifacts / "compose-start.log").write_text(started.stdout)
        FODMount(str(ROOT)).init_schema()
        primary, replica = connect(primary_port), connect(replica_port)
        connections.extend([primary, replica])

        def barrier():
            lsn = query(primary, "SELECT pg_current_wal_flush_lsn()::text")[0][0]
            wait_for(lambda: query(replica, "SELECT pg_last_wal_replay_lsn() >= %s::pg_lsn", (lsn,))[0][0], "replica replay")

        def pause():
            barrier()
            compose("exec", "-T", "--user", "postgres", "replica", "psql", "-U", "foduser", "-d", os.environ["POSTGRES_DB"], "-Atqc", "SELECT pg_wal_replay_pause()")
            wait_for(lambda: query(replica, "SELECT pg_is_wal_replay_paused()")[0][0], "replica paused")

        def resume():
            compose("exec", "-T", "--user", "postgres", "replica", "psql", "-U", "foduser", "-d", os.environ["POSTGRES_DB"], "-Atqc", "SELECT pg_wal_replay_resume()")
            barrier()

        def seed(path):
            with path.open("wb") as handle:
                handle.write(PAYLOAD)
                handle.flush()
                os.fsync(handle.fileno())

        def visible(name):
            return query(primary, "SELECT (SELECT count(*) FROM fod.files WHERE name=%s AND NOT unlinked) + (SELECT count(*) FROM fod.hardlinks WHERE name=%s)", (name, name))[0][0]

        ma, mb = temp / "a", temp / "b"
        a = FODMount(str(ROOT)); mounts.append(a)
        a.start(str(ma))
        os.environ["FOD_PG_REPLICA_READ_ROUTING_ENABLED"] = "1"
        b = FODMount(str(ROOT)); mounts.append(b)
        b.start(str(mb))

        # Every creator must share one primary namespace across entry tables.
        seed(ma / "namespace-source")
        (ma / "namespace-nested").mkdir()

        def create_kind(root, relative, kind):
            path = root / relative
            if kind == "directory":
                path.mkdir()
            elif kind == "symlink":
                os.symlink("missing-target", path)
            elif kind == "hardlink":
                os.link(root / "namespace-source", path)
            elif kind == "fifo":
                os.mkfifo(path)
            else:
                fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
                try:
                    os.write(fd, PAYLOAD)
                    os.fsync(fd)
                finally:
                    os.close(fd)

        kinds = ("directory", "symlink", "hardlink", "file", "fifo")
        namespace_cases = 0
        for nested in (False, True):
            for winner in kinds:
                for loser in kinds:
                    if winner == loser:
                        continue
                    name = f"namespace-{int(nested)}-{winner}-{loser}"
                    relative = Path("namespace-nested") / name if nested else Path(name)
                    pause()
                    create_kind(ma, relative, winner)
                    if winner == "directory":
                        assert stat.S_ISDIR(os.lstat(ma / relative).st_mode)
                    loser_errno = 0
                    try:
                        create_kind(mb, relative, loser)
                    except OSError as exc:
                        loser_errno = exc.errno
                    count = query(primary, "SELECT (SELECT count(*) FROM fod.files WHERE name=%s AND NOT unlinked) + (SELECT count(*) FROM fod.directories WHERE name=%s) + (SELECT count(*) FROM fod.symlinks WHERE name=%s) + (SELECT count(*) FROM fod.hardlinks WHERE name=%s)", (name,) * 4)[0][0]
                    assert count == 1, f"cross-type namespace duplicated: winner={winner} loser={loser} count={count} errno={loser_errno}"
                    assert loser_errno == errno.EEXIST, f"cross-type create: winner={winner} loser={loser} errno={loser_errno}"
                    namespace_cases += 1
                    resume()
        results["cross_type_namespace_cases"] = namespace_cases
        # IDs are independent across tables: a FIFO must never change directory type.
        collisions = query(primary, "SELECT d.name FROM fod.directories d JOIN fod.special_files s ON s.id_file=d.id_directory WHERE d.id_parent=(SELECT id_directory FROM fod.directories WHERE name='namespace-nested' AND id_parent IS NULL)")
        assert collisions, "test did not exercise a directory/FIFO numeric ID collision"
        for (name,) in collisions:
            assert stat.S_ISDIR(os.lstat(ma / "namespace-nested" / name).st_mode)
        results["directory_special_file_id_collision_preserves_type"] = True

        # Empty on the paused replica must not override primary children.
        (ma / "lag-rmdir").mkdir()
        pause()
        seed(ma / "lag-rmdir/child")
        try:
            (mb / "lag-rmdir").rmdir()
        except OSError as exc:
            assert exc.errno == errno.ENOTEMPTY, f"rmdir primary child: {exc}"
        else:
            raise AssertionError("rmdir removed a directory with a primary child")
        assert (ma / "lag-rmdir/child").read_bytes() == PAYLOAD
        results["rmdir_primary_children_override_lagged_empty_replica"] = True
        resume()

        # A stale replica binding must not make a second rmdir report success.
        (ma / "already-removed-directory").mkdir()
        pause()
        (ma / "already-removed-directory").rmdir()
        try:
            (mb / "already-removed-directory").rmdir()
        except OSError as exc:
            assert exc.errno == errno.ENOENT, f"rmdir stale binding: {exc}"
        else:
            raise AssertionError("rmdir accepted an already removed primary binding")
        results["rmdir_rejects_stale_replica_binding"] = True
        resume()

        # An open, unlinked inode is not a directory child in the namespace.
        (ma / "unlinked-parent").mkdir()
        seed(ma / "unlinked-parent/open")
        fd = os.open(ma / "unlinked-parent/open", os.O_RDWR)
        file_id = query(primary, "SELECT id_file FROM fod.files WHERE name='open'")[0][0]
        try:
            pause()
            (mb / "unlinked-parent/open").unlink()
            (mb / "unlinked-parent").rmdir()
            os.pwrite(fd, b"REMOVED-PARENT", 0)
            os.fsync(fd)
            assert os.pread(fd, 14, 0) == b"REMOVED-PARENT"
            assert os.fstat(fd).st_nlink == 0
        finally:
            os.close(fd)
        wait_for(lambda: query(primary, "SELECT count(*) FROM fod.files WHERE id_file=%s", (file_id,))[0][0] == 0, "last close after parent rmdir")
        results["rmdir_preserves_remote_open_unlinked_writer"] = True
        resume()

        # Link committed on primary but deliberately absent from the replica.
        seed(ma / "lag-original")
        (ma / "nested").mkdir()
        seed(ma / "nested/lag-original")
        pause()
        for prefix in (Path(), Path("nested")):
            os.link(ma / prefix / "lag-original", ma / prefix / "lag-alias")
            assert query(replica, "SELECT count(*) FROM fod.hardlinks WHERE name='lag-alias'")[0][0] == 0
            (mb / prefix / "lag-original").unlink()
            assert (ma / prefix / "lag-alias").read_bytes() == PAYLOAD
            assert not (ma / prefix / "lag-original").exists()
        results["lagged_replica_preserves_root_and_nested_alias"] = True
        resume()

        # A remote open writer must retain its file lease when its original
        # pathname is removed and a different hardlink becomes primary.
        seed(ma / "promoted-original")
        os.link(ma / "promoted-original", ma / "promoted-alias")
        fd = os.open(ma / "promoted-original", os.O_RDWR)
        try:
            pause()
            (mb / "promoted-original").unlink()
            file_id = query(primary, "SELECT id_file FROM fod.files WHERE name='promoted-alias' AND NOT unlinked")[0][0]
            assert query(primary, "SELECT count(*) FROM fod.destination_write_leases WHERE name='promoted-original'")[0][0] == 0
            heartbeat = query(primary, "SELECT heartbeat_at FROM fod.file_write_leases WHERE file_id=%s", (file_id,))[0][0]
            wait_for(lambda: query(primary, "SELECT heartbeat_at > %s FROM fod.file_write_leases WHERE file_id=%s", (heartbeat, file_id))[0][0], "remote writer heartbeat after hardlink promotion")
            os.pwrite(fd, b"PROMOTED", 0)
            os.fsync(fd)
            assert (ma / "promoted-alias").read_bytes() == b"PROMOTED" + PAYLOAD[8:]
            assert os.fstat(fd).st_nlink == 1
        finally:
            os.close(fd)
        results["remote_writer_heartbeat_survives_primary_name_removal"] = True
        resume()

        # Deleting an alias must not release another pathname's writer lease.
        seed(ma / "open-original")
        os.link(ma / "open-original", ma / "open-alias")
        fd = os.open(ma / "open-original", os.O_RDWR)
        try:
            pause()
            (mb / "open-alias").unlink()
            assert query(primary, "SELECT count(*) FROM fod.destination_write_leases WHERE name='open-original'")[0][0] == 1
            (mb / "open-original").unlink()
            assert query(primary, "SELECT count(*) FROM fod.files WHERE name LIKE '.fod-unlinked-%%' AND unlinked")[0][0] == 1
            os.pwrite(fd, b"UPDATED", 0)
            os.fsync(fd)
            assert os.pread(fd, 7, 0) == b"UPDATED"
            assert os.fstat(fd).st_nlink == 0
        finally:
            os.close(fd)
        wait_for(lambda: query(primary, "SELECT count(*) FROM fod.files WHERE unlinked")[0][0] == 0, "last close purges old inode")
        results["remote_open_writer_survives_and_last_close_purges"] = True
        resume()

        # Both operations wait at the shared PostgreSQL lifecycle boundary.
        for order in (("link", "unlink"), ("unlink", "link")):
            tag = "-".join(order)
            original, alias = f"race-original-{tag}", f"race-alias-{tag}"
            seed(ma / original)
            pause()
            file_id = query(primary, "SELECT id_file FROM fod.files WHERE name=%s AND NOT unlinked", (original,))[0][0]
            gate = connect(primary_port); connections.append(gate)
            gate.autocommit = False
            with gate.cursor() as cursor:
                cursor.execute("SELECT pg_advisory_xact_lock(hashtextextended(%s,0))", (f"fod:open:file:{file_id}",))
            outcomes, threads = {}, []

            def worker(operation):
                try:
                    if operation == "link":
                        os.link(ma / original, ma / alias)
                    else:
                        (mb / original).unlink()
                    outcomes[operation] = 0
                except OSError as exc:
                    outcomes[operation] = exc.errno

            try:
                for count, operation in enumerate(order, 1):
                    thread = threading.Thread(target=worker, args=(operation,), daemon=True)
                    threads.append(thread); thread.start()
                    wait_for(lambda: query(primary, "SELECT count(*) FROM pg_locks WHERE locktype='advisory' AND NOT granted")[0][0] >= count, f"{operation} waiting at lifecycle lock")
            finally:
                gate.rollback()
            for thread in threads:
                thread.join(30)
                assert not thread.is_alive(), "link/unlink deadlock"
            assert outcomes.get("unlink") == 0, outcomes
            if outcomes.get("link") == 0:
                assert visible(alias) == 1, outcomes
                assert (ma / alias).read_bytes() == PAYLOAD
            else:
                assert outcomes.get("link") == errno.ENOENT, outcomes
                assert visible(alias) == 0
            assert visible(original) == 0
            results[f"serialized_{tag}"] = outcomes
            resume()

        print(json.dumps(results, sort_keys=True), flush=True)
    finally:
        for index, mount in enumerate(reversed(mounts)):
            if mount.config:
                log = mount.config.log_file
                subprocess.run(["fusermount3", "-u", str(mount.config.mountpoint)], capture_output=True)
                if mount.process:
                    try:
                        mount.process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        pass
                if log.exists():
                    shutil.copyfile(log, artifacts / f"mount-{index}.log")
            mount.stop()
        for connection in connections:
            connection.close()
        shutil.rmtree(temp)
        (artifacts / "results.json").write_text(json.dumps(results, indent=2))
        stopped = compose("down", "-v", "--remove-orphans", check=False)
        (artifacts / "compose-stop.log").write_text(stopped.stdout)
        print(f"artifacts={artifacts}", flush=True)


if __name__ == "__main__":
    main()
