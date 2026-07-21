// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use super::{PgEndpointProbe, PgEndpointRole, PgObservedEndpointRole};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_PG_HEALTH_FAILURE_THRESHOLD: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PgConnectionPurpose {
    Read,
    Write,
    Control,
    Lease,
}

impl PgConnectionPurpose {
    pub const ALL: [Self; 4] = [Self::Read, Self::Write, Self::Control, Self::Lease];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Control => "control",
            Self::Lease => "lease",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgPoolIsolationMode {
    SharedFallback,
    DedicatedLanes,
}

impl PgPoolIsolationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SharedFallback => "shared-fallback",
            Self::DedicatedLanes => "dedicated-lanes",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgPoolPlan {
    pub total_limit: usize,
    pub mode: PgPoolIsolationMode,
    pub read_limit: usize,
    pub write_limit: usize,
    pub control_limit: usize,
    pub lease_limit: usize,
    pub routing_enabled: bool,
}

impl PgPoolPlan {
    pub fn from_total_limit(total_limit: u64) -> Self {
        let total_limit = usize::try_from(total_limit).unwrap_or(usize::MAX).max(1);

        if total_limit < PgConnectionPurpose::ALL.len() {
            return Self {
                total_limit,
                mode: PgPoolIsolationMode::SharedFallback,
                read_limit: total_limit,
                write_limit: total_limit,
                control_limit: total_limit,
                lease_limit: total_limit,
                routing_enabled: false,
            };
        }

        let control_limit = 1;
        let lease_limit = 1;
        let data_limit = total_limit - control_limit - lease_limit;
        let read_limit = (data_limit / 3).max(1);
        let write_limit = data_limit.saturating_sub(read_limit).max(1);

        Self {
            total_limit,
            mode: PgPoolIsolationMode::DedicatedLanes,
            read_limit,
            write_limit,
            control_limit,
            lease_limit,
            routing_enabled: false,
        }
    }

    pub fn limit_for(&self, purpose: PgConnectionPurpose) -> usize {
        match purpose {
            PgConnectionPurpose::Read => self.read_limit,
            PgConnectionPurpose::Write => self.write_limit,
            PgConnectionPurpose::Control => self.control_limit,
            PgConnectionPurpose::Lease => self.lease_limit,
        }
    }

    pub fn dedicated_slots_sum(&self) -> Option<usize> {
        if self.mode == PgPoolIsolationMode::DedicatedLanes {
            Some(
                self.read_limit
                    .saturating_add(self.write_limit)
                    .saturating_add(self.control_limit)
                    .saturating_add(self.lease_limit),
            )
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgEndpointHealthState {
    Unknown,
    Healthy,
    Degraded,
    Unreachable,
    RoleMismatch,
    Inconsistent,
}

impl PgEndpointHealthState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Unreachable => "unreachable",
            Self::RoleMismatch => "role-mismatch",
            Self::Inconsistent => "inconsistent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgEndpointHealthSnapshot {
    pub authority: String,
    pub configured_role: PgEndpointRole,
    pub observed_role: Option<PgObservedEndpointRole>,
    pub role_matches_config: Option<bool>,
    pub state: PgEndpointHealthState,
    pub consecutive_successes: u64,
    pub consecutive_failures: u64,
    pub total_successes: u64,
    pub total_failures: u64,
    pub last_success_unix_ms: Option<u64>,
    pub last_failure_unix_ms: Option<u64>,
    pub last_error: Option<String>,
    pub eligible_purposes: Vec<PgConnectionPurpose>,
    pub automatic_routing_enabled: bool,
}

impl PgEndpointHealthSnapshot {
    fn new(authority: &str, configured_role: PgEndpointRole) -> Self {
        Self {
            authority: authority.to_string(),
            configured_role,
            observed_role: None,
            role_matches_config: None,
            state: PgEndpointHealthState::Unknown,
            consecutive_successes: 0,
            consecutive_failures: 0,
            total_successes: 0,
            total_failures: 0,
            last_success_unix_ms: None,
            last_failure_unix_ms: None,
            last_error: None,
            eligible_purposes: Vec::new(),
            automatic_routing_enabled: false,
        }
    }

    pub fn eligible_for(&self, purpose: PgConnectionPurpose) -> bool {
        self.eligible_purposes.contains(&purpose)
    }
}

#[derive(Debug, Clone)]
pub struct PgEndpointHealthRegistry {
    entries: Arc<Mutex<BTreeMap<String, PgEndpointHealthSnapshot>>>,
    failure_threshold: u64,
}

impl Default for PgEndpointHealthRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_PG_HEALTH_FAILURE_THRESHOLD)
    }
}

impl PgEndpointHealthRegistry {
    pub fn new(failure_threshold: u64) -> Self {
        Self {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            failure_threshold: failure_threshold.max(1),
        }
    }

    pub fn failure_threshold(&self) -> u64 {
        self.failure_threshold
    }

    pub fn record_probe(
        &self,
        authority: &str,
        configured_role: PgEndpointRole,
        probe: PgEndpointProbe,
    ) -> Result<PgEndpointHealthSnapshot, String> {
        self.record_probe_at(authority, configured_role, probe, now_unix_ms())
    }

    pub fn record_probe_result(
        &self,
        authority: &str,
        configured_role: PgEndpointRole,
        result: Result<PgEndpointProbe, String>,
    ) -> Result<PgEndpointHealthSnapshot, String> {
        match result {
            Ok(probe) => self.record_probe(authority, configured_role, probe),
            Err(err) => self.record_failure(authority, configured_role, &err),
        }
    }

    pub fn record_failure(
        &self,
        authority: &str,
        configured_role: PgEndpointRole,
        error: &str,
    ) -> Result<PgEndpointHealthSnapshot, String> {
        self.record_failure_at(authority, configured_role, error, now_unix_ms())
    }

    pub fn record_probe_at(
        &self,
        authority: &str,
        configured_role: PgEndpointRole,
        probe: PgEndpointProbe,
        now_unix_ms: u64,
    ) -> Result<PgEndpointHealthSnapshot, String> {
        validate_authority(authority)?;
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "PostgreSQL endpoint health registry is poisoned".to_string())?;
        let snapshot = entries
            .entry(authority.to_string())
            .or_insert_with(|| PgEndpointHealthSnapshot::new(authority, configured_role));

        snapshot.configured_role = configured_role;
        snapshot.observed_role = Some(probe.observed_role);
        snapshot.role_matches_config = probe.configured_role_matches(configured_role);
        snapshot.consecutive_successes = snapshot.consecutive_successes.saturating_add(1);
        snapshot.consecutive_failures = 0;
        snapshot.total_successes = snapshot.total_successes.saturating_add(1);
        snapshot.last_success_unix_ms = Some(now_unix_ms);
        snapshot.last_error = None;
        snapshot.eligible_purposes.clear();

        snapshot.state = if !probe.is_consistent() {
            PgEndpointHealthState::Inconsistent
        } else if snapshot.role_matches_config == Some(false) {
            PgEndpointHealthState::RoleMismatch
        } else {
            PgEndpointHealthState::Healthy
        };

        if snapshot.state == PgEndpointHealthState::Healthy {
            snapshot.eligible_purposes.push(PgConnectionPurpose::Read);
            if probe.write_capable() {
                snapshot.eligible_purposes.extend_from_slice(&[
                    PgConnectionPurpose::Write,
                    PgConnectionPurpose::Control,
                    PgConnectionPurpose::Lease,
                ]);
            }
        }

        Ok(snapshot.clone())
    }

    pub fn record_failure_at(
        &self,
        authority: &str,
        configured_role: PgEndpointRole,
        error: &str,
        now_unix_ms: u64,
    ) -> Result<PgEndpointHealthSnapshot, String> {
        validate_authority(authority)?;
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "PostgreSQL endpoint health registry is poisoned".to_string())?;
        let snapshot = entries
            .entry(authority.to_string())
            .or_insert_with(|| PgEndpointHealthSnapshot::new(authority, configured_role));

        snapshot.configured_role = configured_role;
        snapshot.consecutive_successes = 0;
        snapshot.consecutive_failures = snapshot.consecutive_failures.saturating_add(1);
        snapshot.total_failures = snapshot.total_failures.saturating_add(1);
        snapshot.last_failure_unix_ms = Some(now_unix_ms);
        snapshot.last_error = Some(sanitize_error(error));
        snapshot.eligible_purposes.clear();
        snapshot.state = if snapshot.consecutive_failures >= self.failure_threshold {
            PgEndpointHealthState::Unreachable
        } else {
            PgEndpointHealthState::Degraded
        };

        Ok(snapshot.clone())
    }

    pub fn snapshot(&self, authority: &str) -> Result<Option<PgEndpointHealthSnapshot>, String> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| "PostgreSQL endpoint health registry is poisoned".to_string())?;
        Ok(entries.get(authority).cloned())
    }

    pub fn snapshots(&self) -> Result<Vec<PgEndpointHealthSnapshot>, String> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| "PostgreSQL endpoint health registry is poisoned".to_string())?;
        Ok(entries.values().cloned().collect())
    }

    pub fn clear(&self) -> Result<(), String> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "PostgreSQL endpoint health registry is poisoned".to_string())?;
        entries.clear();
        Ok(())
    }
}

fn validate_authority(authority: &str) -> Result<(), String> {
    if authority.trim().is_empty() {
        Err("PostgreSQL endpoint authority must not be empty".to_string())
    } else {
        Ok(())
    }
}

fn sanitize_error(error: &str) -> String {
    error
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(512)
        .collect()
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_dedicated_pool_plan_when_four_lanes_fit() {
        let plan = PgPoolPlan::from_total_limit(10);
        assert_eq!(plan.mode, PgPoolIsolationMode::DedicatedLanes);
        assert_eq!(plan.read_limit, 2);
        assert_eq!(plan.write_limit, 6);
        assert_eq!(plan.control_limit, 1);
        assert_eq!(plan.lease_limit, 1);
        assert_eq!(plan.dedicated_slots_sum(), Some(10));
        assert!(!plan.routing_enabled);
    }

    #[test]
    fn preserves_legacy_shared_capacity_for_small_limits() {
        let plan = PgPoolPlan::from_total_limit(3);
        assert_eq!(plan.mode, PgPoolIsolationMode::SharedFallback);
        for purpose in PgConnectionPurpose::ALL {
            assert_eq!(plan.limit_for(purpose), 3);
        }
        assert_eq!(plan.dedicated_slots_sum(), None);
    }

    #[test]
    fn health_registry_promotes_writable_primary_after_failures() {
        let registry = PgEndpointHealthRegistry::new(2);
        let first = registry
            .record_failure_at("db:5432", PgEndpointRole::Unknown, "connection lost", 10)
            .unwrap();
        assert_eq!(first.state, PgEndpointHealthState::Degraded);
        let second = registry
            .record_failure_at("db:5432", PgEndpointRole::Unknown, "still down", 20)
            .unwrap();
        assert_eq!(second.state, PgEndpointHealthState::Unreachable);

        let healthy = registry
            .record_probe_at(
                "db:5432",
                PgEndpointRole::Unknown,
                PgEndpointProbe::from_flags(false, false),
                30,
            )
            .unwrap();
        assert_eq!(healthy.state, PgEndpointHealthState::Healthy);
        assert_eq!(healthy.consecutive_failures, 0);
        assert_eq!(healthy.consecutive_successes, 1);
        assert_eq!(healthy.total_failures, 2);
        assert_eq!(healthy.total_successes, 1);
        for purpose in PgConnectionPurpose::ALL {
            assert!(healthy.eligible_for(purpose));
        }
    }

    #[test]
    fn replica_is_eligible_only_for_reads() {
        let registry = PgEndpointHealthRegistry::default();
        let snapshot = registry
            .record_probe_at(
                "replica:5432",
                PgEndpointRole::Replica,
                PgEndpointProbe::from_flags(true, true),
                10,
            )
            .unwrap();
        assert_eq!(snapshot.state, PgEndpointHealthState::Healthy);
        assert!(snapshot.eligible_for(PgConnectionPurpose::Read));
        assert!(!snapshot.eligible_for(PgConnectionPurpose::Write));
        assert!(!snapshot.eligible_for(PgConnectionPurpose::Control));
        assert!(!snapshot.eligible_for(PgConnectionPurpose::Lease));
    }

    #[test]
    fn role_mismatch_and_inconsistent_probe_are_not_eligible() {
        let registry = PgEndpointHealthRegistry::default();
        let mismatch = registry
            .record_probe_at(
                "declared-primary:5432",
                PgEndpointRole::Primary,
                PgEndpointProbe::from_flags(true, true),
                10,
            )
            .unwrap();
        assert_eq!(mismatch.state, PgEndpointHealthState::RoleMismatch);
        assert!(mismatch.eligible_purposes.is_empty());

        let inconsistent = registry
            .record_probe_at(
                "broken:5432",
                PgEndpointRole::Unknown,
                PgEndpointProbe::from_flags(true, false),
                20,
            )
            .unwrap();
        assert_eq!(inconsistent.state, PgEndpointHealthState::Inconsistent);
        assert!(inconsistent.eligible_purposes.is_empty());
    }

    #[test]
    fn registry_clone_shares_process_persistent_state() {
        let registry = PgEndpointHealthRegistry::default();
        let clone = registry.clone();
        registry
            .record_failure_at("db:5432", PgEndpointRole::Unknown, "down", 10)
            .unwrap();
        let snapshot = clone.snapshot("db:5432").unwrap().unwrap();
        assert_eq!(snapshot.total_failures, 1);
    }
}
