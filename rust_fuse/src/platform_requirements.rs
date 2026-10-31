// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::ffi::CStr;

pub(crate) const MIN_LINUX_KERNEL: (u32, u32, u32) = (6, 17, 0);
pub(crate) const MIN_LIBFUSE_VERSION: i32 = 300;

unsafe extern "C" {
    fn fuse_version() -> libc::c_int;
    fn fuse_pkgversion() -> *const libc::c_char;
}

pub(crate) fn validate_runtime_requirements() -> Result<(), String> {
    let kernel_release = current_kernel_release()?;
    let libfuse_version = unsafe { fuse_version() };
    let libfuse_package = unsafe {
        let value = fuse_pkgversion();
        if value.is_null() {
            "unknown".to_string()
        } else {
            CStr::from_ptr(value).to_string_lossy().into_owned()
        }
    };

    validate_versions(&kernel_release, libfuse_version)?;
    log::info!(
        "FOD host requirements: linux_kernel={} minimum_linux_kernel={} libfuse={} libfuse_numeric={} minimum_libfuse=3.0.0",
        kernel_release,
        format_version(MIN_LINUX_KERNEL),
        libfuse_package,
        libfuse_version,
    );
    Ok(())
}

fn current_kernel_release() -> Result<String, String> {
    let mut value = std::mem::MaybeUninit::<libc::utsname>::zeroed();
    if unsafe { libc::uname(value.as_mut_ptr()) } != 0 {
        return Err(format!(
            "cannot read Linux kernel version with uname(2): {}",
            std::io::Error::last_os_error()
        ));
    }
    let value = unsafe { value.assume_init() };
    Ok(unsafe { CStr::from_ptr(value.release.as_ptr()) }
        .to_string_lossy()
        .into_owned())
}

fn validate_versions(kernel_release: &str, libfuse_version: i32) -> Result<(), String> {
    let kernel = parse_kernel_version(kernel_release).ok_or_else(|| {
        format!(
            "cannot parse Linux kernel release {kernel_release:?}; FOD requires Linux {} or newer",
            format_version(MIN_LINUX_KERNEL)
        )
    })?;
    if kernel < MIN_LINUX_KERNEL {
        return Err(format!(
            "Linux kernel {kernel_release} is too old; FOD requires Linux {} or newer",
            format_version(MIN_LINUX_KERNEL)
        ));
    }
    if libfuse_version < MIN_LIBFUSE_VERSION {
        return Err(format!(
            "libfuse {}.{} is too old; FOD requires libfuse3 3.0.0 or newer",
            libfuse_version.div_euclid(100),
            libfuse_version.rem_euclid(100)
        ));
    }
    Ok(())
}

fn parse_kernel_version(release: &str) -> Option<(u32, u32, u32)> {
    let mut parts = release.split('.');
    let numeric_prefix = |value: &str| {
        let digits = value
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>();
        (!digits.is_empty())
            .then(|| digits.parse::<u32>().ok())
            .flatten()
    };
    let major = numeric_prefix(parts.next()?)?;
    let minor = numeric_prefix(parts.next()?)?;
    let patch = parts.next().and_then(numeric_prefix).unwrap_or(0);
    Some((major, minor, patch))
}

fn format_version(version: (u32, u32, u32)) -> String {
    format!("{}.{}.{}", version.0, version.1, version.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_minimum_and_distribution_kernel_releases() {
        assert_eq!(parse_kernel_version("4.18.0"), Some((4, 18, 0)));
        assert_eq!(
            parse_kernel_version("4.18.0-553.el8_10.x86_64"),
            Some((4, 18, 0))
        );
        assert_eq!(parse_kernel_version("6.12-amd64"), Some((6, 12, 0)));
        assert!(validate_versions("6.17.0", 300).is_ok());
        assert!(validate_versions("6.17.0-41-generic", 300).is_ok());
        assert!(validate_versions("6.18.0", 300).is_ok());
    }

    #[test]
    fn rejects_old_or_unparseable_kernel_and_old_libfuse() {
        assert!(validate_versions("6.16.99", 317)
            .unwrap_err()
            .contains("too old"));
        assert!(validate_versions("not-a-kernel", 317)
            .unwrap_err()
            .contains("cannot parse"));
        assert!(validate_versions("6.17.0", 299)
            .unwrap_err()
            .contains("libfuse"));
    }
}
