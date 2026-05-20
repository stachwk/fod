CREATE SCHEMA IF NOT EXISTS fod;
SET search_path TO fod, public;

ALTER TABLE IF EXISTS public.directories SET SCHEMA fod;
ALTER TABLE IF EXISTS public.files SET SCHEMA fod;
ALTER TABLE IF EXISTS public.special_files SET SCHEMA fod;
ALTER TABLE IF EXISTS public.hardlinks SET SCHEMA fod;
ALTER TABLE IF EXISTS public.symlinks SET SCHEMA fod;
ALTER TABLE IF EXISTS public.data_blocks SET SCHEMA fod;
ALTER TABLE IF EXISTS public.config SET SCHEMA fod;
ALTER TABLE IF EXISTS public.schema_version SET SCHEMA fod;
ALTER TABLE IF EXISTS public.schema_admin SET SCHEMA fod;
ALTER TABLE IF EXISTS public.journal SET SCHEMA fod;
ALTER TABLE IF EXISTS public.xattrs SET SCHEMA fod;
ALTER TABLE IF EXISTS public.lock_leases SET SCHEMA fod;
ALTER TABLE IF EXISTS public.lock_range_leases SET SCHEMA fod;
ALTER TABLE IF EXISTS public.data_objects SET SCHEMA fod;
ALTER TABLE IF EXISTS public.copy_block_crc SET SCHEMA fod;
ALTER TABLE IF EXISTS public.client_sessions SET SCHEMA fod;
ALTER TABLE IF EXISTS public.client_session_owner_keys SET SCHEMA fod;

DO $$
BEGIN
    IF to_regprocedure('public.fod_prune_client_session_lock_leases()') IS NOT NULL THEN
        EXECUTE 'ALTER FUNCTION public.fod_prune_client_session_lock_leases() SET SCHEMA fod';
    END IF;
END;
$$;

ALTER SEQUENCE IF EXISTS public.directories_id_directory_seq SET SCHEMA fod;
ALTER SEQUENCE IF EXISTS public.files_id_file_seq SET SCHEMA fod;
ALTER SEQUENCE IF EXISTS public.hardlinks_id_hardlink_seq SET SCHEMA fod;
ALTER SEQUENCE IF EXISTS public.symlinks_id_symlink_seq SET SCHEMA fod;
ALTER SEQUENCE IF EXISTS public.data_blocks_id_block_seq SET SCHEMA fod;
ALTER SEQUENCE IF EXISTS public.journal_id_entry_seq SET SCHEMA fod;
ALTER SEQUENCE IF EXISTS public.xattrs_id_seq SET SCHEMA fod;
ALTER SEQUENCE IF EXISTS public.lock_leases_id_lock_seq SET SCHEMA fod;
ALTER SEQUENCE IF EXISTS public.lock_range_leases_id_lock_seq SET SCHEMA fod;
ALTER SEQUENCE IF EXISTS public.data_objects_id_data_object_seq SET SCHEMA fod;
ALTER SEQUENCE IF EXISTS public.client_sessions_session_id_seq SET SCHEMA fod;
