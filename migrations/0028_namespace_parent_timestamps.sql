-- Copyright (c) 2026 Wojciech Stach
-- Licensed under BSL 1.1
--
-- Keep parent directory mtime/ctime in the same PostgreSQL transaction as
-- namespace entry creation, removal, rename and movement. A NULL parent maps
-- to the durable logical root row.
CREATE OR REPLACE FUNCTION fod_touch_namespace_parent(p_parent_id BIGINT)
RETURNS VOID
LANGUAGE plpgsql
AS $$
DECLARE
    touched_at TIMESTAMP := clock_timestamp();
BEGIN
    IF p_parent_id IS NULL THEN
        UPDATE directories
        SET modification_date = touched_at, change_date = touched_at
        WHERE id_directory = (
            SELECT id_directory
            FROM directories
            WHERE id_parent IS NULL AND name = '/'
        );
    ELSE
        UPDATE directories
        SET modification_date = touched_at, change_date = touched_at
        WHERE id_directory = p_parent_id;
    END IF;
END;
$$;

CREATE OR REPLACE FUNCTION fod_touch_namespace_parents()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    parent_column TEXT;
    old_parent_id BIGINT;
    new_parent_id BIGINT;
    old_visible BOOLEAN := TRUE;
    new_visible BOOLEAN := TRUE;
    namespace_changed BOOLEAN := TRUE;
BEGIN
    IF TG_TABLE_NAME IN ('directories', 'symlinks') THEN
        parent_column := 'id_parent';
    ELSE
        parent_column := 'id_directory';
    END IF;

    IF TG_OP <> 'INSERT' THEN
        old_parent_id := NULLIF(to_jsonb(OLD) ->> parent_column, '')::BIGINT;
        IF TG_TABLE_NAME = 'files' THEN
            old_visible := NOT OLD.unlinked;
        END IF;
    END IF;
    IF TG_OP <> 'DELETE' THEN
        new_parent_id := NULLIF(to_jsonb(NEW) ->> parent_column, '')::BIGINT;
        IF TG_TABLE_NAME = 'files' THEN
            new_visible := NOT NEW.unlinked;
        END IF;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        namespace_changed :=
            old_parent_id IS DISTINCT FROM new_parent_id
            OR OLD.name IS DISTINCT FROM NEW.name;
        IF TG_TABLE_NAME = 'files' THEN
            namespace_changed := namespace_changed OR OLD.unlinked IS DISTINCT FROM NEW.unlinked;
        END IF;
    END IF;

    IF NOT namespace_changed THEN
        RETURN NEW;
    END IF;
    IF TG_OP <> 'INSERT' AND old_visible THEN
        PERFORM fod_touch_namespace_parent(old_parent_id);
    END IF;
    IF TG_OP <> 'DELETE' AND new_visible
       AND (TG_OP = 'INSERT' OR old_parent_id IS DISTINCT FROM new_parent_id OR NOT old_visible)
    THEN
        PERFORM fod_touch_namespace_parent(new_parent_id);
    END IF;

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS fod_directories_touch_parent ON directories;
CREATE TRIGGER fod_directories_touch_parent
AFTER INSERT OR DELETE OR UPDATE OF id_parent, name ON directories
FOR EACH ROW EXECUTE FUNCTION fod_touch_namespace_parents();

DROP TRIGGER IF EXISTS fod_files_touch_parent ON files;
CREATE TRIGGER fod_files_touch_parent
AFTER INSERT OR DELETE OR UPDATE OF id_directory, name, unlinked ON files
FOR EACH ROW EXECUTE FUNCTION fod_touch_namespace_parents();

DROP TRIGGER IF EXISTS fod_hardlinks_touch_parent ON hardlinks;
CREATE TRIGGER fod_hardlinks_touch_parent
AFTER INSERT OR DELETE OR UPDATE OF id_directory, name ON hardlinks
FOR EACH ROW EXECUTE FUNCTION fod_touch_namespace_parents();

DROP TRIGGER IF EXISTS fod_symlinks_touch_parent ON symlinks;
CREATE TRIGGER fod_symlinks_touch_parent
AFTER INSERT OR DELETE OR UPDATE OF id_parent, name ON symlinks
FOR EACH ROW EXECUTE FUNCTION fod_touch_namespace_parents();
