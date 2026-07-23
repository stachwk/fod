// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::collections::BTreeMap;

pub const MIN_POSTGRES_SERVER_VERSION_NUM: i32 = 90_500;
pub const POSTGRES_ADMIN_CONNECTION_RESERVE: u64 = 2;

const BASE_SESSION_SETUP_SQL: [&str; 5] = [
    "SET TIME ZONE 'UTC'",
    "SET SESSION default_transaction_isolation TO 'read committed'",
    "SET SESSION statement_timeout TO 0",
    "SET SESSION lock_timeout TO 0",
    "SET SESSION standard_conforming_strings TO on",
];

const EXTENDED_SESSION_SETUP_SQL: [&str; 6] = [
    "SET TIME ZONE 'UTC'",
    "SET SESSION default_transaction_isolation TO 'read committed'",
    "SET SESSION statement_timeout TO 0",
    "SET SESSION lock_timeout TO 0",
    "SET SESSION standard_conforming_strings TO on",
    "SET SESSION idle_in_transaction_session_timeout TO 0",
];

pub const POSTGRES_REQUIREMENT_SETTINGS_SQL: &str = "
    SELECT
        name,
        setting,
        COALESCE(unit, ''),
        context,
        pending_restart::text
    FROM pg_settings
    WHERE name IN (
        'TimeZone',
        'fsync',
        'full_page_writes',
        'idle_in_transaction_session_timeout',
        'lock_timeout',
        'max_connections',
        'standard_conforming_strings',
        'statement_timeout',
        'transaction_isolation'
    )
    ORDER BY name
";

pub fn postgres_session_setup_sql(server_version_num: i32) -> &'static [&'static str] {
    if server_version_num >= 90_600 {
        &EXTENDED_SESSION_SETUP_SQL
    } else {
        &BASE_SESSION_SETUP_SQL
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresSettingObservation {
    pub name: String,
    pub setting: String,
    pub unit: String,
    pub context: String,
    pub pending_restart: bool,
}

impl PostgresSettingObservation {
    fn from_row(row: &[String]) -> Result<Self, String> {
        if row.len() != 5 {
            return Err(format!(
                "PostgreSQL requirements query returned {} columns instead of 5",
                row.len()
            ));
        }
        Ok(Self {
            name: row[0].clone(),
            setting: row[1].clone(),
            unit: row[2].clone(),
            context: row[3].clone(),
            pending_restart: parse_postgres_bool(&row[4], "pending_restart")?,
        })
    }

    pub fn display_value(&self) -> String {
        if self.unit.is_empty() {
            self.setting.clone()
        } else {
            format!("{}{}", self.setting, self.unit)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresRuntimeRequirements {
    pub server_version_num: i32,
    pub minimum_server_version_num: i32,
    pub pool_max_connections: u64,
    pub required_max_connections: u64,
    pub settings: BTreeMap<String, PostgresSettingObservation>,
}

impl PostgresRuntimeRequirements {
    pub fn from_pg_settings_rows(
        server_version_num: i32,
        pool_max_connections: u64,
        rows: Vec<Vec<String>>,
    ) -> Result<Self, String> {
        let mut settings = BTreeMap::new();
        for row in rows {
            let observation = PostgresSettingObservation::from_row(&row)?;
            if settings
                .insert(observation.name.clone(), observation)
                .is_some()
            {
                return Err(
                    "PostgreSQL requirements query returned a duplicate setting".to_string()
                );
            }
        }

        let required = [
            "TimeZone",
            "fsync",
            "full_page_writes",
            "lock_timeout",
            "max_connections",
            "standard_conforming_strings",
            "statement_timeout",
            "transaction_isolation",
        ];
        for name in required {
            if !settings.contains_key(name) {
                return Err(format!(
                    "PostgreSQL requirements query did not return required setting {name}"
                ));
            }
        }
        if server_version_num >= 90_600
            && !settings.contains_key("idle_in_transaction_session_timeout")
        {
            return Err(
                "PostgreSQL requirements query did not return required setting idle_in_transaction_session_timeout"
                    .to_string(),
            );
        }

        Ok(Self {
            server_version_num,
            minimum_server_version_num: MIN_POSTGRES_SERVER_VERSION_NUM,
            pool_max_connections,
            required_max_connections: pool_max_connections
                .saturating_add(POSTGRES_ADMIN_CONNECTION_RESERVE),
            settings,
        })
    }

    pub fn unsupported_version_error(&self) -> Option<String> {
        (self.server_version_num < self.minimum_server_version_num).then(|| {
            format!(
                "PostgreSQL server_version_num={} is unsupported; FOD requires {} or newer. Upgrade the PostgreSQL instance before starting FOD.",
                self.server_version_num, self.minimum_server_version_num
            )
        })
    }

    pub fn session_configuration_errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        self.require_session_value(
            "TimeZone",
            "UTC",
            |value| value.eq_ignore_ascii_case("UTC"),
            &mut errors,
        );
        self.require_session_value(
            "transaction_isolation",
            "read committed",
            |value| value.eq_ignore_ascii_case("read committed"),
            &mut errors,
        );
        self.require_session_value("statement_timeout", "0", |value| value == "0", &mut errors);
        self.require_session_value("lock_timeout", "0", |value| value == "0", &mut errors);
        self.require_session_value(
            "standard_conforming_strings",
            "on",
            |value| parse_postgres_bool(value, "standard_conforming_strings").unwrap_or(false),
            &mut errors,
        );
        if self.server_version_num >= 90_600 {
            self.require_session_value(
                "idle_in_transaction_session_timeout",
                "0",
                |value| value == "0",
                &mut errors,
            );
        }
        errors
    }

    pub fn server_configuration_warnings(&self) -> Result<Vec<String>, String> {
        let mut warnings = Vec::new();
        let max_connections = self
            .setting("max_connections")?
            .setting
            .parse::<u64>()
            .map_err(|err| format!("invalid PostgreSQL max_connections value: {err}"))?;
        if max_connections < self.required_max_connections {
            warnings.push(self.server_change_message(
                "max_connections",
                &format!(">={}", self.required_max_connections),
                "raise the instance connection limit and restart PostgreSQL",
            )?);
        }

        if !parse_postgres_bool(&self.setting("fsync")?.setting, "fsync")? {
            warnings.push(self.server_change_message(
                "fsync",
                "on",
                "enable crash-safe WAL flushing and reload or restart PostgreSQL as required",
            )?);
        }
        if !parse_postgres_bool(
            &self.setting("full_page_writes")?.setting,
            "full_page_writes",
        )? {
            warnings.push(self.server_change_message(
                "full_page_writes",
                "on",
                "enable full-page WAL protection and reload or restart PostgreSQL as required",
            )?);
        }
        Ok(warnings)
    }

    pub fn max_connections(&self) -> Result<u64, String> {
        self.setting("max_connections")?
            .setting
            .parse::<u64>()
            .map_err(|err| format!("invalid PostgreSQL max_connections value: {err}"))
    }

    fn setting(&self, name: &str) -> Result<&PostgresSettingObservation, String> {
        self.settings
            .get(name)
            .ok_or_else(|| format!("PostgreSQL setting {name} is missing"))
    }

    fn require_session_value<F>(
        &self,
        name: &str,
        required: &str,
        matches: F,
        errors: &mut Vec<String>,
    ) where
        F: FnOnce(&str) -> bool,
    {
        let Some(setting) = self.settings.get(name) else {
            errors.push(format!("PostgreSQL session setting {name} is missing"));
            return;
        };
        if !matches(setting.setting.trim()) {
            errors.push(format!(
                "PostgreSQL session setting {name}={} does not match required value {required}",
                setting.display_value()
            ));
        }
    }

    fn server_change_message(
        &self,
        name: &str,
        required: &str,
        action: &str,
    ) -> Result<String, String> {
        let setting = self.setting(name)?;
        Ok(format!(
            "setting={} observed={} required={} context={} pending_restart={} action={}",
            setting.name,
            setting.display_value(),
            required,
            setting.context,
            setting.pending_restart,
            action
        ))
    }
}

fn parse_postgres_bool(value: &str, name: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "t" | "true" | "yes" | "on" => Ok(true),
        "0" | "f" | "false" | "no" | "off" => Ok(false),
        other => Err(format!("invalid PostgreSQL {name} boolean value: {other}")),
    }
}
