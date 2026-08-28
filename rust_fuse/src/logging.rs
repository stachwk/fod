// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use env_logger::Target;
use fod_rust_runtime::ini_config::{load_config_parser, IniConfig};
use fod_rust_runtime::{env_var_with_legacy_alias, parse_bool};
use log::LevelFilter;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};

const DEFAULT_LOG_DIRECTORY: &str = "/var/log/fod";

fn configured_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn derived_log_filename(config_path: Option<&Path>) -> String {
    let raw = config_path
        .and_then(Path::file_stem)
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("fod");
    let sanitized: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let stem = if sanitized.is_empty() { "fod" } else { &sanitized };
    format!("{stem}.log")
}

fn validate_absolute_path(path: &Path, name: &str) -> Result<(), String> {
    if path.is_absolute() {
        Ok(())
    } else {
        Err(format!("{name} must be an absolute path: {}", path.display()))
    }
}

fn validate_filename(filename: &str) -> Result<(), String> {
    let path = Path::new(filename);
    if filename.trim().is_empty()
        || path.is_absolute()
        || path.components().count() != 1
        || matches!(path.components().next(), Some(Component::ParentDir | Component::CurDir))
    {
        return Err(format!(
            "logging.filename must be a single file name, not a path: {filename}"
        ));
    }
    Ok(())
}

fn resolve_log_file_path(
    config: Option<&IniConfig>,
    config_path: Option<&Path>,
    explicit_file: Option<PathBuf>,
    explicit_dir: Option<PathBuf>,
) -> Result<Option<PathBuf>, String> {
    if let Some(path) = explicit_file {
        validate_absolute_path(&path, "FOD_LOG_FILE")?;
        return Ok(Some(path));
    }

    let logging = config.and_then(|config| config.section("logging"));
    let enabled = if explicit_dir.is_some() {
        true
    } else if let Some(logging) = logging {
        logging
            .get("enabled")
            .map(|value| parse_bool(value).map_err(|err| format!("logging.enabled: {err}")))
            .transpose()?
            .unwrap_or(true)
    } else {
        false
    };
    if !enabled {
        return Ok(None);
    }

    let directory = explicit_dir
        .or_else(|| logging.and_then(|section| section.get("directory").map(PathBuf::from)))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_LOG_DIRECTORY));
    validate_absolute_path(&directory, "logging.directory")?;

    let filename = logging
        .and_then(|section| section.get("filename"))
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| derived_log_filename(config_path));
    validate_filename(&filename)?;
    Ok(Some(directory.join(filename)))
}

fn resolve_from_environment() -> Result<Option<PathBuf>, String> {
    let explicit_file = configured_path("FOD_LOG_FILE");
    let explicit_dir = configured_path("FOD_LOG_DIR");
    if explicit_file.is_some() {
        return resolve_log_file_path(None, None, explicit_file, explicit_dir);
    }

    let config_selector = configured_path("FOD_CONFIG");
    if let Some(config_path) = config_selector.as_deref() {
        match load_config_parser(Some(config_path)) {
            Ok((config, resolved_path)) => resolve_log_file_path(
                Some(&config),
                Some(&resolved_path),
                None,
                explicit_dir,
            ),
            Err(err) if explicit_dir.is_some() => resolve_log_file_path(
                None,
                Some(config_path),
                None,
                explicit_dir,
            )
            .map_err(|path_err| format!("{err}; {path_err}")),
            Err(err) => Err(err),
        }
    } else {
        resolve_log_file_path(None, None, None, explicit_dir)
    }
}

fn open_log_file(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true).write(true);
    #[cfg(unix)]
    options.mode(0o640);
    options.open(path)
}

pub fn init() {
    let level = env_var_with_legacy_alias("FOD_LOG_LEVEL").unwrap_or_else(|| "info".to_string());
    let filter = level.parse::<LevelFilter>().unwrap_or(LevelFilter::Info);
    let mut builder = env_logger::Builder::new();
    builder.filter_level(filter);

    let mut active_log_file = None;
    match resolve_from_environment() {
        Ok(Some(path)) => match open_log_file(&path) {
            Ok(file) => {
                builder.target(Target::Pipe(Box::new(file)));
                active_log_file = Some(path);
            }
            Err(err) => eprintln!(
                "fod-rust-fuse: cannot open configured log file {}: {}; falling back to stderr",
                path.display(),
                err
            ),
        },
        Ok(None) => {}
        Err(err) => eprintln!(
            "fod-rust-fuse: invalid file logging configuration: {err}; falling back to stderr"
        ),
    }

    builder.format(|buf, record| {
        writeln!(
            buf,
            "{} pid={} - {} - {}",
            buf.timestamp_seconds(),
            std::process::id(),
            record.level(),
            record.args()
        )
    });
    if env::var_os("RUST_LOG").is_none() {
        builder.parse_filters(&level);
    }
    builder.init();

    if let Some(path) = active_log_file {
        log::info!(
            "FOD file logging enabled: path={} config={}",
            path.display(),
            env::var("FOD_CONFIG").unwrap_or_else(|_| "<unset>".to_string())
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn config_with_logging(values: &[(&str, &str)]) -> IniConfig {
        let logging = values
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect::<HashMap<_, _>>();
        IniConfig {
            sections: HashMap::from([("logging".to_string(), logging)]),
        }
    }

    #[test]
    fn legacy_config_without_logging_keeps_stderr() {
        let config = IniConfig {
            sections: HashMap::new(),
        };
        assert_eq!(
            resolve_log_file_path(
                Some(&config),
                Some(Path::new("/etc/fod/fod_config.ini")),
                None,
                None,
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn derives_log_name_from_ini_basename() {
        let config = config_with_logging(&[("enabled", "true"), ("directory", "/var/log/fod")]);
        assert_eq!(
            resolve_log_file_path(
                Some(&config),
                Some(Path::new("/etc/fod/db-primary.ini")),
                None,
                None,
            )
            .unwrap(),
            Some(PathBuf::from("/var/log/fod/db-primary.log"))
        );
        assert_eq!(
            resolve_log_file_path(
                Some(&config),
                Some(Path::new("/etc/fod/db-archive.ini")),
                None,
                None,
            )
            .unwrap(),
            Some(PathBuf::from("/var/log/fod/db-archive.log"))
        );
    }

    #[test]
    fn supports_explicit_filename_and_directory() {
        let config = config_with_logging(&[
            ("enabled", "true"),
            ("directory", "/var/log/fod"),
            ("filename", "finance.log"),
        ]);
        assert_eq!(
            resolve_log_file_path(
                Some(&config),
                Some(Path::new("/etc/fod/db.ini")),
                None,
                None,
            )
            .unwrap(),
            Some(PathBuf::from("/var/log/fod/finance.log"))
        );
    }

    #[test]
    fn environment_file_override_has_highest_priority() {
        let config = config_with_logging(&[("enabled", "false")]);
        assert_eq!(
            resolve_log_file_path(
                Some(&config),
                Some(Path::new("/etc/fod/db.ini")),
                Some(PathBuf::from("/tmp/fod-explicit.log")),
                Some(PathBuf::from("/ignored")),
            )
            .unwrap(),
            Some(PathBuf::from("/tmp/fod-explicit.log"))
        );
    }

    #[test]
    fn rejects_relative_directory_and_path_like_filename() {
        let relative = config_with_logging(&[("enabled", "true"), ("directory", "logs")]);
        assert!(resolve_log_file_path(
            Some(&relative),
            Some(Path::new("/etc/fod/db.ini")),
            None,
            None,
        )
        .is_err());

        let path_filename = config_with_logging(&[
            ("enabled", "true"),
            ("directory", "/var/log/fod"),
            ("filename", "../db.log"),
        ]);
        assert!(resolve_log_file_path(
            Some(&path_filename),
            Some(Path::new("/etc/fod/db.ini")),
            None,
            None,
        )
        .is_err());
    }

    #[test]
    fn log_file_open_creates_parent_and_appends() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let base = env::temp_dir().join(format!("fod-log-test-{unique}"));
        let path = base.join("nested/instance.log");
        {
            let mut file = open_log_file(&path).expect("create log file");
            writeln!(file, "first").expect("write first line");
        }
        {
            let mut file = open_log_file(&path).expect("reopen log file");
            writeln!(file, "second").expect("write second line");
        }
        let contents = fs::read_to_string(&path).expect("read log file");
        assert_eq!(contents, "first\nsecond\n");
        let _ = fs::remove_dir_all(&base);
    }
}
