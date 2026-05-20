ALTER TABLE IF EXISTS lock_leases
    ADD COLUMN IF NOT EXISTS session_id BIGINT DEFAULT 0;

UPDATE lock_leases
SET session_id = 0
WHERE session_id IS NULL;

ALTER TABLE IF EXISTS lock_leases
    ALTER COLUMN session_id SET DEFAULT 0;

ALTER TABLE IF EXISTS lock_leases
    ALTER COLUMN session_id SET NOT NULL;

ALTER TABLE IF EXISTS lock_leases
    DROP CONSTRAINT IF EXISTS lock_leases_resource_kind_resource_id_owner_key_lease_kind_key;

CREATE INDEX IF NOT EXISTS idx_lock_leases_session
    ON lock_leases (session_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_lock_leases_identity
    ON lock_leases (resource_kind, resource_id, session_id, owner_key, lease_kind);

ALTER TABLE IF EXISTS lock_range_leases
    ADD COLUMN IF NOT EXISTS session_id BIGINT DEFAULT 0;

UPDATE lock_range_leases
SET session_id = 0
WHERE session_id IS NULL;

ALTER TABLE IF EXISTS lock_range_leases
    ALTER COLUMN session_id SET DEFAULT 0;

ALTER TABLE IF EXISTS lock_range_leases
    ALTER COLUMN session_id SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_lock_range_leases_session
    ON lock_range_leases (session_id);

CREATE OR REPLACE FUNCTION fod_prune_client_session_lock_leases()
RETURNS trigger AS $$
BEGIN
    DELETE FROM lock_leases
    WHERE session_id = OLD.session_id;

    DELETE FROM lock_range_leases
    WHERE session_id = OLD.session_id;

    RETURN OLD;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS fod_client_sessions_prune_lock_leases ON client_sessions;

CREATE TRIGGER fod_client_sessions_prune_lock_leases
BEFORE DELETE ON client_sessions
FOR EACH ROW
EXECUTE FUNCTION fod_prune_client_session_lock_leases();

UPDATE lock_leases
SET session_id = mapping.session_id
FROM (
    SELECT DISTINCT ON (owner_key)
        owner_key,
        session_id
    FROM client_session_owner_keys
    ORDER BY owner_key, last_seen_at DESC, session_id DESC
) AS mapping
WHERE lock_leases.session_id = 0
  AND lock_leases.owner_key = mapping.owner_key;

UPDATE lock_range_leases
SET session_id = mapping.session_id
FROM (
    SELECT DISTINCT ON (owner_key)
        owner_key,
        session_id
    FROM client_session_owner_keys
    ORDER BY owner_key, last_seen_at DESC, session_id DESC
) AS mapping
WHERE lock_range_leases.session_id = 0
  AND lock_range_leases.owner_key = mapping.owner_key;

DELETE FROM lock_leases
WHERE session_id = 0
  AND lease_expires_at <= NOW();

DELETE FROM lock_range_leases
WHERE session_id = 0
  AND lease_expires_at <= NOW();

ALTER TABLE IF EXISTS lock_leases
    ALTER COLUMN session_id DROP DEFAULT;

ALTER TABLE IF EXISTS lock_range_leases
    ALTER COLUMN session_id DROP DEFAULT;
