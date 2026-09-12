CREATE TABLE IF NOT EXISTS destination_write_leases (
    fencing_token BIGSERIAL PRIMARY KEY,
    parent_key BIGINT NOT NULL CHECK (parent_key >= 0),
    name TEXT NOT NULL,
    session_id BIGINT NOT NULL REFERENCES client_sessions(session_id) ON DELETE CASCADE,
    owner_key NUMERIC(20,0) NOT NULL,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    heartbeat_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_destination_write_leases_resource
    ON destination_write_leases (parent_key, name);
CREATE INDEX IF NOT EXISTS idx_destination_write_leases_session
    ON destination_write_leases (session_id);
CREATE INDEX IF NOT EXISTS idx_destination_write_leases_expires
    ON destination_write_leases (lease_expires_at);

CREATE TABLE IF NOT EXISTS file_write_leases (
    fencing_token BIGSERIAL PRIMARY KEY,
    file_id INTEGER NOT NULL REFERENCES files(id_file) ON DELETE CASCADE,
    session_id BIGINT NOT NULL REFERENCES client_sessions(session_id) ON DELETE CASCADE,
    owner_key NUMERIC(20,0) NOT NULL,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    heartbeat_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_file_write_leases_resource
    ON file_write_leases (file_id);
CREATE INDEX IF NOT EXISTS idx_file_write_leases_session
    ON file_write_leases (session_id);
CREATE INDEX IF NOT EXISTS idx_file_write_leases_expires
    ON file_write_leases (lease_expires_at);
