-- Older FUSE clients persisted the full st_mode for regular files and
-- directories. Current FOD stores inode type separately and expects only
-- permission plus special bits in these columns.
CREATE FUNCTION pg_temp.fod_normalize_legacy_mode(mode_text TEXT, expected_file_type INTEGER)
RETURNS TEXT
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    normalized TEXT := mode_text;
    parsed INTEGER := 0;
    result TEXT := '';
    digit_index INTEGER;
BEGIN
    IF left(normalized, 2) = '0o' THEN
        normalized := substr(normalized, 3);
    END IF;
    IF normalized !~ '^[0-7]{1,6}$' THEN
        RAISE EXCEPTION 'cannot normalize invalid FOD mode: %', mode_text;
    END IF;

    FOR digit_index IN 1..length(normalized) LOOP
        parsed := parsed * 8 + ascii(substr(normalized, digit_index, 1)) - 48;
    END LOOP;

    IF parsed > 4095 THEN
        IF (parsed & 61440) <> expected_file_type THEN
            RAISE EXCEPTION 'cannot normalize unexpected FOD mode type bits: %', mode_text;
        END IF;
        parsed := parsed & 4095;
    END IF;

    IF parsed = 0 THEN
        RETURN '0';
    END IF;
    WHILE parsed > 0 LOOP
        result := chr(48 + (parsed % 8)) || result;
        parsed := parsed / 8;
    END LOOP;
    RETURN result;
END;
$$;

UPDATE files
SET mode = pg_temp.fod_normalize_legacy_mode(mode, 32768);
UPDATE directories
SET mode = pg_temp.fod_normalize_legacy_mode(mode, 16384);

DROP FUNCTION pg_temp.fod_normalize_legacy_mode(TEXT, INTEGER);

ALTER TABLE files
    ADD CONSTRAINT files_mode_octal_check
        CHECK (mode ~ '^(0o)?[0-7]{1,4}$');
ALTER TABLE directories
    ADD CONSTRAINT directories_mode_octal_check
        CHECK (mode ~ '^(0o)?[0-7]{1,4}$');
