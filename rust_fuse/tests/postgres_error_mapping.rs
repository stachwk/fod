// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod support;

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use rust_hotpath::pg::DbRepo;
use support::{unique_suffix, MountedFs};

struct DiskFullTrigger {
    repo: DbRepo,
    trigger_name: String,
    function_name: String,
}

struct DirectoryDiskFullTrigger {
    repo: DbRepo,
    trigger_name: String,
    function_name: String,
}

impl Drop for DirectoryDiskFullTrigger {
    fn drop(&mut self) {
        let _ = self.repo.exec(&format!(
            "DROP TRIGGER IF EXISTS {} ON directories",
            self.trigger_name
        ));
        let _ = self
            .repo
            .exec(&format!("DROP FUNCTION IF EXISTS {}()", self.function_name));
    }
}

impl Drop for DiskFullTrigger {
    fn drop(&mut self) {
        let _ = self.repo.exec(&format!(
            "DROP TRIGGER IF EXISTS {} ON data_blocks",
            self.trigger_name
        ));
        let _ = self
            .repo
            .exec(&format!("DROP FUNCTION IF EXISTS {}()", self.function_name));
    }
}

#[test]
fn postgres_errors_are_reported_as_linux_errno() -> Result<(), String> {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("skipping PostgreSQL ENOSPC mount test: run via sudo for FUSE coverage");
        return Ok(());
    }
    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping PostgreSQL ENOSPC mount test: /dev/fuse is unavailable");
        return Ok(());
    }

    support::ensure_schema_initialized()?;
    let repo = support::db_repo()?;
    let suffix = unique_suffix().replace('-', "_");
    let trigger_name = format!("fod_test_disk_full_{suffix}");
    let function_name = format!("fod_test_disk_full_fn_{suffix}");

    repo.exec(&format!(
        "CREATE FUNCTION {function_name}() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
             RAISE EXCEPTION 'forced disk-full error from FOD regression'
                 USING ERRCODE = '53100';
         END;
         $$"
    ))?;
    let _trigger_cleanup = DiskFullTrigger {
        repo: repo.clone(),
        trigger_name: trigger_name.clone(),
        function_name: function_name.clone(),
    };
    repo.exec(&format!(
        "CREATE TRIGGER {trigger_name}
         BEFORE INSERT OR UPDATE ON data_blocks
         FOR EACH ROW EXECUTE FUNCTION {function_name}()"
    ))?;

    let mounted = MountedFs::start("postgres-disk-full-enospc")?;
    let directory_name = format!("disk-full-dir-{}", unique_suffix());
    let mkdir_io_error_name = format!("mkdir-io-error-dir-{}", unique_suffix());
    let rmdir_io_error_name = format!("rmdir-io-error-dir-{}", unique_suffix());
    let rmdir_io_error_path = mounted.mountpoint.join(&rmdir_io_error_name);
    std::fs::create_dir(&rmdir_io_error_path)
        .map_err(|err| format!("create rmdir EIO fixture failed: {err}"))?;
    let directory_trigger_name = format!("fod_test_directory_disk_full_{suffix}");
    let directory_function_name = format!("fod_test_directory_disk_full_fn_{suffix}");
    repo.exec(&format!(
        "CREATE FUNCTION {directory_function_name}() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
             IF TG_OP = 'INSERT' AND NEW.name = '{directory_name}' THEN
                 RAISE EXCEPTION 'forced namespace disk-full error from FOD regression' USING ERRCODE = '53100';
             ELSIF TG_OP = 'INSERT' AND NEW.name = '{mkdir_io_error_name}' THEN
                 RAISE EXCEPTION 'forced mkdir I/O error from FOD regression' USING ERRCODE = '58030';
             ELSIF TG_OP = 'DELETE' AND OLD.name = '{rmdir_io_error_name}' THEN
                 RAISE EXCEPTION 'forced rmdir I/O error from FOD regression' USING ERRCODE = '58030';
             END IF;
             IF TG_OP = 'DELETE' THEN
                 RETURN OLD;
             END IF;
             RETURN NEW;
         END;
         $$"
    ))?;
    let _directory_trigger_cleanup = DirectoryDiskFullTrigger {
        repo: repo.clone(),
        trigger_name: directory_trigger_name.clone(),
        function_name: directory_function_name.clone(),
    };
    repo.exec(&format!(
        "CREATE TRIGGER {directory_trigger_name}
         BEFORE INSERT OR DELETE ON directories
         FOR EACH ROW EXECUTE FUNCTION {directory_function_name}()"
    ))?;
    let directory_path = mounted.mountpoint.join(&directory_name);
    let mkdir_error = std::fs::create_dir(&directory_path)
        .expect_err("mkdir succeeded despite PostgreSQL SQLSTATE 53100");
    if mkdir_error.raw_os_error() != Some(libc::ENOSPC) {
        return Err(format!(
            "PostgreSQL SQLSTATE 53100 from mkdir reached userspace as errno={:?}, expected ENOSPC",
            mkdir_error.raw_os_error()
        ));
    }
    if directory_path.exists() {
        return Err("mkdir left a directory behind after PostgreSQL SQLSTATE 53100".to_string());
    }

    let mkdir_io_error_path = mounted.mountpoint.join(&mkdir_io_error_name);
    let mkdir_io_error = std::fs::create_dir(&mkdir_io_error_path)
        .expect_err("mkdir succeeded despite PostgreSQL SQLSTATE 58030");
    if mkdir_io_error.raw_os_error() != Some(libc::EIO) {
        return Err(format!(
            "PostgreSQL SQLSTATE 58030 from mkdir reached userspace as errno={:?}, expected EIO",
            mkdir_io_error.raw_os_error()
        ));
    }
    if mkdir_io_error_path.exists() {
        return Err("mkdir left a directory behind after PostgreSQL SQLSTATE 58030".to_string());
    }

    let rmdir_io_error = std::fs::remove_dir(&rmdir_io_error_path)
        .expect_err("rmdir succeeded despite PostgreSQL SQLSTATE 58030");
    if rmdir_io_error.raw_os_error() != Some(libc::EIO) {
        return Err(format!(
            "PostgreSQL SQLSTATE 58030 from rmdir reached userspace as errno={:?}, expected EIO",
            rmdir_io_error.raw_os_error()
        ));
    }
    if !rmdir_io_error_path.is_dir() {
        return Err("rmdir removed its directory after PostgreSQL SQLSTATE 58030".to_string());
    }

    let path = mounted
        .mountpoint
        .join(format!("disk-full-{}.bin", unique_suffix()));
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|err| format!("create test file failed: {err}"))?;

    let persist_error = match file.write_all(b"trigger PostgreSQL disk full") {
        Err(err) => err,
        Ok(()) => match file.sync_all() {
            Err(err) => err,
            Ok(()) => return Err("fsync succeeded despite PostgreSQL SQLSTATE 53100".to_string()),
        },
    };
    let actual_errno = persist_error.raw_os_error();
    if actual_errno != Some(libc::ENOSPC) {
        return Err(format!(
            "PostgreSQL SQLSTATE 53100 reached userspace as errno={actual_errno:?}, expected ENOSPC"
        ));
    }

    println!(
        "OK: PostgreSQL 53100 from mkdir/fsync maps to ENOSPC; 58030 from mkdir/rmdir maps to EIO"
    );
    Ok(())
}
