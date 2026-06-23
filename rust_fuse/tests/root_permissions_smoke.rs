// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

mod support;

use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use support::{
    admp_trace_env_pairs, bootstrap_binary, conninfo_from_config, create_workspace,
    mountpoint_ready, repo_root, tail_log, try_unmount, unique_suffix,
};

struct MountedRootFs {
    workspace: std::path::PathBuf,
    mountpoint: std::path::PathBuf,
    child: Child,
}

impl Drop for MountedRootFs {
    fn drop(&mut self) {
        try_unmount(&self.mountpoint);
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.workspace);
    }
}

fn user_allow_other_enabled() -> bool {
    fs::read_to_string("/etc/fuse.conf")
        .ok()
        .map(|contents| {
            contents
                .lines()
                .any(|line| line.trim() == "user_allow_other")
        })
        .unwrap_or(false)
}

fn sudo_available() -> bool {
    Command::new("sudo")
        .arg("-n")
        .arg("true")
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn start_root_mount(name: &str) -> Result<MountedRootFs, String> {
    let (workspace, mountpoint, log_path) = create_workspace(name)?;
    let log_file = fs::File::create(&log_path).map_err(|err| err.to_string())?;
    let bootstrap = bootstrap_binary();
    let conninfo = conninfo_from_config()?;
    let config = support::config_path();
    let trace_env = admp_trace_env_pairs()?;
    let current_uid = unsafe { libc::geteuid() };
    let mut command = if current_uid == 0 {
        let mut command = Command::new(&bootstrap);
        command
            .current_dir(repo_root())
            .arg("--debug")
            .arg("--role")
            .arg("auto")
            .arg("-f")
            .arg(&mountpoint)
            .env("FOD_CONFIG", &config)
            .env("FOD_DSN_CONNINFO", &conninfo)
            .env("FOD_ALLOW_OTHER", "1")
            .env("FOD_DEFAULT_PERMISSIONS", "1")
            .env("FOD_SELINUX", "off")
            .env("FOD_ACL", "off");
        for (key, value) in &trace_env {
            command.env(key, value);
        }
        command
    } else {
        let mut command = Command::new("sudo");
        command.current_dir(repo_root()).arg("-n").arg("env");
        for (key, value) in &trace_env {
            command.arg(format!("{key}={value}"));
        }
        command
            .arg(format!("FOD_CONFIG={}", config.display()))
            .arg(format!("FOD_DSN_CONNINFO={}", conninfo))
            .arg("FOD_ALLOW_OTHER=1")
            .arg("FOD_DEFAULT_PERMISSIONS=1")
            .arg("FOD_SELINUX=off")
            .arg("FOD_ACL=off")
            .arg(bootstrap)
            .arg("--debug")
            .arg("--role")
            .arg("auto")
            .arg("-f")
            .arg(&mountpoint);
        command
    };
    command
        .stdout(Stdio::from(
            log_file.try_clone().map_err(|err| err.to_string())?,
        ))
        .stderr(Stdio::from(log_file));

    let mut child = command.spawn().map_err(|err| err.to_string())?;
    for _ in 0..60 {
        if mountpoint_ready(&mountpoint) {
            return Ok(MountedRootFs {
                workspace,
                mountpoint,
                child,
            });
        }
        if let Some(status) = child.try_wait().map_err(|err| err.to_string())? {
            return Err(format!(
                "root mount exited too early with status {status:?}\n{}",
                tail_log(&log_path, 200)
            ));
        }
        sleep(Duration::from_secs(1));
    }

    Err(format!(
        "root mount did not become ready\n{}",
        tail_log(&log_path, 200)
    ))
}

#[test]
fn root_mount_allows_nobody_to_list_and_write() -> Result<(), String> {
    if !Path::new("/dev/fuse").exists() {
        eprintln!("skipping root permissions smoke: /dev/fuse is unavailable in this environment");
        return Ok(());
    }
    if !user_allow_other_enabled() {
        eprintln!(
            "skipping root permissions smoke: user_allow_other is disabled in /etc/fuse.conf"
        );
        return Ok(());
    }
    if !sudo_available() {
        eprintln!("skipping root permissions smoke: sudo -n is unavailable");
        return Ok(());
    }

    let mounted = start_root_mount(&format!("root-permissions-{}", unique_suffix()))?;
    let allowed_dir = mounted.mountpoint.join("shared");
    let allowed_file = allowed_dir.join("nobody-write.txt");

    let mkdir_status = Command::new("sudo")
        .arg("-n")
        .arg("install")
        .arg("-d")
        .arg("-m")
        .arg("0777")
        .arg(&allowed_dir)
        .status()
        .map_err(|err| format!("sudo install -d failed: {err}"))?;
    if !mkdir_status.success() {
        return Err(format!(
            "failed to create shared dir as root: {mkdir_status:?}"
        ));
    }

    let touch_status = Command::new("sudo")
        .arg("-n")
        .arg("sh")
        .arg("-c")
        .arg(": > \"$1\" && chmod 0666 \"$1\"")
        .arg("sh")
        .arg(&allowed_file)
        .status()
        .map_err(|err| format!("sudo create file failed: {err}"))?;
    if !touch_status.success() {
        return Err(format!(
            "failed to create shared file as root: {touch_status:?}"
        ));
    }

    let ls_status = Command::new("sudo")
        .arg("-n")
        .arg("-u")
        .arg("nobody")
        .arg("ls")
        .arg(&allowed_dir)
        .status()
        .map_err(|err| format!("sudo ls failed: {err}"))?;
    if !ls_status.success() {
        return Err(format!(
            "nobody could not list root-mounted shared dir: {ls_status:?}"
        ));
    }

    let write_status = Command::new("sudo")
        .arg("-n")
        .arg("-u")
        .arg("nobody")
        .arg("sh")
        .arg("-c")
        .arg("printf 'nobody-write\\n' >> \"$1\"")
        .arg("sh")
        .arg(&allowed_file)
        .status()
        .map_err(|err| format!("sudo write failed: {err}"))?;
    if !write_status.success() {
        return Err(format!(
            "nobody could not write into root-mounted shared file: {write_status:?}"
        ));
    }

    let contents =
        fs::read_to_string(&allowed_file).map_err(|err| format!("read back failed: {err}"))?;
    if !contents.contains("nobody-write") {
        return Err(format!("shared file missing nobody write: {contents:?}"));
    }

    drop(mounted);
    Ok(())
}
