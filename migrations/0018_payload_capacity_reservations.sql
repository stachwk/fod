SET search_path TO fod, public;

CREATE TABLE IF NOT EXISTS payload_capacity_reservations (
    request_token TEXT PRIMARY KEY,
    reserved_bytes BIGINT NOT NULL CHECK (reserved_bytes > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_payload_capacity_reservations_expires
    ON payload_capacity_reservations (expires_at);
