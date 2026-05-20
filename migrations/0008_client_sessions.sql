CREATE TABLE IF NOT EXISTS client_sessions (
    session_id BIGSERIAL PRIMARY KEY,
    host_name VARCHAR(255) NOT NULL,
    mountpoint TEXT NOT NULL,
    mount_mode VARCHAR(20) NOT NULL,
    lock_backend VARCHAR(20) NOT NULL,
    pid BIGINT NOT NULL,
    lease_expires_at TIMESTAMP NOT NULL,
    heartbeat_at TIMESTAMP NOT NULL,
    last_lock_at TIMESTAMP NULL,
    last_write_at TIMESTAMP NULL,
    started_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_client_sessions_expires
    ON client_sessions (lease_expires_at);

CREATE TABLE IF NOT EXISTS client_session_owner_keys (
    session_id BIGINT NOT NULL REFERENCES client_sessions(session_id) ON DELETE CASCADE,
    owner_key NUMERIC(20,0) NOT NULL,
    first_seen_at TIMESTAMP NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    PRIMARY KEY(session_id, owner_key)
);

CREATE INDEX IF NOT EXISTS idx_client_session_owner_keys_owner
    ON client_session_owner_keys (owner_key);

CREATE INDEX IF NOT EXISTS idx_client_session_owner_keys_last_seen
    ON client_session_owner_keys (last_seen_at);
