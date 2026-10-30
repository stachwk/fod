-- Copyright (c) 2026 Wojciech Stach
-- Licensed under BSL 1.1
--
-- Persist the oldest FOD client release allowed to mount this upgraded database.
-- Renaming the legacy column also makes pre-gate FOD clients fail startup instead
-- of silently ignoring the client compatibility floor.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = current_schema()
          AND table_name = 'schema_version'
          AND column_name = 'version'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = current_schema()
          AND table_name = 'schema_version'
          AND column_name = 'schema_number'
    ) THEN
        ALTER TABLE schema_version RENAME COLUMN version TO schema_number;
    END IF;
END;
$$;

ALTER TABLE schema_version
    ADD COLUMN IF NOT EXISTS minimum_client_version TEXT NOT NULL DEFAULT '0.0.0';
