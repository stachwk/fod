// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
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

    pub fn queue_snapshot(&self) -> Result<LogicalTaskQueueSnapshot, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "logical task queue observability state is poisoned".to_string())?;
        Ok(LogicalTaskQueueSnapshot {
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
        })
    }

    pub fn throughput_snapshot(&self) -> Result<LogicalTaskThroughputSnapshot, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "logical task queue observability state is poisoned".to_string())?;
        Ok(LogicalTaskThroughputSnapshot {
            class: self.class,
            completed_files: state.completed_files,
            completed_bytes: state.completed_bytes,
            database_batches: state.database_batches,
            database_batch_rows: state.database_batch_rows,
            database_batch_bytes: state.database_batch_bytes,
            elapsed_micros: state.elapsed_micros,
        })
    }
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
    pub transaction_count: u64,
    pub transaction_failures: u64,
    pub transaction_micros_total: u64,
    pub transaction_micros_max: u64,
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
pub struct DbRepoObservabilitySnapshot {
    pub pool: DbRepoPoolObservabilitySnapshot,
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
    pub accounting_errors: u64,
    pub persist_operation_count: u64,
    pub persist_operation_failures: u64,
    pub persist_input_rows_total: u64,
    pub persist_input_rows_max: u64,
    pub persist_input_bytes_total: u64,
    pub persist_input_bytes_max: u64,
    pub persist_micros_total: u64,
    pub persist_micros_max: u64,
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
}

#[derive(Debug, Default)]
pub struct DbRepoPayloadObservability {
    state: Mutex<DbRepoPayloadObservabilityState>,
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

    pub fn snapshot(&self) -> Result<DbRepoPayloadObservabilitySnapshot, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "payload observability state is poisoned".to_string())?;
        Ok(DbRepoPayloadObservabilitySnapshot {
            in_flight_bytes: state.in_flight_bytes,
            peak_in_flight_bytes: state.peak_in_flight_bytes,
            accounting_errors: state.accounting_errors,
            persist_operation_count: state.persist_operation_count,
            persist_operation_failures: state.persist_operation_failures,
            persist_input_rows_total: state.persist_input_rows_total,
            persist_input_rows_max: state.persist_input_rows_max,
            persist_input_bytes_total: state.persist_input_bytes_total,
            persist_input_bytes_max: state.persist_input_bytes_max,
            persist_micros_total: state.persist_micros_total,
            persist_micros_max: state.persist_micros_max,
        })
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

pub struct LogicalTaskObservabilitySampler {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LogicalTaskObservabilitySampler {
    pub fn spawn(
        trackers: Vec<Arc<LogicalTaskQueueObservability>>,
        interval: Duration,
    ) -> Result<Self, String> {
        if interval.is_zero() {
            return Err(
                "logical task observability interval must be greater than zero".to_string(),
            );
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("fod-task-observability".to_string())
            .spawn(move || {
                while !stop_thread.load(Ordering::Relaxed) {
                    thread::park_timeout(interval);
                    if stop_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    log_logical_task_observability("periodic", &trackers);
                }
            })
            .map_err(|err| format!("failed to spawn logical task observability thread: {err}"))?;
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
        match (tracker.queue_snapshot(), tracker.throughput_snapshot()) {
            (Ok(queue), Ok(throughput)) if queue.admitted_tasks > 0 => {
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
            (Err(err), _) | (_, Err(err)) => {
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
                let payload = snapshot.payload;
                log::info!(
                    "FOD PostgreSQL lane observability: stage={} lane={} connection_limit={} live_connections={} idle_connections={} idle_write_connections={} idle_control_connections={} active_connections={} queued_acquisitions={} peak_active_connections={} peak_queued_acquisitions={} acquisition_count={} acquisition_wait_micros_total={} acquisition_wait_micros_max={} connection_create_count={} connection_create_failures={} connection_create_micros_total={} connection_create_micros_max={} operation_count={} operation_failures={} operation_micros_total={} operation_micros_max={} replay_count={} transaction_count={} transaction_failures={} transaction_micros_total={} transaction_micros_max={} heartbeat_count={} heartbeat_failures={} heartbeat_schedule_delay_micros_total={} heartbeat_schedule_delay_micros_max={} heartbeat_execution_micros_total={} heartbeat_execution_micros_max={} payload_in_flight_bytes={} payload_peak_in_flight_bytes={} payload_accounting_errors={} persist_operation_count={} persist_operation_failures={} persist_input_rows_total={} persist_input_rows_max={} persist_input_bytes_total={} persist_input_bytes_max={} persist_micros_total={} persist_micros_max={} persist_buffer_chunk_blocks={} persist_copy_send_buffer_bytes={} routing_enabled=false",
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
                    pool.transaction_count,
                    pool.transaction_failures,
                    pool.transaction_micros_total,
                    pool.transaction_micros_max,
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
                    snapshot.persist_buffer_chunk_blocks,
                    snapshot.persist_copy_send_buffer_bytes,
                );
                if !global_payload_logged {
                    let global_payload = snapshot.global_payload;
                    log::info!(
                        "FOD PostgreSQL global payload observability: stage={} payload_in_flight_bytes={} payload_peak_in_flight_bytes={} payload_accounting_errors={} persist_operation_count={} persist_operation_failures={} persist_input_rows_total={} persist_input_rows_max={} persist_input_bytes_total={} persist_input_bytes_max={} persist_micros_total={} persist_micros_max={}",
                        stage,
                        global_payload.in_flight_bytes,
                        global_payload.peak_in_flight_bytes,
                        global_payload.accounting_errors,
                        global_payload.persist_operation_count,
                        global_payload.persist_operation_failures,
                        global_payload.persist_input_rows_total,
                        global_payload.persist_input_rows_max,
                        global_payload.persist_input_bytes_total,
                        global_payload.persist_input_bytes_max,
                        global_payload.persist_micros_total,
                        global_payload.persist_micros_max,
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
