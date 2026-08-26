SET search_path TO fod, public;

-- idx_data_blocks_object_order(data_object_id, _order) already covers
-- lookups by data_object_id alone. Keeping the shorter prefix index gives
-- PostgreSQL an additional plan that can become pathological while statistics
-- are stale after large block loads.
DROP INDEX IF EXISTS fod.idx_data_blocks_data_object_id;

-- Refresh the planner immediately after the schema change. This is a one-time
-- upgrade operation, not part of the FOD read/write hot path.
ANALYZE fod.data_blocks;
