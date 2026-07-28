// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use rust_hotpath::pg::DbRepo;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct LaneObservabilitySampler {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LaneObservabilitySampler {
    pub fn spawn(
        repositories: Vec<(&'static str, DbRepo)>,
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

pub fn log_lane_observability(
    stage: &str,
    repositories: &[(&str, DbRepo)],
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
