-- Capture high-call metadata and lookup statements from pg_stat_statements.
--
-- This report is intentionally narrower than top_statements_io_wal.sql. Use it
-- after a workload to verify whether path walking, child lookup, attr fetch,
-- xattr, and block lookup statements are still material enough to optimize.

SELECT
    CASE
        WHEN query ILIKE '%WITH RECURSIVE parts%' THEN 'path_walk'
        WHEN query ILIKE '%SELECT kind, entry_id FROM (%' THEN 'child_lookup'
        WHEN query ILIKE '%SELECT id_file, size, mode,%FROM files WHERE id_file%' THEN 'file_attrs'
        WHEN query ILIKE '%SELECT id_hardlink, files.size,%' THEN 'hardlink_attrs'
        WHEN query ILIKE '%SELECT id_directory, 0, mode,%' THEN 'directory_attrs'
        WHEN query ILIKE '%SELECT id_symlink, target,%' THEN 'symlink_attrs'
        WHEN query ILIKE '%SELECT file_type, rdev_major, rdev_minor FROM special_files%' THEN 'special_file_metadata'
        WHEN query ILIKE '%SELECT target FROM symlinks WHERE id_symlink%' THEN 'symlink_target'
        WHEN query ILIKE '%SELECT encode(value,%FROM xattrs%' THEN 'xattr_value'
        WHEN query ILIKE '%SELECT name FROM xattrs%' THEN 'xattr_names'
        WHEN query ILIKE '%JOIN data_blocks%' THEN 'block_lookup'
        WHEN query ILIKE '%JOIN data_extents%' THEN 'extent_lookup'
        WHEN query ILIKE '%SELECT data_object_id FROM files WHERE id_file%' THEN 'data_object_lookup'
        ELSE 'other_metadata'
    END AS category,
    queryid,
    calls,
    round(total_exec_time::numeric, 3) AS total_exec_ms,
    round(mean_exec_time::numeric, 3) AS mean_exec_ms,
    rows,
    shared_blks_hit,
    shared_blks_read,
    local_blks_hit,
    local_blks_read,
    wal_records,
    wal_bytes,
    left(regexp_replace(query, '\s+', ' ', 'g'), 240) AS query
FROM pg_stat_statements
WHERE
    query ILIKE '%WITH RECURSIVE parts%'
    OR query ILIKE '%SELECT kind, entry_id FROM (%'
    OR query ILIKE '%SELECT id_file, size, mode,%FROM files WHERE id_file%'
    OR query ILIKE '%SELECT id_hardlink, files.size,%'
    OR query ILIKE '%SELECT id_directory, 0, mode,%'
    OR query ILIKE '%SELECT id_symlink, target,%'
    OR query ILIKE '%SELECT file_type, rdev_major, rdev_minor FROM special_files%'
    OR query ILIKE '%SELECT target FROM symlinks WHERE id_symlink%'
    OR query ILIKE '%SELECT encode(value,%FROM xattrs%'
    OR query ILIKE '%SELECT name FROM xattrs%'
    OR query ILIKE '%JOIN data_blocks%'
    OR query ILIKE '%JOIN data_extents%'
    OR query ILIKE '%SELECT data_object_id FROM files WHERE id_file%'
ORDER BY total_exec_time DESC
LIMIT 40;
