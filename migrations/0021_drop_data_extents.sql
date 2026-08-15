BEGIN;

SET search_path TO fod, public;

DO $$
BEGIN
    IF to_regclass('fod.data_extents') IS NULL THEN
        RETURN;
    END IF;

    LOCK TABLE data_extents IN ACCESS EXCLUSIVE MODE;

    IF EXISTS (SELECT 1 FROM data_extents) THEN
        RAISE EXCEPTION 'cannot drop data_extents while legacy extent rows remain; run migration 20 and resolve any integrity failures first';
    END IF;

    DROP TABLE data_extents;
END
$$;

COMMIT;
