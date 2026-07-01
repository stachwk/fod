SELECT
    now() AS captured_at,
    pid,
    application_name,
    state,
    wait_event_type,
    wait_event,
    backend_type,
    now() - query_start AS query_age,
    left(regexp_replace(query, '\s+', ' ', 'g'), 240) AS query
FROM pg_stat_activity
WHERE datname = current_database()
ORDER BY query_start NULLS LAST;
