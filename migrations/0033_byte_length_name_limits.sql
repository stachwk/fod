-- VARCHAR(n) counts PostgreSQL characters, while Linux NAME_MAX and
-- XATTR_NAME_MAX count bytes. Preserve the FOD/LINUX 255-byte boundary in SQL.
ALTER TABLE directories
    ADD CONSTRAINT directories_name_octet_length_check
    CHECK (octet_length(name) <= 255);

ALTER TABLE files
    ADD CONSTRAINT files_name_octet_length_check
    CHECK (octet_length(name) <= 255);

ALTER TABLE hardlinks
    ADD CONSTRAINT hardlinks_name_octet_length_check
    CHECK (octet_length(name) <= 255);

ALTER TABLE symlinks
    ADD CONSTRAINT symlinks_name_octet_length_check
    CHECK (octet_length(name) <= 255);

ALTER TABLE xattrs
    ADD CONSTRAINT xattrs_name_octet_length_check
    CHECK (octet_length(name) <= 255);

ALTER TABLE destination_write_leases
    ADD CONSTRAINT destination_write_leases_name_octet_length_check
    CHECK (octet_length(name) <= 255);
