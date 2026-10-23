-- Copyright (c) 2026 Wojciech Stach
-- Licensed under BSL 1.1
--
-- Content generations are advanced explicitly in the transaction that commits
-- the affected file's content. Timestamps and metadata updates are not content
-- change signals.
CREATE OR REPLACE FUNCTION fod_bump_file_content_generation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.content_generation IS DISTINCT FROM OLD.content_generation THEN
        IF OLD.content_generation >= 9223372036854775807 THEN
            RAISE EXCEPTION 'FOD content generation exhausted for file %', OLD.id_file;
        END IF;
        IF NEW.content_generation <> OLD.content_generation + 1 THEN
            RAISE EXCEPTION 'FOD content generation must advance by exactly one for file %', OLD.id_file;
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
