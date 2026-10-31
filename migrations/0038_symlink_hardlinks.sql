-- Additional names share the canonical symlink inode, target and metadata.
CREATE TABLE IF NOT EXISTS symlink_hardlinks (
    id_symlink_hardlink BIGSERIAL PRIMARY KEY,
    id_symlink BIGINT NOT NULL REFERENCES symlinks(id_symlink),
    id_parent BIGINT REFERENCES directories(id_directory),
    name VARCHAR(255) NOT NULL
        CONSTRAINT symlink_hardlinks_name_octet_length_check CHECK (octet_length(name) <= 255)
);

CREATE UNIQUE INDEX IF NOT EXISTS uniq_symlink_hardlinks_root_name
    ON symlink_hardlinks (name) WHERE id_parent IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS uniq_symlink_hardlinks_parent_name
    ON symlink_hardlinks (id_parent, name) WHERE id_parent IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_symlink_hardlinks_parent_id
    ON symlink_hardlinks (id_parent);
CREATE INDEX IF NOT EXISTS idx_symlink_hardlinks_symlink_id
    ON symlink_hardlinks (id_symlink);

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
    IF TG_TABLE_NAME IN ('directories', 'symlinks', 'symlink_hardlinks') THEN
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

DROP TRIGGER IF EXISTS fod_symlink_hardlinks_touch_parent ON symlink_hardlinks;
CREATE TRIGGER fod_symlink_hardlinks_touch_parent
AFTER INSERT OR DELETE OR UPDATE OF id_parent, name ON symlink_hardlinks
FOR EACH ROW EXECUTE FUNCTION fod_touch_namespace_parents();
