// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use fod_rust_monitor::{
    LogicalTaskClass, LogicalTaskLane, LogicalTaskObservabilitySampler, LogicalTaskOperation,
    LogicalTaskQueueObservability,
};
use std::sync::Arc;
use std::time::Duration;

fn tracker() -> Arc<LogicalTaskQueueObservability> {
    Arc::new(LogicalTaskQueueObservability::new(
        LogicalTaskClass::new(LogicalTaskLane::Write, LogicalTaskOperation::FileWrite),
        0,
        0,
        4096,
    ))
}

#[test]
fn completed_observation_balances_active_payload_and_throughput() {
    let tracker = tracker();
    let observation = tracker.observe_task(4096, true);

    let active = tracker.queue_snapshot().unwrap();
    assert_eq!(active.admitted_tasks, 1);
    assert_eq!(active.queued_tasks, 0);
    assert_eq!(active.peak_queued_tasks, 1);
    assert_eq!(active.active_tasks, 1);
    assert_eq!(active.peak_active_tasks, 1);
    assert_eq!(active.active_transactions, 1);
    assert_eq!(active.payload_in_flight_bytes, 4096);

    observation.complete(1, 4096);

    let finished = tracker.queue_snapshot().unwrap();
    assert_eq!(finished.completed_tasks, 1);
    assert_eq!(finished.failed_tasks, 0);
    assert_eq!(finished.active_tasks, 0);
    assert_eq!(finished.active_transactions, 0);
    assert_eq!(finished.payload_in_flight_bytes, 0);
    assert_eq!(finished.accounting_errors, 0);

    let throughput = tracker.throughput_snapshot().unwrap();
    assert_eq!(throughput.completed_files, 1);
    assert_eq!(throughput.completed_bytes, 4096);
}

#[test]
fn dropped_observation_is_recorded_as_failure_and_releases_counters() {
    let tracker = tracker();

    {
        let _observation = tracker.observe_task(2048, false);
        let active = tracker.queue_snapshot().unwrap();
        assert_eq!(active.active_tasks, 1);
        assert_eq!(active.payload_in_flight_bytes, 2048);
    }

    let finished = tracker.queue_snapshot().unwrap();
    assert_eq!(finished.completed_tasks, 1);
    assert_eq!(finished.failed_tasks, 1);
    assert_eq!(finished.active_tasks, 0);
    assert_eq!(finished.payload_in_flight_bytes, 0);
    assert_eq!(finished.accounting_errors, 0);
}

#[test]
fn explicit_failure_is_counted_once() {
    let tracker = tracker();

    tracker.observe_task(512, false).fail();

    let finished = tracker.queue_snapshot().unwrap();
    assert_eq!(finished.completed_tasks, 1);
    assert_eq!(finished.failed_tasks, 1);
    assert_eq!(finished.active_tasks, 0);
    assert_eq!(finished.payload_in_flight_bytes, 0);
}

#[test]
fn logical_task_sampler_starts_reports_and_stops() {
    let tracker = tracker();
    tracker.observe_task(256, false).complete(0, 256);

    let mut sampler =
        LogicalTaskObservabilitySampler::spawn(vec![tracker], Duration::from_millis(1)).unwrap();
    std::thread::sleep(Duration::from_millis(5));
    sampler.stop();
}
