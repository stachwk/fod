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
fn postgres_disk_full_is_reported_as_enospc() -> Result<(), String> {
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

    println!("OK: PostgreSQL SQLSTATE 53100 surfaced through FUSE as ENOSPC");
    Ok(())
}
