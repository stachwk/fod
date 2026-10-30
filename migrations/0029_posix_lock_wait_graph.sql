-- Copyright (c) 2026 Wojciech Stach
-- Licensed under BSL 1.1
--
-- Store POSIX lock dependencies in PostgreSQL so cycle detection spans mounts.
CREATE TABLE IF NOT EXISTS lock_wait_edges (
    waiter_session_id BIGINT NOT NULL,
    wait_id NUMERIC(20,0) NOT NULL,
    waiter_owner_key NUMERIC(20,0) NOT NULL,
    waiter_host_name VARCHAR(255) NOT NULL,
    waiter_pid BIGINT NOT NULL,
    blocked_session_id BIGINT NOT NULL,
    blocked_owner_key NUMERIC(20,0) NOT NULL,
    blocked_host_name VARCHAR(255) NOT NULL,
    blocked_pid BIGINT NOT NULL,
    lease_expires_at TIMESTAMP NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    PRIMARY KEY (waiter_session_id, wait_id, blocked_session_id, blocked_owner_key),
    CHECK (waiter_host_name <> blocked_host_name OR waiter_pid <> blocked_pid)
);

CREATE INDEX IF NOT EXISTS idx_lock_wait_edges_waiter
    ON lock_wait_edges (waiter_host_name, waiter_pid, lease_expires_at);
CREATE INDEX IF NOT EXISTS idx_lock_wait_edges_blocked
    ON lock_wait_edges (blocked_host_name, blocked_pid, lease_expires_at);

CREATE OR REPLACE FUNCTION fod_prune_client_session_lock_leases()
RETURNS trigger AS $$
BEGIN
    PERFORM pg_advisory_xact_lock(hashtextextended('fod-posix-lock-wait-graph', 0));

    DELETE FROM lock_leases WHERE session_id = OLD.session_id;
    DELETE FROM lock_range_leases WHERE session_id = OLD.session_id;
    DELETE FROM lock_wait_edges
    WHERE waiter_session_id = OLD.session_id
       OR blocked_session_id = OLD.session_id;
    RETURN OLD;
END;
$$ LANGUAGE plpgsql;
