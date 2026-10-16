-- Copyright (c) 2026 Wojciech Stach
-- Licensed under BSL 1.1

ALTER TABLE files
    ADD COLUMN IF NOT EXISTS unlinked BOOLEAN NOT NULL DEFAULT FALSE;

CREATE TABLE IF NOT EXISTS file_open_leases (
    file_id INTEGER NOT NULL REFERENCES files(id_file) ON DELETE CASCADE,
    session_id BIGINT NOT NULL REFERENCES client_sessions(session_id) ON DELETE CASCADE,
    handle_id NUMERIC(20,0) NOT NULL,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    heartbeat_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (session_id, handle_id)
);

CREATE INDEX IF NOT EXISTS idx_file_open_leases_file
    ON file_open_leases (file_id);
CREATE INDEX IF NOT EXISTS idx_file_open_leases_expires
    ON file_open_leases (lease_expires_at);
