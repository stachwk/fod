BEGIN;

SET search_path TO fod, public;

-- FOD is block-only. Historical experimental payload rows are intentionally
-- not migrated or preserved.
DROP TABLE IF EXISTS data_extents CASCADE;

COMMIT;
