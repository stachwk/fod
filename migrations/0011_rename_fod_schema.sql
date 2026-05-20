DO $$
BEGIN
    IF to_regnamespace('dbfs') IS NOT NULL AND to_regnamespace('fod') IS NULL THEN
        EXECUTE 'ALTER SCHEMA dbfs RENAME TO fod';
    END IF;
END;
$$;
