SET search_path TO fod, public;

ALTER TABLE index_scan_runs
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    ADD COLUMN IF NOT EXISTS request_token TEXT;

UPDATE index_scan_runs
SET request_token = COALESCE(request_token, format('legacy-scan-%s', id_scan_run))
WHERE request_token IS NULL OR request_token = '';

ALTER TABLE index_scan_runs
    ALTER COLUMN request_token SET NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_index_scan_runs_request_token
    ON index_scan_runs (request_token);

ALTER TABLE index_import_plans
    ADD COLUMN IF NOT EXISTS request_token TEXT;

UPDATE index_import_plans
SET request_token = COALESCE(request_token, format('legacy-plan-%s', id_import_plan))
WHERE request_token IS NULL OR request_token = '';

ALTER TABLE index_import_plans
    ALTER COLUMN request_token SET NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_index_import_plans_request_token
    ON index_import_plans (request_token);
