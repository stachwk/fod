// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::ffi::CStr;

pub(crate) const MIN_LINUX_KERNEL: (u32, u32, u32) = (6, 17, 0);
pub(crate) const MIN_LIBFUSE_VERSION: (u32, u32, u32) = (3, 18, 2);
const MIN_LIBFUSE_VERSION_NUMERIC: i32 =
    (MIN_LIBFUSE_VERSION.0 as i32) * 100 + MIN_LIBFUSE_VERSION.1 as i32;

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

    validate_versions(&kernel_release, libfuse_version, &libfuse_package)?;
    log::info!(
        "FOD host requirements: linux_kernel={} minimum_linux_kernel={} libfuse={} libfuse_numeric={} minimum_libfuse={}",
        kernel_release,
        format_version(MIN_LINUX_KERNEL),
        libfuse_package,
        libfuse_version,
        format_version(MIN_LIBFUSE_VERSION),
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

fn validate_versions(
    kernel_release: &str,
    libfuse_version: i32,
    libfuse_package: &str,
) -> Result<(), String> {
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
    if libfuse_version < MIN_LIBFUSE_VERSION_NUMERIC {
        return Err(format!(
            "libfuse {}.{} is too old; FOD requires libfuse3 {} or newer",
            libfuse_version.div_euclid(100),
            libfuse_version.rem_euclid(100),
            format_version(MIN_LIBFUSE_VERSION),
        ));
    }
    let parsed_libfuse_version = parse_libfuse_version(libfuse_package).ok_or_else(|| {
        format!(
            "cannot parse libfuse package version {libfuse_package:?}; FOD requires libfuse3 {} or newer",
            format_version(MIN_LIBFUSE_VERSION)
        )
    })?;
    if parsed_libfuse_version < MIN_LIBFUSE_VERSION {
        return Err(format!(
            "libfuse3 {libfuse_package} is too old; FOD requires libfuse3 {} or newer",
            format_version(MIN_LIBFUSE_VERSION)
        ));
    }
    if libfuse_version != (parsed_libfuse_version.0 as i32) * 100 + parsed_libfuse_version.1 as i32
    {
        return Err(format!(
            "libfuse version metadata is inconsistent: numeric={libfuse_version} package={libfuse_package:?}"
        ));
    }
    Ok(())
}

fn parse_kernel_version(release: &str) -> Option<(u32, u32, u32)> {
    let (major_text, remaining) = release.split_once('.')?;
    let major = major_text.parse::<u32>().ok()?;
    let (minor_with_suffix, patch_with_suffix) = match remaining.split_once('.') {
        Some((minor, patch)) => (minor, Some(patch)),
        None => (remaining, None),
    };
    let (minor_text, minor_suffix) = split_numeric_component(minor_with_suffix)?;
    if minor_text.is_empty()
        || !valid_kernel_version_suffix(minor_suffix)
        || (patch_with_suffix.is_some() && !minor_suffix.is_empty())
    {
        return None;
    }
    let minor = minor_text.parse::<u32>().ok()?;
    let patch = match patch_with_suffix {
        Some(value) => {
            let (patch_text, suffix) = split_numeric_component(value)?;
            if patch_text.is_empty() || !valid_kernel_version_suffix(suffix) {
                return None;
            }
            patch_text.parse::<u32>().ok()?
        }
        None => 0,
    };
    Some((major, minor, patch))
}

fn split_numeric_component(value: &str) -> Option<(&str, &str)> {
    let digits_len = value.bytes().take_while(u8::is_ascii_digit).count();
    if digits_len == 0 {
        return None;
    }
    Some(value.split_at(digits_len))
}

fn valid_kernel_version_suffix(suffix: &str) -> bool {
    suffix.is_empty() || suffix.starts_with('-') || suffix.starts_with('+')
}

fn parse_libfuse_version(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    let patch_with_suffix = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let patch_digits = patch_with_suffix
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    let suffix = &patch_with_suffix[patch_digits.len()..];
    if patch_digits.is_empty()
        || (!suffix.is_empty() && !suffix.starts_with('-') && !suffix.starts_with('+'))
    {
        return None;
    }
    Some((major, minor, patch_digits.parse::<u32>().ok()?))
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
        assert_eq!(
            parse_kernel_version("4.18.0-553.el8_10.x86_64"),
            Some((4, 18, 0))
        );
        assert_eq!(parse_kernel_version("6.17.0+custom"), Some((6, 17, 0)));
        assert_eq!(parse_kernel_version("6.17malformed.0"), None);
        assert_eq!(parse_kernel_version("6.17-malformed.0"), None);
        assert_eq!(parse_kernel_version("6.17.0malformed"), None);
        assert_eq!(parse_kernel_version("6.17."), None);
        assert!(validate_versions("6.17.0", 318, "3.18.2").is_ok());
        assert!(validate_versions("6.17.0-41-generic", 318, "3.18.3").is_ok());
        assert!(validate_versions("6.18.0", 319, "3.19.0-1").is_ok());
    }

    #[test]
    fn rejects_old_or_unparseable_kernel_and_old_libfuse() {
        assert!(validate_versions("4.18.0", 318, "3.18.2")
            .unwrap_err()
            .contains("requires Linux 6.17.0 or newer"));
        assert!(validate_versions("6.16.99", 318, "3.18.2")
            .unwrap_err()
            .contains("too old"));
        assert!(validate_versions("not-a-kernel", 318, "3.18.2")
            .unwrap_err()
            .contains("cannot parse"));
        assert!(validate_versions("6.17.0", 317, "3.17.4")
            .unwrap_err()
            .contains("libfuse"));
        assert!(validate_versions("6.17.0", 318, "3.18.1")
            .unwrap_err()
            .contains("3.18.2 or newer"));
        assert!(validate_versions("6.17.0", 318, "unknown")
            .unwrap_err()
            .contains("cannot parse libfuse package version"));
        assert!(validate_versions("6.17.0", 318, "3.19.0")
            .unwrap_err()
            .contains("inconsistent"));
        assert!(parse_libfuse_version("3x.18.2").is_none());
        assert!(parse_libfuse_version("3.18.2suffix").is_none());
        assert!(parse_libfuse_version("3.18.2-1ubuntu").is_some());
    }
}
