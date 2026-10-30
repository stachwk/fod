-- Copyright (c) 2026 Wojciech Stach
-- Licensed under BSL 1.1
--
-- Store an upgrade admission gate in the existing, replicated schema row.
ALTER TABLE schema_version
    ADD COLUMN IF NOT EXISTS upgrade_in_progress BOOLEAN NOT NULL DEFAULT FALSE;

CREATE OR REPLACE FUNCTION fod_reject_client_session_during_schema_upgrade()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF COALESCE((
        SELECT upgrade_in_progress
        FROM schema_version
        ORDER BY applied_at DESC
        LIMIT 1
    ), FALSE) THEN
        RAISE EXCEPTION 'FOD schema upgrade is in progress or was interrupted';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_fod_client_session_schema_upgrade ON client_sessions;
CREATE TRIGGER trg_fod_client_session_schema_upgrade
BEFORE INSERT ON client_sessions
FOR EACH ROW
EXECUTE FUNCTION fod_reject_client_session_during_schema_upgrade();
