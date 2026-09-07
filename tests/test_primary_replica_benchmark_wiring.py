#!/usr/bin/env python3
# Copyright (c) 2026 Wojciech Stach
# Licensed under BSL 1.1

from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]


class PrimaryReplicaBenchmarkWiringTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.makefile = (ROOT / "make/fod-internal.mk").read_text(encoding="utf-8")
        cls.compose = (ROOT / "docker-compose.replica-read.yml").read_text(
            encoding="utf-8"
        )
        cls.single = (
            ROOT / "tests/integration/test_fio_primary_write_replica_read_docker.sh"
        ).read_text(encoding="utf-8")
        cls.matrix = (
            ROOT / "tests/integration/test_fio_primary_write_replica_read_matrix.sh"
        ).read_text(encoding="utf-8")
        cls.dockerfile = (ROOT / "docker/replica-read/Dockerfile").read_text(
            encoding="utf-8"
        )

    def test_qnap_target_uses_isolated_remote_docker_stack(self) -> None:
        self.assertIn("test-fio-primary-write-replica-read-qnap:", self.makefile)
        self.assertIn('DOCKER_HOST="$(QNAP_DOCKER_HOST)"', self.makefile)
        self.assertIn(
            'REPLICA_READ_BIND_ADDRESS="$(QNAP_REPLICA_READ_BIND_ADDRESS)"',
            self.makefile,
        )
        self.assertIn('REPLICA_READ_PRIMARY_HOST="$(QNAP_PG_HOST)"', self.makefile)
        self.assertIn('REPLICA_READ_REPLICA_HOST="$(QNAP_PG_HOST)"', self.makefile)
        self.assertIn(
            "QNAP_REPLICA_READ_FIO_BLOCK_SIZES ?= 4k 16k 64k 256k 512k 1m",
            self.makefile,
        )
        self.assertIn('FOD_PG_HOST="$(QNAP_PG_HOST)"', self.makefile)
        self.assertIn('FOD_PG_PORT="$(QNAP_REPLICA_READ_PRIMARY_PORT)"', self.makefile)
        self.assertIn('FOD_PG_DBNAME="$(QNAP_PG_DBNAME)"', self.makefile)
        self.assertIn('FOD_PG_USER="$(QNAP_PG_USER)"', self.makefile)
        self.assertIn('FOD_PG_PASSWORD="$(QNAP_PG_PASSWORD)"', self.makefile)

    def test_compose_is_remote_docker_safe(self) -> None:
        self.assertIn("context: ./docker/replica-read", self.compose)
        self.assertIn(
            "${REPLICA_READ_BIND_ADDRESS:-127.0.0.1}:"
            "${REPLICA_READ_PRIMARY_PORT:-55441}:5432",
            self.compose,
        )
        self.assertNotIn(
            "./docker/replica-read/primary-init.sh:/docker-entrypoint-initdb.d",
            self.compose,
        )
        self.assertNotIn(
            "./docker/replica-read/replica-entrypoint.sh:/usr/local/bin",
            self.compose,
        )
        self.assertIn("COPY primary-init.sh", self.dockerfile)
        self.assertIn("COPY replica-entrypoint.sh", self.dockerfile)

    def test_single_run_uses_configurable_hosts_and_three_measurement_phases(self) -> None:
        self.assertIn(
            'PRIMARY_HOST="${REPLICA_READ_PRIMARY_HOST:-127.0.0.1}"',
            self.single,
        )
        self.assertIn(
            'REPLICA_HOST="${REPLICA_READ_REPLICA_HOST:-${PRIMARY_HOST}}"',
            self.single,
        )
        self.assertNotIn("psql -h 127.0.0.1", self.single)
        self.assertIn('export FOD_PG_DBNAME="${POSTGRES_DB:-foddbname}"', self.single)
        self.assertIn('export FOD_PG_USER="${POSTGRES_USER:-foduser}"', self.single)
        self.assertIn('export FOD_PG_PASSWORD="${POSTGRES_PASSWORD:-cichosza}"', self.single)
        self.assertIn('PAYLOAD_MODE="${FIO_PAYLOAD_MODE:-pattern}"', self.single)
        self.assertIn('FIO_WRITE_PAYLOAD_ARGS=(--buffer_pattern=0x5a)', self.single)
        self.assertIn(
            'FIO_WRITE_PAYLOAD_ARGS=(--refill_buffers=1 --randrepeat=0)',
            self.single,
        )
        self.assertIn("=== PHASE 1: PRIMARY WRITE ===", self.single)
        self.assertIn("FRESH PRIMARY READ", self.single)
        self.assertIn("FRESH REPLICA READ", self.single)
        self.assertIn("replica_write_guard=read_only_rejected", self.single)
        self.assertIn("payload_mode=${PAYLOAD_MODE}", self.single)
        self.assertIn("PERF_RESULT block_size=", self.single)

    def test_single_run_records_persisted_storage_geometry(self) -> None:
        self.assertIn(
            "SELECT value FROM fod.config WHERE key = 'block_size'",
            self.single,
        )
        self.assertIn('STORAGE_BLOCK_SIZE_BYTES="$(', self.single)
        self.assertIn("storage-geometry.txt", self.single)
        self.assertIn("storage_block_size_bytes=%s", self.single)
        self.assertIn(
            "storage_block_size_bytes=${STORAGE_BLOCK_SIZE_BYTES}",
            self.single,
        )
        self.assertIn('"storage_block_size_bytes"', self.single)

    def test_single_run_parameterizes_and_records_read_policy(self) -> None:
        expected = {
            "READ_CACHE_BLOCKS": "FOD_READ_CACHE_BLOCKS",
            "READ_AHEAD_BLOCKS": "FOD_READ_AHEAD_BLOCKS",
            "SEQUENTIAL_READ_AHEAD_BLOCKS": "FOD_SEQUENTIAL_READ_AHEAD_BLOCKS",
            "DIRECT_IO_READ_PREFETCH_BLOCKS": "FOD_DIRECT_IO_READ_PREFETCH_BLOCKS",
            "SMALL_FILE_READ_THRESHOLD_BLOCKS": "FOD_SMALL_FILE_READ_THRESHOLD_BLOCKS",
        }
        for local_name, env_name in expected.items():
            self.assertIn(f'{local_name}="${{{env_name}:-0}}"', self.single)
            self.assertIn(f'export {env_name}="${{{local_name}}}"', self.single)
        self.assertNotIn("export FOD_READ_CACHE_BLOCKS=0", self.single)
        self.assertIn("read-policy.txt", self.single)
        self.assertIn("read_policy_label=${READ_POLICY_LABEL}", self.single)
        self.assertIn("read_cache_blocks=${READ_CACHE_BLOCKS}", self.single)
        self.assertIn("direct_io_read_prefetch_blocks=${DIRECT_IO_READ_PREFETCH_BLOCKS}", self.single)
        self.assertIn(
            'METADATA_CACHE_TTL_SECONDS="${FOD_METADATA_CACHE_TTL_SECONDS:-0}"',
            self.single,
        )
        self.assertIn(
            'export FOD_METADATA_CACHE_TTL_SECONDS="${METADATA_CACHE_TTL_SECONDS}"',
            self.single,
        )
        self.assertIn(
            "metadata_cache_ttl_seconds=${METADATA_CACHE_TTL_SECONDS}",
            self.single,
        )

    def test_matrix_collects_primary_and_replica_results(self) -> None:
        self.assertIn('BLOCK_SIZES="${FIO_BLOCK_SIZES:-', self.matrix)
        self.assertIn('PAYLOAD_MODES="${FIO_PAYLOAD_MODES:-pattern}"', self.matrix)
        self.assertIn("for block_size in ${BLOCK_SIZES}; do", self.matrix)
        self.assertIn("for payload_mode in ${PAYLOAD_MODES}; do", self.matrix)
        self.assertIn('FIO_PAYLOAD_MODE="${payload_mode}"', self.matrix)
        self.assertIn("payload_mode", self.matrix)
        self.assertIn("storage_block_size_bytes", self.matrix)
        self.assertIn('field "${result}" storage_block_size_bytes', self.matrix)
        self.assertIn("read_policy_label", self.matrix)
        self.assertIn('field "${result}" read_policy_label', self.matrix)
        self.assertIn("read_cache_blocks", self.matrix)
        self.assertIn("read_ahead_blocks", self.matrix)
        self.assertIn("sequential_read_ahead_blocks", self.matrix)
        self.assertIn("direct_io_read_prefetch_blocks", self.matrix)
        self.assertIn("small_file_read_threshold_blocks", self.matrix)
        self.assertIn("metadata_cache_ttl_seconds", self.matrix)
        self.assertIn('field "${result}" metadata_cache_ttl_seconds', self.matrix)
        self.assertIn("primary_write_mib_s", self.matrix)
        self.assertIn("primary_read_mib_s", self.matrix)
        self.assertIn("replica_read_mib_s", self.matrix)
        self.assertIn("summary.tsv", self.matrix)


if __name__ == "__main__":
    unittest.main()
