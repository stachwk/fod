-- Copyright (c) 2026 Wojciech Stach
-- Licensed under BSL 1.1
--
-- POSIX lock owners are mount-local in FUSE. Re-key wait-graph nodes by host and
-- requesting process so a cycle spanning two mounts can be recognized.
DELETE FROM lock_wait_edges;

DROP INDEX IF EXISTS idx_lock_wait_edges_waiter;
DROP INDEX IF EXISTS idx_lock_wait_edges_blocked;
ALTER TABLE lock_wait_edges DROP CONSTRAINT IF EXISTS lock_wait_edges_check;
ALTER TABLE lock_wait_edges
    ADD COLUMN IF NOT EXISTS waiter_host_name VARCHAR(255) NOT NULL,
    ADD COLUMN IF NOT EXISTS waiter_pid BIGINT NOT NULL,
    ADD COLUMN IF NOT EXISTS blocked_host_name VARCHAR(255) NOT NULL,
    ADD COLUMN IF NOT EXISTS blocked_pid BIGINT NOT NULL;
ALTER TABLE lock_wait_edges
    ADD CONSTRAINT lock_wait_edges_check
    CHECK (waiter_host_name <> blocked_host_name OR waiter_pid <> blocked_pid);

CREATE INDEX idx_lock_wait_edges_waiter
    ON lock_wait_edges (waiter_host_name, waiter_pid, lease_expires_at);
CREATE INDEX idx_lock_wait_edges_blocked
    ON lock_wait_edges (blocked_host_name, blocked_pid, lease_expires_at);
