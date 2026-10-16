-- Copyright (c) 2026 Wojciech Stach
-- Licensed under BSL 1.1
--
-- TX03J: one authoritative generation per file, incremented in the same
-- PostgreSQL transaction as a committed file content mapping/size update.
-- Metadata-only changes must not evict content for unrelated files.
ALTER TABLE files
    ADD COLUMN IF NOT EXISTS content_generation BIGINT NOT NULL DEFAULT 1;

CREATE OR REPLACE FUNCTION fod_bump_file_content_generation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.data_object_id IS DISTINCT FROM OLD.data_object_id
       OR NEW.size IS DISTINCT FROM OLD.size
       OR NEW.modification_date IS DISTINCT FROM OLD.modification_date
    THEN
        IF OLD.content_generation = 9223372036854775807 THEN
            RAISE EXCEPTION 'FOD content generation exhausted for file %', OLD.id_file;
        END IF;
        NEW.content_generation := OLD.content_generation + 1;
    ELSE
        -- Other writers cannot roll generations back through generic updates.
        NEW.content_generation := OLD.content_generation;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_fod_file_content_generation ON files;
CREATE TRIGGER trg_fod_file_content_generation
BEFORE UPDATE ON files
FOR EACH ROW
EXECUTE FUNCTION fod_bump_file_content_generation();
