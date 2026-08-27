// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalTaskLane {
    Read,
    Write,
    Control,
    Lease,
}

impl LogicalTaskLane {
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
pub enum LogicalTaskOperation {
    FileRead,
    FileWrite,
    FileCopy,
    FileImport,
    MetadataRead,
    MetadataWrite,
    LockLease,
    SessionHeartbeat,
    Maintenance,
}

impl LogicalTaskOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileRead => "file-read",
            Self::FileWrite => "file-write",
            Self::FileCopy => "file-copy",
            Self::FileImport => "file-import",
            Self::MetadataRead => "metadata-read",
            Self::MetadataWrite => "metadata-write",
            Self::LockLease => "lock-lease",
            Self::SessionHeartbeat => "session-heartbeat",
            Self::Maintenance => "maintenance",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogicalTaskClass {
    pub lane: LogicalTaskLane,
    pub operation: LogicalTaskOperation,
}

impl LogicalTaskClass {
    pub const fn new(lane: LogicalTaskLane, operation: LogicalTaskOperation) -> Self {
        Self { lane, operation }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalTaskQueueSnapshot {
    pub class: LogicalTaskClass,
    pub admitted_tasks: u64,
    pub completed_tasks: u64,
    pub failed_tasks: u64,
    pub queued_tasks: u64,
    pub active_tasks: u64,
    pub peak_queued_tasks: u64,
    pub peak_active_tasks: u64,
    pub active_transactions: u64,
    pub active_transaction_limit: u64,
    pub payload_in_flight_bytes: u64,
    pub payload_in_flight_limit_bytes: u64,
    pub per_task_buffer_limit_bytes: u64,
    pub backpressure_events: u64,
    pub fairness_yields: u64,
    pub accounting_errors: u64,
}

impl LogicalTaskQueueSnapshot {
    pub fn completed_successfully(&self) -> u64 {
        self.completed_tasks.saturating_sub(self.failed_tasks)
    }

    pub fn active_transaction_headroom(&self) -> u64 {
        self.active_transaction_limit
            .saturating_sub(self.active_transactions)
    }

    pub fn payload_headroom_bytes(&self) -> u64 {
        self.payload_in_flight_limit_bytes
            .saturating_sub(self.payload_in_flight_bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalTaskThroughputSnapshot {
    pub class: LogicalTaskClass,
    pub completed_files: u64,
    pub completed_bytes: u64,
    pub database_batches: u64,
    pub database_batch_rows: u64,
    pub database_batch_bytes: u64,
    pub elapsed_micros: u64,
}

impl LogicalTaskThroughputSnapshot {
    pub fn completed_files_per_second_milli(&self) -> u64 {
        if self.elapsed_micros == 0 {
            return 0;
        }
        self.completed_files
            .saturating_mul(1_000_000)
            .saturating_mul(1_000)
            / self.elapsed_micros
    }

    pub fn completed_bytes_per_second(&self) -> u64 {
        if self.elapsed_micros == 0 {
            return 0;
        }
        self.completed_bytes.saturating_mul(1_000_000) / self.elapsed_micros
    }

    pub fn average_database_batch_rows(&self) -> u64 {
        if self.database_batches == 0 {
            return 0;
        }
        self.database_batch_rows / self.database_batches
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalTaskObservabilitySnapshot {
    pub queue: LogicalTaskQueueSnapshot,
    pub throughput: LogicalTaskThroughputSnapshot,
}

#[derive(Debug, Default)]
struct LogicalTaskQueueObservabilityState {
    admitted_tasks: u64,
    completed_tasks: u64,
    failed_tasks: u64,
    queued_tasks: u64,
    active_tasks: u64,
    peak_queued_tasks: u64,
    peak_active_tasks: u64,
    active_transactions: u64,
    payload_in_flight_bytes: u64,
    backpressure_events: u64,
    fairness_yields: u64,
    completed_files: u64,
    completed_bytes: u64,
    database_batches: u64,
    database_batch_rows: u64,
    database_batch_bytes: u64,
    elapsed_micros: u64,
    accounting_errors: u64,
}

#[derive(Debug)]
pub struct LogicalTaskQueueObservability {
    class: LogicalTaskClass,
    active_transaction_limit: u64,
    payload_in_flight_limit_bytes: u64,
    per_task_buffer_limit_bytes: u64,
    state: Mutex<LogicalTaskQueueObservabilityState>,
}

#[derive(Debug)]
struct LogicalTaskAdmissionWaiter {
    ticket: u64,
    ready: Mutex<bool>,
    ready_changed: Condvar,
    observability: Arc<LogicalTaskQueueObservability>,
}

#[derive(Debug, Default)]
struct LogicalTaskAdmissionState {
    active_tasks: u64,
    next_ticket: u64,
    serving_ticket: u64,
    waiters: VecDeque<Arc<LogicalTaskAdmissionWaiter>>,
}

#[derive(Debug)]
pub struct LogicalTaskAdmissionGate {
    active_task_limit: u64,
    state: Mutex<LogicalTaskAdmissionState>,
}

#[derive(Debug)]
pub struct LogicalTaskAdmissionPermit {
    gate: Arc<LogicalTaskAdmissionGate>,
    observability: Option<Arc<LogicalTaskQueueObservability>>,
    acquired: bool,
}

#[derive(Debug)]
pub struct LogicalTaskObservation {
    observability: Arc<LogicalTaskQueueObservability>,
    payload_bytes: u64,
    transaction_active: bool,
    started: Instant,
    completed_files: u64,
    completed_bytes: u64,
    failed: bool,
    finished: bool,
}

impl LogicalTaskObservation {
    fn new(
        observability: Arc<LogicalTaskQueueObservability>,
        payload_bytes: u64,
        transaction_active: bool,
    ) -> Self {
        Self {
            observability,
            payload_bytes,
            transaction_active,
            started: Instant::now(),
            completed_files: 0,
            completed_bytes: 0,
            failed: true,
            finished: false,
        }
    }

    pub fn complete(mut self, completed_files: u64, completed_bytes: u64) {
        self.completed_files = completed_files;
        self.completed_bytes = completed_bytes;
        self.failed = false;
        self.finish();
    }

    pub fn fail(mut self) {
        self.finish();
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.observability.finish_task(
            self.payload_bytes,
            self.transaction_active,
            self.failed,
            self.started.elapsed(),
            self.completed_files,
            self.completed_bytes,
        );
        self.finished = true;
    }
}

impl Drop for LogicalTaskObservation {
    fn drop(&mut self) {
        self.finish();
    }
}

impl LogicalTaskAdmissionWaiter {
    fn new(ticket: u64, observability: Arc<LogicalTaskQueueObservability>) -> Self {
        Self {
            ticket,
            ready: Mutex::new(false),
            ready_changed: Condvar::new(),
            observability,
        }
    }

    fn wait(&self) {
        let mut ready = match self.ready.lock() {
            Ok(guard) => guard,
            Err(err) => {
                self.observability.record_accounting_error();
                err.into_inner()
            }
        };
        while !*ready {
            ready = match self.ready_changed.wait(ready) {
                Ok(guard) => guard,
                Err(err) => {
                    self.observability.record_accounting_error();
                    err.into_inner()
                }
            };
        }
    }

    fn signal(&self) {
        let mut ready = match self.ready.lock() {
            Ok(guard) => guard,
            Err(err) => {
                self.observability.record_accounting_error();
                err.into_inner()
            }
        };
        *ready = true;
        drop(ready);
        self.ready_changed.notify_one();
    }
}

impl LogicalTaskAdmissionGate {
    pub fn new(active_task_limit: u64) -> Self {
        Self {
            active_task_limit,
            state: Mutex::new(LogicalTaskAdmissionState::default()),
        }
    }

    pub fn active_task_limit(&self) -> u64 {
        self.active_task_limit
    }

    fn reserve_next_waiter(
        &self,
        state: &mut LogicalTaskAdmissionState,
    ) -> Option<Arc<LogicalTaskAdmissionWaiter>> {
        if state.active_tasks >= self.active_task_limit {
            return None;
        }

        let waiter = state.waiters.pop_front()?;
        if waiter.ticket != state.serving_ticket {
            waiter.observability.record_accounting_error();
            state.serving_ticket = waiter.ticket;
        }

        state.active_tasks = state.active_tasks.saturating_add(1);
        state.serving_ticket = state.serving_ticket.wrapping_add(1);
        Some(waiter)
    }

    pub fn observe_task(
        self: &Arc<Self>,
        observability: &Arc<LogicalTaskQueueObservability>,
        payload_bytes: u64,
        starts_transaction: bool,
    ) -> (LogicalTaskAdmissionPermit, LogicalTaskObservation) {
        if self.active_task_limit == 0 {
            return (
                LogicalTaskAdmissionPermit {
                    gate: Arc::clone(self),
                    observability: None,
                    acquired: false,
                },
                observability.observe_task(payload_bytes, starts_transaction),
            );
        }

        observability.admit_task();
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(err) => {
                observability.record_accounting_error();
                err.into_inner()
            }
        };

        let ticket = state.next_ticket;
        state.next_ticket = state.next_ticket.wrapping_add(1);
        let has_older_waiter = !state.waiters.is_empty();
        let capacity_is_full = state.active_tasks >= self.active_task_limit;
        let waiter = Arc::new(LogicalTaskAdmissionWaiter::new(
            ticket,
            Arc::clone(observability),
        ));
        state.waiters.push_back(Arc::clone(&waiter));
        let waiter_to_wake = self.reserve_next_waiter(&mut state);
        drop(state);

        if capacity_is_full || has_older_waiter {
            observability.record_backpressure();
        }
        if has_older_waiter {
            observability.record_fairness_yield();
        }
        if let Some(waiter_to_wake) = waiter_to_wake {
            waiter_to_wake.signal();
        }
        waiter.wait();

        let observation = observability.observe_admitted_task(payload_bytes, starts_transaction);
        (
            LogicalTaskAdmissionPermit {
                gate: Arc::clone(self),
                observability: Some(Arc::clone(observability)),
                acquired: true,
            },
            observation,
        )
    }
}

impl Drop for LogicalTaskAdmissionPermit {
    fn drop(&mut self) {
        if !self.acquired {
            return;
        }
        let Some(observability) = self.observability.as_ref() else {
            return;
        };

        let mut state = match self.gate.state.lock() {
            Ok(guard) => guard,
            Err(err) => {
                observability.record_accounting_error();
                err.into_inner()
            }
        };
        let waiter_to_wake = match state.active_tasks.checked_sub(1) {
            Some(value) => {
                state.active_tasks = value;
                self.gate.reserve_next_waiter(&mut state)
            }
            None => {
                observability.record_accounting_error();
                None
            }
        };
        drop(state);

        if let Some(waiter_to_wake) = waiter_to_wake {
            waiter_to_wake.signal();
        }
    }
}

impl LogicalTaskQueueObservability {
    pub fn new(
        class: LogicalTaskClass,
        active_transaction_limit: u64,
        payload_in_flight_limit_bytes: u64,
        per_task_buffer_limit_bytes: u64,
    ) -> Self {
        Self {
            class,
            active_transaction_limit,
            payload_in_flight_limit_bytes,
            per_task_buffer_limit_bytes,
            state: Mutex::new(LogicalTaskQueueObservabilityState::default()),
        }
    }

    pub fn observe_task(
        self: &Arc<Self>,
        payload_bytes: u64,
        starts_transaction: bool,
    ) -> LogicalTaskObservation {
        self.admit_and_start_task(payload_bytes, starts_transaction);
        LogicalTaskObservation::new(Arc::clone(self), payload_bytes, starts_transaction)
    }

    pub fn observe_admitted_task(
        self: &Arc<Self>,
        payload_bytes: u64,
        starts_transaction: bool,
    ) -> LogicalTaskObservation {
        self.start_task(payload_bytes, starts_transaction);
        LogicalTaskObservation::new(Arc::clone(self), payload_bytes, starts_transaction)
    }

    fn admit_and_start_task(&self, payload_bytes: u64, starts_transaction: bool) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.admitted_tasks = state.admitted_tasks.saturating_add(1);
        state.queued_tasks = state.queued_tasks.saturating_add(1);
        state.peak_queued_tasks = state.peak_queued_tasks.max(state.queued_tasks);
        state.queued_tasks = state.queued_tasks.saturating_sub(1);
        state.active_tasks = state.active_tasks.saturating_add(1);
        state.peak_active_tasks = state.peak_active_tasks.max(state.active_tasks);
        state.payload_in_flight_bytes = state.payload_in_flight_bytes.saturating_add(payload_bytes);
        if starts_transaction {
            state.active_transactions = state.active_transactions.saturating_add(1);
        }
    }

    pub fn admit_task(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.admitted_tasks = state.admitted_tasks.saturating_add(1);
        state.queued_tasks = state.queued_tasks.saturating_add(1);
        state.peak_queued_tasks = state.peak_queued_tasks.max(state.queued_tasks);
    }

    pub fn start_task(&self, payload_bytes: u64, starts_transaction: bool) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        match state.queued_tasks.checked_sub(1) {
            Some(queued_tasks) => state.queued_tasks = queued_tasks,
            None => state.accounting_errors = state.accounting_errors.saturating_add(1),
        }
        state.active_tasks = state.active_tasks.saturating_add(1);
        state.peak_active_tasks = state.peak_active_tasks.max(state.active_tasks);
        state.payload_in_flight_bytes = state.payload_in_flight_bytes.saturating_add(payload_bytes);
        if starts_transaction {
            state.active_transactions = state.active_transactions.saturating_add(1);
        }
    }

    pub fn finish_task(
        &self,
        payload_bytes: u64,
        ends_transaction: bool,
        failed: bool,
        elapsed: Duration,
        completed_files: u64,
        completed_bytes: u64,
    ) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.completed_tasks = state.completed_tasks.saturating_add(1);
        if failed {
            state.failed_tasks = state.failed_tasks.saturating_add(1);
        }
        match state.active_tasks.checked_sub(1) {
            Some(active_tasks) => state.active_tasks = active_tasks,
            None => state.accounting_errors = state.accounting_errors.saturating_add(1),
        }
        match state.payload_in_flight_bytes.checked_sub(payload_bytes) {
            Some(payload_in_flight_bytes) => {
                state.payload_in_flight_bytes = payload_in_flight_bytes
            }
            None => {
                state.payload_in_flight_bytes = 0;
                state.accounting_errors = state.accounting_errors.saturating_add(1);
            }
        }
        if ends_transaction {
            match state.active_transactions.checked_sub(1) {
                Some(active_transactions) => state.active_transactions = active_transactions,
                None => state.accounting_errors = state.accounting_errors.saturating_add(1),
            }
        }
        state.completed_files = state.completed_files.saturating_add(completed_files);
        state.completed_bytes = state.completed_bytes.saturating_add(completed_bytes);
        state.elapsed_micros = state
            .elapsed_micros
            .saturating_add(duration_micros(elapsed));
    }

    pub fn record_database_batch(&self, rows: u64, bytes: u64) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.database_batches = state.database_batches.saturating_add(1);
        state.database_batch_rows = state.database_batch_rows.saturating_add(rows);
        state.database_batch_bytes = state.database_batch_bytes.saturating_add(bytes);
    }

    pub fn record_backpressure(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.backpressure_events = state.backpressure_events.saturating_add(1);
    }

    pub fn record_fairness_yield(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.fairness_yields = state.fairness_yields.saturating_add(1);
    }

    pub fn record_accounting_error(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.accounting_errors = state.accounting_errors.saturating_add(1);
    }

    pub fn snapshot(&self) -> Result<LogicalTaskObservabilitySnapshot, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "logical task queue observability state is poisoned".to_string())?;
        Ok(LogicalTaskObservabilitySnapshot {
            queue: LogicalTaskQueueSnapshot {
                class: self.class,
                admitted_tasks: state.admitted_tasks,
                completed_tasks: state.completed_tasks,
                failed_tasks: state.failed_tasks,
                queued_tasks: state.queued_tasks,
                active_tasks: state.active_tasks,
                peak_queued_tasks: state.peak_queued_tasks,
                peak_active_tasks: state.peak_active_tasks,
                active_transactions: state.active_transactions,
                active_transaction_limit: self.active_transaction_limit,
                payload_in_flight_bytes: state.payload_in_flight_bytes,
                payload_in_flight_limit_bytes: self.payload_in_flight_limit_bytes,
                per_task_buffer_limit_bytes: self.per_task_buffer_limit_bytes,
                backpressure_events: state.backpressure_events,
                fairness_yields: state.fairness_yields,
                accounting_errors: state.accounting_errors,
            },
            throughput: LogicalTaskThroughputSnapshot {
                class: self.class,
                completed_files: state.completed_files,
                completed_bytes: state.completed_bytes,
                database_batches: state.database_batches,
                database_batch_rows: state.database_batch_rows,
                database_batch_bytes: state.database_batch_bytes,
                elapsed_micros: state.elapsed_micros,
            },
        })
    }

    pub fn queue_snapshot(&self) -> Result<LogicalTaskQueueSnapshot, String> {
        Ok(self.snapshot()?.queue)
    }

    pub fn throughput_snapshot(&self) -> Result<LogicalTaskThroughputSnapshot, String> {
        Ok(self.snapshot()?.throughput)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbRepoTransactionAdmissionSnapshot {
    pub limit: u64,
    pub active: u64,
    pub queued: u64,
    pub peak_active: u64,
    pub peak_queued: u64,
    pub admission_count: u64,
    pub backpressure_events: u64,
    pub fairness_yields: u64,
    pub accounting_errors: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbRepoPoolObservabilitySnapshot {
    pub connection_limit: usize,
    pub live_connections: usize,
    pub idle_write_connections: usize,
    pub idle_control_connections: usize,
    pub active_connections: usize,
    pub queued_acquisitions: usize,
    pub peak_active_connections: usize,
    pub peak_queued_acquisitions: usize,
    pub acquisition_count: u64,
    pub acquisition_wait_micros_total: u64,
    pub acquisition_wait_micros_max: u64,
    pub connection_create_count: u64,
    pub connection_create_failures: u64,
    pub connection_create_micros_total: u64,
    pub connection_create_micros_max: u64,
    pub operation_count: u64,
    pub operation_failures: u64,
    pub operation_micros_total: u64,
    pub operation_micros_max: u64,
    pub replay_count: u64,
    pub stale_connection_discards: u64,
    pub transaction_count: u64,
    pub transaction_failures: u64,
    pub transaction_micros_total: u64,
    pub transaction_micros_max: u64,
    pub write_transaction_admission: DbRepoTransactionAdmissionSnapshot,
    pub control_transaction_admission: DbRepoTransactionAdmissionSnapshot,
    pub heartbeat_count: u64,
    pub heartbeat_failures: u64,
    pub heartbeat_schedule_delay_micros_total: u64,
    pub heartbeat_schedule_delay_micros_max: u64,
    pub heartbeat_execution_micros_total: u64,
    pub heartbeat_execution_micros_max: u64,
}

impl DbRepoPoolObservabilitySnapshot {
    pub fn idle_connections(&self) -> usize {
        self.idle_write_connections
            .saturating_add(self.idle_control_connections)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbRepoConnectionRoutingSnapshot {
    pub endpoint_routing_enabled: bool,
    pub runtime_failover_enabled: bool,
    pub target_count: usize,
    pub active_authority: String,
    pub generation: u64,
    pub failover_count: u64,
    pub connection_failures: u64,
    pub role_rejections: u64,
    pub last_failed_authority: Option<String>,
    pub replica_read_routing_enabled: bool,
    pub replica_target_count: usize,
    pub replica_active_authority: Option<String>,
    pub required_primary_wal_lsn: Option<String>,
    pub primary_wal_lsn_updates: u64,
    pub primary_wal_lsn_capture_failures: u64,
    pub replica_consistency_checks: u64,
    pub replica_consistency_passes: u64,
    pub replica_reads: u64,
    pub replica_lag_fallbacks: u64,
    pub replica_read_failures: u64,
    pub primary_read_fallbacks: u64,
    pub replica_scoring_enabled: bool,
    pub replica_active_score: Option<u64>,
    pub replica_active_replay_lag_bytes: Option<u64>,
    pub replica_active_connection_latency_micros: Option<u64>,
    pub replica_active_operation_latency_micros: Option<u64>,
    pub replica_score_selections: u64,
    pub replica_score_switches: u64,
    pub replica_hysteresis_keeps: u64,
    pub replica_circuit_breaker_skips: u64,
    pub replica_circuit_open_targets: usize,
    pub replica_pool_pressure_fallbacks: u64,
    pub primary_promotion_guard_enabled: bool,
    pub primary_system_identifier: Option<String>,
    pub primary_server_fingerprint: Option<String>,
    pub primary_guard_scans: u64,
    pub primary_guard_unreachable_candidates: u64,
    pub primary_guard_role_rejections: u64,
    pub primary_guard_cluster_identity_rejections: u64,
    pub primary_guard_split_brain_rejections: u64,
    pub primary_guard_no_primary_rejections: u64,
    pub primary_guard_last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbRepoObservabilitySnapshot {
    pub pool: DbRepoPoolObservabilitySnapshot,
    pub routing: DbRepoConnectionRoutingSnapshot,
    pub payload: DbRepoPayloadObservabilitySnapshot,
    pub global_payload: DbRepoPayloadObservabilitySnapshot,
    pub persist_buffer_chunk_blocks: u64,
    pub persist_copy_send_buffer_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresPressureSnapshot {
    pub database_connections: u64,
    pub activity_connections: u64,
    pub activity_active: u64,
    pub activity_idle: u64,
    pub activity_idle_in_transaction: u64,
    pub temp_files: u64,
    pub temp_bytes: u64,
    pub deadlocks: u64,
    pub shared_buffers: String,
    pub work_mem: String,
    pub maintenance_work_mem: String,
    pub temp_buffers: String,
    pub current_backend_memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbRepoPayloadObservabilitySnapshot {
    pub in_flight_bytes: u64,
    pub peak_in_flight_bytes: u64,
    pub in_flight_limit_bytes: u64,
    pub reserved_bytes: u64,
    pub queued_bytes: u64,
    pub peak_reserved_bytes: u64,
    pub peak_queued_bytes: u64,
    pub queued_requests: u64,
    pub peak_queued_requests: u64,
    pub admission_count: u64,
    pub backpressure_events: u64,
    pub fairness_yields: u64,
    pub oversized_admissions: u64,
    pub budget_accounting_errors: u64,
    pub accounting_errors: u64,
    pub persist_operation_count: u64,
    pub persist_operation_failures: u64,
    pub persist_input_rows_total: u64,
    pub persist_input_rows_max: u64,
    pub persist_input_bytes_total: u64,
    pub persist_input_bytes_max: u64,
    pub persist_micros_total: u64,
    pub persist_micros_max: u64,
    pub persist_transaction_count: u64,
    pub persist_transaction_failures: u64,
    pub persist_transaction_micros_total: u64,
    pub persist_transaction_micros_max: u64,
    pub persist_copy_stage_count: u64,
    pub persist_copy_stage_micros_total: u64,
    pub persist_copy_stage_micros_max: u64,
    pub persist_data_blocks_merge_count: u64,
    pub persist_data_blocks_merge_micros_total: u64,
    pub persist_data_blocks_merge_micros_max: u64,
    pub quota_lock_wait_count: u64,
    pub quota_lock_wait_micros_total: u64,
    pub quota_lock_wait_micros_max: u64,
    pub quota_lock_held_count: u64,
    pub quota_lock_held_micros_total: u64,
    pub quota_lock_held_micros_max: u64,
    pub quota_final_check_count: u64,
    pub quota_final_check_micros_total: u64,
    pub quota_final_check_micros_max: u64,
}

#[derive(Debug, Default)]
struct DbRepoPayloadObservabilityState {
    in_flight_bytes: u64,
    peak_in_flight_bytes: u64,
    accounting_errors: u64,
    persist_operation_count: u64,
    persist_operation_failures: u64,
    persist_input_rows_total: u64,
    persist_input_rows_max: u64,
    persist_input_bytes_total: u64,
    persist_input_bytes_max: u64,
    persist_micros_total: u64,
    persist_micros_max: u64,
    persist_transaction_count: u64,
    persist_transaction_failures: u64,
    persist_transaction_micros_total: u64,
    persist_transaction_micros_max: u64,
    persist_copy_stage_count: u64,
    persist_copy_stage_micros_total: u64,
    persist_copy_stage_micros_max: u64,
    persist_data_blocks_merge_count: u64,
    persist_data_blocks_merge_micros_total: u64,
    persist_data_blocks_merge_micros_max: u64,
    quota_lock_wait_count: u64,
    quota_lock_wait_micros_total: u64,
    quota_lock_wait_micros_max: u64,
    quota_lock_held_count: u64,
    quota_lock_held_micros_total: u64,
    quota_lock_held_micros_max: u64,
    quota_final_check_count: u64,
    quota_final_check_micros_total: u64,
    quota_final_check_micros_max: u64,
}

#[derive(Debug)]
struct DbRepoPayloadBudgetWaiter {
    ticket: u64,
    requested_bytes: u64,
    ready: Mutex<bool>,
    ready_changed: Condvar,
}

impl DbRepoPayloadBudgetWaiter {
    fn new(ticket: u64, requested_bytes: u64) -> Self {
        Self {
            ticket,
            requested_bytes,
            ready: Mutex::new(false),
            ready_changed: Condvar::new(),
        }
    }

    fn wait(&self) {
        let mut ready = self.ready.lock().unwrap_or_else(|err| err.into_inner());
        while !*ready {
            ready = self
                .ready_changed
                .wait(ready)
                .unwrap_or_else(|err| err.into_inner());
        }
    }

    fn signal(&self) {
        let mut ready = self.ready.lock().unwrap_or_else(|err| err.into_inner());
        *ready = true;
        drop(ready);
        self.ready_changed.notify_one();
    }
}

#[derive(Debug, Default)]
struct DbRepoPayloadBudgetState {
    reserved_bytes: u64,
    queued_bytes: u64,
    peak_reserved_bytes: u64,
    peak_queued_bytes: u64,
    queued_requests: u64,
    peak_queued_requests: u64,
    next_ticket: u64,
    serving_ticket: u64,
    waiters: VecDeque<Arc<DbRepoPayloadBudgetWaiter>>,
    admission_count: u64,
    backpressure_events: u64,
    fairness_yields: u64,
    oversized_admissions: u64,
    accounting_errors: u64,
}

#[derive(Debug)]
pub struct DbRepoPayloadBudgetPermit {
    tracker: Arc<DbRepoPayloadObservability>,
    reserved_bytes: u64,
    acquired: bool,
}

#[derive(Debug, Default)]
pub struct DbRepoPayloadObservability {
    state: Mutex<DbRepoPayloadObservabilityState>,
    in_flight_limit_bytes: u64,
    budget_state: Mutex<DbRepoPayloadBudgetState>,
}

fn accumulate_observation(target: &mut u64, value: u64) -> bool {
    match target.checked_add(value) {
        Some(total) => {
            *target = total;
            true
        }
        None => {
            *target = u64::MAX;
            false
        }
    }
}

fn duration_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

impl DbRepoPayloadObservability {
    pub fn with_in_flight_limit_bytes(in_flight_limit_bytes: u64) -> Self {
        Self {
            state: Mutex::new(DbRepoPayloadObservabilityState::default()),
            in_flight_limit_bytes,
            budget_state: Mutex::new(DbRepoPayloadBudgetState::default()),
        }
    }

    fn lock_budget_state(&self) -> std::sync::MutexGuard<'_, DbRepoPayloadBudgetState> {
        match self.budget_state.lock() {
            Ok(state) => state,
            Err(err) => {
                let mut state = err.into_inner();
                state.accounting_errors = state.accounting_errors.saturating_add(1);
                state
            }
        }
    }

    fn request_fits_budget(&self, reserved_bytes: u64, requested_bytes: u64) -> bool {
        if self.in_flight_limit_bytes == 0 {
            return true;
        }
        if reserved_bytes == 0 && requested_bytes > self.in_flight_limit_bytes {
            return true;
        }
        requested_bytes <= self.in_flight_limit_bytes.saturating_sub(reserved_bytes)
    }

    fn reserve_ready_waiters(
        &self,
        state: &mut DbRepoPayloadBudgetState,
    ) -> Vec<Arc<DbRepoPayloadBudgetWaiter>> {
        let mut ready = Vec::new();
        while let Some(waiter) = state.waiters.front() {
            let requested_bytes = waiter.requested_bytes;
            if !self.request_fits_budget(state.reserved_bytes, requested_bytes) {
                break;
            }
            let waiter = state.waiters.pop_front().expect("front waiter disappeared");
            if waiter.ticket != state.serving_ticket {
                state.accounting_errors = state.accounting_errors.saturating_add(1);
                state.serving_ticket = waiter.ticket;
            }
            match state.queued_requests.checked_sub(1) {
                Some(value) => state.queued_requests = value,
                None => state.accounting_errors = state.accounting_errors.saturating_add(1),
            }
            match state.queued_bytes.checked_sub(requested_bytes) {
                Some(value) => state.queued_bytes = value,
                None => {
                    state.queued_bytes = 0;
                    state.accounting_errors = state.accounting_errors.saturating_add(1);
                }
            }
            match state.reserved_bytes.checked_add(requested_bytes) {
                Some(value) => state.reserved_bytes = value,
                None => {
                    state.reserved_bytes = u64::MAX;
                    state.accounting_errors = state.accounting_errors.saturating_add(1);
                }
            }
            state.peak_reserved_bytes = state.peak_reserved_bytes.max(state.reserved_bytes);
            state.admission_count = state.admission_count.saturating_add(1);
            if requested_bytes > self.in_flight_limit_bytes {
                state.oversized_admissions = state.oversized_admissions.saturating_add(1);
            }
            state.serving_ticket = state.serving_ticket.wrapping_add(1);
            ready.push(waiter);
        }
        ready
    }

    pub fn acquire_payload_budget(
        self: &Arc<Self>,
        requested_bytes: u64,
    ) -> DbRepoPayloadBudgetPermit {
        if self.in_flight_limit_bytes == 0 || requested_bytes == 0 {
            return DbRepoPayloadBudgetPermit {
                tracker: Arc::clone(self),
                reserved_bytes: 0,
                acquired: false,
            };
        }

        let mut state = self.lock_budget_state();
        let ticket = state.next_ticket;
        state.next_ticket = state.next_ticket.wrapping_add(1);
        let had_older_waiter = !state.waiters.is_empty();
        let capacity_blocked = !self.request_fits_budget(state.reserved_bytes, requested_bytes);
        let waiter = Arc::new(DbRepoPayloadBudgetWaiter::new(ticket, requested_bytes));
        state.waiters.push_back(Arc::clone(&waiter));
        state.queued_requests = state.queued_requests.saturating_add(1);
        state.peak_queued_requests = state.peak_queued_requests.max(state.queued_requests);
        match state.queued_bytes.checked_add(requested_bytes) {
            Some(value) => state.queued_bytes = value,
            None => {
                state.queued_bytes = u64::MAX;
                state.accounting_errors = state.accounting_errors.saturating_add(1);
            }
        }
        state.peak_queued_bytes = state.peak_queued_bytes.max(state.queued_bytes);
        if had_older_waiter || capacity_blocked {
            state.backpressure_events = state.backpressure_events.saturating_add(1);
        }
        if had_older_waiter {
            state.fairness_yields = state.fairness_yields.saturating_add(1);
        }
        let ready = self.reserve_ready_waiters(&mut state);
        drop(state);
        for ready_waiter in ready {
            ready_waiter.signal();
        }
        waiter.wait();

        DbRepoPayloadBudgetPermit {
            tracker: Arc::clone(self),
            reserved_bytes: requested_bytes,
            acquired: true,
        }
    }

    pub fn begin_persist(&self, input_bytes: u64) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let Some(in_flight_bytes) = state.in_flight_bytes.checked_add(input_bytes) else {
            state.accounting_errors = state.accounting_errors.saturating_add(1);
            return false;
        };
        state.in_flight_bytes = in_flight_bytes;
        state.peak_in_flight_bytes = state.peak_in_flight_bytes.max(state.in_flight_bytes);
        true
    }

    pub fn finish_persist(
        &self,
        input_bytes: u64,
        input_rows: u64,
        elapsed: Duration,
        failed: bool,
    ) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        match state.in_flight_bytes.checked_sub(input_bytes) {
            Some(in_flight_bytes) => state.in_flight_bytes = in_flight_bytes,
            None => {
                state.in_flight_bytes = 0;
                state.accounting_errors = state.accounting_errors.saturating_add(1);
            }
        }
        let mut accounting_error = !accumulate_observation(&mut state.persist_operation_count, 1);
        if failed {
            accounting_error |= !accumulate_observation(&mut state.persist_operation_failures, 1);
        }
        accounting_error |=
            !accumulate_observation(&mut state.persist_input_rows_total, input_rows);
        state.persist_input_rows_max = state.persist_input_rows_max.max(input_rows);
        accounting_error |=
            !accumulate_observation(&mut state.persist_input_bytes_total, input_bytes);
        state.persist_input_bytes_max = state.persist_input_bytes_max.max(input_bytes);
        let elapsed_micros = duration_micros(elapsed);
        accounting_error |=
            !accumulate_observation(&mut state.persist_micros_total, elapsed_micros);
        state.persist_micros_max = state.persist_micros_max.max(elapsed_micros);
        if accounting_error {
            state.accounting_errors = state.accounting_errors.saturating_add(1);
        }
    }

    pub fn record_persist_transaction(&self, elapsed: Duration, failed: bool) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let elapsed_micros = duration_micros(elapsed);
        let mut accounting_error = !accumulate_observation(&mut state.persist_transaction_count, 1);
        accounting_error |=
            !accumulate_observation(&mut state.persist_transaction_micros_total, elapsed_micros);
        state.persist_transaction_micros_max =
            state.persist_transaction_micros_max.max(elapsed_micros);
        if failed {
            accounting_error |= !accumulate_observation(&mut state.persist_transaction_failures, 1);
        }
        if accounting_error {
            state.accounting_errors = state.accounting_errors.saturating_add(1);
        }
    }

    pub fn record_persist_copy_stage(&self, elapsed: Duration) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let elapsed_micros = duration_micros(elapsed);
        let mut accounting_error = !accumulate_observation(&mut state.persist_copy_stage_count, 1);
        accounting_error |=
            !accumulate_observation(&mut state.persist_copy_stage_micros_total, elapsed_micros);
        state.persist_copy_stage_micros_max =
            state.persist_copy_stage_micros_max.max(elapsed_micros);
        if accounting_error {
            state.accounting_errors = state.accounting_errors.saturating_add(1);
        }
    }

    pub fn record_persist_data_blocks_merge(&self, elapsed: Duration) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let elapsed_micros = duration_micros(elapsed);
        let mut accounting_error =
            !accumulate_observation(&mut state.persist_data_blocks_merge_count, 1);
        accounting_error |= !accumulate_observation(
            &mut state.persist_data_blocks_merge_micros_total,
            elapsed_micros,
        );
        state.persist_data_blocks_merge_micros_max = state
            .persist_data_blocks_merge_micros_max
            .max(elapsed_micros);
        if accounting_error {
            state.accounting_errors = state.accounting_errors.saturating_add(1);
        }
    }

    pub fn record_quota_lock_wait(&self, elapsed: Duration) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let elapsed_micros = duration_micros(elapsed);
        let mut accounting_error = !accumulate_observation(&mut state.quota_lock_wait_count, 1);
        accounting_error |=
            !accumulate_observation(&mut state.quota_lock_wait_micros_total, elapsed_micros);
        state.quota_lock_wait_micros_max = state.quota_lock_wait_micros_max.max(elapsed_micros);
        if accounting_error {
            state.accounting_errors = state.accounting_errors.saturating_add(1);
        }
    }

    pub fn record_quota_lock_held(&self, elapsed: Duration) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let elapsed_micros = duration_micros(elapsed);
        let mut accounting_error = !accumulate_observation(&mut state.quota_lock_held_count, 1);
        accounting_error |=
            !accumulate_observation(&mut state.quota_lock_held_micros_total, elapsed_micros);
        state.quota_lock_held_micros_max = state.quota_lock_held_micros_max.max(elapsed_micros);
        if accounting_error {
            state.accounting_errors = state.accounting_errors.saturating_add(1);
        }
    }

    pub fn record_quota_final_check(&self, elapsed: Duration) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let elapsed_micros = duration_micros(elapsed);
        let mut accounting_error = !accumulate_observation(&mut state.quota_final_check_count, 1);
        accounting_error |=
            !accumulate_observation(&mut state.quota_final_check_micros_total, elapsed_micros);
        state.quota_final_check_micros_max = state.quota_final_check_micros_max.max(elapsed_micros);
        if accounting_error {
            state.accounting_errors = state.accounting_errors.saturating_add(1);
        }
    }

    pub fn snapshot(&self) -> Result<DbRepoPayloadObservabilitySnapshot, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "payload observability state is poisoned".to_string())?;
        let budget = self.lock_budget_state();
        Ok(DbRepoPayloadObservabilitySnapshot {
            in_flight_bytes: state.in_flight_bytes,
            peak_in_flight_bytes: state.peak_in_flight_bytes,
            in_flight_limit_bytes: self.in_flight_limit_bytes,
            reserved_bytes: budget.reserved_bytes,
            queued_bytes: budget.queued_bytes,
            peak_reserved_bytes: budget.peak_reserved_bytes,
            peak_queued_bytes: budget.peak_queued_bytes,
            queued_requests: budget.queued_requests,
            peak_queued_requests: budget.peak_queued_requests,
            admission_count: budget.admission_count,
            backpressure_events: budget.backpressure_events,
            fairness_yields: budget.fairness_yields,
            oversized_admissions: budget.oversized_admissions,
            budget_accounting_errors: budget.accounting_errors,
            accounting_errors: state.accounting_errors,
            persist_operation_count: state.persist_operation_count,
            persist_operation_failures: state.persist_operation_failures,
            persist_input_rows_total: state.persist_input_rows_total,
            persist_input_rows_max: state.persist_input_rows_max,
            persist_input_bytes_total: state.persist_input_bytes_total,
            persist_input_bytes_max: state.persist_input_bytes_max,
            persist_micros_total: state.persist_micros_total,
            persist_micros_max: state.persist_micros_max,
            persist_transaction_count: state.persist_transaction_count,
            persist_transaction_failures: state.persist_transaction_failures,
            persist_transaction_micros_total: state.persist_transaction_micros_total,
            persist_transaction_micros_max: state.persist_transaction_micros_max,
            persist_copy_stage_count: state.persist_copy_stage_count,
            persist_copy_stage_micros_total: state.persist_copy_stage_micros_total,
            persist_copy_stage_micros_max: state.persist_copy_stage_micros_max,
            persist_data_blocks_merge_count: state.persist_data_blocks_merge_count,
            persist_data_blocks_merge_micros_total: state.persist_data_blocks_merge_micros_total,
            persist_data_blocks_merge_micros_max: state.persist_data_blocks_merge_micros_max,
            quota_lock_wait_count: state.quota_lock_wait_count,
            quota_lock_wait_micros_total: state.quota_lock_wait_micros_total,
            quota_lock_wait_micros_max: state.quota_lock_wait_micros_max,
            quota_lock_held_count: state.quota_lock_held_count,
            quota_lock_held_micros_total: state.quota_lock_held_micros_total,
            quota_lock_held_micros_max: state.quota_lock_held_micros_max,
            quota_final_check_count: state.quota_final_check_count,
            quota_final_check_micros_total: state.quota_final_check_micros_total,
            quota_final_check_micros_max: state.quota_final_check_micros_max,
        })
    }
}

impl Drop for DbRepoPayloadBudgetPermit {
    fn drop(&mut self) {
        if !self.acquired {
            return;
        }
        let mut state = self.tracker.lock_budget_state();
        match state.reserved_bytes.checked_sub(self.reserved_bytes) {
            Some(value) => state.reserved_bytes = value,
            None => {
                state.reserved_bytes = 0;
                state.accounting_errors = state.accounting_errors.saturating_add(1);
            }
        }
        let ready = self.tracker.reserve_ready_waiters(&mut state);
        drop(state);
        for waiter in ready {
            waiter.signal();
        }
    }
}

pub const SHARED_MONITOR_STATS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SharedMonitorTaskStats {
    pub admitted_tasks: u64,
    pub completed_tasks: u64,
    pub failed_tasks: u64,
    pub queued_tasks: u64,
    pub active_tasks: u64,
    pub peak_queued_tasks: u64,
    pub peak_active_tasks: u64,
    pub active_transactions: u64,
    pub active_transaction_limit: u64,
    pub payload_in_flight_bytes: u64,
    pub payload_in_flight_limit_bytes: u64,
    pub per_task_buffer_limit_bytes: u64,
    pub backpressure_events: u64,
    pub fairness_yields: u64,
    pub accounting_errors: u64,
    pub completed_files: u64,
    pub completed_bytes: u64,
    pub database_batches: u64,
    pub database_batch_rows: u64,
    pub database_batch_bytes: u64,
    pub elapsed_micros: u64,
}

impl SharedMonitorTaskStats {
    pub fn from_snapshot(snapshot: &LogicalTaskObservabilitySnapshot) -> Self {
        Self {
            admitted_tasks: snapshot.queue.admitted_tasks,
            completed_tasks: snapshot.queue.completed_tasks,
            failed_tasks: snapshot.queue.failed_tasks,
            queued_tasks: snapshot.queue.queued_tasks,
            active_tasks: snapshot.queue.active_tasks,
            peak_queued_tasks: snapshot.queue.peak_queued_tasks,
            peak_active_tasks: snapshot.queue.peak_active_tasks,
            active_transactions: snapshot.queue.active_transactions,
            active_transaction_limit: snapshot.queue.active_transaction_limit,
            payload_in_flight_bytes: snapshot.queue.payload_in_flight_bytes,
            payload_in_flight_limit_bytes: snapshot.queue.payload_in_flight_limit_bytes,
            per_task_buffer_limit_bytes: snapshot.queue.per_task_buffer_limit_bytes,
            backpressure_events: snapshot.queue.backpressure_events,
            fairness_yields: snapshot.queue.fairness_yields,
            accounting_errors: snapshot.queue.accounting_errors,
            completed_files: snapshot.throughput.completed_files,
            completed_bytes: snapshot.throughput.completed_bytes,
            database_batches: snapshot.throughput.database_batches,
            database_batch_rows: snapshot.throughput.database_batch_rows,
            database_batch_bytes: snapshot.throughput.database_batch_bytes,
            elapsed_micros: snapshot.throughput.elapsed_micros,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SharedMonitorDatabaseStats {
    pub pool_scope: String,
    pub connection_limit: u64,
    pub live_connections: u64,
    pub idle_write_connections: u64,
    pub idle_control_connections: u64,
    pub active_connections: u64,
    pub queued_acquisitions: u64,
    pub peak_active_connections: u64,
    pub peak_queued_acquisitions: u64,
    pub acquisition_count: u64,
    pub acquisition_wait_micros_total: u64,
    pub acquisition_wait_micros_max: u64,
    pub connection_create_count: u64,
    pub connection_create_failures: u64,
    pub operation_count: u64,
    pub operation_failures: u64,
    pub operation_micros_total: u64,
    pub operation_micros_max: u64,
    pub replay_count: u64,
    pub stale_connection_discards: u64,
    pub transaction_count: u64,
    pub transaction_failures: u64,
    pub transaction_micros_total: u64,
    pub transaction_micros_max: u64,
    pub heartbeat_count: u64,
    pub heartbeat_failures: u64,
    pub active_authority: String,
    pub failover_count: u64,
    pub connection_failures: u64,
    pub role_rejections: u64,
    pub replica_read_routing_enabled: bool,
    pub replica_active_authority: Option<String>,
    pub replica_reads: u64,
    pub replica_lag_fallbacks: u64,
    pub replica_read_failures: u64,
    pub primary_read_fallbacks: u64,
    pub replica_pool_pressure_fallbacks: u64,
}

impl SharedMonitorDatabaseStats {
    pub fn from_snapshot(snapshot: &DbRepoObservabilitySnapshot) -> Self {
        let pool = &snapshot.pool;
        let routing = &snapshot.routing;
        Self {
            pool_scope: "fuse-repository".to_string(),
            connection_limit: pool.connection_limit as u64,
            live_connections: pool.live_connections as u64,
            idle_write_connections: pool.idle_write_connections as u64,
            idle_control_connections: pool.idle_control_connections as u64,
            active_connections: pool.active_connections as u64,
            queued_acquisitions: pool.queued_acquisitions as u64,
            peak_active_connections: pool.peak_active_connections as u64,
            peak_queued_acquisitions: pool.peak_queued_acquisitions as u64,
            acquisition_count: pool.acquisition_count,
            acquisition_wait_micros_total: pool.acquisition_wait_micros_total,
            acquisition_wait_micros_max: pool.acquisition_wait_micros_max,
            connection_create_count: pool.connection_create_count,
            connection_create_failures: pool.connection_create_failures,
            operation_count: pool.operation_count,
            operation_failures: pool.operation_failures,
            operation_micros_total: pool.operation_micros_total,
            operation_micros_max: pool.operation_micros_max,
            replay_count: pool.replay_count,
            stale_connection_discards: pool.stale_connection_discards,
            transaction_count: pool.transaction_count,
            transaction_failures: pool.transaction_failures,
            transaction_micros_total: pool.transaction_micros_total,
            transaction_micros_max: pool.transaction_micros_max,
            heartbeat_count: pool.heartbeat_count,
            heartbeat_failures: pool.heartbeat_failures,
            active_authority: routing.active_authority.clone(),
            failover_count: routing.failover_count,
            connection_failures: routing.connection_failures,
            role_rejections: routing.role_rejections,
            replica_read_routing_enabled: routing.replica_read_routing_enabled,
            replica_active_authority: routing.replica_active_authority.clone(),
            replica_reads: routing.replica_reads,
            replica_lag_fallbacks: routing.replica_lag_fallbacks,
            replica_read_failures: routing.replica_read_failures,
            primary_read_fallbacks: routing.primary_read_fallbacks,
            replica_pool_pressure_fallbacks: routing.replica_pool_pressure_fallbacks,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SharedMonitorPersistenceStats {
    pub in_flight_bytes: u64,
    pub peak_in_flight_bytes: u64,
    pub in_flight_limit_bytes: u64,
    pub reserved_bytes: u64,
    pub queued_bytes: u64,
    pub peak_reserved_bytes: u64,
    pub peak_queued_bytes: u64,
    pub queued_requests: u64,
    pub peak_queued_requests: u64,
    pub admission_count: u64,
    pub backpressure_events: u64,
    pub fairness_yields: u64,
    pub oversized_admissions: u64,
    pub accounting_errors: u64,
    pub persist_operation_count: u64,
    pub persist_operation_failures: u64,
    pub persist_input_rows_total: u64,
    pub persist_input_rows_max: u64,
    pub persist_input_bytes_total: u64,
    pub persist_input_bytes_max: u64,
    pub persist_micros_total: u64,
    pub persist_micros_max: u64,
    pub persist_transaction_count: u64,
    pub persist_transaction_failures: u64,
    pub persist_transaction_micros_total: u64,
    pub persist_transaction_micros_max: u64,
    pub persist_copy_stage_count: u64,
    pub persist_copy_stage_micros_total: u64,
    pub persist_copy_stage_micros_max: u64,
    pub persist_data_blocks_merge_count: u64,
    pub persist_data_blocks_merge_micros_total: u64,
    pub persist_data_blocks_merge_micros_max: u64,
}

impl SharedMonitorPersistenceStats {
    pub fn from_snapshot(snapshot: &DbRepoObservabilitySnapshot) -> Self {
        let payload = &snapshot.global_payload;
        Self {
            in_flight_bytes: payload.in_flight_bytes,
            peak_in_flight_bytes: payload.peak_in_flight_bytes,
            in_flight_limit_bytes: payload.in_flight_limit_bytes,
            reserved_bytes: payload.reserved_bytes,
            queued_bytes: payload.queued_bytes,
            peak_reserved_bytes: payload.peak_reserved_bytes,
            peak_queued_bytes: payload.peak_queued_bytes,
            queued_requests: payload.queued_requests,
            peak_queued_requests: payload.peak_queued_requests,
            admission_count: payload.admission_count,
            backpressure_events: payload.backpressure_events,
            fairness_yields: payload.fairness_yields,
            oversized_admissions: payload.oversized_admissions,
            accounting_errors: payload
                .accounting_errors
                .saturating_add(payload.budget_accounting_errors),
            persist_operation_count: payload.persist_operation_count,
            persist_operation_failures: payload.persist_operation_failures,
            persist_input_rows_total: payload.persist_input_rows_total,
            persist_input_rows_max: payload.persist_input_rows_max,
            persist_input_bytes_total: payload.persist_input_bytes_total,
            persist_input_bytes_max: payload.persist_input_bytes_max,
            persist_micros_total: payload.persist_micros_total,
            persist_micros_max: payload.persist_micros_max,
            persist_transaction_count: payload.persist_transaction_count,
            persist_transaction_failures: payload.persist_transaction_failures,
            persist_transaction_micros_total: payload.persist_transaction_micros_total,
            persist_transaction_micros_max: payload.persist_transaction_micros_max,
            persist_copy_stage_count: payload.persist_copy_stage_count,
            persist_copy_stage_micros_total: payload.persist_copy_stage_micros_total,
            persist_copy_stage_micros_max: payload.persist_copy_stage_micros_max,
            persist_data_blocks_merge_count: payload.persist_data_blocks_merge_count,
            persist_data_blocks_merge_micros_total: payload.persist_data_blocks_merge_micros_total,
            persist_data_blocks_merge_micros_max: payload.persist_data_blocks_merge_micros_max,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SharedMonitorTimingStats {
    pub fuse_read_total_us: u64,
    pub fuse_write_total_us: u64,
    pub read_block_map_us: u64,
    pub assemble_read_slice_us: u64,
    pub repo_fetch_block_range_us: u64,
    pub repo_persist_blocks_us: u64,
    pub update_write_buffer_us: u64,
    pub flush_write_state_us: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SharedMonitorSourceStats {
    pub data_source_role: String,
    pub data_source_transaction_read_only: bool,
    pub wal_replay_lag_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SharedMonitorSessionStats {
    pub schema_version: u32,
    pub sample_seq: u64,
    pub publish_interval_millis: u64,
    pub source: SharedMonitorSourceStats,
    pub read: SharedMonitorTaskStats,
    pub write: SharedMonitorTaskStats,
    pub copy: SharedMonitorTaskStats,
    pub database: SharedMonitorDatabaseStats,
    pub persistence: SharedMonitorPersistenceStats,
    pub timings: SharedMonitorTimingStats,
}

pub struct SharedMonitorSessionStatsInput<'a> {
    pub sample_seq: u64,
    pub publish_interval_millis: u64,
    pub read: &'a LogicalTaskObservabilitySnapshot,
    pub write: &'a LogicalTaskObservabilitySnapshot,
    pub copy: &'a LogicalTaskObservabilitySnapshot,
    pub database: &'a DbRepoObservabilitySnapshot,
    pub source: SharedMonitorSourceStats,
    pub timings: SharedMonitorTimingStats,
}

impl SharedMonitorSessionStats {
    pub fn from_snapshots(input: SharedMonitorSessionStatsInput<'_>) -> Self {
        Self {
            schema_version: SHARED_MONITOR_STATS_SCHEMA_VERSION,
            sample_seq: input.sample_seq,
            publish_interval_millis: input.publish_interval_millis,
            source: input.source,
            read: SharedMonitorTaskStats::from_snapshot(input.read),
            write: SharedMonitorTaskStats::from_snapshot(input.write),
            copy: SharedMonitorTaskStats::from_snapshot(input.copy),
            database: SharedMonitorDatabaseStats::from_snapshot(input.database),
            persistence: SharedMonitorPersistenceStats::from_snapshot(input.database),
            timings: input.timings,
        }
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self)
            .map_err(|err| format!("unable to serialize shared monitor stats: {err}"))
    }

    pub fn from_json(value: &str) -> Result<Self, String> {
        serde_json::from_str(value)
            .map_err(|err| format!("unable to parse shared monitor stats: {err}"))
    }
}

#[cfg(test)]
mod shared_monitor_tests {
    use super::{SharedMonitorSessionStats, SHARED_MONITOR_STATS_SCHEMA_VERSION};

    #[test]
    fn shared_monitor_json_roundtrip_preserves_counters() {
        let mut stats = SharedMonitorSessionStats {
            schema_version: SHARED_MONITOR_STATS_SCHEMA_VERSION,
            sample_seq: 17,
            publish_interval_millis: 5_000,
            ..SharedMonitorSessionStats::default()
        };
        stats.read.completed_tasks = 11;
        stats.read.completed_bytes = 4096;
        stats.write.completed_tasks = 7;
        stats.write.completed_bytes = 8192;
        stats.database.operation_count = 31;
        stats.persistence.persist_operation_count = 3;
        stats.timings.repo_persist_blocks_us = 1234;
        let encoded = stats.to_json().unwrap();
        let decoded = SharedMonitorSessionStats::from_json(&encoded).unwrap();
        assert_eq!(decoded, stats);
    }

    #[test]
    fn shared_monitor_json_defaults_missing_fields() {
        let decoded = SharedMonitorSessionStats::from_json(
            r#"{"schema_version":1,"sample_seq":4,"read":{"completed_bytes":55}}"#,
        )
        .unwrap();
        assert_eq!(decoded.sample_seq, 4);
        assert_eq!(decoded.publish_interval_millis, 0);
        assert_eq!(decoded.read.completed_bytes, 55);
        assert_eq!(decoded.write.completed_bytes, 0);
    }
}

pub trait LaneObservabilitySource {
    fn observability_snapshot(&self) -> Result<DbRepoObservabilitySnapshot, String>;
    fn postgres_pressure_snapshot(&self) -> Result<PostgresPressureSnapshot, String>;
}

pub struct LaneObservabilitySampler {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LaneObservabilitySampler {
    pub fn spawn(
        repositories: Vec<(&'static str, Arc<dyn LaneObservabilitySource + Send + Sync>)>,
        process_rss_peak: Arc<AtomicU64>,
        interval: Duration,
    ) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("fod-pg-observability".to_string())
            .spawn(move || {
                while !stop_thread.load(Ordering::Relaxed) {
                    thread::park_timeout(interval);
                    if stop_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    log_lane_observability("periodic", &repositories, &process_rss_peak);
                }
            })
            .map_err(|err| format!("failed to spawn PostgreSQL observability thread: {err}"))?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

impl Drop for LaneObservabilitySampler {
    fn drop(&mut self) {
        self.stop();
    }
}

pub const LOGICAL_TASK_OBSERVABILITY_INTERVAL_MS_ENV: &str = "FOD_TASK_OBSERVABILITY_INTERVAL_MS";
const MIN_LOGICAL_TASK_OBSERVABILITY_INTERVAL_MS: u64 = 100;
const MAX_LOGICAL_TASK_OBSERVABILITY_INTERVAL_MS: u64 = 3_600_000;

fn parse_logical_task_observability_interval(value: &str) -> Result<Duration, String> {
    let interval_ms = value.parse::<u64>().map_err(|err| {
        format!(
            "{LOGICAL_TASK_OBSERVABILITY_INTERVAL_MS_ENV} must be an integer number of milliseconds: {err}"
        )
    })?;
    if !(MIN_LOGICAL_TASK_OBSERVABILITY_INTERVAL_MS..=MAX_LOGICAL_TASK_OBSERVABILITY_INTERVAL_MS)
        .contains(&interval_ms)
    {
        return Err(format!(
            "{LOGICAL_TASK_OBSERVABILITY_INTERVAL_MS_ENV} must be between {MIN_LOGICAL_TASK_OBSERVABILITY_INTERVAL_MS} and {MAX_LOGICAL_TASK_OBSERVABILITY_INTERVAL_MS} milliseconds"
        ));
    }
    Ok(Duration::from_millis(interval_ms))
}

fn logical_task_observability_interval(default_interval: Duration) -> Result<Duration, String> {
    match std::env::var(LOGICAL_TASK_OBSERVABILITY_INTERVAL_MS_ENV) {
        Ok(value) => parse_logical_task_observability_interval(&value),
        Err(std::env::VarError::NotPresent) => {
            if default_interval.is_zero() {
                Err("logical task observability interval must be greater than zero".to_string())
            } else {
                Ok(default_interval)
            }
        }
        Err(std::env::VarError::NotUnicode(_)) => Err(format!(
            "{LOGICAL_TASK_OBSERVABILITY_INTERVAL_MS_ENV} must contain valid UTF-8"
        )),
    }
}

pub struct LogicalTaskObservabilitySampler {
    stop: Arc<AtomicBool>,
    trackers: Arc<Vec<Arc<LogicalTaskQueueObservability>>>,
    thread: Option<JoinHandle<()>>,
}

impl LogicalTaskObservabilitySampler {
    pub fn spawn(
        trackers: Vec<Arc<LogicalTaskQueueObservability>>,
        default_interval: Duration,
    ) -> Result<Self, String> {
        let interval = logical_task_observability_interval(default_interval)?;
        let trackers = Arc::new(trackers);
        let trackers_thread = Arc::clone(&trackers);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        log::info!(
            "FOD logical task observability sampler: interval_ms={} env={}",
            interval.as_millis(),
            LOGICAL_TASK_OBSERVABILITY_INTERVAL_MS_ENV
        );
        let thread = thread::Builder::new()
            .name("fod-task-observability".to_string())
            .spawn(move || {
                while !stop_thread.load(Ordering::Relaxed) {
                    thread::park_timeout(interval);
                    if stop_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    log_logical_task_observability("periodic", trackers_thread.as_slice());
                }
            })
            .map_err(|err| format!("failed to spawn logical task observability thread: {err}"))?;
        Ok(Self {
            stop,
            trackers,
            thread: Some(thread),
        })
    }

    pub fn sample_now(&self) {
        log_logical_task_observability("manual", self.trackers.as_slice());
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

impl Drop for LogicalTaskObservabilitySampler {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn log_logical_task_observability(
    stage: &str,
    trackers: &[Arc<LogicalTaskQueueObservability>],
) {
    for tracker in trackers {
        match tracker.snapshot() {
            Ok(snapshot) if snapshot.queue.admitted_tasks > 0 => {
                let queue = snapshot.queue;
                let throughput = snapshot.throughput;
                log::info!(
                    "FOD logical task observability: stage={} lane={} operation={} admitted_tasks={} completed_tasks={} failed_tasks={} queued_tasks={} active_tasks={} peak_queued_tasks={} peak_active_tasks={} active_transactions={} payload_in_flight_bytes={} completed_files={} completed_bytes={} completed_bytes_per_second={} database_batches={} database_batch_rows={} database_batch_bytes={} elapsed_micros={} accounting_errors={}",
                    stage,
                    queue.class.lane.as_str(),
                    queue.class.operation.as_str(),
                    queue.admitted_tasks,
                    queue.completed_tasks,
                    queue.failed_tasks,
                    queue.queued_tasks,
                    queue.active_tasks,
                    queue.peak_queued_tasks,
                    queue.peak_active_tasks,
                    queue.active_transactions,
                    queue.payload_in_flight_bytes,
                    throughput.completed_files,
                    throughput.completed_bytes,
                    throughput.completed_bytes_per_second(),
                    throughput.database_batches,
                    throughput.database_batch_rows,
                    throughput.database_batch_bytes,
                    throughput.elapsed_micros,
                    queue.accounting_errors,
                );
            }
            Err(err) => {
                log::warn!(
                    "FOD logical task observability unavailable: stage={} error={}",
                    stage,
                    err
                );
            }
            _ => {}
        }
    }
}

pub fn log_lane_observability(
    stage: &str,
    repositories: &[(&str, Arc<dyn LaneObservabilitySource + Send + Sync>)],
    process_rss_peak: &AtomicU64,
) {
    match current_process_rss_bytes() {
        Ok(process_rss_bytes) => {
            let previous_peak = process_rss_peak.fetch_max(process_rss_bytes, Ordering::Relaxed);
            log::info!(
                "FOD PostgreSQL lane process observability: stage={} process_rss_bytes={} process_rss_peak_bytes={}",
                stage,
                process_rss_bytes,
                previous_peak.max(process_rss_bytes)
            );
        }
        Err(err) => log::warn!(
            "FOD PostgreSQL lane process observability unavailable: stage={} error={}",
            stage,
            err
        ),
    }

    let pressure_repository = repositories
        .iter()
        .find(|(lane, _)| *lane == "control")
        .or_else(|| repositories.first());
    if let Some((lane, repo)) = pressure_repository {
        match repo.postgres_pressure_snapshot() {
            Ok(pressure) => log::info!(
                "FOD PostgreSQL pressure observability: stage={} source_lane={} database_connections={} activity_connections={} activity_active={} activity_idle={} activity_idle_in_transaction={} temp_files={} temp_bytes={} deadlocks={} shared_buffers={} work_mem={} maintenance_work_mem={} temp_buffers={} current_backend_memory_bytes={:?}",
                stage,
                lane,
                pressure.database_connections,
                pressure.activity_connections,
                pressure.activity_active,
                pressure.activity_idle,
                pressure.activity_idle_in_transaction,
                pressure.temp_files,
                pressure.temp_bytes,
                pressure.deadlocks,
                pressure.shared_buffers,
                pressure.work_mem,
                pressure.maintenance_work_mem,
                pressure.temp_buffers,
                pressure.current_backend_memory_bytes,
            ),
            Err(err) => log::warn!(
                "FOD PostgreSQL pressure observability unavailable: stage={} source_lane={} error={}",
                stage,
                lane,
                err
            ),
        }
    }

    let mut global_payload_logged = false;
    for (lane, repo) in repositories {
        match repo.observability_snapshot() {
            Ok(snapshot) => {
                let pool = snapshot.pool;
                let routing = snapshot.routing;
                let payload = snapshot.payload;
                log::info!(
                    "FOD PostgreSQL lane observability: stage={} lane={} connection_limit={} live_connections={} idle_connections={} idle_write_connections={} idle_control_connections={} active_connections={} queued_acquisitions={} peak_active_connections={} peak_queued_acquisitions={} acquisition_count={} acquisition_wait_micros_total={} acquisition_wait_micros_max={} connection_create_count={} connection_create_failures={} connection_create_micros_total={} connection_create_micros_max={} operation_count={} operation_failures={} operation_micros_total={} operation_micros_max={} replay_count={} stale_connection_discards={} transaction_count={} transaction_failures={} transaction_micros_total={} transaction_micros_max={} write_transaction_limit={} write_active_transactions={} write_queued_transactions={} write_peak_active_transactions={} write_peak_queued_transactions={} write_transaction_admission_count={} write_transaction_backpressure_events={} write_transaction_fairness_yields={} write_transaction_accounting_errors={} control_transaction_limit={} control_active_transactions={} control_queued_transactions={} control_peak_active_transactions={} control_peak_queued_transactions={} control_transaction_admission_count={} control_transaction_backpressure_events={} control_transaction_fairness_yields={} control_transaction_accounting_errors={} heartbeat_count={} heartbeat_failures={} heartbeat_schedule_delay_micros_total={} heartbeat_schedule_delay_micros_max={} heartbeat_execution_micros_total={} heartbeat_execution_micros_max={} payload_in_flight_bytes={} payload_peak_in_flight_bytes={} payload_accounting_errors={} persist_operation_count={} persist_operation_failures={} persist_input_rows_total={} persist_input_rows_max={} persist_input_bytes_total={} persist_input_bytes_max={} persist_micros_total={} persist_micros_max={} persist_transaction_count={} persist_transaction_failures={} persist_transaction_micros_total={} persist_transaction_micros_max={} persist_copy_stage_count={} persist_copy_stage_micros_total={} persist_copy_stage_micros_max={} persist_data_blocks_merge_count={} persist_data_blocks_merge_micros_total={} persist_data_blocks_merge_micros_max={} quota_lock_wait_count={} quota_lock_wait_micros_total={} quota_lock_wait_micros_max={} quota_lock_held_count={} quota_lock_held_micros_total={} quota_lock_held_micros_max={} quota_final_check_count={} quota_final_check_micros_total={} quota_final_check_micros_max={} persist_buffer_chunk_blocks={} persist_copy_send_buffer_bytes={} routing_enabled={} runtime_failover_enabled={} routing_target_count={} routing_active_authority={} routing_generation={} runtime_failover_count={} routing_connection_failures={} routing_role_rejections={} routing_last_failed_authority={} replica_read_routing_enabled={} replica_target_count={} replica_active_authority={} required_primary_wal_lsn={} primary_wal_lsn_updates={} primary_wal_lsn_capture_failures={} replica_consistency_checks={} replica_consistency_passes={} replica_reads={} replica_lag_fallbacks={} replica_read_failures={} primary_read_fallbacks={} replica_scoring_enabled={} replica_active_score={:?} replica_active_replay_lag_bytes={:?} replica_active_connection_latency_micros={:?} replica_active_operation_latency_micros={:?} replica_score_selections={} replica_score_switches={} replica_hysteresis_keeps={} replica_circuit_breaker_skips={} replica_circuit_open_targets={} replica_pool_pressure_fallbacks={} primary_promotion_guard_enabled={} primary_system_identifier={} primary_server_fingerprint={} primary_guard_scans={} primary_guard_unreachable_candidates={} primary_guard_role_rejections={} primary_guard_cluster_identity_rejections={} primary_guard_split_brain_rejections={} primary_guard_no_primary_rejections={} primary_guard_last_error={}",
                    stage,
                    lane,
                    pool.connection_limit,
                    pool.live_connections,
                    pool.idle_connections(),
                    pool.idle_write_connections,
                    pool.idle_control_connections,
                    pool.active_connections,
                    pool.queued_acquisitions,
                    pool.peak_active_connections,
                    pool.peak_queued_acquisitions,
                    pool.acquisition_count,
                    pool.acquisition_wait_micros_total,
                    pool.acquisition_wait_micros_max,
                    pool.connection_create_count,
                    pool.connection_create_failures,
                    pool.connection_create_micros_total,
                    pool.connection_create_micros_max,
                    pool.operation_count,
                    pool.operation_failures,
                    pool.operation_micros_total,
                    pool.operation_micros_max,
                    pool.replay_count,
                    pool.stale_connection_discards,
                    pool.transaction_count,
                    pool.transaction_failures,
                    pool.transaction_micros_total,
                    pool.transaction_micros_max,
                    pool.write_transaction_admission.limit,
                    pool.write_transaction_admission.active,
                    pool.write_transaction_admission.queued,
                    pool.write_transaction_admission.peak_active,
                    pool.write_transaction_admission.peak_queued,
                    pool.write_transaction_admission.admission_count,
                    pool.write_transaction_admission.backpressure_events,
                    pool.write_transaction_admission.fairness_yields,
                    pool.write_transaction_admission.accounting_errors,
                    pool.control_transaction_admission.limit,
                    pool.control_transaction_admission.active,
                    pool.control_transaction_admission.queued,
                    pool.control_transaction_admission.peak_active,
                    pool.control_transaction_admission.peak_queued,
                    pool.control_transaction_admission.admission_count,
                    pool.control_transaction_admission.backpressure_events,
                    pool.control_transaction_admission.fairness_yields,
                    pool.control_transaction_admission.accounting_errors,
                    pool.heartbeat_count,
                    pool.heartbeat_failures,
                    pool.heartbeat_schedule_delay_micros_total,
                    pool.heartbeat_schedule_delay_micros_max,
                    pool.heartbeat_execution_micros_total,
                    pool.heartbeat_execution_micros_max,
                    payload.in_flight_bytes,
                    payload.peak_in_flight_bytes,
                    payload.accounting_errors,
                    payload.persist_operation_count,
                    payload.persist_operation_failures,
                    payload.persist_input_rows_total,
                    payload.persist_input_rows_max,
                    payload.persist_input_bytes_total,
                    payload.persist_input_bytes_max,
                    payload.persist_micros_total,
                    payload.persist_micros_max,
                    payload.persist_transaction_count,
                    payload.persist_transaction_failures,
                    payload.persist_transaction_micros_total,
                    payload.persist_transaction_micros_max,
                    payload.persist_copy_stage_count,
                    payload.persist_copy_stage_micros_total,
                    payload.persist_copy_stage_micros_max,
                    payload.persist_data_blocks_merge_count,
                    payload.persist_data_blocks_merge_micros_total,
                    payload.persist_data_blocks_merge_micros_max,
                    payload.quota_lock_wait_count,
                    payload.quota_lock_wait_micros_total,
                    payload.quota_lock_wait_micros_max,
                    payload.quota_lock_held_count,
                    payload.quota_lock_held_micros_total,
                    payload.quota_lock_held_micros_max,
                    payload.quota_final_check_count,
                    payload.quota_final_check_micros_total,
                    payload.quota_final_check_micros_max,
                    snapshot.persist_buffer_chunk_blocks,
                    snapshot.persist_copy_send_buffer_bytes,
                    routing.endpoint_routing_enabled,
                    routing.runtime_failover_enabled,
                    routing.target_count,
                    routing.active_authority.as_str(),
                    routing.generation,
                    routing.failover_count,
                    routing.connection_failures,
                    routing.role_rejections,
                    routing.last_failed_authority.as_deref().unwrap_or("none"),
                    routing.replica_read_routing_enabled,
                    routing.replica_target_count,
                    routing.replica_active_authority.as_deref().unwrap_or("none"),
                    routing.required_primary_wal_lsn.as_deref().unwrap_or("none"),
                    routing.primary_wal_lsn_updates,
                    routing.primary_wal_lsn_capture_failures,
                    routing.replica_consistency_checks,
                    routing.replica_consistency_passes,
                    routing.replica_reads,
                    routing.replica_lag_fallbacks,
                    routing.replica_read_failures,
                    routing.primary_read_fallbacks,
                    routing.replica_scoring_enabled,
                    routing.replica_active_score,
                    routing.replica_active_replay_lag_bytes,
                    routing.replica_active_connection_latency_micros,
                    routing.replica_active_operation_latency_micros,
                    routing.replica_score_selections,
                    routing.replica_score_switches,
                    routing.replica_hysteresis_keeps,
                    routing.replica_circuit_breaker_skips,
                    routing.replica_circuit_open_targets,
                    routing.replica_pool_pressure_fallbacks,
                    routing.primary_promotion_guard_enabled,
                    routing.primary_system_identifier.as_deref().unwrap_or("none"),
                    routing.primary_server_fingerprint.as_deref().unwrap_or("none"),
                    routing.primary_guard_scans,
                    routing.primary_guard_unreachable_candidates,
                    routing.primary_guard_role_rejections,
                    routing.primary_guard_cluster_identity_rejections,
                    routing.primary_guard_split_brain_rejections,
                    routing.primary_guard_no_primary_rejections,
                    routing.primary_guard_last_error.as_deref().unwrap_or("none"),
                );
                if !global_payload_logged {
                    let global_payload = snapshot.global_payload;
                    log::info!(
                        "FOD PostgreSQL global payload observability: stage={} payload_in_flight_bytes={} payload_peak_in_flight_bytes={} payload_in_flight_limit_bytes={} payload_reserved_bytes={} payload_queued_bytes={} payload_peak_reserved_bytes={} payload_peak_queued_bytes={} payload_queued_requests={} payload_peak_queued_requests={} payload_admission_count={} payload_backpressure_events={} payload_fairness_yields={} payload_oversized_admissions={} payload_budget_accounting_errors={} payload_accounting_errors={} persist_operation_count={} persist_operation_failures={} persist_input_rows_total={} persist_input_rows_max={} persist_input_bytes_total={} persist_input_bytes_max={} persist_micros_total={} persist_micros_max={} persist_transaction_count={} persist_transaction_failures={} persist_transaction_micros_total={} persist_transaction_micros_max={} persist_copy_stage_count={} persist_copy_stage_micros_total={} persist_copy_stage_micros_max={} persist_data_blocks_merge_count={} persist_data_blocks_merge_micros_total={} persist_data_blocks_merge_micros_max={} quota_lock_wait_count={} quota_lock_wait_micros_total={} quota_lock_wait_micros_max={} quota_lock_held_count={} quota_lock_held_micros_total={} quota_lock_held_micros_max={} quota_final_check_count={} quota_final_check_micros_total={} quota_final_check_micros_max={}",
                        stage,
                        global_payload.in_flight_bytes,
                        global_payload.peak_in_flight_bytes,
                        global_payload.in_flight_limit_bytes,
                        global_payload.reserved_bytes,
                        global_payload.queued_bytes,
                        global_payload.peak_reserved_bytes,
                        global_payload.peak_queued_bytes,
                        global_payload.queued_requests,
                        global_payload.peak_queued_requests,
                        global_payload.admission_count,
                        global_payload.backpressure_events,
                        global_payload.fairness_yields,
                        global_payload.oversized_admissions,
                        global_payload.budget_accounting_errors,
                        global_payload.accounting_errors,
                        global_payload.persist_operation_count,
                        global_payload.persist_operation_failures,
                        global_payload.persist_input_rows_total,
                        global_payload.persist_input_rows_max,
                        global_payload.persist_input_bytes_total,
                        global_payload.persist_input_bytes_max,
                        global_payload.persist_micros_total,
                        global_payload.persist_micros_max,
                        global_payload.persist_transaction_count,
                        global_payload.persist_transaction_failures,
                        global_payload.persist_transaction_micros_total,
                        global_payload.persist_transaction_micros_max,
                        global_payload.persist_copy_stage_count,
                        global_payload.persist_copy_stage_micros_total,
                        global_payload.persist_copy_stage_micros_max,
                        global_payload.persist_data_blocks_merge_count,
                        global_payload.persist_data_blocks_merge_micros_total,
                        global_payload.persist_data_blocks_merge_micros_max,
                        global_payload.quota_lock_wait_count,
                        global_payload.quota_lock_wait_micros_total,
                        global_payload.quota_lock_wait_micros_max,
                        global_payload.quota_lock_held_count,
                        global_payload.quota_lock_held_micros_total,
                        global_payload.quota_lock_held_micros_max,
                        global_payload.quota_final_check_count,
                        global_payload.quota_final_check_micros_total,
                        global_payload.quota_final_check_micros_max,
                    );
                    global_payload_logged = true;
                }
            }
            Err(err) => log::warn!(
                "FOD PostgreSQL lane observability unavailable: stage={} lane={} error={}",
                stage,
                lane,
                err
            ),
        }
    }
}

pub fn current_process_rss_bytes() -> Result<u64, String> {
    let status = std::fs::read_to_string("/proc/self/status")
        .map_err(|err| format!("unable to read /proc/self/status: {err}"))?;
    let line = status
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .ok_or_else(|| "VmRSS is missing from /proc/self/status".to_string())?;
    let mut fields = line.split_whitespace();
    let _label = fields.next();
    let kib = fields
        .next()
        .ok_or_else(|| "VmRSS value is missing".to_string())?
        .parse::<u64>()
        .map_err(|err| format!("invalid VmRSS value: {err}"))?;
    let unit = fields
        .next()
        .ok_or_else(|| "VmRSS unit is missing".to_string())?;
    if unit != "kB" {
        return Err(format!("unsupported VmRSS unit: {unit}"));
    }
    kib.checked_mul(1024)
        .ok_or_else(|| "VmRSS byte value overflowed".to_string())
}

#[cfg(test)]
mod payload_budget_tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn payload_budget_blocks_until_reserved_bytes_are_released() {
        let tracker = Arc::new(DbRepoPayloadObservability::with_in_flight_limit_bytes(10));
        let first = tracker.acquire_payload_budget(7);

        let (acquired_tx, acquired_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker_tracker = Arc::clone(&tracker);
        let worker = std::thread::spawn(move || {
            let _permit = worker_tracker.acquire_payload_budget(6);
            acquired_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let snapshot = tracker.snapshot().unwrap();
            if snapshot.queued_requests >= 1 && snapshot.queued_bytes >= 6 {
                assert_eq!(snapshot.reserved_bytes, 7);
                assert!(snapshot.backpressure_events >= 1);
                break;
            }
            assert!(Instant::now() < deadline, "payload request did not queue");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(acquired_rx.try_recv().is_err());

        drop(first);
        acquired_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("queued payload request was not admitted");
        assert_eq!(tracker.snapshot().unwrap().reserved_bytes, 6);

        release_tx.send(()).unwrap();
        worker.join().unwrap();

        let final_snapshot = tracker.snapshot().unwrap();
        assert_eq!(final_snapshot.reserved_bytes, 0);
        assert_eq!(final_snapshot.queued_bytes, 0);
        assert_eq!(final_snapshot.queued_requests, 0);
        assert_eq!(final_snapshot.admission_count, 2);
        assert_eq!(final_snapshot.budget_accounting_errors, 0);
    }

    #[test]
    fn oversized_payload_is_admitted_alone_to_avoid_deadlock() {
        let tracker = Arc::new(DbRepoPayloadObservability::with_in_flight_limit_bytes(10));
        let permit = tracker.acquire_payload_budget(15);
        let snapshot = tracker.snapshot().unwrap();
        assert_eq!(snapshot.reserved_bytes, 15);
        assert_eq!(snapshot.oversized_admissions, 1);
        assert_eq!(snapshot.admission_count, 1);
        drop(permit);
        assert_eq!(tracker.snapshot().unwrap().reserved_bytes, 0);
    }

    #[test]
    fn payload_persist_quota_timings_are_reported() {
        let tracker = DbRepoPayloadObservability::default();

        tracker.record_persist_transaction(Duration::from_micros(100), false);
        tracker.record_persist_transaction(Duration::from_micros(250), true);
        tracker.record_persist_copy_stage(Duration::from_micros(30));
        tracker.record_persist_data_blocks_merge(Duration::from_micros(40));
        tracker.record_quota_lock_wait(Duration::from_micros(50));
        tracker.record_quota_lock_held(Duration::from_micros(60));
        tracker.record_quota_final_check(Duration::from_micros(70));

        let snapshot = tracker.snapshot().unwrap();
        assert_eq!(snapshot.persist_transaction_count, 2);
        assert_eq!(snapshot.persist_transaction_failures, 1);
        assert_eq!(snapshot.persist_transaction_micros_total, 350);
        assert_eq!(snapshot.persist_transaction_micros_max, 250);
        assert_eq!(snapshot.persist_copy_stage_count, 1);
        assert_eq!(snapshot.persist_copy_stage_micros_total, 30);
        assert_eq!(snapshot.persist_copy_stage_micros_max, 30);
        assert_eq!(snapshot.persist_data_blocks_merge_count, 1);
        assert_eq!(snapshot.persist_data_blocks_merge_micros_total, 40);
        assert_eq!(snapshot.persist_data_blocks_merge_micros_max, 40);
        assert_eq!(snapshot.quota_lock_wait_count, 1);
        assert_eq!(snapshot.quota_lock_wait_micros_total, 50);
        assert_eq!(snapshot.quota_lock_wait_micros_max, 50);
        assert_eq!(snapshot.quota_lock_held_count, 1);
        assert_eq!(snapshot.quota_lock_held_micros_total, 60);
        assert_eq!(snapshot.quota_lock_held_micros_max, 60);
        assert_eq!(snapshot.quota_final_check_count, 1);
        assert_eq!(snapshot.quota_final_check_micros_total, 70);
        assert_eq!(snapshot.quota_final_check_micros_max, 70);
        assert_eq!(snapshot.accounting_errors, 0);
    }
}

#[cfg(test)]
mod logical_task_interval_tests {
    use super::*;

    #[test]
    fn parses_valid_logical_task_interval() {
        assert_eq!(
            parse_logical_task_observability_interval("250").unwrap(),
            Duration::from_millis(250)
        );
    }

    #[test]
    fn rejects_logical_task_interval_outside_bounds() {
        assert!(parse_logical_task_observability_interval("99").is_err());
        assert!(parse_logical_task_observability_interval("3600001").is_err());
    }

    #[test]
    fn rejects_non_numeric_logical_task_interval() {
        assert!(parse_logical_task_observability_interval("thirty").is_err());
    }
}

#[cfg(test)]
mod logical_task_admission_tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread::JoinHandle;
    use std::time::Instant;

    const TEST_WAITERS: usize = 8;
    const BENCHMARK_WAITERS: usize = 500;
    const WORKER_STACK_BYTES: usize = 128 * 1024;

    fn tracker() -> Arc<LogicalTaskQueueObservability> {
        Arc::new(LogicalTaskQueueObservability::new(
            LogicalTaskClass::new(LogicalTaskLane::Write, LogicalTaskOperation::FileWrite),
            0,
            0,
            4096,
        ))
    }

    fn wait_for_next_ticket(gate: &LogicalTaskAdmissionGate, expected: u64) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let next_ticket = gate
                .state
                .lock()
                .expect("logical task admission state")
                .next_ticket;
            if next_ticket == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for next_ticket={expected}; observed {next_ticket}"
            );
            thread::yield_now();
        }
    }

    fn spawn_waiter(
        id: usize,
        gate: Arc<LogicalTaskAdmissionGate>,
        tracker: Arc<LogicalTaskQueueObservability>,
        admitted: mpsc::Sender<usize>,
    ) -> JoinHandle<()> {
        thread::Builder::new()
            .name(format!("fod-fifo-waiter-{id}"))
            .stack_size(WORKER_STACK_BYTES)
            .spawn(move || {
                let (permit, observation) = gate.observe_task(&tracker, 1, false);
                admitted.send(id).expect("send admitted waiter id");
                observation.complete(0, 1);
                drop(permit);
            })
            .expect("spawn FIFO waiter")
    }

    fn queue_waiters(
        count: usize,
        gate: &Arc<LogicalTaskAdmissionGate>,
        tracker: &Arc<LogicalTaskQueueObservability>,
        admitted: &mpsc::Sender<usize>,
    ) -> Vec<JoinHandle<()>> {
        let mut workers = Vec::with_capacity(count);
        for id in 0..count {
            workers.push(spawn_waiter(
                id,
                Arc::clone(gate),
                Arc::clone(tracker),
                admitted.clone(),
            ));
            wait_for_next_ticket(gate, u64::try_from(id + 2).unwrap());
        }
        workers
    }

    fn receive_fifo_order(count: usize, admitted: &mpsc::Receiver<usize>, timeout: Duration) {
        for expected in 0..count {
            let observed = admitted
                .recv_timeout(timeout)
                .expect("FIFO waiter did not acquire within timeout");
            assert_eq!(observed, expected);
        }
    }

    #[test]
    fn fifo_admission_preserves_ticket_order_and_balances_counters() {
        let gate = Arc::new(LogicalTaskAdmissionGate::new(1));
        let tracker = tracker();
        let (admitted_tx, admitted_rx) = mpsc::channel();

        let (initial_permit, initial_observation) = gate.observe_task(&tracker, 1, false);
        let workers = queue_waiters(TEST_WAITERS, &gate, &tracker, &admitted_tx);
        drop(admitted_tx);

        let queued = tracker.queue_snapshot().unwrap();
        assert_eq!(queued.admitted_tasks, (TEST_WAITERS + 1) as u64);
        assert_eq!(queued.queued_tasks, TEST_WAITERS as u64);
        assert_eq!(queued.active_tasks, 1);
        assert_eq!(queued.peak_queued_tasks, TEST_WAITERS as u64);
        assert_eq!(queued.peak_active_tasks, 1);

        initial_observation.complete(0, 1);
        drop(initial_permit);

        receive_fifo_order(TEST_WAITERS, &admitted_rx, Duration::from_secs(2));
        for worker in workers {
            worker.join().expect("FIFO waiter thread");
        }

        let finished = tracker.queue_snapshot().unwrap();
        assert_eq!(finished.admitted_tasks, (TEST_WAITERS + 1) as u64);
        assert_eq!(finished.completed_tasks, (TEST_WAITERS + 1) as u64);
        assert_eq!(finished.failed_tasks, 0);
        assert_eq!(finished.queued_tasks, 0);
        assert_eq!(finished.active_tasks, 0);
        assert_eq!(finished.backpressure_events, TEST_WAITERS as u64);
        assert_eq!(
            finished.fairness_yields,
            TEST_WAITERS.saturating_sub(1) as u64
        );
        assert_eq!(finished.accounting_errors, 0);
    }

    #[test]
    #[ignore = "diagnostic 500-waiter targeted FIFO wake benchmark"]
    fn fifo_targeted_wake_benchmark_500_waiters() {
        let gate = Arc::new(LogicalTaskAdmissionGate::new(1));
        let tracker = tracker();
        let (admitted_tx, admitted_rx) = mpsc::channel();

        let (initial_permit, initial_observation) = gate.observe_task(&tracker, 1, false);
        let workers = queue_waiters(BENCHMARK_WAITERS, &gate, &tracker, &admitted_tx);
        drop(admitted_tx);

        let queued = tracker.queue_snapshot().unwrap();
        assert_eq!(queued.queued_tasks, BENCHMARK_WAITERS as u64);
        assert_eq!(queued.peak_queued_tasks, BENCHMARK_WAITERS as u64);

        let started = Instant::now();
        initial_observation.complete(0, 1);
        drop(initial_permit);

        receive_fifo_order(BENCHMARK_WAITERS, &admitted_rx, Duration::from_secs(10));
        for worker in workers {
            worker.join().expect("FIFO benchmark waiter thread");
        }
        let elapsed = started.elapsed();

        let elapsed_nanos = elapsed.as_nanos();
        let nanos_per_waiter = elapsed_nanos / BENCHMARK_WAITERS as u128;
        eprintln!(
            "FOD FIFO targeted-wake benchmark: waiters={} elapsed_us={} nanos_per_waiter={}",
            BENCHMARK_WAITERS,
            elapsed.as_micros(),
            nanos_per_waiter
        );

        let finished = tracker.queue_snapshot().unwrap();
        assert_eq!(finished.completed_tasks, (BENCHMARK_WAITERS + 1) as u64);
        assert_eq!(finished.queued_tasks, 0);
        assert_eq!(finished.active_tasks, 0);
        assert_eq!(finished.backpressure_events, BENCHMARK_WAITERS as u64);
        assert_eq!(
            finished.fairness_yields,
            BENCHMARK_WAITERS.saturating_sub(1) as u64
        );
        assert_eq!(finished.accounting_errors, 0);
    }
}
