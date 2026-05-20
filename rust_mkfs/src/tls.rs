// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use fod_rust_runtime::expand_user;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn validate_tls_common_name(common_name: &str) -> Result<(), String> {
    if common_name.is_empty() {
        return Err("TLS common name cannot be empty".to_string());
    }
    if common_name.chars().all(|ch| {
        matches!(
            ch,
            'A'..='Z' | 'a'..='z' | '0'..='9' | '.' | '_' | '-'
        )
    }) {
        return Ok(());
    }
    Err(
        "TLS common name may contain only ASCII letters, digits, dot, underscore, and hyphen."
            .to_string(),
    )
}

pub(crate) fn generate_client_tls_pair(
    material_dir: &Path,
    common_name: &str,
    days: i64,
) -> Result<(PathBuf, PathBuf), String> {
    validate_tls_common_name(common_name)?;
    let material_dir = expand_user(material_dir);
    fs::create_dir_all(&material_dir).map_err(|e| {
        format!(
            "Unable to create TLS material directory {}: {}",
            material_dir.display(),
            e
        )
    })?;
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(&material_dir, fs::Permissions::from_mode(0o700));
    }

    let cert_path = material_dir.join("client.crt");
    let key_path = material_dir.join("client.key");
    if cert_path.exists() && key_path.exists() {
        return Ok((cert_path, key_path));
    }

    let status = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-sha256",
            "-nodes",
            "-days",
            &days.max(1).to_string(),
            "-subj",
            &format!("/CN={}", common_name),
            "-keyout",
            key_path.to_string_lossy().as_ref(),
            "-out",
            cert_path.to_string_lossy().as_ref(),
        ])
        .status()
        .map_err(|_| "openssl is required to generate a PostgreSQL TLS client pair".to_string())?;
    if !status.success() {
        return Err("Failed to generate PostgreSQL TLS client pair".to_string());
    }
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600));
        let _ = fs::set_permissions(&cert_path, fs::Permissions::from_mode(0o644));
    }
    Ok((cert_path, key_path))
}

#[cfg(test)]
mod tests {
    use super::validate_tls_common_name;

    #[test]
    fn validate_tls_common_name_accepts_safe_ascii() {
        assert!(validate_tls_common_name("fod-01.example_2").is_ok());
    }

    #[test]
    fn validate_tls_common_name_rejects_empty_and_unsafe_chars() {
        assert!(validate_tls_common_name("").is_err());
        assert!(validate_tls_common_name("fod/example").is_err());
        assert!(validate_tls_common_name("fod\nexample").is_err());
        assert!(validate_tls_common_name("fod example").is_err());
    }
}
