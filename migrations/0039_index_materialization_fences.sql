ALTER TABLE index_import_plans
    ADD COLUMN materialize_fencing_token BIGINT NOT NULL DEFAULT 0
        CONSTRAINT index_import_plans_materialize_fencing_token_nonnegative_check
            CHECK (materialize_fencing_token >= 0),
    ADD COLUMN materialize_lease_expires_at TIMESTAMPTZ;

-- Existing running plans have no owner identity and intentionally remain
-- unreclaimable. A new materialization receives token 1 and a lease atomically
-- when its plan row is inserted after this migration.
