-- Capture real PostgreSQL WAL/checkpointer counters in machine-readable form.
--
-- This script is intended for before/after real workload snapshots:
--
--     make profile-pg-wal-snapshot PROFILE_CAPTURE_LABEL=before
--     FOD_PROFILE_IO=1 make test-large-copy-benchmark
--     make profile-pg-wal-snapshot PROFILE_CAPTURE_LABEL=after
--
-- It deliberately does not use temp-table EXPLAIN WAL numbers.

\pset pager off
\pset tuples_only on
\pset format unaligned
\pset footer off

SELECT 'captured_at=' || now()::text;
SELECT 'database_name=' || current_database();
SELECT 'source=pg_stat_wal_pg_stat_bgwriter';

SELECT 'wal_records=' || COALESCE(wal_records, 0)::text FROM pg_stat_wal;
SELECT 'wal_fpi=' || COALESCE(wal_fpi, 0)::text FROM pg_stat_wal;
SELECT 'wal_bytes=' || COALESCE(wal_bytes, 0)::text FROM pg_stat_wal;
SELECT 'wal_buffers_full=' || COALESCE(wal_buffers_full, 0)::text FROM pg_stat_wal;
SELECT 'wal_write=' || COALESCE(wal_write, 0)::text FROM pg_stat_wal;
SELECT 'wal_sync=' || COALESCE(wal_sync, 0)::text FROM pg_stat_wal;
SELECT 'wal_write_time=' || COALESCE(wal_write_time, 0)::text FROM pg_stat_wal;
SELECT 'wal_sync_time=' || COALESCE(wal_sync_time, 0)::text FROM pg_stat_wal;
SELECT 'wal_stats_reset=' || COALESCE(stats_reset::text, '') FROM pg_stat_wal;

SELECT 'checkpoints_timed=' || COALESCE(checkpoints_timed, 0)::text FROM pg_stat_bgwriter;
SELECT 'checkpoints_req=' || COALESCE(checkpoints_req, 0)::text FROM pg_stat_bgwriter;
SELECT 'checkpoint_write_time=' || COALESCE(checkpoint_write_time, 0)::text FROM pg_stat_bgwriter;
SELECT 'checkpoint_sync_time=' || COALESCE(checkpoint_sync_time, 0)::text FROM pg_stat_bgwriter;
SELECT 'buffers_checkpoint=' || COALESCE(buffers_checkpoint, 0)::text FROM pg_stat_bgwriter;
SELECT 'buffers_clean=' || COALESCE(buffers_clean, 0)::text FROM pg_stat_bgwriter;
SELECT 'maxwritten_clean=' || COALESCE(maxwritten_clean, 0)::text FROM pg_stat_bgwriter;
SELECT 'buffers_backend=' || COALESCE(buffers_backend, 0)::text FROM pg_stat_bgwriter;
SELECT 'buffers_backend_fsync=' || COALESCE(buffers_backend_fsync, 0)::text FROM pg_stat_bgwriter;
SELECT 'buffers_alloc=' || COALESCE(buffers_alloc, 0)::text FROM pg_stat_bgwriter;
SELECT 'bgwriter_stats_reset=' || COALESCE(stats_reset::text, '') FROM pg_stat_bgwriter;
