SELECT now() AS captured_at;

SELECT * FROM pg_stat_wal;

SELECT
    CASE
        WHEN to_regclass('pg_catalog.pg_stat_checkpointer') IS NULL
        THEN 'pg_stat_checkpointer is unavailable; using pg_stat_bgwriter on this PostgreSQL version'
        ELSE 'pg_stat_checkpointer is available'
    END AS checkpointer_source;

SELECT
    CASE
        WHEN to_regclass('pg_catalog.pg_stat_checkpointer') IS NULL
        THEN 'SELECT * FROM pg_stat_bgwriter;'
        ELSE 'SELECT * FROM pg_stat_checkpointer;'
    END AS sql_to_execute
\gexec
